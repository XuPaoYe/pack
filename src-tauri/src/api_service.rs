//! SuperAI 本地 API 服务（阶段 2）
//!
//! 我们对外暴露 OpenAI / Anthropic 兼容入口（`/v1/...`），
//! 实际由打包进 Tauri 的 `superai-api` sidecar（bun --compile）+
//! SuperAI runtime 二进制处理推理。
//!
//! 本模块负责：
//! - 起停服务（spawn sidecar 子进程 + tiny_http 反向代理）
//! - 双层鉴权：外层 `Bearer agt_superai_*` 由我们校验，内层 sidecar 用我们生成的 inner key
//! - 状态查询、自动恢复
//!
//! 测试模式（`#[cfg(test)]`）下不 spawn sidecar，只验证 HTTP 服务自身的鉴权与路由占位。

use std::io::{BufRead, BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

/// 默认监听主机：`0.0.0.0` 表示同时监听本机与局域网。
pub const DEFAULT_HOST: &str = "0.0.0.0";
/// 默认固定端口。端口被占用时直接报错，用户可在设置里改端口后重启服务。
pub const DEFAULT_PORT: u16 = 51888;
/// 默认 API Key 前缀；首次启动会生成 `agt_superai_<随机串>`。
pub const API_KEY_PREFIX: &str = "agt_superai_";
const CLAUDE_RECOMMENDED_MODEL: &str = "claude-sonnet-4.6";
const CODEX_RECOMMENDED_MODEL: &str = "gpt-5.3-codex";

/// sidecar 启动后等待 stdout 报告端口的最长时长。
/// sidecar 在打印 "Server on http://..." 之前会先 await
/// startLanguageServer + waitForReady(30s)，因此这里给到 60s 留余量。
const SIDECAR_BOOT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiServiceStatus {
    pub running: bool,
    pub bind_host: String,
    pub bind_port: u16,
    /// 实际监听的端口。
    pub actual_port: Option<u16>,
    /// 拼好的 base URL，例如 `http://127.0.0.1:63721/v1`。
    pub address: Option<String>,
    pub api_key: String,
    pub default_model: String,
    pub last_error: Option<String>,
}

/// 反向代理目标。生产模式下指向我们 spawn 的 sidecar；测试模式为 None。
///
/// `default_model` 用 `Arc<RwLock<_>>` 包起来，是为了让 UI 上切模型 /
/// 调推理强度时能热更：`update_default_model` 写一次，运行中的服务线程
/// （持有的是同一份 Arc 的克隆）下一次请求读到的就是新值，无需重启。
#[derive(Clone)]
struct ProxyTarget {
    base_url: String, // e.g. http://127.0.0.1:39721
    inner_key: String,
    /// 聊天接口统一写入的默认模型；为空表示不注入。
    default_model: Arc<RwLock<String>>,
}

impl ProxyTarget {
    fn default_model_snapshot(&self) -> String {
        self.default_model
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

fn claude_preferred_model(model_id: &str) -> String {
    let trimmed = model_id.trim();
    if trimmed.starts_with("claude-") {
        trimmed.to_string()
    } else {
        CLAUDE_RECOMMENDED_MODEL.to_string()
    }
}

fn codex_preferred_model(_model_id: &str) -> String {
    CODEX_RECOMMENDED_MODEL.to_string()
}

struct Sidecar {
    child: Child,
    ls_bin: PathBuf,
    /// stdout/stderr 读取线程，sidecar 退出后会自然结束。
    _stdout_join: Option<JoinHandle<()>>,
    _stderr_join: Option<JoinHandle<()>>,
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        // 先发 SIGTERM 给 sidecar 一个机会执行自己的 cleanup（含停 LS 子进程）；
        // 没退出再发 SIGKILL 兜底。Windows 没有 SIGTERM，直接 kill。
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();

        // SIGTERM 后给 sidecar 1s 执行自身 cleanup（关 HTTP + kill LS 子进程）；
        // 之前是 2s 偏保守，实测 bun 的 SIGTERM handler 百毫秒级完成，
        // 1s 足够；超时也有后面的 cleanup_language_server_processes 兜底。
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        cleanup_language_server_processes(&self.ls_bin);
    }
}

struct Runtime {
    stop_flag: Arc<AtomicBool>,
    accept_join: Option<JoinHandle<()>>,
    bind_host: String,
    bind_port: u16,
    actual_port: u16,
    api_key: String,
    last_error: Option<String>,
    sidecar: Option<Sidecar>,
    proxy_target: Option<ProxyTarget>,
}

static RUNTIME: LazyLock<Mutex<Option<Runtime>>> = LazyLock::new(|| Mutex::new(None));
static LAST_USED_ACCOUNT_LABEL: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));
/// `sync_api_service_active_account` 上次成功打过"当前"标签的 sidecar label。
/// 命中时直接返回空 vec，跳过 sqlite 解密 + 全表 upsert。前端 setInterval
/// 调到 3s 也几乎零开销。start/stop 时清空，避免跨服务生命周期串号。
static LAST_SYNCED_ACTIVE_LABEL: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_USED_ACCOUNT_PROBE_AT: LazyLock<Mutex<Option<Instant>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_PROBE_PENDING_REFRESH_AT: LazyLock<Mutex<Option<Instant>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_POLICY_BLOCK_LOG: LazyLock<Mutex<Option<(String, Instant)>>> =
    LazyLock::new(|| Mutex::new(None));

fn lock() -> std::sync::MutexGuard<'static, Option<Runtime>> {
    RUNTIME.lock().expect("SuperAI API 运行态锁失败")
}

pub fn last_used_account_label() -> Option<String> {
    LAST_USED_ACCOUNT_LABEL
        .lock()
        .ok()
        .and_then(|label| label.clone())
}

fn set_last_used_account_label(label: String) {
    if let Ok(mut current) = LAST_USED_ACCOUNT_LABEL.lock() {
        *current = Some(label);
    }
}

/// 取上次同步过的 label；命令端用它做幂等短路。
pub fn last_synced_active_label() -> Option<String> {
    LAST_SYNCED_ACTIVE_LABEL
        .lock()
        .ok()
        .and_then(|label| label.clone())
}

/// 标记本轮同步完成的 label。
pub fn record_synced_active_label(label: String) {
    if let Ok(mut current) = LAST_SYNCED_ACTIVE_LABEL.lock() {
        *current = Some(label);
    }
}

/// 服务启停时清掉 active label 缓存，避免新一轮启动后用陈旧值短路。
pub fn clear_synced_active_label() {
    if let Ok(mut current) = LAST_SYNCED_ACTIVE_LABEL.lock() {
        *current = None;
    }
    if let Ok(mut current) = LAST_USED_ACCOUNT_LABEL.lock() {
        *current = None;
    }
    if let Ok(mut current) = LAST_USED_ACCOUNT_PROBE_AT.lock() {
        *current = None;
    }
    if let Ok(mut current) = LAST_PROBE_PENDING_REFRESH_AT.lock() {
        *current = None;
    }
    if let Ok(mut current) = LAST_POLICY_BLOCK_LOG.lock() {
        *current = None;
    }
}

fn should_probe_last_used_account() -> bool {
    let Ok(mut last_probe) = LAST_USED_ACCOUNT_PROBE_AT.lock() else {
        return true;
    };
    let now = Instant::now();
    if last_probe.is_some_and(|instant| now.duration_since(instant) < Duration::from_secs(10)) {
        return false;
    }
    *last_probe = Some(now);
    true
}

fn should_refresh_capabilities_for_probe_pending() -> bool {
    let Ok(mut last_refresh) = LAST_PROBE_PENDING_REFRESH_AT.lock() else {
        return true;
    };
    let now = Instant::now();
    if last_refresh.is_some_and(|instant| now.duration_since(instant) < Duration::from_secs(15)) {
        return false;
    }
    *last_refresh = Some(now);
    true
}

fn should_suppress_policy_block_log(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("stream error after retries:")
        || !lower.contains("content policy")
        || !lower.contains("trace id:")
    {
        return false;
    }

    let normalized = lower
        .split("(trace id:")
        .next()
        .unwrap_or(&lower)
        .trim()
        .to_string();
    let Ok(mut slot) = LAST_POLICY_BLOCK_LOG.lock() else {
        return false;
    };
    let now = Instant::now();
    if let Some((last_text, last_at)) = slot.as_ref() {
        if last_text == &normalized && now.duration_since(*last_at) < Duration::from_secs(20) {
            return true;
        }
    }
    *slot = Some((normalized, now));
    false
}

/// 生成形如 `agt_superai_xxxxxxxxxxxxxxxx` 的密钥。
pub fn generate_api_key() -> String {
    let bytes: [u8; 24] = rand::random();
    let token = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    format!("{API_KEY_PREFIX}{token}")
}

fn deprecated_api_key_prefix() -> String {
    [97, 103, 116, 95, 119, 115, 102, 95]
        .iter()
        .map(|c| char::from(*c))
        .collect()
}

pub fn is_legacy_api_key(key: &str) -> bool {
    key.trim().starts_with(&deprecated_api_key_prefix())
}

fn generate_inner_key() -> String {
    let bytes: [u8; 24] = rand::random();
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

/// 构造对外展示的状态。
pub fn current_status(
    default_host: &str,
    default_port: u16,
    api_key: &str,
    default_model: &str,
) -> ApiServiceStatus {
    let mut guard = lock();
    if sidecar_runtime_unhealthy(guard.as_ref()) {
        let _ = guard.take();
    }

    if let Some(runtime) = guard.as_ref() {
        let address = build_address(&runtime.bind_host, runtime.actual_port);
        let default_model = runtime
            .proxy_target
            .as_ref()
            .map(|target| target.default_model_snapshot())
            .unwrap_or_default();
        ApiServiceStatus {
            running: true,
            bind_host: runtime.bind_host.clone(),
            bind_port: runtime.bind_port,
            actual_port: Some(runtime.actual_port),
            address: Some(address),
            api_key: runtime.api_key.clone(),
            default_model,
            last_error: runtime.last_error.clone(),
        }
    } else {
        ApiServiceStatus {
            running: false,
            bind_host: default_host.to_string(),
            bind_port: default_port,
            actual_port: None,
            address: None,
            api_key: api_key.to_string(),
            default_model: default_model.trim().to_string(),
            last_error: None,
        }
    }
}

fn sidecar_runtime_unhealthy(runtime: Option<&Runtime>) -> bool {
    let Some(runtime) = runtime else {
        return false;
    };
    let Some(target) = runtime.proxy_target.as_ref() else {
        return false;
    };

    !sidecar_is_reachable(target)
}

fn sidecar_is_reachable(target: &ProxyTarget) -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };

    client
        .get(format!("{}/v1/models", target.base_url))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        .send()
        .map(|resp| resp.status().is_success())
        .unwrap_or(false)
}

fn build_address(host: &str, port: u16) -> String {
    let visible = if host == "0.0.0.0" || host.is_empty() {
        "127.0.0.1"
    } else {
        host
    };
    format!("http://{visible}:{port}/v1")
}

// ---------- 平台 / 二进制路径 ----------

fn current_target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else {
        ""
    }
}

fn binary_filename(name: &str) -> String {
    let triple = current_target_triple();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    format!("{name}-{triple}{suffix}")
}

/// 解析 sidecar / LS 二进制路径。优先 exe 同目录：
///   - 打包后 tauri 会把 externalBin **去掉 triple 后缀**放在主程序旁
///     （Mac 是 `Contents/MacOS/<name>`，Win 是 exe 同目录的 `<name>.exe`）。
///   - dev 模式（`tauri dev`）也会复制到 target/debug 旁，名字一样去后缀。
///
/// 找不到再退回仓库的 `src-tauri/binaries/<name>-<triple>`，方便 `cargo run`
/// 这种不走 tauri-cli 的开发场景。
fn resolve_bundled_binary(name: &str) -> Option<PathBuf> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let bundled_name = format!("{name}{suffix}");
    let triple_name = binary_filename(name);

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // 打包 / tauri dev：去掉 triple 后的二进制
            let candidate = parent.join(&bundled_name);
            if candidate.is_file() {
                return Some(candidate);
            }
            // 兜底：旧版 / 某些平台 tauri 行为变化时仍可能保留 triple
            let candidate = parent.join(&triple_name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(target_os = "macos")]
            {
                let universal = parent.join(format!("{name}-universal-apple-darwin"));
                if universal.is_file() {
                    return Some(universal);
                }
            }
            // 源码仓库 fallback：cargo run 这种不走 tauri-cli 的场景，binaries/
            // 目录里仍然是 triple-suffixed 名字。
            for ancestor in parent.ancestors() {
                let dev = ancestor.join("src-tauri/binaries").join(&triple_name);
                if dev.is_file() {
                    return Some(dev);
                }
                #[cfg(target_os = "macos")]
                {
                    let universal = ancestor
                        .join("src-tauri/binaries")
                        .join(format!("{name}-universal-apple-darwin"));
                    if universal.is_file() {
                        return Some(universal);
                    }
                }
            }
        }
    }
    None
}

/// 当前上游自带孤儿 LS 清理，但当前是在设置
/// `LS_BINARY_PATH` 前调用 cleanup，打包后无法命中我们的 externalBin 路径。
/// 这里按 argv[0] 精确匹配同一个 LS 二进制，避免旧进程占住固定端口导致
/// 新 sidecar 等待 LS ready 超时。
#[cfg(unix)]
fn cleanup_language_server_processes(ls_bin: &Path) {
    let Ok(ls_bin) = ls_bin.canonicalize() else {
        return;
    };
    let ls_name = ls_bin
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("language_server");
    let Ok(output) = Command::new("ps").args(["-e", "-o", "pid=,args="]).output() else {
        return;
    };

    let current_pid = std::process::id() as libc::pid_t;
    let mut matched_pids = Vec::new();
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        let Some((pid_text, argv)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_text.trim().parse::<libc::pid_t>() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let argv0 = argv.split_whitespace().next().unwrap_or("");
        if argv0.is_empty() {
            continue;
        }
        let argv0_matches = Path::new(argv0)
            .canonicalize()
            .map(|argv0_path| argv0_path == ls_bin)
            .unwrap_or(false);
        if !argv0_matches && !argv.contains(ls_name) {
            continue;
        }
        unsafe {
            let _ = libc::kill(pid, libc::SIGTERM);
        }
        matched_pids.push(pid);
    }
    // 没扫到残留 LS 进程就直接返回，省掉一次 300ms 硬等 —— stop→start 热路径最常见场景。
    if matched_pids.is_empty() {
        return;
    }
    thread::sleep(Duration::from_millis(300));
    for pid in matched_pids {
        unsafe {
            if libc::kill(pid, 0) == 0 {
                let _ = libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn cleanup_language_server_processes(ls_bin: &Path) {
    cleanup_windows_process_image(ls_bin);
}

#[cfg(unix)]
fn cleanup_sidecar_processes(sidecar_bin: &Path) {
    let Ok(sidecar_bin) = sidecar_bin.canonicalize() else {
        return;
    };
    let sidecar_name = sidecar_bin
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("superai-api");
    let Ok(output) = Command::new("ps").args(["-e", "-o", "pid=,args="]).output() else {
        return;
    };

    let current_pid = std::process::id() as libc::pid_t;
    let mut matched_pids = Vec::new();
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        let Some((pid_text, argv)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_text.trim().parse::<libc::pid_t>() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let argv0 = argv.split_whitespace().next().unwrap_or("");
        if argv0.is_empty() {
            continue;
        }
        let argv0_matches = Path::new(argv0)
            .canonicalize()
            .map(|argv0_path| argv0_path == sidecar_bin)
            .unwrap_or(false);
        if !argv0_matches && !argv.contains(sidecar_name) {
            continue;
        }
        unsafe {
            let _ = libc::kill(pid, libc::SIGTERM);
        }
        matched_pids.push(pid);
    }
    if matched_pids.is_empty() {
        return;
    }
    thread::sleep(Duration::from_millis(300));
    for pid in matched_pids {
        unsafe {
            if libc::kill(pid, 0) == 0 {
                let _ = libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn cleanup_sidecar_processes(sidecar_bin: &Path) {
    cleanup_windows_process_image(sidecar_bin);
}

#[cfg(windows)]
fn cleanup_windows_process_image(image_path: &Path) {
    use std::os::windows::process::CommandExt;

    let Some(image_name) = image_path.file_name().and_then(|name| name.to_str()) else {
        return;
    };

    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/IM", image_name])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    thread::sleep(Duration::from_millis(500));
}

#[cfg(all(not(windows), not(unix)))]
fn cleanup_windows_process_image(_image_path: &Path) {}

#[cfg(target_os = "macos")]
fn repair_macos_binary(path: &Path) {
    use std::collections::HashSet;
    use std::time::SystemTime;

    // 缓存同一 App 生命周期内 (canonical path, mtime, size) 已修复过的二进制，
    // 避免每次 start 都跑 codesign --verify（对 ~100MB 的 bun/LS 二进制要 1~4s）。
    // 文件被替换（mtime/size 变）会自动失效，重新走一遍修复流程。
    type CacheKey = (PathBuf, SystemTime, u64);
    static REPAIRED: LazyLock<Mutex<HashSet<CacheKey>>> =
        LazyLock::new(|| Mutex::new(HashSet::new()));

    let cache_key: Option<CacheKey> = path.canonicalize().ok().and_then(|canonical| {
        let meta = std::fs::metadata(&canonical).ok()?;
        let mtime = meta.modified().ok()?;
        Some((canonical, mtime, meta.len()))
    });

    if let Some(key) = cache_key.as_ref() {
        if let Ok(set) = REPAIRED.lock() {
            if set.contains(key) {
                return;
            }
        }
    }

    let _ = Command::new("xattr")
        .arg("-d")
        .arg("com.apple.quarantine")
        .arg(path)
        .output();
    let signature_ok = Command::new("codesign")
        .args(["--verify", "--verbose=1"])
        .arg(path)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !signature_ok {
        let _ = Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(path)
            .output();
    }

    if let Some(key) = cache_key {
        if let Ok(mut set) = REPAIRED.lock() {
            set.insert(key);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn repair_macos_binary(_path: &Path) {}

// ---------- sidecar 启动 ----------

fn pick_free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("分配本机空闲端口失败: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("读取本机端口失败: {error}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn spawn_sidecar(
    sidecar_bin: &Path,
    ls_bin: &Path,
    data_dir: &Path,
    inner_key: &str,
) -> Result<(Sidecar, u16), String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("创建 sidecar 数据目录失败: {error}"))?;

    // 清掉上一次的 accounts.json，让 sidecar 完全以我们 DB 为准重建账号池。
    let stale_accounts = data_dir.join("accounts.json");
    if stale_accounts.exists() {
        let _ = std::fs::remove_file(&stale_accounts);
    }

    // sidecar 当前版本日志只回显 PORT env，自己 bind 的实际端口拿不到。
    // 我们预先挑两个空闲端口给它（HTTP 服务 + 内部 LS）。
    let http_port = pick_free_port()?;
    let ls_port = pick_free_port()?;

    cleanup_sidecar_processes(sidecar_bin);
    cleanup_language_server_processes(ls_bin);
    repair_macos_binary(sidecar_bin);
    repair_macos_binary(ls_bin);

    let mut cmd = Command::new(sidecar_bin);
    cmd.env("PORT", http_port.to_string())
        .env("HOST", "127.0.0.1")
        .env("API_KEY", inner_key)
        .env("LS_BINARY_PATH", ls_bin)
        .env("LS_PORT", ls_port.to_string())
        .env("LS_DATA_DIR", data_dir)
        // bun --compile 后 sidecar 的 __dirname 指向只读的 /$bunfs/，
        // 必须显式给它一个可写目录写 accounts.json / logs/，否则 logger 启动就崩。
        .env("DATA_DIR", data_dir)
        .env("LOG_LEVEL", "info")
        .current_dir(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Windows 下如果不设 CREATE_NO_WINDOW，子进程会弹出一个 cmd 控制台窗口。
    // sidecar 是 bun 编出的 console 子系统可执行文件，必须显式隐藏。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|error| format!("启动 sidecar 失败: {error}"))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let (port_tx, port_rx) = mpsc::channel::<u16>();

    // stdout 解析端口；同步打到主进程 stderr 便于调试
    let stdout_join = thread::Builder::new()
        .name("superai-api-stdout".into())
        .spawn(move || {
            let reader = BufReader::new(stdout);
            let mut sent_port = false;
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if !sent_port {
                    if let Some(port) = parse_listen_port(&line) {
                        if port_tx.send(port).is_ok() {
                            sent_port = true;
                        }
                    }
                }
                if !should_suppress_sidecar_log_line(&line) {
                    eprintln!("[SuperAI sidecar] {}", sanitize_sidecar_log_line(&line));
                }
            }
        })
        .map_err(|error| format!("无法启动 stdout 读线程: {error}"))?;

    let stderr_join = thread::Builder::new()
        .name("superai-api-stderr".into())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if !should_suppress_sidecar_log_line(&line) {
                    eprintln!("[SuperAI sidecar:err] {}", sanitize_sidecar_log_line(&line));
                }
            }
        })
        .map_err(|error| format!("无法启动 stderr 读线程: {error}"))?;

    // 等 stdout 报告端口；超时则放弃并杀进程
    let deadline = Instant::now() + SIDECAR_BOOT_TIMEOUT;
    let port = loop {
        match port_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(port) => break port,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(status)) = child.try_wait() {
                    let _ = stdout_join.join();
                    let _ = stderr_join.join();
                    return Err(format!("sidecar 启动后立刻退出，code={status}"));
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    cleanup_language_server_processes(ls_bin);
                    return Err(format!(
                        "等待 sidecar 启动超时（{}s）",
                        SIDECAR_BOOT_TIMEOUT.as_secs()
                    ));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                cleanup_language_server_processes(ls_bin);
                return Err("sidecar stdout 通道意外关闭".to_string());
            }
        }
    };

    Ok((
        Sidecar {
            child,
            ls_bin: ls_bin.to_path_buf(),
            _stdout_join: Some(stdout_join),
            _stderr_join: Some(stderr_join),
        },
        port,
    ))
}

/// 从 sidecar 的日志里抓 `Server on http://0.0.0.0:NNNN`。
fn parse_listen_port(line: &str) -> Option<u16> {
    let needle = "Server on http://";
    let idx = line.find(needle)?;
    let tail = &line[idx + needle.len()..];
    let after_colon = tail.split(':').nth(1)?;
    let port_str: String = after_colon
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    port_str.parse::<u16>().ok()
}

fn sanitize_sidecar_log_line(line: &str) -> String {
    let brand_upper: String = [87, 105, 110, 100, 115, 117, 114, 102]
        .iter()
        .map(|c| char::from(*c))
        .collect();
    let brand_lower = brand_upper.to_ascii_lowercase();
    let mut text = line
        .replace(&brand_upper, "SuperAI")
        .replace(&brand_lower, "superai");
    let mut sanitized = String::with_capacity(text.len());
    let mut token = String::new();

    let flush_token = |token: &mut String, sanitized: &mut String| {
        if token.is_empty() {
            return;
        }
        let lower = token.to_ascii_lowercase();
        let is_email = token.contains('@') && token.contains('.');
        let is_jwt = token.starts_with("eyJ") && token.matches('.').count() >= 1;
        let is_known_secret = lower.starts_with("auth1_")
            || lower.starts_with("devin-session-token$")
            || lower.starts_with("agt_superai_")
            || lower.starts_with(&deprecated_api_key_prefix())
            || lower.contains("api_key")
            || lower.contains("apikey")
            || lower.contains("session_token")
            || lower.contains("auth1_token")
            || lower.contains("refresh_token")
            || lower.contains("access_token")
            || lower.contains("id_token")
            || lower.contains("password");
        if is_email {
            sanitized.push_str("[account]");
        } else if is_jwt || is_known_secret {
            sanitized.push_str("[secret]");
        } else {
            sanitized.push_str(token);
        }
        token.clear();
    };

    for ch in text.drain(..) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '-' | '$' | '%' | '+') {
            token.push(ch);
        } else {
            flush_token(&mut token, &mut sanitized);
            sanitized.push(ch);
        }
    }
    flush_token(&mut token, &mut sanitized);
    sanitized
}

fn should_suppress_sidecar_log_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();

    // 上游 LS 管理器在固定端口 42100 上做 child handoff / 自恢复时会打印一大串
    // "address already in use"、lock file、Exit RPC refused、堆栈等噪音。
    // 这些日志经常成片出现，但通常会在几百毫秒后自愈并重新连上，不代表
    // SuperAI API 启动失败。这里仅压掉这组高频已知噪音，真正的 sidecar
    // 启动失败仍由 stdout 超时 / 进程退出路径上抛给 UI。
    should_suppress_policy_block_log(line)
        || lower.contains("language server listening on fixed port at 42100")
        || lower.contains("child process attempting to acquire lock file")
        || lower.contains("child process acquired lock file")
        || lower.contains("manager process acquired child process lock")
        || lower.contains("failed exit rpc on language server")
        || lower.contains("listen tcp 127.0.0.1:42100: bind: address already in use")
        || lower.contains("language server failed - listen tcp 127.0.0.1:42100")
        || lower.contains("starting language server process with pid")
        || lower.contains("language server will attempt to listen on host 127.0.0.1")
        || lower.contains("successfully connected to new language server at 127.0.0.1:42100")
        || lower.contains("exit requested on language server process")
        || lower.contains("language server shutting down")
        || lower.contains("attempting to connect to language server at 127.0.0.1:42100")
        || lower.contains("attached stack trace")
        || lower.contains("-- stack trace:")
        || lower.contains("error types:")
        || lower.starts_with("| github.com/")
        || lower.starts_with("|       ")
        || lower.starts_with("wraps: ")
        || lower.contains("no accounts configured. add via")
        || lower.contains("post /auth/login {\"token\":\"...\"}")
        || lower.contains("post /auth/login {\"api_key\":\"...\"}")
        || lower.contains("[meta_tag_audit] unknown xml tags in user message:")
}

// ---------- 启停 ----------

/// 生产模式启动：spawn sidecar + 起 HTTP 服务（反向代理）。
pub fn start(
    app_data_dir: &Path,
    host: &str,
    port: u16,
    api_key: &str,
    default_model: &str,
) -> Result<ApiServiceStatus, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 为空，无法启动".to_string());
    }
    stop()?;
    clear_synced_active_label();

    let sidecar_bin = resolve_bundled_binary("superai-api").ok_or_else(|| {
        format!(
            "未找到 sidecar 二进制 {}。请先运行 `npm run build:sidecar`",
            binary_filename("superai-api")
        )
    })?;
    let ls_bin = resolve_bundled_binary("language_server").ok_or_else(|| {
        format!(
            "未找到 SuperAI runtime 二进制 {}。请先运行 `npm run build:sidecar`",
            binary_filename("language_server")
        )
    })?;

    let inner_key = generate_inner_key();
    let sidecar_data_dir = app_data_dir.join("superai-api");
    let (sidecar, sidecar_port) =
        spawn_sidecar(&sidecar_bin, &ls_bin, &sidecar_data_dir, &inner_key)?;

    let target = ProxyTarget {
        base_url: format!("http://127.0.0.1:{sidecar_port}"),
        inner_key,
        default_model: Arc::new(RwLock::new(default_model.trim().to_string())),
    };

    start_internal(host, port, api_key, Some(target), Some(sidecar))
}

/// 更新当前正在跑的服务的默认模型。无运行时返回 Err。
///
/// 通过共享的 `Arc<RwLock<String>>` 改写，服务线程下一次请求读到的就是新值，
/// 不需要重启 API 服务（之前是 clone 进线程的快照，必须重启才生效）。
pub fn update_default_model(model: &str) -> Result<(), String> {
    let guard = lock();
    let runtime = guard.as_ref().ok_or_else(|| "API 服务未运行".to_string())?;
    let target = runtime
        .proxy_target
        .as_ref()
        .ok_or_else(|| "API 服务未挂 sidecar".to_string())?;
    let mut slot = target
        .default_model
        .write()
        .map_err(|error| format!("获取默认模型写锁失败: {error}"))?;
    *slot = model.trim().to_string();
    Ok(())
}

/// 测试模式启动：只起 HTTP 服务，不 spawn sidecar。
#[cfg(test)]
fn start_no_sidecar(host: &str, port: u16, api_key: &str) -> Result<ApiServiceStatus, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 为空，无法启动".to_string());
    }
    stop()?;
    start_internal(host, port, api_key, None, None)
}

fn start_internal(
    host: &str,
    port: u16,
    api_key: &str,
    target: Option<ProxyTarget>,
    sidecar: Option<Sidecar>,
) -> Result<ApiServiceStatus, String> {
    let bind = format!("{host}:{port}");
    let server = Server::http(&bind).map_err(|error| {
        format!("绑定 {bind} 失败: {error}。请在设置中修改 API 服务端口后重试。")
    })?;
    let actual_port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "读取服务监听端口失败".to_string())?
        .port();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_for_thread = stop_flag.clone();
    let api_key_owned = api_key.to_string();
    let host_owned = host.to_string();
    let target_for_thread = target.clone();
    let default_model = target
        .as_ref()
        .map(|proxy| proxy.default_model_snapshot())
        .unwrap_or_default();

    let accept_join = thread::Builder::new()
        .name("superai-api".into())
        .spawn(move || {
            run_server(
                server,
                stop_flag_for_thread,
                api_key_owned,
                target_for_thread,
            );
        })
        .map_err(|error| format!("创建服务线程失败: {error}"))?;

    *lock() = Some(Runtime {
        stop_flag,
        accept_join: Some(accept_join),
        bind_host: host_owned.clone(),
        bind_port: port,
        actual_port,
        api_key: api_key.to_string(),
        last_error: None,
        sidecar,
        proxy_target: target,
    });

    Ok(ApiServiceStatus {
        running: true,
        bind_host: host_owned.clone(),
        bind_port: port,
        actual_port: Some(actual_port),
        address: Some(build_address(&host_owned, actual_port)),
        api_key: api_key.to_string(),
        default_model,
        last_error: None,
    })
}

/// 列出 sidecar 当前对外提供的所有模型（OpenAI list 形式）。
pub fn list_models() -> Result<Vec<Value>, String> {
    let target = clone_target()?;
    let client = build_inner_client()?;
    let resp = client
        .get(format!("{}/v1/models", target.base_url))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        .send()
        .map_err(|error| format!("调用 sidecar /v1/models 失败: {error}"))?;
    if !resp.status().is_success() {
        return Err(format!("sidecar /v1/models HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .map_err(|error| format!("解析模型列表失败: {error}"))?;
    Ok(body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// API 服务是否正在运行且挂了 sidecar。
pub fn is_running_with_sidecar() -> bool {
    let guard = lock();
    guard.as_ref().is_some_and(|r| r.proxy_target.is_some())
}

fn clone_target() -> Result<ProxyTarget, String> {
    let guard = lock();
    let runtime = guard.as_ref().ok_or_else(|| "API 服务未运行".to_string())?;
    runtime
        .proxy_target
        .clone()
        .ok_or_else(|| "API 服务未挂 sidecar，无法同步账号".to_string())
}

fn build_inner_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("初始化 HTTP 客户端失败: {error}"))
}

/// 让 sidecar 的账号池与传入列表保持一致：
/// - sidecar 已有但传入没有 → DELETE
/// - 传入有 sidecar 没有 → POST /auth/login
///
/// 用 `label` 做匹配。空 label 的项跳过远端 diff，只 POST。
pub fn reconcile_accounts(desired: Vec<Value>) -> Result<Value, String> {
    let target = clone_target()?;
    let client = build_inner_client()?;

    // 1) 拉 sidecar 当前账号
    let list_resp = client
        .get(format!("{}/auth/accounts", target.base_url))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        .send()
        .map_err(|error| format!("调用 sidecar /auth/accounts 失败: {error}"))?;
    if !list_resp.status().is_success() {
        return Err(format!(
            "sidecar /auth/accounts HTTP {}",
            list_resp.status()
        ));
    }
    let list_body: Value = list_resp
        .json()
        .map_err(|error| format!("解析 sidecar 列表失败: {error}"))?;
    let current = list_body
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // label -> sidecar id（小写归一化）
    let mut existing: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for entry in &current {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
        let label = entry
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if !id.is_empty() && !label.is_empty() {
            existing.insert(label, id.to_string());
        }
    }

    // 期望 label 集合
    let desired_labels: std::collections::HashSet<String> = desired
        .iter()
        .filter_map(|p| p.get("label").and_then(Value::as_str))
        .map(|s| s.to_ascii_lowercase())
        .collect();

    // 2) 删除 sidecar 多出来的
    let mut removed = 0usize;
    let mut failed_to_remove: Vec<Value> = Vec::new();
    for (label, id) in &existing {
        if !desired_labels.contains(label) {
            match client
                .delete(format!("{}/auth/accounts/{}", target.base_url, id))
                .header("Authorization", format!("Bearer {}", target.inner_key))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    removed += 1;
                }
                Ok(resp) => {
                    failed_to_remove.push(json!({
                        "label": label,
                        "status": resp.status().as_u16(),
                    }));
                }
                Err(error) => {
                    failed_to_remove.push(json!({
                        "label": label,
                        "error": error.to_string(),
                    }));
                }
            }
        }
    }

    // 3) 添加缺失的（sidecar 内部按 apiKey 去重，不会重复）
    let to_add: Vec<Value> = desired
        .into_iter()
        .filter(|p| {
            p.get("label")
                .and_then(Value::as_str)
                .map(|l| !existing.contains_key(&l.to_ascii_lowercase()))
                .unwrap_or(true)
        })
        .collect();

    let add_count = to_add.len();
    if !to_add.is_empty() {
        let resp = client
            .post(format!("{}/auth/login", target.base_url))
            .header("Authorization", format!("Bearer {}", target.inner_key))
            .json(&json!({ "accounts": to_add }))
            .send()
            .map_err(|error| format!("调用 sidecar /auth/login 失败: {error}"))?;
        if !resp.status().is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(format!("sidecar /auth/login HTTP: {body}"));
        }
    }

    // 这里只做账号同步与额度刷新，不再主动打 sidecar 的 probe-all。
    // sidecar 账号池是内存态，应用重启后会把 DB 里的账号重新 /auth/login 一遍；
    // 如果这里顺手触发 probe-all，就会在每次启动时额外跑一轮能力探测，
    // 产生无意义的上游流量并刷爆日志。能力探测交给 sidecar 自己的按需
    // 路径或内部定时 re-probe，不由宿主层强推。
    let refresh = if add_count > 0 || removed > 0 {
        refresh_sidecar_account_capabilities(&client, &target)
    } else {
        json!({"skipped": "no account changes"})
    };

    Ok(json!({
        "added": add_count,
        "removed": removed,
        "failedToRemove": failed_to_remove,
        "kept": existing.len().saturating_sub(removed),
        "refresh": refresh,
    }))
}

fn refresh_sidecar_account_capabilities(
    client: &reqwest::blocking::Client,
    target: &ProxyTarget,
) -> Value {
    let credits = post_sidecar_dashboard_api(client, target, "/accounts/refresh-credits");
    json!({
        "credits": credits,
    })
}

fn post_sidecar_dashboard_api(
    client: &reqwest::blocking::Client,
    target: &ProxyTarget,
    subpath: &str,
) -> Value {
    let response = match client
        .post(format!("{}/dashboard/api{}", target.base_url, subpath))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        // sidecar dashboard 路由不看 Bearer，只看 X-Dashboard-Password；
        // localhost bind 下 effectiveApiKey 等同 dashboard 密码。不加这个头
        // 会被防爆破锁累计 5 次后封 30 分钟（issue: 启用账号 429）。
        .header("X-Dashboard-Password", target.inner_key.as_str())
        .json(&json!({}))
        .send()
    {
        Ok(response) => response,
        Err(error) => return json!({ "ok": false, "error": error.to_string() }),
    };
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return json!({
            "ok": false,
            "status": status.as_u16(),
            "body": body,
        });
    }
    match serde_json::from_str::<Value>(&body) {
        Ok(value) => json!({ "ok": true, "body": value }),
        Err(_) => json!({ "ok": true, "body": body }),
    }
}

pub fn activate_account_by_label(label: &str) -> Result<(), String> {
    let wanted_label = label.trim().to_ascii_lowercase();
    if wanted_label.is_empty() {
        return Err("SuperAI 账号缺少 sidecar 标识，无法同步 API 启用状态".to_string());
    }

    let target = clone_target()?;
    let client = build_inner_client()?;
    let list_resp = client
        .get(format!("{}/auth/accounts", target.base_url))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        .send()
        .map_err(|error| format!("调用 sidecar /auth/accounts 失败: {error}"))?;
    if !list_resp.status().is_success() {
        return Err(format!(
            "sidecar /auth/accounts HTTP {}",
            list_resp.status()
        ));
    }
    let list_body: Value = list_resp
        .json()
        .map_err(|error| format!("解析 sidecar 账号列表失败: {error}"))?;
    let account_id = list_body
        .get("accounts")
        .and_then(Value::as_array)
        .and_then(|accounts| {
            accounts.iter().find_map(|account| {
                let sidecar_label = account
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if sidecar_label == wanted_label {
                    account
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| format!("API 服务中未找到 SuperAI 账号: {label}"))?;

    let resp = client
        .patch(format!(
            "{}/dashboard/api/accounts/{}",
            target.base_url, account_id
        ))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        // 见 post_sidecar_dashboard_api 注释：dashboard 路由只看
        // X-Dashboard-Password；不带会撞防爆破锁 → 30min IP ban。
        .header("X-Dashboard-Password", target.inner_key.as_str())
        .json(&json!({ "status": "active", "resetErrors": true }))
        .send()
        .map_err(|error| format!("调用 sidecar 启用账号失败: {error}"))?;
    if resp.status().is_success() {
        return Ok(());
    }
    Err(format!(
        "sidecar 启用账号 HTTP {}: {}",
        resp.status(),
        resp.text().unwrap_or_default()
    ))
}

/// 停止服务（幂等）。
pub fn stop() -> Result<(), String> {
    clear_synced_active_label();
    let runtime = { lock().take() };
    let Some(mut runtime) = runtime else {
        return Ok(());
    };
    runtime.stop_flag.store(true, Ordering::SeqCst);
    if let Some(handle) = runtime.accept_join.take() {
        let _ = handle.join();
    }
    // sidecar 在 Drop 中被 kill + wait
    drop(runtime.sidecar.take());
    Ok(())
}

pub fn cleanup_update_blockers() {
    if let Some(sidecar_bin) = resolve_bundled_binary("superai-api") {
        cleanup_sidecar_processes(&sidecar_bin);
    }
    if let Some(ls_bin) = resolve_bundled_binary("language_server") {
        cleanup_language_server_processes(&ls_bin);
    }
}

// ---------- 请求路由 ----------

fn run_server(
    server: Server,
    stop_flag: Arc<AtomicBool>,
    api_key: String,
    target: Option<ProxyTarget>,
) {
    let api_key = Arc::new(api_key);
    let target = target.map(Arc::new);
    while !stop_flag.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(request)) => {
                let key = api_key.clone();
                let target = target.clone();
                // 每个请求独立线程，避免 SSE 长连接阻塞 accept 循环。
                let _ = thread::Builder::new()
                    .name("superai-api-req".into())
                    .spawn(move || handle_request(request, &key, target.as_deref()));
            }
            Ok(None) => continue,
            Err(_) => break,
        }
    }
}

fn handle_request(mut request: Request, api_key: &str, target: Option<&ProxyTarget>) {
    // CORS 预检
    if matches!(request.method(), Method::Options) {
        let _ = request.respond(cors_response(204, b""));
        return;
    }

    let (path, query) = match request.url().split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (request.url().to_string(), String::new()),
    };

    // 鉴权
    if !is_authorized(&request, api_key) {
        let _ = request.respond(json_response(
            401,
            &json!({
                "error": {
                    "message": "无效或缺失的 Authorization 头",
                    "type": "invalid_request_error",
                    "code": "invalid_api_key",
                }
            }),
        ));
        return;
    }

    // 反向代理：所有 /v1/* 与 /auth/* 都转发给 sidecar。
    if request.method() == &Method::Get && wants_anthropic_models_api(&request) {
        if path == "/v1/models" || path == "/v1/models/" {
            let _ = request.respond(json_response(200, &anthropic_models_payload()));
            return;
        }
        if let Some(model_id) = path.strip_prefix("/v1/models/") {
            let decoded_model_id = decode_model_path_segment(model_id);
            if let Some(payload) = anthropic_model_payload_by_id(&decoded_model_id) {
                let _ = request.respond(json_response(200, &payload));
            } else {
                let _ = request.respond(json_response(
                    404,
                    &json!({
                        "type": "error",
                        "error": {
                            "type": "not_found_error",
                            "message": format!("Model not found: {decoded_model_id}"),
                        }
                    }),
                ));
            }
            return;
        }
    }

    if let Some(target) = target {
        if path.starts_with("/v1/") || path.starts_with("/auth/") || path == "/v1/models" {
            proxy_to_sidecar(request, target, &path, &query);
            return;
        }
    }

    // 占位模式 / 未挂 sidecar
    match (request.method().clone(), path.as_str()) {
        (Method::Get, "/v1/models") | (Method::Get, "/v1/models/") => {
            let _ = request.respond(json_response(200, &fallback_models_payload()));
        }
        (Method::Post, "/v1/chat/completions") | (Method::Post, "/v1/messages") => {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let _ = request.respond(json_response(
                501,
                &json!({
                    "error": {
                        "message": "SuperAI 本地 API 服务尚未接入运行时，请等待后续版本。",
                        "type": "not_implemented",
                        "code": "ls_unavailable",
                    }
                }),
            ));
        }
        _ => {
            let _ = request.respond(json_response(
                404,
                &json!({
                    "error": {
                        "message": format!("未知路径: {path}"),
                        "type": "not_found",
                    }
                }),
            ));
        }
    }
}

// ---------- 反向代理 ----------

fn proxy_to_sidecar(mut request: Request, target: &ProxyTarget, path: &str, query: &str) {
    // 读 body
    let mut body = Vec::new();
    if let Err(error) = request.as_reader().read_to_end(&mut body) {
        let _ = request.respond(json_response(
            400,
            &json!({"error": {"message": format!("读取请求体失败: {error}"), "type": "bad_request"}}),
        ));
        return;
    }

    let mut requested_model_for_client: Option<String> = None;
    if path == "/v1/messages" && !body.is_empty() {
        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
            requested_model_for_client = value
                .get("model")
                .and_then(Value::as_str)
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty());
        }
    }

    // SuperAI 仍然保留模型选择权，但不同客户端走各自推荐模型：
    // - /v1/messages  (Claude Code) → Claude 推荐模型
    // - /v1/responses (Codex CLI)   → Codex 推荐模型
    // - /v1/chat/completions         → 保持全局默认模型
    let current_default_model = target.default_model_snapshot();
    let effective_model_for_path = match path {
        "/v1/messages" => claude_preferred_model(&current_default_model),
        "/v1/responses" => codex_preferred_model(&current_default_model),
        "/v1/chat/completions" => current_default_model.clone(),
        _ => String::new(),
    };
    if !effective_model_for_path.is_empty()
        && (path == "/v1/chat/completions" || path == "/v1/messages" || path == "/v1/responses")
        && !body.is_empty()
    {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&body) {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("model".to_string(), Value::String(effective_model_for_path));
                if let Ok(new_body) = serde_json::to_vec(&value) {
                    body = new_body;
                }
            }
        }
    }

    let url = if query.is_empty() {
        format!("{}{}", target.base_url, path)
    } else {
        format!("{}{}?{}", target.base_url, path, query)
    };

    let method = request.method().clone();
    let upstream_method = match method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Delete => reqwest::Method::DELETE,
        Method::Patch => reqwest::Method::PATCH,
        Method::Head => reqwest::Method::HEAD,
        Method::Options => reqwest::Method::OPTIONS,
        _ => {
            let _ = request.respond(json_response(
                405,
                &json!({"error": {"message": "不支持的方法", "type": "method_not_allowed"}}),
            ));
            return;
        }
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(None) // SSE 长连接
        .build()
    {
        Ok(c) => c,
        Err(error) => {
            let _ = request.respond(json_response(
                500,
                &json!({"error": {"message": format!("初始化 HTTP 客户端失败: {error}"), "type": "internal"}}),
            ));
            return;
        }
    };

    // 透传 Content-Type / Accept 等，但忽略 hop-by-hop 与外层 Authorization
    let mut passthrough_headers: Vec<(String, String)> = Vec::new();
    for header in request.headers() {
        let name = header.field.as_str().as_str().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "host"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "transfer-encoding"
                | "upgrade"
                | "authorization"
                | "content-length"
        ) {
            continue;
        }
        passthrough_headers.push((
            header.field.as_str().as_str().to_string(),
            header.value.as_str().to_string(),
        ));
    }

    let mut upstream_resp = match send_sidecar_proxy_request(
        &client,
        upstream_method.clone(),
        &url,
        target,
        &passthrough_headers,
        requested_model_for_client.as_deref(),
        body.clone(),
    ) {
        Ok(r) => r,
        Err(error) => {
            let _ = request.respond(json_response(
                502,
                &json!({"error": {"message": format!("sidecar 不可达: {error}"), "type": "bad_gateway"}}),
            ));
            return;
        }
    };

    if is_chat_path(path) && upstream_resp.status().as_u16() == 403 {
        let first_headers = response_headers(&upstream_resp);
        let first_status = upstream_resp.status().as_u16();
        let first_body = upstream_resp
            .bytes()
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();

        if is_probe_pending_error(&first_body) {
            if should_refresh_capabilities_for_probe_pending() {
                if let Ok(inner_client) = build_inner_client() {
                    let refresh = refresh_sidecar_account_capabilities(&inner_client, target);
                    eprintln!(
                        "[SuperAI API] account capability check triggered by probe_pending: {}",
                        sanitize_sidecar_log_line(&refresh.to_string())
                    );
                }
            } else {
                eprintln!(
                    "[SuperAI API] skipped duplicate capability check for probe_pending within 15s window"
                );
            }

            match send_sidecar_proxy_request(
                &client,
                upstream_method,
                &url,
                target,
                &passthrough_headers,
                requested_model_for_client.as_deref(),
                body.clone(),
            ) {
                Ok(retried) => {
                    if retried.status().as_u16() == first_status {
                        let retried_headers = response_headers(&retried);
                        let retried_body = retried
                            .bytes()
                            .map(|bytes| bytes.to_vec())
                            .unwrap_or_default();
                        let response = Response::new(
                            StatusCode(first_status),
                            retried_headers,
                            Cursor::new(retried_body),
                            None,
                            None,
                        );
                        let _ = request.respond(response);
                        return;
                    } else {
                        upstream_resp = retried;
                    }
                }
                Err(error) => {
                    let _ = request.respond(json_response(
                        502,
                        &json!({"error": {"message": format!("sidecar 不可达: {error}"), "type": "bad_gateway"}}),
                    ));
                    return;
                }
            }
        } else {
            let response = Response::new(
                StatusCode(first_status),
                first_headers,
                Cursor::new(first_body),
                None,
                None,
            );
            let _ = request.respond(response);
            return;
        }
    }
    // 把最近使用账号的探测放后台线程，避免给客户端 respond 之前再多打
    // 一次 sidecar GET — 之前是同步调用，会拖慢 SSE 首字节，并在高并发
    // 下让 /auth/accounts 被反复打。fire-and-forget 即可，结果只用于
    // UI 高亮，丢失一次没关系。
    if matches!(
        path,
        "/v1/chat/completions" | "/v1/messages" | "/v1/responses"
    ) && should_probe_last_used_account()
    {
        let target_for_probe = target.clone();
        let _ = thread::Builder::new()
            .name("superai-api-last-used".into())
            .spawn(move || {
                if let Ok(client) = build_inner_client() {
                    update_last_used_account_from_sidecar(&client, &target_for_probe);
                }
            });
    }

    // 收集响应头（除 hop-by-hop 与 Content-Length；body 长度让 tiny_http 自行决定）。
    let status = upstream_resp.status().as_u16();
    let headers = response_headers(&upstream_resp);

    let response = Response::new(StatusCode(status), headers, upstream_resp, None, None);
    let _ = request.respond(response);
}

fn send_sidecar_proxy_request(
    client: &reqwest::blocking::Client,
    method: reqwest::Method,
    url: &str,
    target: &ProxyTarget,
    passthrough_headers: &[(String, String)],
    requested_model_for_client: Option<&str>,
    body: Vec<u8>,
) -> Result<reqwest::blocking::Response, reqwest::Error> {
    let mut builder = client
        .request(method, url)
        .header("Authorization", format!("Bearer {}", target.inner_key));

    for (name, value) in passthrough_headers {
        builder = builder.header(name, value);
    }
    if let Some(model) = requested_model_for_client {
        builder = builder.header("x-superai-requested-model", model);
    }
    if !body.is_empty() {
        builder = builder.body(body);
    }
    builder.send()
}

fn response_headers(upstream_resp: &reqwest::blocking::Response) -> Vec<Header> {
    let mut headers: Vec<Header> = Vec::new();
    for (k, v) in upstream_resp.headers().iter() {
        let name_lower = k.as_str().to_ascii_lowercase();
        if matches!(
            name_lower.as_str(),
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "transfer-encoding"
                | "upgrade"
                | "content-length"
        ) {
            continue;
        }
        if let Ok(bytes) = v.to_str() {
            if let Ok(h) = Header::from_bytes(k.as_str().as_bytes(), bytes.as_bytes()) {
                headers.push(h);
            }
        }
    }
    // 始终带 CORS
    for h in cors_headers(None) {
        headers.push(h);
    }
    headers
}

fn is_chat_path(path: &str) -> bool {
    matches!(
        path,
        "/v1/chat/completions" | "/v1/messages" | "/v1/responses"
    )
}

fn is_probe_pending_error(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    value
        .get("error")
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "probe_pending")
}

fn update_last_used_account_from_sidecar(client: &reqwest::blocking::Client, target: &ProxyTarget) {
    let Ok(resp) = client
        .get(format!("{}/auth/accounts", target.base_url))
        .header("Authorization", format!("Bearer {}", target.inner_key))
        .send()
    else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(body) = resp.json::<Value>() else {
        return;
    };
    let Some(accounts) = body.get("accounts").and_then(Value::as_array) else {
        return;
    };

    let mut latest: Option<(&str, &str)> = None;
    for account in accounts {
        let Some(email) = account.get("email").and_then(Value::as_str) else {
            continue;
        };
        let Some(last_used) = account.get("lastUsed").and_then(Value::as_str) else {
            continue;
        };
        if latest
            .map(|(_, current_last_used)| last_used > current_last_used)
            .unwrap_or(true)
        {
            latest = Some((email, last_used));
        }
    }

    if let Some((label, _)) = latest {
        set_last_used_account_label(label.to_ascii_lowercase());
    }
}

// ---------- 辅助 ----------

fn is_authorized(request: &Request, api_key: &str) -> bool {
    request.headers().iter().any(|h| {
        if h.field.equiv("Authorization") {
            return h
                .value
                .as_str()
                .strip_prefix("Bearer ")
                .is_some_and(|token| token.trim() == api_key);
        }
        if h.field.equiv("x-api-key") {
            return h.value.as_str().trim() == api_key;
        }
        false
    })
}

fn wants_anthropic_models_api(request: &Request) -> bool {
    request
        .headers()
        .iter()
        .any(|h| h.field.equiv("anthropic-version"))
}

fn anthropic_model_entries() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("claude-opus-4-7", "Claude Opus 4.7", "2026-02-19T00:00:00Z"),
        (
            "claude-opus-4-7[1m]",
            "Claude Opus 4.7 (1M context)",
            "2026-02-19T00:00:00Z",
        ),
        ("claude-opus-4.7", "Claude Opus 4.7", "2026-02-19T00:00:00Z"),
        (
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "2025-08-01T00:00:00Z",
        ),
        (
            "claude-sonnet-4.6",
            "Claude Sonnet 4.6",
            "2025-08-01T00:00:00Z",
        ),
        (
            "claude-sonnet-4-6[1m]",
            "Claude Sonnet 4.6 (1M context)",
            "2025-08-01T00:00:00Z",
        ),
        (
            "claude-sonnet-4.6[1m]",
            "Claude Sonnet 4.6 (1M context)",
            "2025-08-01T00:00:00Z",
        ),
        (
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            "2025-10-01T00:00:00Z",
        ),
        (
            "claude-haiku-4.5",
            "Claude Haiku 4.5",
            "2025-10-01T00:00:00Z",
        ),
    ]
}

fn anthropic_models_payload() -> Value {
    let data: Vec<Value> = anthropic_model_entries()
        .iter()
        .map(|(id, display_name, created_at)| {
            json!({
                "created_at": created_at,
                "display_name": display_name,
                "id": id,
                "type": "model",
            })
        })
        .collect();
    let first_id = data
        .first()
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let last_id = data
        .last()
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    json!({
        "data": data,
        "first_id": first_id,
        "has_more": false,
        "last_id": last_id,
    })
}

fn anthropic_model_payload_by_id(model_id: &str) -> Option<Value> {
    anthropic_model_entries()
        .into_iter()
        .find(|(id, _, _)| *id == model_id)
        .map(|(id, display_name, created_at)| {
            json!({
                "created_at": created_at,
                "display_name": display_name,
                "id": id,
                "type": "model",
            })
        })
}

fn decode_model_path_segment(segment: &str) -> String {
    Url::parse(&format!("http://localhost/v1/models/{segment}"))
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_string))
        })
        .unwrap_or_else(|| segment.to_string())
}

fn json_response(code: u16, value: &Value) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let len = body.len();
    Response::new(
        StatusCode(code),
        cors_headers(Some("application/json; charset=utf-8")),
        Cursor::new(body),
        Some(len),
        None,
    )
}

fn cors_response(code: u16, body: &[u8]) -> Response<Cursor<Vec<u8>>> {
    let body = body.to_vec();
    let len = body.len();
    Response::new(
        StatusCode(code),
        cors_headers(None),
        Cursor::new(body),
        Some(len),
        None,
    )
}

fn cors_headers(content_type: Option<&str>) -> Vec<Header> {
    let mut headers = vec![
        Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).expect("cors origin"),
        Header::from_bytes(
            &b"Access-Control-Allow-Methods"[..],
            &b"GET, POST, PUT, DELETE, PATCH, OPTIONS"[..],
        )
        .expect("cors methods"),
        Header::from_bytes(
            &b"Access-Control-Allow-Headers"[..],
            &b"Authorization, Content-Type, X-Requested-With, x-api-key, anthropic-version"[..],
        )
        .expect("cors headers"),
    ];
    if let Some(ct) = content_type {
        if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()) {
            headers.push(header);
        }
    }
    headers
}

/// 没挂 sidecar 时的占位模型列表（仅在测试或 sidecar 缺失时短暂使用）。
fn fallback_models_payload() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ids = [
        "superai-swe-1",
        "claude-3-5-sonnet",
        "claude-3-7-sonnet",
        "claude-sonnet-4",
        "gpt-4o",
        "gpt-4.1",
        "gemini-2.5-pro",
    ];
    let data: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "created": now,
                "owned_by": "superai",
            })
        })
        .collect();
    json!({ "object": "list", "data": data })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    /// 串行化所有会动 RUNTIME 全局态的测试，避免 `cargo test`
    /// 默认并发线程时互相 stop() 掉对方的监听器。
    static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock_serial() -> std::sync::MutexGuard<'static, ()> {
        // 中毒锁也强行拿到，单条用例 panic 不应阻塞后续。
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn http_get(addr: &str, path: &str, auth: Option<&str>) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).expect("connect");
        let auth_line = auth
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        let req =
            format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n{auth_line}Connection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        let status = buf
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    fn http_get_with_x_api_key(addr: &str, path: &str, key: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).expect("connect");
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nx-api-key: {key}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        let status = buf
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    fn http_post(addr: &str, path: &str, auth: &str, body: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).expect("connect");
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        let status = buf
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let resp_body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, resp_body)
    }

    #[test]
    fn lifecycle_and_routes() {
        let _guard = lock_serial();
        let key = "agt_superai_test_key_12345";
        let status = start_no_sidecar("127.0.0.1", 0, key).expect("start");
        assert!(status.running);
        let port = status.actual_port.expect("actual port");
        let addr = format!("127.0.0.1:{port}");

        let (code, _) = http_get(&addr, "/v1/models", None);
        assert_eq!(code, 401);

        let (code, _) = http_get(&addr, "/v1/models", Some("wrong"));
        assert_eq!(code, 401);

        let (code, body) = http_get(&addr, "/v1/models", Some(key));
        assert_eq!(code, 200);
        assert!(body.contains("\"object\":\"list\""), "body: {body}");
        assert!(body.contains("claude-sonnet-4"), "body: {body}");

        let (code, body) = http_get_with_x_api_key(&addr, "/v1/models", key);
        assert_eq!(code, 200);
        assert!(body.contains("\"object\":\"list\""), "body: {body}");

        let (code, _) = http_get(&addr, "/nope", Some(key));
        assert_eq!(code, 404);

        let (code, body) = http_post(
            &addr,
            "/v1/chat/completions",
            key,
            r#"{"model":"x","messages":[]}"#,
        );
        assert_eq!(code, 501);
        assert!(body.contains("not_implemented"), "body: {body}");

        stop().unwrap();
        stop().unwrap();
    }

    #[test]
    fn restart_replaces_existing_runtime() {
        let _guard = lock_serial();
        let key = "agt_superai_restart_test";
        let s1 = start_no_sidecar("127.0.0.1", 0, key).unwrap();
        let p1 = s1.actual_port.unwrap();
        let s2 = start_no_sidecar("127.0.0.1", 0, key).unwrap();
        let p2 = s2.actual_port.unwrap();
        assert!(p1 > 0 && p2 > 0);
        let cur = current_status("127.0.0.1", 0, key, "");
        assert!(cur.running);
        assert_eq!(cur.actual_port, Some(p2));
        stop().unwrap();
    }

    #[test]
    fn empty_key_rejected() {
        let _guard = lock_serial();
        let err = start_no_sidecar("127.0.0.1", 0, "").unwrap_err();
        assert!(err.contains("API Key"));
    }

    /// 真跑：spawn sidecar + 反向代理 /v1/models。
    /// 依赖 src-tauri/binaries/{superai-api,language_server}-<triple> 已经构建好；
    /// 默认忽略，按需 `cargo test e2e_proxy_models -- --ignored --test-threads=1` 跑。
    #[test]
    #[ignore = "needs prebuilt sidecar binaries; run with --ignored"]
    fn e2e_proxy_models() {
        let _guard = lock_serial();
        let key = "agt_superai_e2e_test_key";
        let tmp = std::env::temp_dir().join("super-ai-api-service-e2e");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let status = start(&tmp, "127.0.0.1", 0, key, "").expect("start with sidecar");
        let port = status.actual_port.expect("port");
        let addr = format!("127.0.0.1:{port}");

        let (code, body) = http_get(&addr, "/v1/models", Some(key));
        assert_eq!(code, 200, "body: {body}");
        // 真实模型清单包含来自上游 catalog 的标识
        assert!(
            body.contains("\"object\":\"list\"") && body.contains("claude"),
            "unexpected body: {body}",
        );

        stop().unwrap();
    }

    #[test]
    fn parse_listen_port_basic() {
        assert_eq!(
            parse_listen_port("[INFO] Server on http://0.0.0.0:39001"),
            Some(39001)
        );
        assert_eq!(
            parse_listen_port("Server on http://127.0.0.1:65530 ready"),
            Some(65530)
        );
        assert_eq!(parse_listen_port("Listening on 39001"), None);
    }
}
