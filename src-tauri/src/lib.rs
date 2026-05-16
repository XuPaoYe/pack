mod api_service;

use aes::Aes256;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{ActivationPolicy, Emitter, LogicalSize, Manager};
#[cfg(desktop)]
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tiny_http::{Header, Response, Server, StatusCode};
use url::Url;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const GEMINI_KEYCHAIN_SERVICE: &str = "gemini-cli-oauth";
const GEMINI_KEYCHAIN_ACCOUNT: &str = "main-account";
const GEMINI_FILE_KEYCHAIN_FILE: &str = "gemini-credentials.json";
const CODEX_ACCOUNT_CHECK_URL: &str =
    "https://chatgpt.com/backend-api/accounts/check/v4-2023-04-27";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_API_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";
const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_OAUTH_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
const CODEX_OAUTH_CALLBACK_PORT: u16 = 1455;
const OAUTH_TIMEOUT_SECONDS: i64 = 300;
const GEMINI_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GEMINI_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GEMINI_OAUTH_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_OAUTH_CLIENT_SECRET: &str = "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";
const GEMINI_OAUTH_CALLBACK_PATH: &str = "/oauth2callback";
const GEMINI_CODE_ASSIST_LOAD_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const GEMINI_CODE_ASSIST_QUOTA_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
const WINDSURF_FIREBASE_API_KEY: &str = "AIzaSyDsOl-1XpT5err0Tcnx8FFod1H8gVGIycY";
const WINDSURF_FIREBASE_SIGNIN_URL: &str =
    "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword";
const WINDSURF_FIREBASE_REFRESH_URL: &str = "https://securetoken.googleapis.com/v1/token";
const WINDSURF_FIREBASE_LOOKUP_URL: &str =
    "https://identitytoolkit.googleapis.com/v1/accounts:lookup";
const WINDSURF_CODEIUM_REGISTER_URL: &str = "https://api.codeium.com/register_user/";
const WINDSURF_BACKEND_URL: &str = "https://web-backend.windsurf.com";
const WINDSURF_AUTH1_PASSWORD_LOGIN_URL: &str = "https://windsurf.com/_devin-auth/password/login";
const WINDSURF_POST_AUTH_URL_BACKEND: &str =
    "https://web-backend.windsurf.com/exa.seat_management_pb.SeatManagementService/WindsurfPostAuth";
const WINDSURF_POST_AUTH_URL_NEW: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/WindsurfPostAuth";
const WINDSURF_POST_AUTH_URL_LEGACY: &str =
    "https://server.self-serve.windsurf.com/exa.seat_management_pb.SeatManagementService/WindsurfPostAuth";
const WINDSURF_USER_STATUS_PATH: &str =
    "/exa.seat_management_pb.SeatManagementService/GetUserStatus";
const WINDSURF_API_SERVER_HOSTS: [&str; 2] =
    ["server.codeium.com", "server.self-serve.windsurf.com"];
const DEFAULT_WINDSURF_API_MODEL: &str = "gpt-5.5";
const SUPERAI_AES_KEY_HEX: &str =
    "b9c1e79783adb25cdb3667ae62c168e18868438d62a47428abeb7b41491ff2ee";
const SUPERAI_AES_IV_HEX: &str = "36c38e9f6f27302c0f784f7b6556be95";
const TRAY_MENU_SHOW: &str = "tray-show-main";
const TRAY_MENU_QUIT: &str = "tray-quit-app";

fn is_public_build() -> bool {
    option_env!("VITE_SUPERAI_PUBLIC_BUILD") == Some("1")
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        #[cfg(target_os = "macos")]
        let _ = app.show();
    }
}

fn hide_main_window(window: &tauri::Window) {
    let _ = window.hide();
    #[cfg(target_os = "macos")]
    {
        let _ = window.app_handle().hide();
    }
}

#[derive(Debug, Clone, Default)]
struct WindsurfPostAuthResult {
    session_token: String,
    auth1_token: Option<String>,
    account_id: Option<String>,
    primary_org_id: Option<String>,
    orgs: Vec<WindsurfOrg>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WindsurfCodeiumRegisterResult {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    api_server_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct WindsurfOrg {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenMeta {
    has_access_token: bool,
    has_refresh_token: bool,
    has_id_token: bool,
    expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountStatus {
    state: String,
    label: String,
    reason: Option<String>,
    updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuotaMetric {
    key: String,
    label: String,
    remaining_percent: Option<i64>,
    reset_at: Option<Value>,
    detail: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountQuota {
    metrics: Vec<QuotaMetric>,
    last_updated: Option<i64>,
    error: Option<String>,
    is_forbidden: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedAccount {
    id: String,
    provider: String,
    email: String,
    display_name: Option<String>,
    #[serde(default)]
    account_name: Option<String>,
    #[serde(default)]
    organization_id: Option<String>,
    plan: Option<String>,
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    auth_file_plan_type: Option<String>,
    #[serde(default)]
    subscription_active_until: Option<Value>,
    account_id: Option<String>,
    user_id: Option<String>,
    source: String,
    token_meta: TokenMeta,
    status: Option<AccountStatus>,
    quota: Option<AccountQuota>,
    created_at: i64,
    updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportFailure {
    label: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportResult {
    imported: Vec<ManagedAccount>,
    failed: Vec<ImportFailure>,
}

struct WindsurfBatchCredential {
    account: String,
    password: String,
    expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuthStartResult {
    login_id: String,
    provider: String,
    command: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default)]
    auto_launch: bool,
    #[serde(default)]
    mask_sensitive: bool,
    #[serde(default = "default_true")]
    show_startup_check: bool,
    #[serde(default = "default_true")]
    auto_detect: bool,
    #[serde(default, alias = "apiServiceEnabled")]
    api_service_enabled: bool,
    #[serde(default = "default_api_service_host", alias = "apiServiceHost")]
    api_service_host: String,
    #[serde(default, alias = "apiServicePort")]
    api_service_port: u16,
    #[serde(default, alias = "apiServiceKey")]
    api_service_key: String,
    #[serde(default, alias = "apiServiceDefaultModel")]
    api_service_default_model: String,
}

fn default_api_service_host() -> String {
    api_service::DEFAULT_HOST.to_string()
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_app_settings() -> AppSettings {
    AppSettings {
        theme: default_theme(),
        auto_launch: false,
        mask_sensitive: false,
        show_startup_check: true,
        auto_detect: true,
        api_service_enabled: false,
        api_service_host: default_api_service_host(),
        api_service_port: api_service::DEFAULT_PORT,
        api_service_key: String::new(),
        api_service_default_model: DEFAULT_WINDSURF_API_MODEL.to_string(),
    }
}

fn effective_api_service_model(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        DEFAULT_WINDSURF_API_MODEL.to_string()
    } else {
        model.to_string()
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
struct OAuthPending {
    provider: String,
    redirect_uri: String,
    state: String,
    code_verifier: Option<String>,
    port: u16,
    expires_at: i64,
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfoResponse {
    id: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

static OAUTH_PENDING: LazyLock<Mutex<HashMap<String, OAuthPending>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn codex_last_refresh_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.6fZ")
        .to_string()
}

fn now_ts_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn random_urlsafe_token(byte_len: usize) -> String {
    let mut rng = rand::thread_rng();
    let bytes = (0..byte_len).map(|_| rng.gen::<u8>()).collect::<Vec<_>>();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn code_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn open_oauth_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("打开 OAuth 授权页失败: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("打开 OAuth 授权页失败: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("打开 OAuth 授权页失败: {e}"))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持自动打开 OAuth 授权页".to_string())
}

fn app_db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("读取应用数据目录失败: {error}"))?;
    fs::create_dir_all(&data_dir)
        .map_err(|error| format!("创建应用数据目录失败 {}: {error}", data_dir.display()))?;
    Ok(data_dir.join("super_ai.sqlite"))
}

fn open_app_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let path = app_db_path(app)?;
    let conn = Connection::open(&path)
        .map_err(|error| format!("打开 SQLite 数据库失败 {}: {error}", path.display()))?;
    init_app_db(&conn)?;
    Ok(conn)
}

fn init_app_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
      PRAGMA journal_mode = WAL;
      PRAGMA foreign_keys = ON;

      CREATE TABLE IF NOT EXISTS accounts (
        id TEXT PRIMARY KEY,
        provider TEXT NOT NULL,
        email TEXT NOT NULL,
        display_name TEXT,
        account_json TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      );

      CREATE INDEX IF NOT EXISTS idx_accounts_provider_updated
        ON accounts(provider, updated_at DESC);

      CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value_json TEXT NOT NULL,
        updated_at INTEGER NOT NULL
      );

      -- 公开版 SuperAI 账号的本地累计用量缓存。
      -- key = batch_key 的 sha256 hex；value 是 baseline / consumed / last_remote /
      -- exhausted_at 的 JSON 快照。删账号时写入，重新导入同一 batch_key 时
      -- 24h 内会恢复，避免用户误删 / 重导后使用记录被清零。
      CREATE TABLE IF NOT EXISTS public_usage_history (
        key TEXT PRIMARY KEY,
        snapshot_json TEXT NOT NULL,
        saved_at INTEGER NOT NULL
      );
      "#,
    )
    .map_err(|error| format!("初始化 SQLite 数据库失败: {error}"))?;
    conn.execute(
        "DELETE FROM accounts WHERE id IN ('codex_preview', 'gemini_preview')",
        [],
    )
    .map_err(|error| format!("清理演示账号失败: {error}"))?;
    // 历史数据里 provider 列存的是协议字面量；重命名成对用户透明的 "superai"，
    // 避免用户用 sqlite cli 打开 DB 时看到内部协议代号。代码里所有内部比较仍用
    // 协议字面量，parse_stored_account_json / 各 SELECT 都接受两种值做向后兼容。
    conn.execute(
        "UPDATE accounts SET provider = 'superai' WHERE provider = 'windsurf'",
        [],
    )
    .map_err(|error| format!("迁移 provider 列失败: {error}"))?;
    Ok(())
}

fn set_account_current_state(
    conn: &Connection,
    provider: &str,
    account_id: &str,
) -> Result<Vec<ManagedAccount>, String> {
    let mut accounts = read_accounts_from_conn(conn)?;
    let now = now_ts();
    for account in &mut accounts {
        if account.id == account_id {
            account.status = Some(AccountStatus {
                state: "available".to_string(),
                label: "当前".to_string(),
                reason: None,
                updated_at: Some(now),
            });
            account.updated_at = now;
        } else if account.provider == provider
            && account
                .status
                .as_ref()
                .map(|status| status.label.as_str() == "当前")
                .unwrap_or(false)
        {
            mark_account_available(account);
            account.updated_at = now;
        }
    }

    for account in &accounts {
        upsert_account(conn, account)?;
    }

    Ok(accounts
        .into_iter()
        .filter(|account| account.provider == provider || account.id == account_id)
        .collect())
}

fn read_accounts_from_conn(conn: &Connection) -> Result<Vec<ManagedAccount>, String> {
    let mut stmt = conn
        .prepare("SELECT account_json FROM accounts ORDER BY updated_at DESC")
        .map_err(|error| format!("读取账号列表失败: {error}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("读取账号列表失败: {error}"))?;
    let mut accounts = Vec::new();
    for row in rows {
        let account_json = row.map_err(|error| format!("读取账号记录失败: {error}"))?;
        let account = parse_stored_account_json(&account_json)?;
        accounts.push(account);
    }
    Ok(accounts)
}

/// 扫一遍 DB，把所有 license_expires_at 已过期的 SuperAI 账号 DELETE 掉。
///
/// 返回 (被删 id 列表, 这一批里是否包含原"当前"账号)。
///
/// 仅作用于 provider == "windsurf" 且携带 `license_expires_at` 的账号。
/// Codex / Gemini、完全版 windsurf（无 license_expires_at）不会被误删。
fn delete_expired_windsurf_accounts(
    app: &tauri::AppHandle,
    conn: &Connection,
) -> Result<(Vec<String>, bool), String> {
    let accounts = read_accounts_from_conn(conn)?;
    let mut expired: Vec<ManagedAccount> = Vec::new();
    let mut current_was_expired = false;
    for account in accounts {
        if account.provider != "windsurf" {
            continue;
        }
        let Some(expires_at) = windsurf_license_expires_at(&account) else {
            continue;
        };
        if !windsurf_license_expired_at(expires_at) {
            continue;
        }
        if is_current_status(&account.status) {
            current_was_expired = true;
        }
        expired.push(account);
    }
    // 删之前为公开版 + batch_key 的账号 stash 一份使用记录，
    // 24h 内重新导入同一 batch_key 时由 restore_public_usage_history 自动恢复，
    // 避免到期被自动清理后用户重新导入时 consumed 被清零回 100%。
    for account in &expired {
        stash_public_usage_history(app, account);
    }
    let expired_ids: Vec<String> = expired.into_iter().map(|account| account.id).collect();
    for id in &expired_ids {
        conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])
            .map_err(|error| format!("删除过期 SuperAI 账号失败: {error}"))?;
    }
    Ok((expired_ids, current_was_expired))
}

/// 顶层清理入口：删除过期 SuperAI 账号；如果删掉的是"当前"账号，自动从
/// 剩余可用 SuperAI 账号里挑一个接管为新的当前账号；没有可用账号就静默。
///
/// 选择策略：剩余 windsurf 账号里，先排除已耗尽（公开版本地累计 100%）的，
/// 再按 `license_expires_at` 升序——优先用快到期的，确保过期前能榨干。
fn cleanup_expired_windsurf_and_handoff(app: &tauri::AppHandle) -> Result<usize, String> {
    let conn = open_app_db(app)?;
    let (expired_ids, current_was_expired) = delete_expired_windsurf_accounts(app, &conn)?;
    if expired_ids.is_empty() {
        return Ok(0);
    }

    // 通知前端：后台 cleanup 删掉了账号，UI 需要 re-fetch 列表，
    // 否则已过期卡片会一直残留直到用户手动刷新。
    let _ = app.emit(
        "accounts-expired-removed",
        serde_json::json!({
            "ids": expired_ids,
            "count": expired_ids.len(),
        }),
    );

    schedule_windsurf_sync(app.clone());

    if current_was_expired {
        // 在剩余账号里挑一个可用的接管。完全版的 windsurf 账号没有
        // license_expires_at，排序时排在最后即可；公开版按 license 升序。
        let remaining = read_accounts_from_conn(&conn)?;
        let mut candidates: Vec<ManagedAccount> = remaining
            .into_iter()
            .filter(|account| account.provider == "windsurf" && !public_usage_is_exhausted(account))
            .collect();
        candidates.sort_by_key(|account| windsurf_license_expires_at(account).unwrap_or(i64::MAX));

        if let Some(next) = candidates.into_iter().next() {
            // 写 DB current 标记 + 通知 sidecar 切到这个账号。
            if let Err(error) = set_account_current_state(&conn, "windsurf", &next.id) {
                eprintln!("[cleanup] 标记新当前账号失败: {error}");
            }
            if let Err(error) = activate_windsurf_account_for_api(app, &next) {
                eprintln!("[cleanup] 启用接管账号失败: {error}");
            }
        }
    }

    Ok(expired_ids.len())
}

/// 后台定时任务：每 5 秒清扫已过期的 SuperAI 账号。
///
/// 性能：单次 = 一次 DB 读 + 每行 AES 解密 + 数值比较，纯本地无网络。
/// 50 个账号约 50ms，CPU 占用 < 1%；典型用户基本无感。
/// 没有过期账号时早返回，不写 DB、不通知 sidecar，开销近零。
///
/// 5s 间隔是为了日卡场景下到期能尽快下架，避免用户已到期还能多用 1 分钟。
/// 启动时 setup 也会立即同步跑一次，避免新启动时残留过期账号。
fn spawn_expired_windsurf_cleanup(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // 启动后稍等几秒，避开 setup 阶段对 DB 的写竞争。
        std::thread::sleep(std::time::Duration::from_secs(5));
        loop {
            if let Err(error) = cleanup_expired_windsurf_and_handoff(&app) {
                eprintln!("[cleanup] 清理过期 SuperAI 账号失败: {error}");
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

fn encrypt_plain_windsurf_accounts(conn: &Connection) -> Result<usize, String> {
    // DB 迁移后 provider 列值是 "superai"，但老数据可能还残留 "windsurf"，
    // 两种都扫一遍，确保新老 DB 都能正确加密。
    let mut stmt = conn
        .prepare("SELECT id, account_json FROM accounts WHERE provider IN ('windsurf', 'superai')")
        .map_err(|error| format!("读取 SuperAI 账号记录失败: {error}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("读取 SuperAI 账号记录失败: {error}"))?;
    let mut migrated = 0usize;
    for row in rows {
        let (id, account_json) =
            row.map_err(|error| format!("读取 SuperAI 账号记录失败: {error}"))?;
        let value: Value = match serde_json::from_str(&account_json) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value
            .get("encrypted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        // 明文 account_json 里 provider 字段是内部协议值（"windsurf"）。
        if value.get("provider").and_then(Value::as_str) != Some("windsurf") {
            continue;
        }
        let account = serde_json::from_value::<ManagedAccount>(value)
            .map_err(|error| format!("解析 SuperAI 明文账号失败: {error}"))?;
        let encrypted_json = serialize_account_for_storage(&account)?;
        conn.execute(
            "UPDATE accounts SET email = ?1, display_name = NULL, account_json = ?2 WHERE id = ?3",
            params![account.id, encrypted_json, id],
        )
        .map_err(|error| format!("迁移 SuperAI 加密账号失败: {error}"))?;
        migrated += 1;
    }
    Ok(migrated)
}

fn parse_stored_account_json(account_json: &str) -> Result<ManagedAccount, String> {
    let value: Value =
        serde_json::from_str(account_json).map_err(|error| format!("解析账号记录失败: {error}"))?;
    // wrapper.provider 老数据是 "windsurf"，新数据是 "superai"。两种都接受，
    // 升级用户的历史 DB 不会因为换 wrapper 标识而读不出来。
    let is_encrypted_superai_wrapper = value
        .get("encrypted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && matches!(
            value.get("provider").and_then(Value::as_str),
            Some("windsurf") | Some("superai")
        );
    if is_encrypted_superai_wrapper {
        let payload = value
            .get("payload")
            .and_then(Value::as_str)
            .ok_or_else(|| "SuperAI 加密账号记录缺少 payload".to_string())?;
        let decrypted = superai_decrypt_text(payload)?;
        serde_json::from_str::<ManagedAccount>(&decrypted)
            .map_err(|error| format!("解析 SuperAI 加密账号记录失败: {error}"))
    } else {
        serde_json::from_value::<ManagedAccount>(value)
            .map_err(|error| format!("解析账号记录失败: {error}"))
    }
}

fn is_current_status(status: &Option<AccountStatus>) -> bool {
    status
        .as_ref()
        .map(|status| status.state == "available" && status.label == "当前")
        .unwrap_or(false)
}

fn mark_account_current(account: &mut ManagedAccount) {
    account.status = Some(AccountStatus {
        state: "available".to_string(),
        label: "当前".to_string(),
        reason: None,
        updated_at: Some(now_ts()),
    });
}

fn mark_account_available(account: &mut ManagedAccount) {
    account.status = Some(AccountStatus {
        state: "available".to_string(),
        label: "可用".to_string(),
        reason: None,
        updated_at: Some(now_ts()),
    });
}

fn account_for_frontend(account: &ManagedAccount) -> ManagedAccount {
    let mut redacted = account.clone();
    // IPC 出口处把内部协议名（"windsurf"）改写成对前端透明的 "superai"。
    // 配合 upsert_accounts 入口的反向归一化，前端 DevTools 监听 IPC 也只
    // 能看到 "superai"，看不到协议代号。
    if redacted.provider == "windsurf" {
        redacted.provider = "superai".to_string();
    }
    // 公开版下的 windsurf 账号：有效期展示**每次从 batch_key 现解**（密钥本体被
    // AES 封死，用户即使解开外层 account_json 改 license_expires_at 也无效）。
    // 现解失败再退回缓存的 license_expires_at 兜底；都没有就不改。
    // 完全版直接用 DB 里的 subscription_active_until（= 上游 plan_end）。
    if is_public_build() && account.provider == "windsurf" {
        let payload = account.auth_payload.as_ref().and_then(Value::as_object);
        let derived = payload
            .and_then(|payload| payload.get("batch_key"))
            .and_then(Value::as_str)
            .and_then(|key| parse_windsurf_batch_key_line(key).ok())
            .map(|credential| credential.expires_at)
            .filter(|expires_at| *expires_at != i64::MAX)
            .map(|expires_at| Value::Number(expires_at.into()));
        let cached = payload
            .and_then(|payload| payload.get("license_expires_at"))
            .cloned();
        if let Some(value) = derived.or(cached) {
            redacted.subscription_active_until = Some(value);
        }

        // 用户用 DevTools 监听 Tauri IPC 也只能看到 hash 占位，看不到任何
        // 真实邮箱 / 显示名 / 上游 plan_name / windsurf 错误原文。前端 UI
        // 已经 publicAccountCode() 派生 SUPERAI-XXXXX，与这里清空互不冲突。
        redacted.email = String::new();
        redacted.display_name = None;
        redacted.account_name = None;
        redacted.plan = None;
        redacted.plan_type = None;
        redacted.auth_file_plan_type = None;
        // account_id / user_id 留 None，前端 fallback 到 account.id（DB 里
        // 是稳定 hash，本身不含敏感串）。
        redacted.account_id = None;
        redacted.user_id = None;
        // status / quota 里的 reason / error 字段可能直接来自 windsurf 上游
        // 报错原文，里面常含 windsurf.com、内部 plan_name 等可识别串。
        if let Some(status) = redacted.status.as_mut() {
            status.reason = None;
        }
        if let Some(quota) = redacted.quota.as_mut() {
            quota.error = None;
            // 兜底：剔除所有 `windsurf-*` metric key（windsurf-daily / -weekly /
            // -credits 等）。常规路径下 `rewrite_quota_for_public_usage` 已经把
            // metrics 替换成单一 `superai-public`，但若上一次 refresh 失败 / 旧
            // 数据迁移残留，这里再砍一次确保 IPC 输出干净。
            quota
                .metrics
                .retain(|metric| !metric.key.starts_with("windsurf-"));
        }
    }
    redacted.auth_payload = None;
    redacted
}

fn accounts_for_frontend(accounts: Vec<ManagedAccount>) -> Vec<ManagedAccount> {
    accounts
        .into_iter()
        .map(|account| account_for_frontend(&account))
        .collect()
}

fn import_result_for_frontend(mut result: ImportResult) -> ImportResult {
    result.imported = accounts_for_frontend(result.imported);
    result
}

fn enforce_single_current_account(conn: &Connection) -> Result<(), String> {
    let mut accounts = read_accounts_from_conn(conn)?;
    let mut keep_by_provider: HashMap<String, String> = HashMap::new();
    for provider in ["codex", "gemini", "windsurf"] {
        let mut current_ids = accounts
            .iter()
            .filter(|account| account.provider == provider && is_current_status(&account.status))
            .map(|account| (account.id.clone(), account.updated_at))
            .collect::<Vec<_>>();
        if current_ids.len() <= 1 {
            continue;
        }
        current_ids.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        keep_by_provider.insert(provider.to_string(), current_ids[0].0.clone());
    }

    if keep_by_provider.is_empty() {
        return Ok(());
    }

    for account in &mut accounts {
        if keep_by_provider
            .get(&account.provider)
            .is_some_and(|keep_id| keep_id != &account.id)
            && is_current_status(&account.status)
        {
            mark_account_available(account);
            let account_json = serialize_account_for_storage(account)?;
            conn.execute(
                "UPDATE accounts SET account_json = ?1, updated_at = ?2 WHERE id = ?3",
                params![account_json, account.updated_at, account.id],
            )
            .map_err(|error| format!("清理当前账号状态失败: {error}"))?;
        }
    }
    Ok(())
}

fn load_account_from_db(conn: &Connection, account_id: &str) -> Result<ManagedAccount, String> {
    let account_json = conn
        .query_row(
            "SELECT account_json FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("账号不存在或读取失败: {error}"))?;
    parse_stored_account_json(&account_json)
}

fn upsert_account(conn: &Connection, account: &ManagedAccount) -> Result<(), String> {
    let mut account_to_write = account.clone();
    if account_to_write.auth_payload.is_none() {
        if let Ok(existing) = load_account_from_db(conn, &account.id) {
            if existing.provider == account.provider {
                account_to_write.auth_payload = existing.auth_payload;
            }
        }
    }
    if !is_current_status(&account_to_write.status) {
        if let Ok(existing) = load_account_from_db(conn, &account.id) {
            if existing.provider == account.provider && is_current_status(&existing.status) {
                mark_account_current(&mut account_to_write);
            }
        }
    }
    if is_current_status(&account_to_write.status) {
        let mut accounts = read_accounts_from_conn(conn)?;
        for existing in &mut accounts {
            if existing.provider == account_to_write.provider
                && existing.id != account_to_write.id
                && is_current_status(&existing.status)
            {
                mark_account_available(existing);
                let existing_json = serialize_account_for_storage(existing)?;
                conn.execute(
                    "UPDATE accounts SET account_json = ?1, updated_at = ?2 WHERE id = ?3",
                    params![existing_json, existing.updated_at, existing.id],
                )
                .map_err(|error| format!("清理当前账号状态失败: {error}"))?;
            }
        }
    }
    let account_json = serialize_account_for_storage(&account_to_write)?;
    let stored_email = if account_to_write.provider == "windsurf" {
        account_to_write.id.clone()
    } else {
        account_to_write.email.clone()
    };
    let stored_display_name = if account_to_write.provider == "windsurf" {
        None
    } else {
        account_to_write.display_name.clone()
    };
    // DB 列里只暴露对外可见的 provider 名（windsurf → superai）。内部代码、协议
    // 字面量、加密的 account_json 内部仍保留原值；这里只动 sqlite cli 直接能看
    // 到的 provider 列，避免用户开 DB 文件就看到协议代号。
    let stored_provider = redact_provider_to_storage(&account_to_write.provider);
    conn.execute(
        r#"
      INSERT INTO accounts (
        id, provider, email, display_name, account_json, created_at, updated_at
      ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
      ON CONFLICT(id) DO UPDATE SET
        provider = excluded.provider,
        email = excluded.email,
        display_name = excluded.display_name,
        account_json = excluded.account_json,
        updated_at = excluded.updated_at
      "#,
        params![
            account_to_write.id,
            stored_provider,
            stored_email,
            stored_display_name,
            account_json,
            account_to_write.created_at,
            account_to_write.updated_at
        ],
    )
    .map_err(|error| format!("写入账号 SQLite 失败: {error}"))?;
    Ok(())
}

/// 把代码内部使用的 provider 字面量改写成 DB 列里要存的对外可见值。
/// 目前只有 "windsurf" → "superai"；其它（codex、gemini）原样返回。
fn redact_provider_to_storage(provider: &str) -> String {
    if provider == "windsurf" {
        "superai".to_string()
    } else {
        provider.to_string()
    }
}

/// 把前端 / IPC 入参中的 provider 字符串归一化回内部协议字面量。
/// 前端通过 account_for_frontend 看到的是 "superai"；当它把这个值通过命令参数
/// 或 ManagedAccount 字段回传时，必须翻译回 "windsurf"，否则内部 provider
/// 分发（codex / gemini / windsurf 三路 match）会全部漏掉 SuperAI 账号。
fn normalize_provider_from_frontend(provider: &str) -> String {
    if provider == "superai" {
        "windsurf".to_string()
    } else {
        provider.to_string()
    }
}

fn serialize_account_for_storage(account: &ManagedAccount) -> Result<String, String> {
    let account_json =
        serde_json::to_string(account).map_err(|error| format!("序列化账号失败: {error}"))?;
    if account.provider != "windsurf" {
        return Ok(account_json);
    }
    let encrypted = superai_encrypt_text(&account_json)?;
    // wrapper 里只是个路由标识，跟解密后的内部 provider 解耦。用 "superai"
    // 让用户即使绕过外层 AES 看到 wrapper JSON，也不会看到协议代号。
    let wrapper = serde_json::json!({
        "encrypted": true,
        "provider": "superai",
        "payload": encrypted,
    });
    serde_json::to_string(&wrapper).map_err(|error| format!("序列化 SuperAI 加密账号失败: {error}"))
}

fn upsert_accounts_into_db(
    app: &tauri::AppHandle,
    accounts: &[ManagedAccount],
) -> Result<(), String> {
    if accounts.is_empty() {
        return Ok(());
    }
    let mut conn = open_app_db(app)?;
    let tx = conn
        .transaction()
        .map_err(|error| format!("开启 SQLite 事务失败: {error}"))?;
    for account in accounts {
        upsert_account(&tx, account)?;
    }
    enforce_single_current_account(&tx)?;
    tx.commit()
        .map_err(|error| format!("提交 SQLite 事务失败: {error}"))?;
    drop(conn);
    // 若有 SuperAI 账号且 API 服务正在跑，把账号推送给 sidecar 让它能用最新凭据。
    if accounts.iter().any(|a| a.provider == "windsurf") {
        schedule_windsurf_sync(app.clone());
    }
    Ok(())
}

fn account_exists(conn: &Connection, account_id: &str) -> Result<bool, String> {
    let count = conn
        .query_row(
            "SELECT COUNT(1) FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("检查账号是否存在失败: {error}"))?;
    Ok(count > 0)
}

fn upsert_existing_accounts_into_db(
    app: &tauri::AppHandle,
    accounts: &[ManagedAccount],
) -> Result<Vec<ManagedAccount>, String> {
    let mut conn = open_app_db(app)?;
    let tx = conn
        .transaction()
        .map_err(|error| format!("开启 SQLite 事务失败: {error}"))?;
    let mut written = Vec::new();
    for account in accounts {
        if account_exists(&tx, &account.id)? {
            upsert_account(&tx, account)?;
            written.push(account.clone());
        }
    }
    enforce_single_current_account(&tx)?;
    tx.commit()
        .map_err(|error| format!("提交 SQLite 事务失败: {error}"))?;
    if written.iter().any(|account| account.provider == "windsurf") {
        schedule_windsurf_sync(app.clone());
    }
    Ok(written)
}

fn stable_hash(input: &str) -> String {
    let mut hash: u32 = 2166136261;
    for byte in input.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16777619);
    }
    format!("{hash:08x}")
}

fn string_field(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn number_field(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(num)) => num.as_i64(),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn normalize_unix_seconds_str(value: &str) -> Option<i64> {
    let parsed = value.trim().parse::<i64>().ok()?;
    if parsed > 1_000_000_000_000 {
        Some(parsed / 1000)
    } else if parsed > 0 {
        Some(parsed)
    } else {
        None
    }
}

fn normalize_unix_seconds_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().and_then(|value| {
            if value > 1_000_000_000_000 {
                Some(value / 1000)
            } else if value > 0 {
                Some(value)
            } else {
                None
            }
        }),
        Value::String(text) => normalize_unix_seconds_str(text),
        _ => None,
    }
}

/// 把 Connect-RPC JSON 里各种形态的时间戳吃成 Unix 秒：
/// - 数字（秒或毫秒）/ 数字串
/// - RFC3339 字符串，如 "2025-11-15T13:34:50Z"（protobuf well-known Timestamp 默认编码）
/// - 对象 `{seconds: <num|str>, nanos: <num>}`（少数 grpc-gateway 实现）
fn coerce_unix_seconds(value: &Value) -> Option<i64> {
    if let Some(seconds) = normalize_unix_seconds_value(value) {
        return Some(seconds);
    }
    if let Some(text) = value.as_str() {
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text.trim()) {
            return Some(parsed.timestamp());
        }
    }
    if let Some(obj) = value.as_object() {
        if let Some(seconds) = obj.get("seconds").and_then(normalize_unix_seconds_value) {
            return Some(seconds);
        }
    }
    None
}

fn windsurf_license_expired_at(expires_at: i64) -> bool {
    expires_at != i64::MAX && now_ts() / 60 >= expires_at / 60
}

fn windsurf_license_expires_at(account: &ManagedAccount) -> Option<i64> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    // 优先按 UI 同款逻辑现解 batch_key，避免 auth_payload 里缓存的
    // license_expires_at 字段缺失 / 滞后导致 cleanup 漏删（UI 已经显示"已过期"
    // 但后端读不到缓存值就以为没过期，账号永远不会被自动清掉）。
    let derived = payload
        .get("batch_key")
        .and_then(Value::as_str)
        .and_then(|key| parse_windsurf_batch_key_line(key).ok())
        .map(|credential| credential.expires_at)
        .filter(|expires_at| *expires_at != i64::MAX);
    if derived.is_some() {
        return derived;
    }
    payload
        .get("license_expires_at")
        .and_then(normalize_unix_seconds_value)
}

fn apply_windsurf_license_expiry(account: &mut ManagedAccount) {
    if !is_public_build() || account.provider != "windsurf" {
        return;
    }
    if let Some(expires_at) = windsurf_license_expires_at(account) {
        account.subscription_active_until = Some(Value::Number(expires_at.into()));
    }
}

fn superai_aes_key_iv() -> Result<([u8; 32], [u8; 16]), String> {
    let key = hex::decode(SUPERAI_AES_KEY_HEX)
        .map_err(|error| format!("解析 SuperAI AES key 失败: {error}"))?;
    let iv = hex::decode(SUPERAI_AES_IV_HEX)
        .map_err(|error| format!("解析 SuperAI AES iv 失败: {error}"))?;
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| "SuperAI AES key 长度必须为 32 字节".to_string())?;
    let iv: [u8; 16] = iv
        .try_into()
        .map_err(|_| "SuperAI AES iv 长度必须为 16 字节".to_string())?;
    Ok((key, iv))
}

fn superai_encrypt_text(plain: &str) -> Result<String, String> {
    type Aes256CbcEnc = cbc::Encryptor<Aes256>;
    let (key, iv) = superai_aes_key_iv()?;
    let encrypted = Aes256CbcEnc::new(&key.into(), &iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(plain.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

fn superai_decrypt_text(cipher_text: &str) -> Result<String, String> {
    type Aes256CbcDec = cbc::Decryptor<Aes256>;
    let (key, iv) = superai_aes_key_iv()?;
    let raw = cipher_text.trim();
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .or_else(|_| URL_SAFE_NO_PAD.decode(raw))
        .map_err(|_| "AES 密文不是有效 base64".to_string())?;
    let decrypted = Aes256CbcDec::new(&key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&encrypted)
        .map_err(|_| "AES 解密失败或 PKCS#7 填充无效".to_string())?;
    String::from_utf8(decrypted).map_err(|_| "AES 明文不是有效 UTF-8".to_string())
}

fn bool_field(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(text)) => match text.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn percent_field(value: Option<&Value>) -> Option<i64> {
    number_field(value).map(|value| value.clamp(0, 100))
}

fn parse_jwt_payload(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    parts.next()?;
    let payload = parts.next()?;
    let normalized = payload.replace('-', "+").replace('_', "/");
    let padded = match normalized.len() % 4 {
        2 => format!("{normalized}=="),
        3 => format!("{normalized}="),
        _ => normalized,
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()?;
    serde_json::from_slice::<Value>(&decoded).ok()
}

fn codex_auth_claims(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
}

fn codex_record_key(user_id: &str, account_id: &str) -> String {
    format!("{user_id}::{account_id}")
}

fn quota_state(remaining: Option<i64>) -> String {
    match remaining {
        None => "unknown".to_string(),
        Some(value) if value <= 0 => "unavailable".to_string(),
        Some(value) if value <= 15 => "warning".to_string(),
        Some(_) => "available".to_string(),
    }
}

fn parse_codex_quota(obj: &serde_json::Map<String, Value>) -> Option<AccountQuota> {
    let quota = obj.get("quota").and_then(Value::as_object);
    let quota_error = obj.get("quota_error").and_then(Value::as_object);
    let mut metrics = Vec::new();

    let hourly = percent_field(
        quota
            .and_then(|q| q.get("hourly_percentage"))
            .or_else(|| obj.get("hourly_percentage")),
    );
    let weekly = percent_field(
        quota
            .and_then(|q| q.get("weekly_percentage"))
            .or_else(|| obj.get("weekly_percentage")),
    );

    if let Some(remaining) = hourly {
        let reset_at = quota
            .and_then(|q| q.get("hourly_reset_time"))
            .or_else(|| obj.get("hourly_reset_time"))
            .cloned();
        metrics.push(QuotaMetric {
            key: "codex-5h".to_string(),
            label: "5H".to_string(),
            remaining_percent: Some(remaining),
            reset_at,
            detail: None,
            state: Some(quota_state(Some(remaining))),
        });
    }

    if let Some(remaining) = weekly {
        let reset_at = quota
            .and_then(|q| q.get("weekly_reset_time"))
            .or_else(|| obj.get("weekly_reset_time"))
            .cloned();
        metrics.push(QuotaMetric {
            key: "codex-weekly".to_string(),
            label: "周限".to_string(),
            remaining_percent: Some(remaining),
            reset_at,
            detail: None,
            state: Some(quota_state(Some(remaining))),
        });
    }

    let error = quota_error
        .and_then(|q| string_field(q.get("message")))
        .or_else(|| string_field(obj.get("quota_query_last_error")));
    let is_forbidden = bool_field(
        quota
            .and_then(|q| q.get("is_forbidden"))
            .or_else(|| obj.get("is_forbidden")),
    )
    .unwrap_or(false);

    if metrics.is_empty() && error.is_none() && !is_forbidden {
        return None;
    }

    Some(AccountQuota {
        metrics,
        last_updated: number_field(obj.get("usage_updated_at"))
            .or_else(|| quota.and_then(|q| number_field(q.get("last_updated")))),
        error,
        is_forbidden: Some(is_forbidden),
    })
}

fn parse_gemini_quota(obj: &serde_json::Map<String, Value>) -> Option<AccountQuota> {
    let raw = obj.get("gemini_usage_raw").and_then(Value::as_object);
    let models = raw
        .and_then(|r| r.get("models"))
        .and_then(Value::as_array)
        .or_else(|| obj.get("models").and_then(Value::as_array));
    let buckets = raw.and_then(|r| r.get("buckets")).and_then(Value::as_array);
    let mut metrics = Vec::new();

    if let Some(buckets) = buckets {
        let mut picked: HashMap<String, QuotaMetric> = HashMap::new();
        for item in buckets {
            let Some(bucket) = item.as_object() else {
                continue;
            };
            let Some(model_id) = string_field(bucket.get("modelId"))
                .or_else(|| string_field(bucket.get("model_id")))
            else {
                continue;
            };
            let remaining_fraction = match bucket
                .get("remainingFraction")
                .or_else(|| bucket.get("remaining_fraction"))
            {
                Some(Value::Number(num)) => num.as_f64(),
                Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
                _ => None,
            };
            let Some(remaining_fraction) = remaining_fraction else {
                continue;
            };
            let remaining = (remaining_fraction * 100.0).round().clamp(0.0, 100.0) as i64;
            let lower = model_id.to_ascii_lowercase();
            let (key, label) = if lower.contains("pro") {
                ("gemini-pro".to_string(), "PRO".to_string())
            } else if lower.contains("flash") {
                ("gemini-flash".to_string(), "FLASH".to_string())
            } else {
                (
                    format!("gemini-{}", metrics.len() + picked.len()),
                    model_id.clone(),
                )
            };
            let metric = QuotaMetric {
                key: key.clone(),
                label,
                remaining_percent: Some(remaining),
                reset_at: bucket
                    .get("resetTime")
                    .or_else(|| bucket.get("reset_time"))
                    .cloned(),
                detail: Some(format!("{model_id} 剩余 {remaining}%")),
                state: Some(quota_state(Some(remaining))),
            };
            match picked.get(&key) {
                Some(existing) if existing.remaining_percent.unwrap_or(101) <= remaining => {}
                _ => {
                    picked.insert(key, metric);
                }
            }
        }
        let mut values = picked.into_values().collect::<Vec<_>>();
        values.sort_by(|left, right| left.key.cmp(&right.key));
        metrics.extend(values);
    }

    if let Some(models) = models {
        for item in models {
            let Some(model) = item.as_object() else {
                continue;
            };
            let remaining = percent_field(
                model
                    .get("percentage")
                    .or_else(|| model.get("remainingPercent"))
                    .or_else(|| model.get("remaining_percent")),
            );
            let label = string_field(model.get("display_name"))
                .or_else(|| string_field(model.get("displayName")))
                .or_else(|| string_field(model.get("name")))
                .unwrap_or_else(|| format!("MODEL {}", metrics.len() + 1));
            metrics.push(QuotaMetric {
                key: format!("gemini-{}", metrics.len()),
                label,
                remaining_percent: remaining,
                reset_at: model
                    .get("reset_time")
                    .or_else(|| model.get("resetTime"))
                    .cloned(),
                detail: None,
                state: Some(quota_state(remaining)),
            });
        }
    }

    if metrics.is_empty() {
        if let Some(total_used) = percent_field(
            raw.and_then(|r| r.get("totalPercentUsed"))
                .or_else(|| raw.and_then(|r| r.get("total_percent_used")))
                .or_else(|| obj.get("totalPercentUsed")),
        ) {
            let remaining = 100 - total_used;
            metrics.push(QuotaMetric {
                key: "gemini-total".to_string(),
                label: "TOTAL".to_string(),
                remaining_percent: Some(remaining),
                reset_at: None,
                detail: None,
                state: Some(quota_state(Some(remaining))),
            });
        }
    }

    let error = string_field(obj.get("quota_query_last_error"));
    if metrics.is_empty() && error.is_none() {
        return None;
    }

    Some(AccountQuota {
        metrics,
        last_updated: number_field(obj.get("usage_updated_at")),
        error,
        is_forbidden: None,
    })
}

fn derive_status(
    obj: &serde_json::Map<String, Value>,
    token_meta: &TokenMeta,
    quota: Option<&AccountQuota>,
) -> AccountStatus {
    let raw_status = string_field(obj.get("status")).map(|value| value.to_lowercase());
    let reason = string_field(obj.get("status_reason"))
        .or_else(|| string_field(obj.get("reauth_reason")))
        .or_else(|| quota.and_then(|q| q.error.clone()));
    let now = now_ts();

    if bool_field(obj.get("requires_reauth")).unwrap_or(false)
        || matches!(raw_status.as_deref(), Some("unavailable" | "disabled"))
        || quota.and_then(|q| q.is_forbidden).unwrap_or(false)
    {
        return AccountStatus {
            state: "unavailable".to_string(),
            label: "不可用".to_string(),
            reason: reason.or_else(|| Some("账号需要重新授权或访问被拒绝".to_string())),
            updated_at: number_field(obj.get("usage_updated_at")),
        };
    }

    if !token_meta.has_access_token {
        return AccountStatus {
            state: "unavailable".to_string(),
            label: "不可用".to_string(),
            reason: Some("缺少 access token".to_string()),
            updated_at: None,
        };
    }

    if token_meta
        .expires_at
        .is_some_and(|expires_at| expires_at <= now)
        && !token_meta.has_refresh_token
    {
        return AccountStatus {
            state: "unavailable".to_string(),
            label: "不可用".to_string(),
            reason: Some("本地 token 已过期".to_string()),
            updated_at: None,
        };
    }

    if let Some(error) = quota.and_then(|q| q.error.clone()) {
        return AccountStatus {
            state: "unavailable".to_string(),
            label: "不可用".to_string(),
            reason: Some(error),
            updated_at: quota.and_then(|q| q.last_updated),
        };
    }

    if let Some(status) = raw_status {
        if !matches!(status.as_str(), "active" | "available" | "ok") {
            return AccountStatus {
                state: "unavailable".to_string(),
                label: "不可用".to_string(),
                reason,
                updated_at: number_field(obj.get("usage_updated_at")),
            };
        }
    }

    AccountStatus {
        state: "available".to_string(),
        label: "可用".to_string(),
        reason: None,
        updated_at: quota.and_then(|q| q.last_updated),
    }
}

fn parse_codex_account(value: &Value, source: &str) -> Option<ManagedAccount> {
    let obj = value.as_object()?;
    let tokens = obj.get("tokens").and_then(Value::as_object);

    let id_token = string_field(
        tokens
            .and_then(|t| t.get("id_token"))
            .or_else(|| obj.get("id_token")),
    );
    let access_token = string_field(
        tokens
            .and_then(|t| t.get("access_token"))
            .or_else(|| obj.get("access_token")),
    );
    let refresh_token = string_field(
        tokens
            .and_then(|t| t.get("refresh_token"))
            .or_else(|| obj.get("refresh_token")),
    );
    let api_key = string_field(obj.get("OPENAI_API_KEY"));
    let auth_mode = string_field(obj.get("auth_mode"))
        .unwrap_or_default()
        .to_lowercase();

    if id_token.is_none() && access_token.is_none() && api_key.is_none() && auth_mode != "apikey" {
        return None;
    }

    let jwt = id_token.as_deref().and_then(parse_jwt_payload);
    let auth = jwt.as_ref().and_then(codex_auth_claims);

    let email = string_field(obj.get("email"))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("email"))))
        .or_else(|| {
            api_key
                .as_ref()
                .map(|k| format!("api-key-{}@local", &stable_hash(k)[..6]))
        })?;

    let account_id = string_field(
        tokens
            .and_then(|t| t.get("account_id"))
            .or_else(|| obj.get("account_id")),
    )
    .or_else(|| auth.and_then(|a| string_field(a.get("chatgpt_account_id"))));
    let user_id = string_field(obj.get("user_id"))
        .or_else(|| auth.and_then(|a| string_field(a.get("chatgpt_user_id"))))
        .or_else(|| auth.and_then(|a| string_field(a.get("user_id"))));
    if api_key.is_none() {
        let token_account_id = string_field(
            tokens
                .and_then(|t| t.get("account_id"))
                .or_else(|| obj.get("account_id")),
        )?;
        let jwt_account_id = auth.and_then(|a| string_field(a.get("chatgpt_account_id")))?;
        if token_account_id != jwt_account_id {
            return None;
        }
        user_id.as_ref()?;
    }
    let plan = string_field(obj.get("plan_type"))
        .or_else(|| auth.and_then(|a| string_field(a.get("chatgpt_plan_type"))))
        .or_else(|| api_key.as_ref().map(|_| "API Key".to_string()));
    let auth_file_plan_type = string_field(obj.get("auth_file_plan_type"))
        .or_else(|| string_field(obj.get("authFilePlanType")));
    let subscription_active_until = obj
        .get("subscription_active_until")
        .or_else(|| obj.get("subscriptionActiveUntil"))
        .cloned()
        .or_else(|| {
            auth.and_then(|a| a.get("chatgpt_subscription_active_until"))
                .cloned()
        });
    let organization_id = auth.and_then(|a| {
        string_field(a.get("organization_id"))
            .or_else(|| string_field(a.get("chatgpt_organization_id")))
            .or_else(|| string_field(a.get("org_id")))
    });
    let record_key = user_id
        .as_deref()
        .zip(account_id.as_deref())
        .map(|(user_id, account_id)| codex_record_key(user_id, account_id));
    let discriminator = record_key
        .clone()
        .or_else(|| api_key.clone())
        .unwrap_or_else(|| email.clone());
    let now = now_ts();
    let token_meta = TokenMeta {
        has_access_token: access_token.is_some() || api_key.is_some(),
        has_refresh_token: refresh_token.is_some(),
        has_id_token: id_token.is_some(),
        expires_at: jwt.as_ref().and_then(|j| number_field(j.get("exp"))),
    };
    let quota = parse_codex_quota(obj);
    let status = derive_status(obj, &token_meta, quota.as_ref());

    Some(ManagedAccount {
        id: string_field(obj.get("id"))
            .unwrap_or_else(|| format!("codex_{}", stable_hash(&discriminator))),
        provider: "codex".to_string(),
        email: email.to_lowercase(),
        display_name: string_field(obj.get("account_name"))
            .or_else(|| string_field(obj.get("name"))),
        account_name: string_field(obj.get("account_name")),
        organization_id,
        plan: plan.clone(),
        plan_type: plan,
        auth_file_plan_type,
        subscription_active_until,
        account_id,
        user_id,
        source: source.to_string(),
        token_meta,
        status: Some(status),
        quota,
        created_at: number_field(obj.get("created_at")).unwrap_or(now),
        updated_at: number_field(obj.get("last_used"))
            .or_else(|| number_field(obj.get("updated_at")))
            .unwrap_or(now),
        auth_payload: Some(value.clone()),
    })
}

fn parse_gemini_account(value: &Value, source: &str) -> Option<ManagedAccount> {
    let obj = value.as_object()?;
    let token = obj.get("token").and_then(Value::as_object);

    let access_token = string_field(obj.get("access_token"))
        .or_else(|| token.and_then(|t| string_field(t.get("access_token"))));
    let refresh_token = string_field(obj.get("refresh_token"))
        .or_else(|| token.and_then(|t| string_field(t.get("refresh_token"))));
    let id_token = string_field(obj.get("id_token"))
        .or_else(|| token.and_then(|t| string_field(t.get("id_token"))));

    if access_token.is_none() && refresh_token.is_none() && id_token.is_none() {
        return None;
    }

    let jwt = id_token.as_deref().and_then(parse_jwt_payload);
    let email = string_field(obj.get("email"))
        .or_else(|| string_field(obj.get("active")))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("email"))))
        .or_else(|| string_field(obj.get("account")))?;

    let auth_id = string_field(obj.get("auth_id"))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("sub"))));
    let expires_at = number_field(obj.get("expiry_date"))
        .or_else(|| token.and_then(|t| number_field(t.get("expires_at"))))
        .or_else(|| jwt.as_ref().and_then(|j| number_field(j.get("exp"))));
    let plan_type = string_field(obj.get("plan_type"))
        .or_else(|| string_field(obj.get("plan_name")))
        .or_else(|| string_field(obj.get("tier_name")));
    let now = now_ts();
    let token_meta = TokenMeta {
        has_access_token: access_token.is_some(),
        has_refresh_token: refresh_token.is_some(),
        has_id_token: id_token.is_some(),
        expires_at,
    };
    let quota = parse_gemini_quota(obj);
    let status = derive_status(obj, &token_meta, quota.as_ref());

    Some(ManagedAccount {
        id: string_field(obj.get("id")).unwrap_or_else(|| {
            format!(
                "gemini_{}",
                stable_hash(&format!(
                    "{}::{}",
                    email.to_lowercase(),
                    auth_id
                        .clone()
                        .unwrap_or_else(|| access_token.clone().unwrap_or(email.clone()))
                ))
            )
        }),
        provider: "gemini".to_string(),
        email: email.to_lowercase(),
        display_name: string_field(obj.get("name")),
        account_name: string_field(obj.get("name")),
        organization_id: None,
        plan: plan_type.clone(),
        plan_type: plan_type.clone(),
        auth_file_plan_type: None,
        subscription_active_until: expires_at.map(|value| Value::Number(value.into())),
        account_id: auth_id.clone(),
        user_id: auth_id,
        source: source.to_string(),
        token_meta,
        status: Some(status),
        quota,
        created_at: number_field(obj.get("created_at")).unwrap_or(now),
        updated_at: number_field(obj.get("last_used"))
            .or_else(|| number_field(obj.get("updated_at")))
            .unwrap_or(now),
        auth_payload: Some(value.clone()),
    })
}

fn looks_like_windsurf(obj: &serde_json::Map<String, Value>) -> bool {
    if string_field(obj.get("provider"))
        .map(|p| p.eq_ignore_ascii_case("windsurf"))
        .unwrap_or(false)
    {
        return true;
    }
    let tokens = obj.get("tokens").and_then(Value::as_object);
    let auth1_token = string_field(obj.get("auth1_token"))
        .or_else(|| string_field(obj.get("auth1Token")))
        .or_else(|| string_field(obj.get("devin_auth1_token")))
        .or_else(|| string_field(obj.get("devinAuth1Token")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("auth1_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("auth1Token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devin_auth1_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devinAuth1Token"))));
    let session_token = string_field(obj.get("session_token"))
        .or_else(|| string_field(obj.get("sessionToken")))
        .or_else(|| string_field(obj.get("devin_session_token")))
        .or_else(|| string_field(obj.get("devinSessionToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("session_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("sessionToken"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devin_session_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devinSessionToken"))));
    if auth1_token
        .as_deref()
        .map(|token| token.starts_with("auth1_"))
        .unwrap_or(false)
        || session_token
            .as_deref()
            .map(|token| token.starts_with("devin-session-token$"))
            .unwrap_or(false)
    {
        return true;
    }
    if string_field(obj.get("local_id"))
        .or_else(|| string_field(obj.get("localId")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("local_id"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("localId"))))
        .is_some()
    {
        return true;
    }
    let refresh_token = string_field(obj.get("refresh_token"))
        .or_else(|| string_field(obj.get("refreshToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("refresh_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("refreshToken"))));
    let id_token = string_field(obj.get("id_token"))
        .or_else(|| string_field(obj.get("idToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("id_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("idToken"))));
    if let Some(jwt) = id_token.as_deref().and_then(parse_jwt_payload) {
        let aud = string_field(jwt.get("aud")).unwrap_or_default();
        let iss = string_field(jwt.get("iss")).unwrap_or_default();
        if aud.contains("exafunction-windsurf") || iss.contains("exafunction-windsurf") {
            return true;
        }
        if let Some(sign_in_provider) = jwt
            .get("firebase")
            .and_then(Value::as_object)
            .and_then(|f| string_field(f.get("sign_in_provider")))
        {
            if sign_in_provider == "password" && refresh_token.is_some() {
                return true;
            }
        }
    }
    false
}

fn parse_windsurf_account(value: &Value, source: &str) -> Option<ManagedAccount> {
    let obj = value.as_object()?;
    if !looks_like_windsurf(obj) {
        return None;
    }
    let tokens = obj.get("tokens").and_then(Value::as_object);

    let id_token = string_field(obj.get("id_token"))
        .or_else(|| string_field(obj.get("idToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("id_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("idToken"))));
    let refresh_token = string_field(obj.get("refresh_token"))
        .or_else(|| string_field(obj.get("refreshToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("refresh_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("refreshToken"))));
    let access_token = string_field(obj.get("access_token"))
        .or_else(|| string_field(obj.get("accessToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("access_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("accessToken"))))
        .or_else(|| id_token.clone());
    let api_key = string_field(obj.get("api_key"))
        .or_else(|| string_field(obj.get("apiKey")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("api_key"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("apiKey"))));
    let auth1_token = string_field(obj.get("auth1_token"))
        .or_else(|| string_field(obj.get("auth1Token")))
        .or_else(|| string_field(obj.get("devin_auth1_token")))
        .or_else(|| string_field(obj.get("devinAuth1Token")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("auth1_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("auth1Token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devin_auth1_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devinAuth1Token"))));
    let session_token = string_field(obj.get("session_token"))
        .or_else(|| string_field(obj.get("sessionToken")))
        .or_else(|| string_field(obj.get("devin_session_token")))
        .or_else(|| string_field(obj.get("devinSessionToken")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("session_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("sessionToken"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devin_session_token"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("devinSessionToken"))));

    if id_token.is_none()
        && refresh_token.is_none()
        && api_key.is_none()
        && auth1_token.is_none()
        && session_token.is_none()
    {
        return None;
    }

    let jwt = id_token.as_deref().and_then(parse_jwt_payload);
    let discriminator_token = session_token
        .clone()
        .or_else(|| auth1_token.clone())
        .or_else(|| api_key.clone())
        .or_else(|| access_token.clone())
        .or_else(|| refresh_token.clone())
        .or_else(|| id_token.clone())
        .unwrap_or_else(|| "windsurf".to_string());
    let email = string_field(obj.get("email"))
        .or_else(|| string_field(obj.get("account")))
        .or_else(|| string_field(obj.get("active")))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("email"))))
        .unwrap_or_else(|| format!("windsurf-{}@local", stable_hash(&discriminator_token)));
    let local_id = string_field(obj.get("local_id"))
        .or_else(|| string_field(obj.get("localId")))
        .or_else(|| tokens.and_then(|t| string_field(t.get("local_id"))))
        .or_else(|| tokens.and_then(|t| string_field(t.get("localId"))))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("user_id"))))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("sub"))));
    let display_name = string_field(obj.get("display_name"))
        .or_else(|| string_field(obj.get("displayName")))
        .or_else(|| string_field(obj.get("name")))
        .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("name"))));
    let expires_at = number_field(obj.get("expires_at"))
        .or_else(|| number_field(obj.get("expiresAt")))
        .or_else(|| tokens.and_then(|t| number_field(t.get("expires_at"))))
        .or_else(|| tokens.and_then(|t| number_field(t.get("expiresAt"))))
        .or_else(|| jwt.as_ref().and_then(|j| number_field(j.get("exp"))));

    let now = now_ts();
    let token_meta = TokenMeta {
        has_access_token: access_token.is_some()
            || api_key.is_some()
            || session_token.is_some()
            || auth1_token.is_some(),
        has_refresh_token: refresh_token.is_some(),
        has_id_token: id_token.is_some(),
        expires_at,
    };
    let discriminator = local_id
        .clone()
        .or_else(|| api_key.clone())
        .or_else(|| session_token.clone())
        .or_else(|| auth1_token.clone())
        .unwrap_or_else(|| email.clone());
    let id = string_field(obj.get("id")).unwrap_or_else(|| {
        format!(
            "windsurf_{}",
            stable_hash(&format!("{}::{}", email.to_lowercase(), discriminator))
        )
    });

    // 规范化存储的凭证 payload，保证 refresh / export 能稳定取出。
    let mut tokens_map = serde_json::Map::new();
    if let Some(value) = id_token.clone() {
        tokens_map.insert("id_token".to_string(), Value::String(value));
    }
    if let Some(value) = refresh_token.clone() {
        tokens_map.insert("refresh_token".to_string(), Value::String(value));
    }
    if let Some(value) = access_token.clone() {
        tokens_map.insert("access_token".to_string(), Value::String(value));
    }
    if let Some(value) = api_key.clone() {
        tokens_map.insert("api_key".to_string(), Value::String(value));
    }
    if let Some(value) = auth1_token.clone() {
        tokens_map.insert("auth1_token".to_string(), Value::String(value));
    }
    if let Some(value) = session_token.clone() {
        tokens_map.insert("session_token".to_string(), Value::String(value));
    }
    if let Some(value) = local_id.clone() {
        tokens_map.insert("local_id".to_string(), Value::String(value));
    }
    if let Some(value) = expires_at {
        tokens_map.insert("expires_at".to_string(), Value::Number(value.into()));
    }
    let mut payload_map = serde_json::Map::new();
    payload_map.insert(
        "provider".to_string(),
        Value::String("windsurf".to_string()),
    );
    payload_map.insert("email".to_string(), Value::String(email.to_lowercase()));
    if let Some(name) = display_name.clone() {
        payload_map.insert("display_name".to_string(), Value::String(name));
    }
    payload_map.insert("tokens".to_string(), Value::Object(tokens_map));
    let auth_payload = Value::Object(payload_map);

    let plan = string_field(obj.get("plan"))
        .or_else(|| string_field(obj.get("plan_type")))
        .or_else(|| string_field(obj.get("planType")));

    let status = AccountStatus {
        state: if token_meta.has_access_token || token_meta.has_refresh_token {
            "available".to_string()
        } else {
            "unavailable".to_string()
        },
        label: if token_meta.has_access_token || token_meta.has_refresh_token {
            "可用".to_string()
        } else {
            "不可用".to_string()
        },
        reason: None,
        updated_at: Some(now),
    };

    Some(ManagedAccount {
        id,
        provider: "windsurf".to_string(),
        email: email.to_lowercase(),
        display_name,
        account_name: None,
        organization_id: None,
        plan: plan.clone(),
        plan_type: plan,
        auth_file_plan_type: None,
        subscription_active_until: expires_at.map(|value| Value::Number(value.into())),
        account_id: local_id.clone(),
        user_id: local_id,
        source: source.to_string(),
        token_meta,
        status: Some(status),
        quota: None,
        created_at: number_field(obj.get("created_at")).unwrap_or(now),
        updated_at: number_field(obj.get("last_used"))
            .or_else(|| number_field(obj.get("updated_at")))
            .unwrap_or(now),
        auth_payload: Some(auth_payload),
    })
}

fn windsurf_payload_string(account: &ManagedAccount, key: &str) -> Option<String> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    let tokens = payload.get("tokens").and_then(Value::as_object);
    string_field(tokens.and_then(|t| t.get(key)).or_else(|| payload.get(key)))
}

fn windsurf_set_token_field(account: &mut ManagedAccount, key: &str, value: Value) {
    let payload = account
        .auth_payload
        .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
    let map = match payload {
        Value::Object(map) => map,
        _ => return,
    };
    let tokens_value = map
        .entry("tokens".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(tokens_map) = tokens_value {
        tokens_map.insert(key.to_string(), value);
    }
}

async fn refresh_windsurf_account_remote(account: &mut ManagedAccount) -> Result<(), String> {
    if account.provider != "windsurf" {
        return Ok(());
    }
    let refresh_token = match windsurf_payload_string(account, "refresh_token") {
        Some(token) => token,
        None => {
            if windsurf_payload_string(account, "api_key").is_some() {
                refresh_windsurf_account_by_api_key(account).await?;
                apply_windsurf_license_expiry(account);
                account.token_meta.has_access_token = true;
                return Ok(());
            }
            if windsurf_payload_string(account, "session_token").is_none() {
                if let Some(auth1_token) = windsurf_payload_string(account, "auth1_token") {
                    let post_auth = windsurf_post_auth(&auth1_token, None).await?;
                    windsurf_set_token_field(
                        account,
                        "session_token",
                        Value::String(post_auth.session_token.clone()),
                    );
                    windsurf_set_token_field(
                        account,
                        "access_token",
                        Value::String(post_auth.session_token),
                    );
                    if let Some(value) = post_auth.auth1_token {
                        windsurf_set_token_field(account, "auth1_token", Value::String(value));
                    }
                    if let Some(value) = post_auth.account_id {
                        windsurf_set_token_field(account, "local_id", Value::String(value));
                    }
                    if let Some(value) = post_auth.primary_org_id {
                        windsurf_set_token_field(account, "primary_org_id", Value::String(value));
                    }
                }
            }
            if windsurf_payload_string(account, "session_token").is_some() {
                enrich_windsurf_account_remote(account).await?;
                apply_windsurf_license_expiry(account);
                account.token_meta.has_access_token = true;
                return Ok(());
            }
            return Err("缺少 refresh_token".to_string());
        }
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 SuperAI 客户端失败: {error}"))?;
    let url = format!("{WINDSURF_FIREBASE_REFRESH_URL}?key={WINDSURF_FIREBASE_API_KEY}");
    let body = format!("grant_type=refresh_token&refresh_token={}", refresh_token);
    let response = client
        .post(&url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .header(
            "Sec-Ch-Ua",
            r#""Chromium";v="142", "Google Chrome";v="142", "Not_A Brand";v="99""#,
        )
        .header("Sec-Ch-Ua-Mobile", "?0")
        .header("Sec-Ch-Ua-Platform", r#""Windows""#)
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "cross-site")
        .header("X-Client-Version", "Chrome/JsCore/11.0.0/FirebaseCore-web")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/")
        .body(body)
        .send()
        .await
        .map_err(|error| format!("SuperAI 凭证刷新请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 SuperAI 刷新响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!("SuperAI 刷新失败 ({status}): {text}"));
    }
    let payload: Value = serde_json::from_str(&text)
        .map_err(|error| format!("解析 SuperAI 刷新响应失败: {error}"))?;
    let id_token = string_field(payload.get("id_token"))
        .ok_or_else(|| "SuperAI 刷新响应缺少 id_token".to_string())?;
    let access_token =
        string_field(payload.get("access_token")).unwrap_or_else(|| id_token.clone());
    let new_refresh_token =
        string_field(payload.get("refresh_token")).unwrap_or(refresh_token.clone());
    let expires_in = number_field(payload.get("expires_in")).unwrap_or(3600);
    let expires_at = now_ts() + expires_in;

    windsurf_set_token_field(account, "id_token", Value::String(id_token.clone()));
    windsurf_set_token_field(account, "access_token", Value::String(access_token));
    windsurf_set_token_field(account, "refresh_token", Value::String(new_refresh_token));
    windsurf_set_token_field(account, "expires_at", Value::Number(expires_at.into()));
    let register = windsurf_register_with_codeium(&id_token).await?;
    windsurf_set_token_field(account, "api_key", Value::String(register.api_key));
    if account.display_name.is_none() {
        account.display_name = register.name;
    }

    account.token_meta = TokenMeta {
        has_access_token: true,
        has_refresh_token: true,
        has_id_token: true,
        expires_at: Some(expires_at),
    };
    account.subscription_active_until = Some(Value::Number(expires_at.into()));
    account.updated_at = now_ts();
    account.status = Some(AccountStatus {
        state: "available".to_string(),
        label: "可用".to_string(),
        reason: None,
        updated_at: Some(account.updated_at),
    });
    let _ = enrich_windsurf_account_remote(account).await;
    apply_windsurf_license_expiry(account);
    Ok(())
}

fn build_windsurf_payload(account: &ManagedAccount) -> Result<Value, String> {
    let payload = account
        .auth_payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| "该账号缺少可导出的 SuperAI 凭证".to_string())?;
    let tokens = payload
        .get("tokens")
        .and_then(Value::as_object)
        .ok_or_else(|| "该账号缺少 tokens 字段".to_string())?;
    let mut tokens_map = serde_json::Map::new();
    for key in [
        "id_token",
        "refresh_token",
        "access_token",
        "auth1_token",
        "session_token",
        "api_key",
        "local_id",
        "expires_at",
    ] {
        if let Some(value) = tokens.get(key) {
            tokens_map.insert(key.to_string(), value.clone());
        }
    }
    let mut result = serde_json::Map::new();
    result.insert(
        "provider".to_string(),
        Value::String("windsurf".to_string()),
    );
    result.insert("email".to_string(), Value::String(account.email.clone()));
    if let Some(name) = account.display_name.clone() {
        result.insert("display_name".to_string(), Value::String(name));
    }
    result.insert("tokens".to_string(), Value::Object(tokens_map));
    Ok(Value::Object(result))
}

async fn windsurf_firebase_sign_in(email: &str, password: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 SuperAI 客户端失败: {error}"))?;
    let url = format!("{WINDSURF_FIREBASE_SIGNIN_URL}?key={WINDSURF_FIREBASE_API_KEY}");
    let body = serde_json::json!({
        "email": email,
        "password": password,
        "returnSecureToken": true,
        "clientType": "CLIENT_TYPE_WEB",
    });
    let response = client
        .post(&url)
        .json(&body)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .header(
            "Sec-Ch-Ua",
            r#""Chromium";v="142", "Google Chrome";v="142", "Not_A Brand";v="99""#,
        )
        .header("Sec-Ch-Ua-Mobile", "?0")
        .header("Sec-Ch-Ua-Platform", r#""Windows""#)
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "cross-site")
        .header("X-Client-Version", "Chrome/JsCore/11.0.0/FirebaseCore-web")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/")
        .send()
        .await
        .map_err(|error| format!("SuperAI 登录请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 SuperAI 登录响应失败: {error}"))?;
    if !status.is_success() {
        if status.as_u16() == 401
            || text.contains("INVALID_LOGIN_CREDENTIALS")
            || text.contains("INVALID_PASSWORD")
        {
            return Err("邮箱或密码错误，或该账号不支持邮箱密码登录".to_string());
        }
        if text.contains("EMAIL_NOT_FOUND") {
            return Err("该邮箱未注册".to_string());
        }
        if text.contains("USER_DISABLED") {
            return Err("该账号已被禁用".to_string());
        }
        if text.contains("TOO_MANY_ATTEMPTS_TRY_LATER") {
            return Err("登录尝试次数过多，请 15-30 分钟后再试".to_string());
        }
        return Err(format!(
            "SuperAI 登录失败 ({status})：{}",
            summarize_windsurf_error_body(&text)
        ));
    }
    serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("解析 SuperAI 登录响应失败: {error}"))
}

async fn windsurf_firebase_lookup(id_token: &str) -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let url = format!("{WINDSURF_FIREBASE_LOOKUP_URL}?key={WINDSURF_FIREBASE_API_KEY}");
    let response = client
        .post(&url)
        .json(&serde_json::json!({ "idToken": id_token }))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "cross-site")
        .header("X-Client-Version", "Chrome/JsCore/11.0.0/FirebaseCore-web")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

async fn windsurf_register_with_codeium(
    id_token: &str,
) -> Result<WindsurfCodeiumRegisterResult, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Codeium 注册客户端失败: {error}"))?;
    let response = client
        .post(WINDSURF_CODEIUM_REGISTER_URL)
        .json(&serde_json::json!({ "firebase_id_token": id_token }))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "*/*")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36",
        )
        .send()
        .await
        .map_err(|error| format!("Codeium 注册请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 Codeium 注册响应失败: {error}"))?;
    if !status.is_success() {
        if status.as_u16() == 401 || text.to_ascii_lowercase().contains("invalid token") {
            return Err("SuperAI 凭证无效或已过期，请重新导入".to_string());
        }
        return Err(format!(
            "Codeium 注册失败 ({status})：{}",
            summarize_windsurf_error_body(&text)
        ));
    }
    let result = serde_json::from_str::<WindsurfCodeiumRegisterResult>(&text)
        .map_err(|error| format!("解析 Codeium 注册响应失败: {error}"))?;
    if result.api_key.trim().is_empty() {
        return Err(format!("Codeium 注册响应缺少 api_key: {text}"));
    }
    Ok(result)
}

fn summarize_windsurf_error_body(text: &str) -> String {
    let parsed = serde_json::from_str::<Value>(text).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| string_field(value.get("message")))
        .unwrap_or_else(|| text.trim().to_string());
    let without_trace = message
        .split("(trace ID:")
        .next()
        .unwrap_or(&message)
        .trim();
    if without_trace.is_empty() {
        "上游服务返回错误".to_string()
    } else {
        without_trace.chars().take(160).collect()
    }
}

fn windsurf_browser_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    headers.insert("Accept-Encoding", HeaderValue::from_static("identity"));
    headers.insert("Origin", HeaderValue::from_static("https://windsurf.com"));
    headers.insert("Referer", HeaderValue::from_static("https://windsurf.com/"));
    headers.insert(
        "Sec-Ch-Ua",
        HeaderValue::from_static(
            r#""Chromium";v="134", "Google Chrome";v="134", "Not-A.Brand";v="99""#,
        ),
    );
    headers.insert("Sec-Ch-Ua-Mobile", HeaderValue::from_static("?0"));
    headers.insert("Sec-Ch-Ua-Platform", HeaderValue::from_static(r#""macOS""#));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("cross-site"));
    headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_2_1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36"));
    headers
}

fn windsurf_auth1_error(text: &str, fallback: &str) -> String {
    let parsed = serde_json::from_str::<Value>(text).ok();
    let detail = parsed
        .as_ref()
        .and_then(|value| value.get("detail"))
        .and_then(|value| {
            if let Some(text) = string_field(Some(value)) {
                Some(text)
            } else {
                value.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            string_field(item.get("msg")).or_else(|| string_field(item.get("type")))
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
            }
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| summarize_windsurf_error_body(text));
    match detail.as_str() {
        "EMAIL_NOT_FOUND" => "该邮箱未注册".to_string(),
        "INVALID_PASSWORD" | "INVALID_LOGIN_CREDENTIALS" | "Invalid email or password" => {
            "邮箱或密码错误".to_string()
        }
        "No password set" | "No password set. Please log in with Google or GitHub." => {
            "该账号没有设置邮箱密码，暂不支持导入".to_string()
        }
        "USER_DISABLED" => "该账号已被禁用".to_string(),
        "TOO_MANY_ATTEMPTS_TRY_LATER" => "登录尝试次数过多，请 15-30 分钟后再试".to_string(),
        "INVALID_EMAIL" => "邮箱格式不正确".to_string(),
        _ if detail.is_empty() => fallback.to_string(),
        _ => detail,
    }
}

async fn windsurf_auth1_password_login(email: &str, password: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 SuperAI Auth1 客户端失败: {error}"))?;
    let response = client
        .post(WINDSURF_AUTH1_PASSWORD_LOGIN_URL)
        .headers(windsurf_browser_headers())
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .map_err(|error| format!("SuperAI Auth1 登录请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 SuperAI Auth1 登录响应失败: {error}"))?;
    if !status.is_success() {
        return Err(windsurf_auth1_error(&text, "SuperAI Auth1 登录失败"));
    }
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("解析 SuperAI Auth1 登录响应失败: {error}"))?;
    string_field(value.get("token")).ok_or_else(|| "SuperAI Auth1 登录响应缺少 token".to_string())
}

fn extract_prefixed_token(text: &str, prefix: &str) -> Option<String> {
    let start = text.find(prefix)?;
    let rest = &text[start..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '$' | '.' | '_' | '-')))
        .unwrap_or(rest.len());
    Some(rest[..end].to_string()).filter(|value| value.len() > prefix.len())
}

fn parse_windsurf_post_auth_body(bytes: &[u8]) -> Result<WindsurfPostAuthResult, String> {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(session_token) = string_field(value.get("sessionToken"))
            .or_else(|| string_field(value.get("session_token")))
        {
            return Ok(WindsurfPostAuthResult {
                session_token,
                auth1_token: string_field(value.get("auth1Token"))
                    .or_else(|| string_field(value.get("auth1_token"))),
                account_id: string_field(value.get("accountId"))
                    .or_else(|| string_field(value.get("account_id"))),
                primary_org_id: string_field(value.get("primaryOrgId"))
                    .or_else(|| string_field(value.get("primary_org_id"))),
                orgs: Vec::new(),
            });
        }
    }
    if let Some(session_token) = extract_prefixed_token(&text, "devin-session-token$") {
        return Ok(WindsurfPostAuthResult {
            session_token,
            auth1_token: extract_prefixed_token(&text, "auth1_"),
            account_id: extract_prefixed_token(&text, "account-"),
            primary_org_id: extract_prefixed_token(&text, "org-"),
            orgs: Vec::new(),
        });
    }
    parse_windsurf_post_auth_response(bytes)
}

fn encode_varint(buf: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        buf.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

fn encode_proto_string_field(buf: &mut Vec<u8>, field_no: u32, value: &str) {
    let tag = (field_no << 3) | 2;
    encode_varint(buf, tag as u64);
    let bytes = value.as_bytes();
    encode_varint(buf, bytes.len() as u64);
    buf.extend_from_slice(bytes);
}

fn decode_varint(bytes: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    let mut i = offset;
    while i < bytes.len() {
        let byte = bytes[i];
        result |= ((byte & 0x7F) as u64) << shift;
        i += 1;
        if byte & 0x80 == 0 {
            return Some((result, i - offset));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn parse_windsurf_org(bytes: &[u8]) -> Option<WindsurfOrg> {
    let mut org = WindsurfOrg::default();
    let mut i = 0;
    while i < bytes.len() {
        let (tag, consumed) = decode_varint(bytes, i)?;
        i += consumed;
        let field_no = (tag >> 3) as u32;
        let wire_type = (tag & 0x7) as u8;
        if wire_type != 2 {
            return None;
        }
        let (len, consumed_len) = decode_varint(bytes, i)?;
        i += consumed_len;
        let end = i + len as usize;
        if end > bytes.len() {
            return None;
        }
        let payload = &bytes[i..end];
        match field_no {
            1 => org.id = String::from_utf8_lossy(payload).into_owned(),
            2 => org.name = String::from_utf8_lossy(payload).into_owned(),
            _ => {}
        }
        i = end;
    }
    if org.id.is_empty() && org.name.is_empty() {
        None
    } else {
        Some(org)
    }
}

fn parse_windsurf_post_auth_response(bytes: &[u8]) -> Result<WindsurfPostAuthResult, String> {
    let mut result = WindsurfPostAuthResult::default();
    let mut i = 0;
    while i < bytes.len() {
        let (tag, consumed) = decode_varint(bytes, i)
            .ok_or_else(|| "SuperAI PostAuth 响应 tag 解码失败".to_string())?;
        i += consumed;
        let field_no = (tag >> 3) as u32;
        let wire_type = (tag & 0x7) as u8;
        if wire_type == 2 {
            let (len, consumed_len) = decode_varint(bytes, i)
                .ok_or_else(|| "SuperAI PostAuth 响应长度解码失败".to_string())?;
            i += consumed_len;
            let end = i + len as usize;
            if end > bytes.len() {
                return Err("SuperAI PostAuth 响应长度越界".to_string());
            }
            let payload = &bytes[i..end];
            match field_no {
                1 => result.session_token = String::from_utf8_lossy(payload).into_owned(),
                2 => {
                    if let Some(org) = parse_windsurf_org(payload) {
                        result.orgs.push(org);
                    }
                }
                3 => result.auth1_token = Some(String::from_utf8_lossy(payload).into_owned()),
                4 => result.account_id = Some(String::from_utf8_lossy(payload).into_owned()),
                5 => result.primary_org_id = Some(String::from_utf8_lossy(payload).into_owned()),
                _ => {}
            }
            i = end;
        } else {
            match wire_type {
                0 => {
                    let (_, consumed_value) = decode_varint(bytes, i)
                        .ok_or_else(|| "SuperAI PostAuth 响应 varint 跳过失败".to_string())?;
                    i += consumed_value;
                }
                1 => i += 8,
                5 => i += 4,
                _ => return Err(format!("SuperAI PostAuth 不支持的 wire type: {wire_type}")),
            }
        }
    }
    if result.session_token.is_empty() {
        return Err("SuperAI PostAuth 响应未包含 session_token".to_string());
    }
    Ok(result)
}

fn parse_proto_message(bytes: &[u8]) -> Result<Value, String> {
    let mut map = serde_json::Map::new();
    let mut i = 0;
    while i < bytes.len() {
        let (tag, consumed) =
            decode_varint(bytes, i).ok_or_else(|| "protobuf tag 解码失败".to_string())?;
        i += consumed;
        if tag == 0 {
            break;
        }
        let field_no = (tag >> 3) as u32;
        let wire_type = (tag & 0x7) as u8;
        match wire_type {
            0 => {
                let (value, consumed_value) = decode_varint(bytes, i)
                    .ok_or_else(|| "protobuf varint 解码失败".to_string())?;
                i += consumed_value;
                map.insert(format!("int_{field_no}"), Value::Number(value.into()));
            }
            1 => {
                if i + 8 > bytes.len() {
                    return Err("protobuf fixed64 长度越界".to_string());
                }
                i += 8;
            }
            2 => {
                let (len, consumed_len) = decode_varint(bytes, i)
                    .ok_or_else(|| "protobuf length 解码失败".to_string())?;
                i += consumed_len;
                let end = i + len as usize;
                if end > bytes.len() {
                    return Err("protobuf length-delimited 长度越界".to_string());
                }
                let payload = &bytes[i..end];
                let value = if let Ok(text) = String::from_utf8(payload.to_vec()) {
                    if !text.is_empty()
                        && text
                            .chars()
                            .all(|ch| ch.is_ascii_graphic() || ch.is_ascii_whitespace())
                    {
                        Value::String(text)
                    } else {
                        parse_proto_message(payload).unwrap_or_else(|_| {
                            Value::Array(
                                payload
                                    .iter()
                                    .map(|byte| Value::Number((*byte).into()))
                                    .collect(),
                            )
                        })
                    }
                } else {
                    parse_proto_message(payload).unwrap_or_else(|_| {
                        Value::Array(
                            payload
                                .iter()
                                .map(|byte| Value::Number((*byte).into()))
                                .collect(),
                        )
                    })
                };
                let key = if value.is_string() {
                    format!("string_{field_no}")
                } else if value.is_object() {
                    format!("subMesssage_{field_no}")
                } else {
                    format!("bytes_{field_no}")
                };
                if let Some(existing) = map.get_mut(&key) {
                    if let Value::Array(items) = existing {
                        items.push(value);
                    } else {
                        let previous = existing.clone();
                        *existing = Value::Array(vec![previous, value]);
                    }
                } else {
                    map.insert(key, value);
                }
                i = end;
            }
            5 => {
                if i + 4 > bytes.len() {
                    return Err("protobuf fixed32 长度越界".to_string());
                }
                i += 4;
            }
            _ => return Err(format!("protobuf 不支持的 wire type: {wire_type}")),
        }
    }
    Ok(Value::Object(map))
}

fn decode_proto_response_body(response_body: &[u8]) -> Vec<u8> {
    let response_text = String::from_utf8_lossy(response_body);
    let maybe_base64 = response_text
        .strip_prefix("data:application/proto;base64,")
        .unwrap_or(response_text.trim());
    if !maybe_base64.is_empty()
        && maybe_base64
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '/' || ch == '=')
    {
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(maybe_base64) {
            return decoded;
        }
    }
    response_body.to_vec()
}

fn proto_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).or_else(|| {
        value
            .and_then(Value::as_u64)
            .and_then(|v| i64::try_from(v).ok())
    })
}

fn extract_windsurf_current_user(response_body: &[u8]) -> Result<Value, String> {
    let decoded = decode_proto_response_body(response_body);
    let parsed = parse_proto_message(&decoded)?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| "GetCurrentUser 响应不是对象".to_string())?;
    let user = obj.get("subMesssage_1").and_then(Value::as_object);
    let plan = obj.get("subMesssage_6").and_then(Value::as_object);
    let subscription = obj.get("subMesssage_4").and_then(Value::as_object);

    let mut user_map = serde_json::Map::new();
    if let Some(user) = user {
        if let Some(value) = string_field(user.get("string_1")) {
            user_map.insert("api_key".to_string(), Value::String(value));
        }
        if let Some(value) = string_field(user.get("string_2")) {
            user_map.insert("name".to_string(), Value::String(value));
        }
        if let Some(value) = string_field(user.get("string_3")) {
            user_map.insert("email".to_string(), Value::String(value));
        }
        if let Some(value) = string_field(user.get("string_6")) {
            user_map.insert("id".to_string(), Value::String(value));
        }
        if let Some(value) = proto_i64(user.get("int_16")) {
            user_map.insert("disable_codeium".to_string(), Value::Bool(value != 0));
        }
    }

    let mut plan_map = serde_json::Map::new();
    let base_quota = plan
        .and_then(|plan| proto_i64(plan.get("int_12")))
        .unwrap_or(0);
    if let Some(plan) = plan {
        if let Some(value) = string_field(plan.get("string_2")) {
            plan_map.insert("plan_name".to_string(), Value::String(value));
        }
        if let Some(value) = proto_i64(plan.get("int_35")) {
            plan_map.insert("billing_strategy".to_string(), Value::Number(value.into()));
        }
    }

    let mut subscription_map = serde_json::Map::new();
    if let Some(subscription) = subscription {
        let extra_quota = proto_i64(subscription.get("int_15")).unwrap_or(0);
        let total_quota = base_quota + extra_quota;
        let used_quota = proto_i64(subscription.get("int_17")).unwrap_or(0);
        subscription_map.insert("quota".to_string(), Value::Number(total_quota.into()));
        subscription_map.insert("used_quota".to_string(), Value::Number(used_quota.into()));
        if let Some(expires_at) = subscription
            .get("subMesssage_18")
            .and_then(|timestamp| proto_i64(timestamp.get("int_1")))
        {
            subscription_map.insert("expires_at".to_string(), Value::Number(expires_at.into()));
        }
        if let Some(active) = proto_i64(subscription.get("int_7")) {
            subscription_map.insert("subscription_active".to_string(), Value::Bool(active != 0));
        }
    }

    Ok(serde_json::json!({
        "parsed_data": parsed,
        "user_info": {
            "user": Value::Object(user_map),
            "plan": Value::Object(plan_map),
            "subscription": Value::Object(subscription_map)
        }
    }))
}

fn apply_windsurf_user_info(account: &mut ManagedAccount, user_info: &Value) {
    let now = now_ts();
    if let Some(user) = user_info.get("user") {
        if let Some(email) = string_field(user.get("email")) {
            account.email = email.to_lowercase();
        }
        if let Some(name) = string_field(user.get("name")) {
            account.display_name = Some(name);
        }
        if let Some(id) = string_field(user.get("id")) {
            account.account_id = Some(id.clone());
            account.user_id = Some(id.clone());
            windsurf_set_token_field(account, "local_id", Value::String(id));
        }
    }
    if let Some(plan) = user_info.get("plan") {
        if let Some(plan_name) = string_field(plan.get("plan_name")) {
            account.plan = Some(plan_name.clone());
            account.plan_type = Some(plan_name);
        }
    }
    let mut metrics = Vec::new();
    if let Some(subscription) = user_info.get("subscription") {
        let used = proto_i64(subscription.get("used_quota"));
        let total = proto_i64(subscription.get("quota"));
        if let Some(expires_at) = proto_i64(subscription.get("expires_at")) {
            account.subscription_active_until = Some(Value::Number(expires_at.into()));
        }
        if let (Some(used), Some(total)) = (used, total) {
            if total > 0 {
                let remaining =
                    (((total - used).max(0) as f64 / total as f64) * 100.0).round() as i64;
                metrics.push(QuotaMetric {
                    key: "windsurf-credits".to_string(),
                    label: "CREDITS".to_string(),
                    remaining_percent: Some(remaining.clamp(0, 100)),
                    reset_at: account.subscription_active_until.clone(),
                    detail: Some(format!("{}/{} used", used, total)),
                    state: Some(if remaining <= 0 {
                        "unavailable".to_string()
                    } else if remaining <= 15 {
                        "warning".to_string()
                    } else {
                        "available".to_string()
                    }),
                });
            }
        }
    }
    account.quota = if metrics.is_empty() {
        None
    } else {
        Some(AccountQuota {
            metrics,
            last_updated: Some(now),
            error: None,
            is_forbidden: Some(false),
        })
    };
    account.updated_at = now;
    account.status = Some(AccountStatus {
        state: "available".to_string(),
        label: "可用".to_string(),
        reason: None,
        updated_at: Some(now),
    });
}

fn extract_windsurf_plan_status(response_body: &[u8]) -> Result<Value, String> {
    let decoded = decode_proto_response_body(response_body);
    let parsed = parse_proto_message(&decoded)?;
    let plan_status = parsed
        .get("subMesssage_1")
        .ok_or_else(|| "GetPlanStatus 响应缺少 plan_status".to_string())?;
    let mut result = serde_json::Map::new();
    result.insert("raw_data".to_string(), parsed.clone());
    if let Some(plan_info) = plan_status.get("subMesssage_1") {
        if let Some(value) = string_field(plan_info.get("string_2")) {
            result.insert("plan_name".to_string(), Value::String(value));
        }
        if let Some(value) = proto_i64(plan_info.get("int_35")) {
            result.insert("billing_strategy".to_string(), Value::Number(value.into()));
        }
    }
    if let Some(value) = plan_status
        .get("subMesssage_3")
        .and_then(|timestamp| proto_i64(timestamp.get("int_1")))
    {
        result.insert("plan_end".to_string(), Value::Number(value.into()));
    }
    for (key, field) in windsurf_plan_status_proto_field_map() {
        if let Some(value) = proto_i64(plan_status.get(field)) {
            result.insert(key.to_string(), Value::Number(value.into()));
        }
    }
    Ok(Value::Object(result))
}

fn windsurf_plan_status_proto_field_map() -> [(&'static str, &'static str); 11] {
    // GetPlanStatus 的 proto tag 名字不直观：int_14 是 weekly，int_15 是 daily。
    // 公开版本地累计用量依赖 daily%，这里映射错会让"日限"显示成周限。
    [
        ("available_flex_credits", "int_4"),
        ("used_flow_credits", "int_5"),
        ("used_prompt_credits", "int_6"),
        ("used_flex_credits", "int_7"),
        ("available_prompt_credits", "int_8"),
        ("available_flow_credits", "int_9"),
        ("weekly_quota_remaining_percent", "int_14"),
        ("daily_quota_remaining_percent", "int_15"),
        ("overage_balance_micros", "int_16"),
        ("weekly_quota_reset_at_unix", "int_17"),
        ("daily_quota_reset_at_unix", "int_18"),
    ]
}

fn quota_state_from_remaining(remaining: i64) -> String {
    if remaining <= 0 {
        "unavailable".to_string()
    } else if remaining <= 15 {
        "warning".to_string()
    } else {
        "available".to_string()
    }
}

fn apply_windsurf_plan_status(account: &mut ManagedAccount, plan_status: &Value) {
    let now = now_ts();
    if let Some(plan_name) = string_field(plan_status.get("plan_name")) {
        account.plan = Some(plan_name.clone());
        account.plan_type = Some(plan_name);
    }
    if let Some(plan_end) = proto_i64(plan_status.get("plan_end")) {
        account.subscription_active_until = Some(Value::Number(plan_end.into()));
    }

    let mut metrics = Vec::new();
    let is_quota_mode = proto_i64(plan_status.get("billing_strategy")) == Some(2)
        || plan_status.get("daily_quota_remaining_percent").is_some()
        || plan_status.get("weekly_quota_remaining_percent").is_some()
        || plan_status.get("daily_quota_reset_at_unix").is_some()
        || plan_status.get("weekly_quota_reset_at_unix").is_some();
    if is_quota_mode {
        let remaining = proto_i64(plan_status.get("daily_quota_remaining_percent"))
            .unwrap_or(0)
            .clamp(0, 100);
        let remaining = remaining.clamp(0, 100);
        metrics.push(QuotaMetric {
            key: "windsurf-daily".to_string(),
            label: "日限".to_string(),
            remaining_percent: Some(remaining),
            reset_at: plan_status
                .get("daily_quota_reset_at_unix")
                .and_then(|value| proto_i64(Some(value)))
                .map(|value| Value::Number(value.into())),
            detail: Some(format!("剩余 {remaining}%")),
            state: Some(quota_state_from_remaining(remaining)),
        });
        let remaining = proto_i64(plan_status.get("weekly_quota_remaining_percent"))
            .unwrap_or(0)
            .clamp(0, 100);
        metrics.push(QuotaMetric {
            key: "windsurf-weekly".to_string(),
            label: "周限".to_string(),
            remaining_percent: Some(remaining),
            reset_at: plan_status
                .get("weekly_quota_reset_at_unix")
                .and_then(|value| proto_i64(Some(value)))
                .map(|value| Value::Number(value.into())),
            detail: Some(format!("剩余 {remaining}%")),
            state: Some(quota_state_from_remaining(remaining)),
        });
    }

    if metrics.is_empty() {
        let used_prompt = proto_i64(plan_status.get("used_prompt_credits")).unwrap_or(0);
        let used_flex = proto_i64(plan_status.get("used_flex_credits")).unwrap_or(0);
        let available_prompt = proto_i64(plan_status.get("available_prompt_credits")).unwrap_or(0);
        let available_flex = proto_i64(plan_status.get("available_flex_credits")).unwrap_or(0);
        let used = used_prompt + used_flex;
        let total = used + available_prompt + available_flex;
        if total > 0 {
            let remaining = (((total - used).max(0) as f64 / total as f64) * 100.0).round() as i64;
            metrics.push(QuotaMetric {
                key: "windsurf-credits".to_string(),
                label: "CREDITS".to_string(),
                remaining_percent: Some(remaining.clamp(0, 100)),
                reset_at: account.subscription_active_until.clone(),
                detail: Some(format!("{}/{} used", used, total)),
                state: Some(quota_state_from_remaining(remaining)),
            });
        }
    }

    if !metrics.is_empty() {
        account.quota = Some(AccountQuota {
            metrics,
            last_updated: Some(now),
            error: None,
            is_forbidden: Some(false),
        });
    }
    account.updated_at = now;
    account.status = Some(AccountStatus {
        state: "available".to_string(),
        label: "可用".to_string(),
        reason: None,
        updated_at: Some(now),
    });
}

fn apply_windsurf_user_status_json(account: &mut ManagedAccount, user_status: &Value) {
    let plan_status = user_status
        .get("userStatus")
        .and_then(|value| value.get("planStatus"))
        .unwrap_or(user_status);
    let plan_info = plan_status.get("planInfo").unwrap_or(&Value::Null);
    let mut normalized = serde_json::Map::new();
    if let Some(value) = string_field(plan_info.get("planName")) {
        normalized.insert("plan_name".to_string(), Value::String(value));
    }
    if let Some(value) = number_field(plan_status.get("dailyQuotaRemainingPercent")) {
        normalized.insert(
            "daily_quota_remaining_percent".to_string(),
            Value::Number(value.into()),
        );
    }
    if let Some(value) = number_field(plan_status.get("weeklyQuotaRemainingPercent")) {
        normalized.insert(
            "weekly_quota_remaining_percent".to_string(),
            Value::Number(value.into()),
        );
    }
    if let Some(value) = plan_status
        .get("dailyQuotaResetAtUnix")
        .and_then(coerce_unix_seconds)
    {
        normalized.insert(
            "daily_quota_reset_at_unix".to_string(),
            Value::Number(value.into()),
        );
    }
    if let Some(value) = plan_status
        .get("weeklyQuotaResetAtUnix")
        .and_then(coerce_unix_seconds)
    {
        normalized.insert(
            "weekly_quota_reset_at_unix".to_string(),
            Value::Number(value.into()),
        );
    }
    if let Some(value) = number_field(plan_status.get("usedPromptCredits")) {
        normalized.insert(
            "used_prompt_credits".to_string(),
            Value::Number(value.into()),
        );
    }
    if let Some(value) = number_field(plan_status.get("availablePromptCredits")) {
        normalized.insert(
            "available_prompt_credits".to_string(),
            Value::Number(value.into()),
        );
    }
    if let Some(value) = number_field(plan_status.get("usedFlexCredits")) {
        normalized.insert("used_flex_credits".to_string(), Value::Number(value.into()));
    }
    if let Some(value) = number_field(plan_status.get("availableFlexCredits")) {
        normalized.insert(
            "available_flex_credits".to_string(),
            Value::Number(value.into()),
        );
    }
    // planEnd 在 Connect-RPC JSON 里可能是数字、数字串、RFC3339 字符串或
    // {seconds, nanos} 对象，全部归一到 Unix 秒。
    if let Some(value) = plan_status.get("planEnd").and_then(coerce_unix_seconds) {
        normalized.insert("plan_end".to_string(), Value::Number(value.into()));
    }
    apply_windsurf_plan_status(account, &Value::Object(normalized));
}

async fn refresh_windsurf_account_by_api_key(account: &mut ManagedAccount) -> Result<(), String> {
    let api_key =
        windsurf_payload_string(account, "api_key").ok_or_else(|| "缺少 api_key".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 SuperAI 状态客户端失败: {error}"))?;
    let body = serde_json::json!({
        "metadata": {
            "apiKey": api_key,
            "ideName": "windsurf",
            "ideVersion": "1.108.2",
            "extensionName": "windsurf",
            "extensionVersion": "1.108.2",
            "locale": "en"
        }
    });
    let mut last_error = None;
    for host in WINDSURF_API_SERVER_HOSTS {
        let url = format!("https://{host}{WINDSURF_USER_STATUS_PATH}");
        let response = match client
            .post(&url)
            .json(&body)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .header("Connect-Protocol-Version", "1")
            .header("User-Agent", "windsurf/1.108.2")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(format!("{host}: {error}"));
                continue;
            }
        };
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("读取 SuperAI 状态响应失败: {error}"))?;
        if !status.is_success() {
            last_error = Some(format!("{host} → {status}: {text}"));
            continue;
        }
        let value = serde_json::from_str::<Value>(&text)
            .map_err(|error| format!("解析 SuperAI 状态响应失败: {error}"))?;
        apply_windsurf_user_status_json(account, &value);
        return Ok(());
    }
    Err(last_error.unwrap_or_else(|| "SuperAI 状态刷新失败".to_string()))
}

fn apply_windsurf_auth_headers(
    request: reqwest::RequestBuilder,
    session_token: &str,
    auth1_token: Option<String>,
    account_id: Option<String>,
    primary_org_id: Option<String>,
) -> reqwest::RequestBuilder {
    let mut request = request.header("x-auth-token", session_token);
    if session_token.starts_with("devin-session-token$") {
        request = request.header("x-devin-session-token", session_token);
        if let Some(value) = account_id {
            request = request.header("x-devin-account-id", value);
        }
        if let Some(value) = auth1_token {
            request = request.header("x-devin-auth1-token", value);
        }
        if let Some(value) = primary_org_id {
            request = request.header("x-devin-primary-org-id", value);
        }
    }
    request
}

/// 调用 Windsurf SeatManagementService 的 connect-rpc 端点。
/// `endpoint` 为方法名（如 `GetCurrentUser`/`GetPlanStatus`）；`extra_body` 为
/// 在 session_token 后追加的额外 proto 字节（不同方法可能需要附加默认字段）。
async fn windsurf_seat_management_call(
    account: &ManagedAccount,
    endpoint: &str,
    missing_token_msg: &str,
    extra_body: &[u8],
) -> Result<Vec<u8>, String> {
    let session_token = windsurf_payload_string(account, "session_token")
        .or_else(|| windsurf_payload_string(account, "id_token"))
        .ok_or_else(|| missing_token_msg.to_string())?;
    let auth1_token = windsurf_payload_string(account, "auth1_token");
    let account_id = windsurf_payload_string(account, "local_id");
    let primary_org_id = windsurf_payload_string(account, "primary_org_id");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 SuperAI {endpoint} 客户端失败: {error}"))?;
    let url =
        format!("{WINDSURF_BACKEND_URL}/exa.seat_management_pb.SeatManagementService/{endpoint}");
    let mut body = Vec::with_capacity(session_token.len() + 8 + extra_body.len());
    encode_proto_string_field(&mut body, 1, &session_token);
    body.extend_from_slice(extra_body);
    let request = client
        .post(&url)
        .body(body)
        .header(ACCEPT, "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Cache-Control", "no-cache")
        .header("connect-protocol-version", "1")
        .header(CONTENT_TYPE, "application/proto")
        .header("Pragma", "no-cache")
        .header("priority", "u=1, i")
        .header(
            "Sec-Ch-Ua",
            r#""Chromium";v="142", "Google Chrome";v="142", "Not_A Brand";v="99""#,
        )
        .header("Sec-Ch-Ua-Mobile", "?0")
        .header("Sec-Ch-Ua-Platform", r#""Windows""#)
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-site")
        .header("x-debug-email", "")
        .header("x-debug-team-name", "")
        .header("Referer", "https://windsurf.com/");
    let request = apply_windsurf_auth_headers(
        request,
        &session_token,
        auth1_token,
        account_id,
        primary_org_id,
    );
    let response = request
        .send()
        .await
        .map_err(|error| format!("{endpoint} 请求失败: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 {endpoint} 响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "{endpoint} 失败 ({status}): {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    Ok(bytes.to_vec())
}

async fn windsurf_get_current_user(account: &ManagedAccount) -> Result<Value, String> {
    let bytes = windsurf_seat_management_call(
        account,
        "GetCurrentUser",
        "缺少可用于查询 SuperAI 账号信息的 token",
        &[0x10, 0x01, 0x18, 0x01, 0x20, 0x01],
    )
    .await?;
    extract_windsurf_current_user(&bytes)
}

async fn windsurf_get_plan_status(account: &ManagedAccount) -> Result<Value, String> {
    let bytes = windsurf_seat_management_call(
        account,
        "GetPlanStatus",
        "缺少可用于查询 SuperAI 套餐状态的 token",
        &[],
    )
    .await?;
    extract_windsurf_plan_status(&bytes)
}

async fn enrich_windsurf_account_remote(account: &mut ManagedAccount) -> Result<(), String> {
    // 有 api_key 时优先走 JSON 路径：上游 GetUserStatus 用 application/json，
    // 字段名是 dailyQuotaRemainingPercent / weeklyQuotaRemainingPercent / planEnd，
    // 不依赖 protobuf 私有 tag mapping，跟 vendor sidecar 对齐。
    if windsurf_payload_string(account, "api_key").is_some() {
        if let Ok(()) = refresh_windsurf_account_by_api_key(account).await {
            return Ok(());
        }
    }
    let mut last_error = None;
    match windsurf_get_current_user(account).await {
        Ok(user_info_result) => {
            if let Some(user_info) = user_info_result.get("user_info") {
                apply_windsurf_user_info(account, user_info);
            }
        }
        Err(error) => last_error = Some(error),
    }
    match windsurf_get_plan_status(account).await {
        Ok(plan_status) => apply_windsurf_plan_status(account, &plan_status),
        Err(error) => {
            if last_error.is_none() {
                last_error = Some(error);
            }
        }
    }
    if account.quota.is_some() || account.plan.is_some() {
        Ok(())
    } else {
        Err(last_error.unwrap_or_else(|| "未能获取 SuperAI 账号信息".to_string()))
    }
}

async fn windsurf_post_auth(
    auth1_token: &str,
    org_id: Option<&str>,
) -> Result<WindsurfPostAuthResult, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 SuperAI PostAuth 客户端失败: {error}"))?;
    let mut last_error = None;
    let mut body = Vec::with_capacity(auth1_token.len() + org_id.unwrap_or_default().len() + 4);
    encode_proto_string_field(&mut body, 1, auth1_token);
    if let Some(org_id) = org_id.filter(|value| !value.trim().is_empty()) {
        encode_proto_string_field(&mut body, 2, org_id);
    }
    for url in [
        WINDSURF_POST_AUTH_URL_BACKEND,
        WINDSURF_POST_AUTH_URL_NEW,
        WINDSURF_POST_AUTH_URL_LEGACY,
    ] {
        let response = client
            .post(url)
            .body(body.clone())
            .header(ACCEPT, "*/*")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .header("connect-protocol-version", "1")
            .header(CONTENT_TYPE, "application/proto")
            .header("Origin", "https://windsurf.com")
            .header("Referer", "https://windsurf.com/account/login")
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-site")
            .header("X-Devin-Auth1-Token", auth1_token)
            .send()
            .await
            .map_err(|error| format!("SuperAI PostAuth 请求失败: {error}"))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("读取 SuperAI PostAuth 响应失败: {error}"))?;
        if !status.is_success() {
            last_error = Some(format!(
                "{url} → {status}: {}",
                String::from_utf8_lossy(&bytes)
            ));
            continue;
        }
        match parse_windsurf_post_auth_body(&bytes) {
            Ok(mut result) => {
                if result.auth1_token.is_none() {
                    result.auth1_token = Some(auth1_token.to_string());
                }
                if result.primary_org_id.is_none() {
                    result.primary_org_id = org_id
                        .filter(|value| !value.trim().is_empty())
                        .map(ToString::to_string);
                }
                return Ok(result);
            }
            Err(error) => last_error = Some(format!("{url} → {error}")),
        }
    }
    Err(last_error.unwrap_or_else(|| "SuperAI PostAuth 失败".to_string()))
}

#[tauri::command]
#[allow(non_snake_case)]
async fn add_superai_account_by_password(
    app: tauri::AppHandle,
    email: String,
    password: String,
) -> Result<ManagedAccount, String> {
    let trimmed_email = email.trim();
    let trimmed_password = password.trim();
    if trimmed_email.is_empty() || trimmed_password.is_empty() {
        return Err("邮箱和密码不能为空".to_string());
    }

    match windsurf_auth1_password_login(trimmed_email, trimmed_password).await {
        Ok(auth1_token) => {
            let post_auth = windsurf_post_auth(&auth1_token, None).await?;
            let mut tokens_map = serde_json::Map::new();
            tokens_map.insert(
                "api_key".to_string(),
                Value::String(post_auth.session_token.clone()),
            );
            tokens_map.insert(
                "session_token".to_string(),
                Value::String(post_auth.session_token.clone()),
            );
            tokens_map.insert("auth1_token".to_string(), Value::String(auth1_token));
            if let Some(value) = post_auth.account_id.clone() {
                tokens_map.insert("local_id".to_string(), Value::String(value));
            }
            if let Some(value) = post_auth.primary_org_id.clone() {
                tokens_map.insert("primary_org_id".to_string(), Value::String(value));
            }

            let mut payload = serde_json::Map::new();
            payload.insert(
                "provider".to_string(),
                Value::String("windsurf".to_string()),
            );
            payload.insert(
                "email".to_string(),
                Value::String(trimmed_email.to_lowercase()),
            );
            payload.insert(
                "display_name".to_string(),
                Value::String(trimmed_email.to_string()),
            );
            payload.insert("tokens".to_string(), Value::Object(tokens_map));

            let mut account = parse_windsurf_account(&Value::Object(payload), "password")
                .ok_or_else(|| "构建 SuperAI 账号记录失败".to_string())?;
            let _ = enrich_windsurf_account_remote(&mut account).await;
            upsert_accounts_into_db(&app, std::slice::from_ref(&account))?;
            return Ok(account_for_frontend(&account));
        }
        Err(auth1_error) => {
            if auth1_error.contains("没有设置邮箱密码")
                || auth1_error.contains("该邮箱未注册")
                || auth1_error.contains("已被禁用")
                || auth1_error.contains("尝试次数过多")
            {
                return Err(auth1_error);
            }
        }
    }

    let signin = windsurf_firebase_sign_in(trimmed_email, trimmed_password).await?;
    let id_token = string_field(signin.get("idToken"))
        .ok_or_else(|| "SuperAI 登录响应缺少 idToken".to_string())?;
    let register = windsurf_register_with_codeium(&id_token).await?;
    let refresh_token = string_field(signin.get("refreshToken"))
        .ok_or_else(|| "SuperAI 登录响应缺少 refreshToken".to_string())?;
    let local_id = string_field(signin.get("localId"));
    let display_name = string_field(signin.get("displayName"));
    let resolved_email =
        string_field(signin.get("email")).unwrap_or_else(|| trimmed_email.to_string());
    let expires_in = string_field(signin.get("expiresIn"))
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(3600);
    let expires_at = now_ts() + expires_in;

    // 通过 lookup 拿到 displayName 等信息（可选）。
    let mut final_display_name = display_name;
    if final_display_name.is_none() {
        if let Some(lookup) = windsurf_firebase_lookup(&id_token).await {
            if let Some(users) = lookup.get("users").and_then(Value::as_array) {
                if let Some(user) = users.first() {
                    final_display_name = string_field(user.get("displayName"));
                }
            }
        }
    }

    let mut tokens_map = serde_json::Map::new();
    tokens_map.insert(
        "api_key".to_string(),
        Value::String(register.api_key.clone()),
    );
    tokens_map.insert("id_token".to_string(), Value::String(id_token.clone()));
    tokens_map.insert("refresh_token".to_string(), Value::String(refresh_token));
    tokens_map.insert("access_token".to_string(), Value::String(id_token));
    if let Some(value) = local_id.clone() {
        tokens_map.insert("local_id".to_string(), Value::String(value));
    }
    tokens_map.insert("expires_at".to_string(), Value::Number(expires_at.into()));

    let mut payload = serde_json::Map::new();
    payload.insert(
        "provider".to_string(),
        Value::String("windsurf".to_string()),
    );
    payload.insert("email".to_string(), Value::String(resolved_email.clone()));
    if final_display_name.is_none() {
        final_display_name = register.name.clone();
    }
    if let Some(name) = final_display_name.clone() {
        payload.insert("display_name".to_string(), Value::String(name));
    }
    if let Some(api_server_url) = register.api_server_url {
        payload.insert("api_server_url".to_string(), Value::String(api_server_url));
    }
    payload.insert("tokens".to_string(), Value::Object(tokens_map));

    let account = parse_windsurf_account(&Value::Object(payload), "password")
        .ok_or_else(|| "构建 SuperAI 账号记录失败".to_string())?;
    upsert_accounts_into_db(&app, std::slice::from_ref(&account))?;
    Ok(account_for_frontend(&account))
}

fn parse_windsurf_batch_key_line(line: &str) -> Result<WindsurfBatchCredential, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("密钥为空".to_string());
    }

    let candidates = [
        trimmed.to_string(),
        decode_batch_key_text(trimmed).unwrap_or_default(),
        superai_decrypt_text(trimmed).unwrap_or_default(),
    ];
    for candidate in candidates.iter().filter(|value| !value.trim().is_empty()) {
        if let Ok(value) = serde_json::from_str::<Value>(candidate) {
            if let Some(credential) = account_password_from_value(&value) {
                return Ok(credential);
            }
        }
        if let Some(credential) = split_account_password(candidate) {
            return Ok(credential);
        }
    }

    Err("密钥格式无效".to_string())
}

fn decode_batch_key_text(value: &str) -> Option<String> {
    let normalized = value.trim();
    for decoded in [
        base64::engine::general_purpose::STANDARD
            .decode(normalized)
            .ok(),
        base64::engine::general_purpose::URL_SAFE
            .decode(normalized)
            .ok(),
        URL_SAFE_NO_PAD.decode(normalized).ok(),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(text) = String::from_utf8(decoded) {
            return Some(text);
        }
    }
    None
}

fn account_password_from_value(value: &Value) -> Option<WindsurfBatchCredential> {
    let obj = value.as_object()?;
    let account = string_field(obj.get("account"))
        .or_else(|| string_field(obj.get("username")))
        .or_else(|| string_field(obj.get("email")))?;
    let password = string_field(obj.get("password")).or_else(|| string_field(obj.get("pwd")))?;
    let expires_at = number_field(obj.get("expires_at"))
        .or_else(|| number_field(obj.get("expiresAt")))
        .or_else(|| number_field(obj.get("expiry")))
        .or_else(|| number_field(obj.get("expired_at")))?;
    Some(WindsurfBatchCredential {
        account,
        password,
        expires_at,
    })
}

fn split_account_password(value: &str) -> Option<WindsurfBatchCredential> {
    for delimiter in ["----", "｜", "|", "\t", ","] {
        let parts = value.split(delimiter).map(str::trim).collect::<Vec<_>>();
        if parts.len() >= 3 {
            let account = parts[0];
            let password = parts[1];
            let expires_at = parts[2].parse::<i64>().ok()?;
            if !account.is_empty() && !password.is_empty() && expires_at > 0 {
                return Some(WindsurfBatchCredential {
                    account: account.to_string(),
                    password: password.to_string(),
                    expires_at,
                });
            }
        } else if let Some((account, password)) = value.split_once(delimiter) {
            let account = account.trim();
            let password = password.trim();
            if !account.is_empty() && !password.is_empty() {
                return Some(WindsurfBatchCredential {
                    account: account.to_string(),
                    password: password.to_string(),
                    expires_at: i64::MAX,
                });
            }
        }
    }
    None
}

#[tauri::command]
async fn add_superai_accounts_by_batch_keys(
    app: tauri::AppHandle,
    keys: Vec<String>,
) -> Result<ImportResult, String> {
    let mut imported = Vec::new();
    let mut failed = Vec::new();

    for (index, key) in keys.into_iter().enumerate() {
        let label = format!("第 {} 行", index + 1);
        let credential = match parse_windsurf_batch_key_line(&key) {
            Ok(value) => value,
            Err(reason) => {
                failed.push(ImportFailure { label, reason });
                continue;
            }
        };
        if windsurf_license_expired_at(credential.expires_at) {
            failed.push(ImportFailure {
                label,
                reason: "账号已到期".to_string(),
            });
            continue;
        }

        match add_superai_account_by_password(app.clone(), credential.account, credential.password)
            .await
        {
            Ok(account) => {
                let mut full_account = {
                    let conn = open_app_db(&app)?;
                    load_account_from_db(&conn, &account.id)?
                };
                // 日卡场景核心保险：必须在导入时点锁住 baseline，否则等下一次
                // refresh 才锁可能跨过上游 16:00 重置，被刷成 100%。
                // 拉不到 daily 就重试一次 enrich；仍失败则回滚刚插入的账号，
                // 让操作员重试，不允许"无 baseline"账号进入正式列表。
                if current_daily_remaining_from_account(&full_account).is_none() {
                    let _ = enrich_windsurf_account_remote(&mut full_account).await;
                }
                if current_daily_remaining_from_account(&full_account).is_none() {
                    let conn = open_app_db(&app)?;
                    let _ = conn.execute(
                        "DELETE FROM accounts WHERE id = ?1",
                        params![&full_account.id],
                    );
                    failed.push(ImportFailure {
                        label,
                        reason: "未能获取上游额度，导入已回滚，请稍后重试".to_string(),
                    });
                    continue;
                }
                attach_windsurf_batch_key(&app, &mut full_account, &key, credential.expires_at);
                // 双重确认 baseline 真的写进去了。理论上前面已校验，这里再兜底
                // 一次：万一 attach 内部 bump 因极端竞争没记录 baseline 也拦下。
                if public_usage_baseline(&full_account).is_none() {
                    let conn = open_app_db(&app)?;
                    let _ = conn.execute(
                        "DELETE FROM accounts WHERE id = ?1",
                        params![&full_account.id],
                    );
                    failed.push(ImportFailure {
                        label,
                        reason: "未能锁定本地额度基线，导入已回滚，请稍后重试".to_string(),
                    });
                    continue;
                }
                upsert_accounts_into_db(&app, std::slice::from_ref(&full_account))?;
                imported.push(account_for_frontend(&full_account));
            }
            Err(error) => failed.push(ImportFailure {
                label,
                reason: error,
            }),
        }
    }

    Ok(ImportResult { imported, failed })
}

fn attach_windsurf_batch_key(
    app: &tauri::AppHandle,
    account: &mut ManagedAccount,
    key: &str,
    expires_at: i64,
) {
    let mut payload = account
        .auth_payload
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    payload.insert(
        "batch_key".to_string(),
        Value::String(key.trim().to_string()),
    );
    payload.insert(
        "license_expires_at".to_string(),
        Value::Number(expires_at.into()),
    );
    account.auth_payload = Some(Value::Object(payload));
    if is_public_build() {
        account.subscription_active_until = Some(Value::Number(expires_at.into()));
    }
    // 删账号 24h 内重新导入同一 batch_key：恢复使用记录，避免被清零。
    // restore 命中后 PUBLIC_USAGE_KEY_LAST_REMOTE 会有值，bump_public_usage
    // 不会再走"首次记录"分支，baseline / consumed 都按历史值继续累加。
    let _ = restore_public_usage_history(app, account, key);
    let _ = bump_public_usage(account);
    rewrite_quota_for_public_usage(account);
}

// ---------------------------------------------------------------------------
// 公开版（批量密钥）账号的"本地累计用量"独立追踪
//
// 上游 windsurf 的 daily% 会按自然周期重置，但公开版 UI 展示的是"日限"：
// 入库时记录 baseline，之后每次刷新做 max(0, last_remote - daily) 单调累加，
// 累计 100% 即软停用账号（保留记录给 license 到期时由删号路径自然清理）。
// 仅 windsurf provider + 含 batch_key 的账号生效，其他账号 helper 全部 no-op。
// ---------------------------------------------------------------------------

const PUBLIC_USAGE_KEY_BASELINE: &str = "usage_baseline_remaining";
const PUBLIC_USAGE_KEY_LAST_REMOTE: &str = "usage_last_remote_remaining";
const PUBLIC_USAGE_KEY_CONSUMED: &str = "usage_consumed_local";
const PUBLIC_USAGE_KEY_EXHAUSTED_AT: &str = "usage_exhausted_at";

fn windsurf_payload_get_value(account: &ManagedAccount, key: &str) -> Option<Value> {
    account
        .auth_payload
        .as_ref()?
        .as_object()?
        .get(key)
        .cloned()
}

fn windsurf_payload_set_value(account: &mut ManagedAccount, key: &str, value: Value) {
    let payload = account
        .auth_payload
        .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
}

fn has_public_usage_tracking(account: &ManagedAccount) -> bool {
    // 完全版（full build）即便账号 auth_payload 残留 batch_key（例如公开版导入后切到完全版）
    // 也不应把 quota 改写成单一 superai-public 额度条 —— 完全版需要保留 windsurf-daily / windsurf-weekly。
    is_public_build()
        && account.provider == "windsurf"
        && account
            .auth_payload
            .as_ref()
            .and_then(Value::as_object)
            .map(|obj| obj.contains_key("batch_key"))
            .unwrap_or(false)
}

fn public_usage_consumed_percent(account: &ManagedAccount) -> i64 {
    windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_CONSUMED)
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .clamp(0, 100)
}

/// 首次导入时拿到的真实剩余额度（baseline）。
/// 没记录时返回 None；UI 显示用 100 兜底，避免上游一次都没返回时白屏。
fn public_usage_baseline(account: &ManagedAccount) -> Option<i64> {
    windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_BASELINE)
        .and_then(|v| v.as_i64())
        .map(|v| v.clamp(0, 100))
}

/// 本地累计可用剩余 = baseline - consumed。clamp 到 [0, 100]。
fn public_usage_remaining_percent(account: &ManagedAccount) -> i64 {
    let baseline = public_usage_baseline(account).unwrap_or(100);
    let consumed = public_usage_consumed_percent(account);
    (baseline - consumed).clamp(0, 100)
}

fn public_usage_is_exhausted(account: &ManagedAccount) -> bool {
    has_public_usage_tracking(account)
        && (windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_EXHAUSTED_AT).is_some()
            || public_usage_remaining_percent(account) <= 0)
}

fn current_daily_remaining_from_account(account: &ManagedAccount) -> Option<i64> {
    account
        .quota
        .as_ref()?
        .metrics
        .iter()
        .find(|m| m.key == "windsurf-daily")
        .and_then(|m| m.remaining_percent)
}

/// 把上游 daily% 的下降量累计到本地。返回 (是否本次首次耗尽, 当前 consumed%)。
///
/// 语义：
///   - 第一次记录：以当前 daily% 当 baseline + last_remote，consumed 起步 0；
///     UI 显示的剩余 = baseline - consumed = 上游真实剩余（不再强行写成 100%）。
///     如果删除后 24h 内重新导入同一 batch_key，会从 usage_history 恢复
///     baseline / consumed / last_remote / exhausted_at，避免使用记录被清零。
///   - 后续：diff = last_remote - this_daily；diff > 0 才累加（单调递增）；
///     上游重置（this_daily > last_remote）丢弃负 diff，只更新 last_remote。
///   - consumed clamp 在 [0,100]；只要 baseline - consumed <= 0 就视为耗尽并
///     软停用账号（不删除，等 license 过期由 cleanup 统一清除）。
fn bump_public_usage(account: &mut ManagedAccount) -> (bool, i64) {
    if !has_public_usage_tracking(account) {
        return (false, 0);
    }
    let Some(daily) = current_daily_remaining_from_account(account) else {
        // 公开版账号刚导入但还没有 quota（极少见），保留当前状态等下次刷新。
        return (false, public_usage_consumed_percent(account));
    };
    let daily = daily.clamp(0, 100);

    let already_exhausted =
        windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_EXHAUSTED_AT).is_some();
    let mut consumed = public_usage_consumed_percent(account);
    let last_remote = windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_LAST_REMOTE)
        .and_then(|v| v.as_i64())
        .map(|v| v.clamp(0, 100));

    if last_remote.is_none() {
        // 首次见到这个账号：baseline = 上游当前真实剩余。consumed = 0 起算。
        windsurf_payload_set_value(
            account,
            PUBLIC_USAGE_KEY_BASELINE,
            Value::Number(daily.into()),
        );
        windsurf_payload_set_value(
            account,
            PUBLIC_USAGE_KEY_LAST_REMOTE,
            Value::Number(daily.into()),
        );
        if windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_CONSUMED).is_none() {
            windsurf_payload_set_value(account, PUBLIC_USAGE_KEY_CONSUMED, Value::Number(0.into()));
        }
        return (false, public_usage_consumed_percent(account));
    }

    let last_remote = last_remote.unwrap();
    let diff = last_remote - daily;
    if diff > 0 {
        consumed = (consumed + diff).clamp(0, 100);
        windsurf_payload_set_value(
            account,
            PUBLIC_USAGE_KEY_CONSUMED,
            Value::Number(consumed.into()),
        );
    }
    // 不论 diff 正负都更新 last_remote；上游重置后从新 100% 起继续观察。
    windsurf_payload_set_value(
        account,
        PUBLIC_USAGE_KEY_LAST_REMOTE,
        Value::Number(daily.into()),
    );

    // 剩余以 baseline 为上限：baseline - consumed <= 0 即耗尽。
    let baseline = public_usage_baseline(account).unwrap_or(100);
    let remaining = (baseline - consumed).max(0);
    let just_exhausted = !already_exhausted && remaining <= 0;
    if remaining <= 0 {
        let now = now_ts();
        if !already_exhausted {
            windsurf_payload_set_value(
                account,
                PUBLIC_USAGE_KEY_EXHAUSTED_AT,
                Value::Number(now.into()),
            );
        }
        // 每次 refresh 都覆盖 status，避免被后续 apply_windsurf_plan_status 改回"可用"。
        account.status = Some(AccountStatus {
            state: "unavailable".to_string(),
            label: "已耗尽".to_string(),
            reason: Some("本地累计额度已用满".to_string()),
            updated_at: Some(now),
        });
        account.updated_at = now;
    }
    (just_exhausted, consumed)
}

/// 公开版下用本地 consumed 覆盖 quota.metrics，UI 进度条因此显示"还剩 N%"
/// 而不是直接透出上游 windsurf daily%（避免上游重置后 UI 假性回血）。
fn rewrite_quota_for_public_usage(account: &mut ManagedAccount) {
    if !has_public_usage_tracking(account) {
        return;
    }
    // remaining = baseline - consumed（首次导入时 baseline 取自上游真实剩余，
    // 之后不再被上游回血污染）。如果 baseline 还没有，公开版兜底用 100，避免
    // UI 在上游接口尚未返回时白屏。
    let baseline = public_usage_baseline(account).unwrap_or(100);
    let remaining = public_usage_remaining_percent(account);
    let used = (baseline - remaining).max(0);
    let now = now_ts();
    let last_updated = account
        .quota
        .as_ref()
        .and_then(|q| q.last_updated)
        .or(Some(now));
    let error = account.quota.as_ref().and_then(|q| q.error.clone());
    let metric = QuotaMetric {
        key: "superai-public".to_string(),
        label: "日限".to_string(),
        remaining_percent: Some(remaining),
        reset_at: account.subscription_active_until.clone(),
        detail: Some(format!("已用 {used}% / 共 {baseline}%")),
        state: Some(quota_state_from_remaining(remaining)),
    };
    account.quota = Some(AccountQuota {
        metrics: vec![metric],
        last_updated,
        error,
        is_forbidden: Some(false),
    });
}

/// 在 refresh 完成后调一次：累计 + 改写 quota。返回是否本次首次耗尽（上层用于 emit）。
fn apply_public_usage_after_refresh(account: &mut ManagedAccount) -> bool {
    let (just_exhausted, _) = bump_public_usage(account);
    rewrite_quota_for_public_usage(account);
    just_exhausted
}

// ---------------------------------------------------------------------------
// 公开版账号"本地累计用量"持久化历史（24h TTL）
//
// 用户误删账号后重新导入同一 batch_key 时，期望使用记录不被清零。我们用
// `public_usage_history` 表存一份 baseline / consumed / last_remote /
// exhausted_at 快照，TTL 24h；超过窗口或没命中即按"首次导入"重新初始化。
// 仅公开版 + windsurf provider + 含 batch_key 的账号生效。
// ---------------------------------------------------------------------------

const PUBLIC_USAGE_HISTORY_TTL_SECS: i64 = 24 * 60 * 60;

/// batch_key 的稳定 hash，作为 history 表 PK。SHA-256 hex，避免明文落库。
fn batch_key_history_key(batch_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(batch_key.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 从 account 当前 payload 提取 usage 快照（baseline / consumed / last_remote /
/// exhausted_at）。任一字段为空则返回 None（无值得保存的记录）。
fn build_public_usage_snapshot(account: &ManagedAccount) -> Option<Value> {
    let baseline = windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_BASELINE);
    let last_remote = windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_LAST_REMOTE);
    let consumed = windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_CONSUMED);
    let exhausted_at = windsurf_payload_get_value(account, PUBLIC_USAGE_KEY_EXHAUSTED_AT);
    if baseline.is_none() && last_remote.is_none() && consumed.is_none() {
        return None;
    }
    let mut obj = serde_json::Map::new();
    if let Some(value) = baseline {
        obj.insert(PUBLIC_USAGE_KEY_BASELINE.to_string(), value);
    }
    if let Some(value) = last_remote {
        obj.insert(PUBLIC_USAGE_KEY_LAST_REMOTE.to_string(), value);
    }
    if let Some(value) = consumed {
        obj.insert(PUBLIC_USAGE_KEY_CONSUMED.to_string(), value);
    }
    if let Some(value) = exhausted_at {
        obj.insert(PUBLIC_USAGE_KEY_EXHAUSTED_AT.to_string(), value);
    }
    Some(Value::Object(obj))
}

/// 删账号前调用：把 public usage 快照写进 history 表。仅对公开版 + 含 batch_key
/// 的 windsurf 账号生效；其它账号、其它构建模式一律 no-op，不报错。
fn stash_public_usage_history(app: &tauri::AppHandle, account: &ManagedAccount) {
    if !is_public_build() || account.provider != "windsurf" {
        return;
    }
    let Some(batch_key) = windsurf_payload_string(account, "batch_key") else {
        return;
    };
    let Some(snapshot) = build_public_usage_snapshot(account) else {
        return;
    };
    let snapshot_json = match serde_json::to_string(&snapshot) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[usage-history] 序列化失败: {error}");
            return;
        }
    };
    // 用与 windsurf 账号正文同一把 AES key 加密落库。明文 snapshot 含 baseline /
    // consumed / last_remote 等本地用量信息，不希望用户直接打开 sqlite 就能改。
    let cipher_text = match superai_encrypt_text(&snapshot_json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[usage-history] 加密失败: {error}");
            return;
        }
    };
    let key = batch_key_history_key(&batch_key);
    let now = now_ts();
    match open_app_db(app) {
        Ok(conn) => {
            // 顺手清掉超过 TTL 的旧记录，避免表无限制膨胀。
            let _ = conn.execute(
                "DELETE FROM public_usage_history WHERE saved_at < ?1",
                params![now - PUBLIC_USAGE_HISTORY_TTL_SECS],
            );
            if let Err(error) = conn.execute(
                "INSERT INTO public_usage_history (key, snapshot_json, saved_at) \
                 VALUES (?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET snapshot_json = excluded.snapshot_json, saved_at = excluded.saved_at",
                params![key, cipher_text, now],
            ) {
                eprintln!("[usage-history] 写入失败: {error}");
            }
        }
        Err(error) => eprintln!("[usage-history] 打开 DB 失败: {error}"),
    }
}

/// 重新导入时调用：若 24h 内有同一 batch_key 的快照，回填到 account.auth_payload。
/// 命中并写回字段返回 true，未命中返回 false。
fn restore_public_usage_history(
    app: &tauri::AppHandle,
    account: &mut ManagedAccount,
    batch_key: &str,
) -> bool {
    if !is_public_build() || account.provider != "windsurf" {
        return false;
    }
    let key = batch_key_history_key(batch_key);
    let conn = match open_app_db(app) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("[usage-history] 打开 DB 失败: {error}");
            return false;
        }
    };
    let now = now_ts();
    // TTL 过期的快照视为不存在；顺手清理掉。
    let _ = conn.execute(
        "DELETE FROM public_usage_history WHERE saved_at < ?1",
        params![now - PUBLIC_USAGE_HISTORY_TTL_SECS],
    );
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT snapshot_json, saved_at FROM public_usage_history WHERE key = ?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((cipher_text, saved_at)) = row else {
        return false;
    };
    if now - saved_at > PUBLIC_USAGE_HISTORY_TTL_SECS {
        return false;
    }
    // snapshot 在 stash 时用 AES 加密；解密失败视为脏数据丢弃。
    let snapshot_json = match superai_decrypt_text(&cipher_text) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[usage-history] 解密失败: {error}");
            return false;
        }
    };
    let snapshot: Value = match serde_json::from_str(&snapshot_json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[usage-history] 反序列化失败: {error}");
            return false;
        }
    };
    let Some(obj) = snapshot.as_object() else {
        return false;
    };
    for field in [
        PUBLIC_USAGE_KEY_BASELINE,
        PUBLIC_USAGE_KEY_LAST_REMOTE,
        PUBLIC_USAGE_KEY_CONSUMED,
        PUBLIC_USAGE_KEY_EXHAUSTED_AT,
    ] {
        if let Some(value) = obj.get(field).cloned() {
            windsurf_payload_set_value(account, field, value);
        }
    }
    true
}

fn emit_account_exhausted(app: &tauri::AppHandle, account: &ManagedAccount) {
    let _ = app.emit(
        "account-exhausted",
        serde_json::json!({
            "id": account.id,
            "provider": account.provider,
            "consumed_percent": public_usage_consumed_percent(account),
        }),
    );
}

fn public_windsurf_export_key(account: &ManagedAccount) -> Result<String, String> {
    if account.provider != "windsurf" {
        return Err("只支持导出 SuperAI 公开版数据".to_string());
    }
    let payload = account.auth_payload.as_ref().and_then(Value::as_object);
    let credential = payload
        .and_then(|payload| payload.get("batch_key"))
        .and_then(Value::as_str)
        .and_then(|key| parse_windsurf_batch_key_line(key).ok())
        .ok_or_else(|| "该账号缺少可导出的批量密钥，请重新通过批量密钥导入".to_string())?;
    let expires_at = account
        .subscription_active_until
        .as_ref()
        .and_then(normalize_unix_seconds_value)
        .or_else(|| {
            windsurf_payload_string(account, "expires_at")
                .as_deref()
                .and_then(normalize_unix_seconds_str)
        })
        .map(|value| value.to_string())
        .unwrap_or_else(|| credential.expires_at.to_string());
    let payload = serde_json::json!({
        "account": credential.account,
        "password": credential.password,
        "expires_at": expires_at,
    });
    superai_encrypt_text(&payload.to_string())
}

#[tauri::command]
#[allow(non_snake_case)]
async fn add_superai_account_by_token(
    app: tauri::AppHandle,
    token: String,
    label: Option<String>,
) -> Result<ManagedAccount, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("凭证不能为空".to_string());
    }

    let register = windsurf_register_with_codeium(trimmed).await?;
    let jwt = parse_jwt_payload(trimmed);
    let email = string_field(jwt.as_ref().and_then(|j| j.get("email")))
        .or_else(|| label.clone())
        .unwrap_or_else(|| {
            format!(
                "windsurf-token-{}@local",
                &stable_hash(&register.api_key)[..6]
            )
        });
    let local_id = string_field(jwt.as_ref().and_then(|j| j.get("user_id")))
        .or_else(|| string_field(jwt.as_ref().and_then(|j| j.get("sub"))));
    let display_name = string_field(jwt.as_ref().and_then(|j| j.get("name")))
        .or_else(|| register.name.clone())
        .or(label);
    let exp = jwt.as_ref().and_then(|j| number_field(j.get("exp")));

    let mut tokens_map = serde_json::Map::new();
    tokens_map.insert(
        "api_key".to_string(),
        Value::String(register.api_key.clone()),
    );
    tokens_map.insert("id_token".to_string(), Value::String(trimmed.to_string()));
    tokens_map.insert(
        "access_token".to_string(),
        Value::String(trimmed.to_string()),
    );
    if let Some(value) = local_id.clone() {
        tokens_map.insert("local_id".to_string(), Value::String(value));
    }
    if let Some(value) = exp {
        tokens_map.insert("expires_at".to_string(), Value::Number(value.into()));
    }

    let mut payload = serde_json::Map::new();
    payload.insert(
        "provider".to_string(),
        Value::String("windsurf".to_string()),
    );
    payload.insert("email".to_string(), Value::String(email));
    if let Some(name) = display_name.clone() {
        payload.insert("display_name".to_string(), Value::String(name));
    }
    if let Some(api_server_url) = register.api_server_url {
        payload.insert("api_server_url".to_string(), Value::String(api_server_url));
    }
    payload.insert("tokens".to_string(), Value::Object(tokens_map));

    let account = parse_windsurf_account(&Value::Object(payload), "windsurf_token")
        .ok_or_else(|| "构建 SuperAI 账号记录失败".to_string())?;
    upsert_accounts_into_db(&app, std::slice::from_ref(&account))?;
    Ok(account_for_frontend(&account))
}

fn codex_access_token(account: &ManagedAccount) -> Option<String> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    let tokens = payload.get("tokens").and_then(Value::as_object);
    string_field(
        tokens
            .and_then(|t| t.get("access_token"))
            .or_else(|| payload.get("access_token")),
    )
}

fn codex_subscription_until_from_payload(account: &ManagedAccount) -> Option<Value> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    payload
        .get("subscription_active_until")
        .or_else(|| payload.get("subscriptionActiveUntil"))
        .cloned()
        .or_else(|| {
            let tokens = payload.get("tokens").and_then(Value::as_object);
            let id_token = string_field(
                tokens
                    .and_then(|t| t.get("id_token"))
                    .or_else(|| payload.get("id_token")),
            )?;
            let jwt = parse_jwt_payload(&id_token)?;
            jwt.get("https://api.openai.com/auth")
                .and_then(Value::as_object)
                .and_then(|auth| auth.get("chatgpt_subscription_active_until"))
                .cloned()
        })
}

fn parse_codex_account_profile(
    payload: &Value,
    account: &ManagedAccount,
) -> (Option<String>, Option<String>) {
    let Some(accounts) = payload.get("accounts").and_then(Value::as_object) else {
        return (None, None);
    };

    let mut fallback: Option<&serde_json::Map<String, Value>> = None;
    let mut selected: Option<&serde_json::Map<String, Value>> = None;
    for (key, entry) in accounts {
        if key == "default" {
            continue;
        }
        let Some(account_obj) = entry
            .get("account")
            .and_then(Value::as_object)
            .or_else(|| entry.as_object())
        else {
            continue;
        };
        let Some(remote_account_id) = string_field(account_obj.get("account_id")) else {
            continue;
        };
        if fallback.is_none() {
            fallback = Some(account_obj);
        }
        if account.account_id.as_deref() == Some(remote_account_id.as_str()) {
            selected = Some(account_obj);
            break;
        }
    }

    let Some(account_obj) = selected.or(fallback) else {
        return (None, None);
    };
    (
        string_field(account_obj.get("name")),
        string_field(account_obj.get("account_id")),
    )
}

fn usage_window_metric(
    key: &str,
    fallback_label: &str,
    window: Option<&Value>,
) -> Option<QuotaMetric> {
    let window = window?.as_object()?;
    let used = number_field(window.get("used_percent"))
        .unwrap_or(0)
        .clamp(0, 100);
    let remaining = 100 - used;
    let window_minutes =
        number_field(window.get("limit_window_seconds")).map(|seconds| (seconds + 59) / 60);
    let reset_at = number_field(window.get("reset_at")).or_else(|| {
        number_field(window.get("reset_after_seconds")).map(|seconds| now_ts() + seconds)
    });
    Some(QuotaMetric {
        key: key.to_string(),
        label: window_minutes
            .map(|minutes| {
                if minutes >= 1440 {
                    format!("{}D", minutes / 1440)
                } else {
                    format!("{}H", (minutes + 59) / 60)
                }
            })
            .unwrap_or_else(|| fallback_label.to_string()),
        remaining_percent: Some(remaining),
        reset_at: reset_at.map(|value| Value::Number(value.into())),
        detail: Some(format!("剩余 {remaining}%")),
        state: Some(quota_state(Some(remaining))),
    })
}

fn parse_codex_usage_quota(payload: &Value) -> AccountQuota {
    let rate_limit = payload.get("rate_limit");
    let mut metrics = Vec::new();
    if let Some(metric) = usage_window_metric(
        "primary",
        "5H",
        rate_limit.and_then(|r| r.get("primary_window")),
    ) {
        metrics.push(metric);
    }
    if let Some(metric) = usage_window_metric(
        "secondary",
        "周限",
        rate_limit.and_then(|r| r.get("secondary_window")),
    ) {
        metrics.push(metric);
    }
    AccountQuota {
        metrics,
        last_updated: Some(now_ts()),
        error: None,
        is_forbidden: None,
    }
}

async fn refresh_codex_account_remote(account: &mut ManagedAccount) -> Result<(), String> {
    if account.provider != "codex" || !account.token_meta.has_access_token {
        return Ok(());
    }
    let access_token =
        codex_access_token(account).ok_or_else(|| "缺少 access token".to_string())?;
    if account.subscription_active_until.is_none() {
        account.subscription_active_until = codex_subscription_until_from_payload(account);
    }
    let account_id = account
        .account_id
        .clone()
        .ok_or_else(|| "缺少 ChatGPT account_id".to_string())?;

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|error| format!("构建 Authorization 头失败: {error}"))?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static(CODEX_API_USER_AGENT));
    headers.insert(
        "ChatGPT-Account-Id",
        HeaderValue::from_str(&account_id)
            .map_err(|error| format!("构建 ChatGPT-Account-Id 头失败: {error}"))?,
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("创建 Codex API 客户端失败: {error}"))?;
    let profile_response = client
        .get(CODEX_ACCOUNT_CHECK_URL)
        .headers(headers.clone())
        .send()
        .await
        .map_err(|error| format!("请求账号信息失败: {error}"))?;
    if profile_response.status().is_success() {
        let payload = profile_response
            .json::<Value>()
            .await
            .map_err(|error| format!("解析账号信息失败: {error}"))?;
        let (account_name, account_id) = parse_codex_account_profile(&payload, account);
        if account_name.is_some() {
            account.display_name = account_name;
        }
        if account_id.is_some() {
            account.account_id = account_id;
        }
    }

    let usage_response = client
        .get(CODEX_USAGE_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|error| format!("请求配额信息失败: {error}"))?;
    let status = usage_response.status();
    let usage_body = usage_response
        .text()
        .await
        .map_err(|error| format!("读取配额响应失败: {error}"))?;
    if status.is_success() {
        let payload: Value = serde_json::from_str(&usage_body)
            .map_err(|error| format!("解析配额信息失败: {error}"))?;
        if let Some(plan_type) = string_field(payload.get("plan_type")) {
            account.plan = Some(plan_type.clone());
            account.plan_type = Some(plan_type);
        }
        account.quota = Some(parse_codex_usage_quota(&payload));
    } else {
        account.quota = Some(AccountQuota {
            metrics: vec![],
            last_updated: Some(now_ts()),
            error: Some(format!("API 返回错误 {status}")),
            is_forbidden: Some(status.as_u16() == 403),
        });
    }

    account.updated_at = now_ts();
    account.status = Some(fallback_status_refreshed(account));
    Ok(())
}

fn gemini_payload_string(account: &ManagedAccount, snake: &str, camel: &str) -> Option<String> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    let token = payload.get("token").and_then(Value::as_object);
    nested_string_field(payload, token, snake, camel)
}

fn gemini_payload_expiry(account: &ManagedAccount) -> Option<i64> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    let token = payload.get("token").and_then(Value::as_object);
    number_field(payload.get("expiry_date"))
        .or_else(|| number_field(payload.get("expiryDate")))
        .or_else(|| token.and_then(|t| number_field(t.get("expires_at"))))
        .or_else(|| token.and_then(|t| number_field(t.get("expiresAt"))))
}

async fn load_gemini_code_assist_status(
    access_token: &str,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let payload = serde_json::json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        }
    });
    let value = post_gemini_code_assist_json(
        access_token,
        GEMINI_CODE_ASSIST_LOAD_URL,
        &payload,
        "loadCodeAssist",
    )
    .await?;
    let current_tier_id = value
        .get("currentTier")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_non_empty(Some(v)));
    let current_tier_name = value
        .get("currentTier")
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_non_empty(Some(v)));
    let paid_tier_id = value
        .get("paidTier")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_non_empty(Some(v)));
    let paid_tier_name = value
        .get("paidTier")
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_non_empty(Some(v)));
    let first_allowed_tier_id = value
        .get("allowedTiers")
        .and_then(Value::as_array)
        .and_then(|tiers| tiers.first())
        .and_then(|tier| tier.get("id"))
        .and_then(Value::as_str)
        .and_then(|v| normalize_non_empty(Some(v)));
    let project_id = value
        .get("cloudaicompanionProject")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("cloudaicompanionProject")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value
                .get("cloudaicompanionProject")
                .and_then(|v| v.get("projectId"))
                .and_then(Value::as_str)
        })
        .and_then(|v| normalize_non_empty(Some(v)));
    Ok((
        paid_tier_id.or(current_tier_id).or(first_allowed_tier_id),
        paid_tier_name.or(current_tier_name),
        project_id,
    ))
}

async fn refresh_gemini_account_remote(account: &mut ManagedAccount) -> Result<(), String> {
    if account.provider != "gemini" {
        return Ok(());
    }
    let mut access_token = gemini_payload_string(account, "access_token", "accessToken")
        .ok_or_else(|| "缺少 Gemini access_token".to_string())?;
    let refresh_token = gemini_payload_string(account, "refresh_token", "refreshToken");

    if gemini_payload_expiry(account)
        .map(|expiry| expiry <= now_ts_ms() + 60_000)
        .unwrap_or(false)
    {
        let refresh_token = refresh_token
            .clone()
            .ok_or_else(|| "Gemini refresh_token 不存在，无法刷新 access_token".to_string())?;
        let refreshed = refresh_gemini_access_token(&refresh_token).await?;
        access_token = refreshed
            .access_token
            .ok_or_else(|| "Gemini token 刷新后 access_token 为空".to_string())?;
        if let Some(payload) = account.auth_payload.as_mut().and_then(Value::as_object_mut) {
            payload.insert(
                "access_token".to_string(),
                Value::String(access_token.clone()),
            );
            if let Some(id_token) = refreshed.id_token {
                payload.insert("id_token".to_string(), Value::String(id_token));
            }
            if let Some(token_type) = refreshed.token_type {
                payload.insert("token_type".to_string(), Value::String(token_type));
            }
            if let Some(scope) = refreshed.scope {
                payload.insert("scope".to_string(), Value::String(scope));
            }
            if let Some(expires_in) = refreshed.expires_in {
                let expiry_date = now_ts_ms() + expires_in.saturating_mul(1000);
                payload.insert("expiry_date".to_string(), Value::Number(expiry_date.into()));
                account.token_meta.expires_at = Some(expiry_date);
                account.subscription_active_until = Some(Value::Number(expiry_date.into()));
            }
        }
    }

    if let Some(userinfo) = fetch_google_userinfo(&access_token).await {
        if let Some(email) = normalize_non_empty(userinfo.email.as_deref()) {
            account.email = email.to_lowercase();
        }
        if account.user_id.is_none() {
            account.user_id = normalize_non_empty(userinfo.id.as_deref());
        }
        if account.account_id.is_none() {
            account.account_id = account.user_id.clone();
        }
        if account.display_name.is_none() {
            account.display_name = normalize_non_empty(userinfo.name.as_deref());
        }
    }

    let mut status = load_gemini_code_assist_status(&access_token).await;
    if let Err(error) = &status {
        if error.contains("UNAUTHORIZED") {
            if let Some(refresh_token) = refresh_token {
                let refreshed = refresh_gemini_access_token(&refresh_token).await?;
                access_token = refreshed
                    .access_token
                    .ok_or_else(|| "Gemini token 刷新后 access_token 为空".to_string())?;
                if let Some(payload) = account.auth_payload.as_mut().and_then(Value::as_object_mut)
                {
                    payload.insert(
                        "access_token".to_string(),
                        Value::String(access_token.clone()),
                    );
                }
                status = load_gemini_code_assist_status(&access_token).await;
            }
        }
    }
    let (tier_id, tier_name, project_id) = status?;
    if let Some(tier_id) = tier_id.clone() {
        account.plan_type = Some(tier_id);
    }
    account.plan = tier_name.or(tier_id);

    if let Some(project_id) = project_id {
        match post_gemini_code_assist_json(
            &access_token,
            GEMINI_CODE_ASSIST_QUOTA_URL,
            &serde_json::json!({ "project": project_id }),
            "retrieveUserQuota",
        )
        .await
        {
            Ok(quota) => {
                if let Some(payload) = account.auth_payload.as_mut().and_then(Value::as_object_mut)
                {
                    payload.insert("gemini_usage_raw".to_string(), quota.clone());
                    payload.insert(
                        "usage_updated_at".to_string(),
                        Value::Number(now_ts().into()),
                    );
                }
                let empty = serde_json::Map::new();
                let payload = account
                    .auth_payload
                    .as_ref()
                    .and_then(Value::as_object)
                    .unwrap_or(&empty);
                account.quota = parse_gemini_quota(payload);
            }
            Err(error) => {
                account.quota = Some(AccountQuota {
                    metrics: vec![],
                    last_updated: Some(now_ts()),
                    error: Some(error.clone()),
                    is_forbidden: Some(
                        error.to_ascii_lowercase().contains("403")
                            || error.to_ascii_lowercase().contains("forbidden"),
                    ),
                });
            }
        }
    }

    account.updated_at = now_ts();
    account.status = Some(fallback_status_refreshed(account));
    Ok(())
}

fn fallback_status_refreshed(account: &ManagedAccount) -> AccountStatus {
    let empty = serde_json::Map::new();
    let obj = account
        .auth_payload
        .as_ref()
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    derive_status(obj, &account.token_meta, account.quota.as_ref())
}

fn mark_account_unavailable(account: &mut ManagedAccount, reason: String) {
    let now = now_ts();
    account.quota = Some(AccountQuota {
        metrics: vec![],
        last_updated: Some(now),
        error: Some(reason.clone()),
        is_forbidden: None,
    });
    account.status = Some(AccountStatus {
        state: "unavailable".to_string(),
        label: "不可用".to_string(),
        reason: Some(reason),
        updated_at: Some(now),
    });
    account.updated_at = now;
}

async fn refresh_imported_accounts(accounts: &mut [ManagedAccount]) {
    for account in accounts {
        match account.provider.as_str() {
            "codex" => {
                if let Err(error) = refresh_codex_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            "gemini" => {
                if let Err(error) = refresh_gemini_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            "windsurf" => {
                if let Err(error) = refresh_windsurf_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            _ => {}
        }
    }
}

fn persist_and_refresh_imported(
    app: tauri::AppHandle,
    result: ImportResult,
) -> Result<ImportResult, String> {
    upsert_accounts_into_db(&app, &result.imported)?;
    refresh_imported_accounts_in_background(app, result.imported.clone());
    Ok(import_result_for_frontend(result))
}

fn oauth_pending_get(login_id: &str) -> Result<Option<OAuthPending>, String> {
    Ok(OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .get(login_id)
        .cloned())
}

fn oauth_pending_remove(login_id: &str) {
    if let Ok(mut guard) = OAUTH_PENDING.lock() {
        guard.remove(login_id);
    }
}

fn refresh_imported_accounts_in_background(app: tauri::AppHandle, accounts: Vec<ManagedAccount>) {
    tauri::async_runtime::spawn(async move {
        let mut refreshed = accounts;
        refresh_imported_accounts(&mut refreshed).await;
        let _ = upsert_existing_accounts_into_db(&app, &refreshed);
    });
}

fn parse_auth_json_content(content: &str, source: &str, label: &str) -> ImportResult {
    let parsed: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => {
            return ImportResult {
                imported: vec![],
                failed: vec![ImportFailure {
                    label: label.to_string(),
                    reason: "JSON 格式无效".to_string(),
                }],
            }
        }
    };

    let items: Vec<Value> = if let Some(arr) = parsed.as_array() {
        arr.clone()
    } else if let Some(accounts) = parsed.get("accounts").and_then(Value::as_array) {
        accounts.clone()
    } else {
        vec![parsed]
    };

    let mut imported = Vec::new();
    let mut failed = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let item_label = if items.len() > 1 {
            format!("{label} #{}", idx + 1)
        } else {
            label.to_string()
        };
        if let Some(account) = parse_codex_account(item, source)
            .or_else(|| parse_windsurf_account(item, source))
            .or_else(|| parse_gemini_account(item, source))
        {
            imported.push(account);
        } else {
            failed.push(ImportFailure {
                label: item_label,
                reason: "未识别到 Codex / Gemini / SuperAI 凭证字段".to_string(),
            });
        }
    }

    let mut dedup = HashMap::<String, ManagedAccount>::new();
    for account in imported {
        dedup.insert(account.id.clone(), account);
    }

    ImportResult {
        imported: dedup.into_values().collect(),
        failed,
    }
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())
}

fn read_to_string(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("读取文件失败: {} ({})", path.display(), e))
}

fn write_string_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录失败 {}: {error}", parent.display()))?;
    }
    let tmp_path = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("json")
    ));
    fs::write(&tmp_path, content)
        .map_err(|error| format!("写入临时文件失败 {}: {error}", tmp_path.display()))?;
    rename_replace_file(&tmp_path, path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

fn rename_replace_file(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if to.exists() {
            fs::remove_file(to)
                .map_err(|error| format!("移除旧文件失败 {}: {error}", to.display()))?;
        }
    }

    fs::rename(from, to).map_err(|error| format!("替换文件失败 {}: {error}", to.display()))
}

fn codex_home_dir() -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("CODEX_HOME") {
        let trimmed = raw.trim().trim_matches('"').trim_matches('\'').trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(home_dir()?.join(".codex"))
}

fn gemini_home_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".gemini"))
}

fn nested_string_field<'a>(
    obj: &'a serde_json::Map<String, Value>,
    nested: Option<&'a serde_json::Map<String, Value>>,
    snake: &str,
    camel: &str,
) -> Option<String> {
    string_field(
        nested
            .and_then(|tokens| tokens.get(snake))
            .or_else(|| nested.and_then(|tokens| tokens.get(camel)))
            .or_else(|| obj.get(snake))
            .or_else(|| obj.get(camel)),
    )
}

fn build_codex_auth_payload(account: &ManagedAccount) -> Result<Value, String> {
    let payload = account
        .auth_payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| "该账号缺少可写入的 Codex 凭证，请重新导入 auth.json".to_string())?;
    let tokens = payload.get("tokens").and_then(Value::as_object);
    let api_key = string_field(payload.get("OPENAI_API_KEY"));
    let auth_mode = string_field(payload.get("auth_mode"))
        .unwrap_or_default()
        .to_lowercase();

    if auth_mode == "apikey" || (tokens.is_none() && api_key.is_some()) {
        let api_key = api_key.ok_or_else(|| "API Key 账号缺少 OPENAI_API_KEY".to_string())?;
        let mut result = serde_json::Map::new();
        result.insert("auth_mode".to_string(), Value::String("apikey".to_string()));
        result.insert("OPENAI_API_KEY".to_string(), Value::String(api_key));
        if let Some(base_url) = string_field(payload.get("base_url")) {
            result.insert("base_url".to_string(), Value::String(base_url));
        }
        return Ok(Value::Object(result));
    }

    let id_token = nested_string_field(payload, tokens, "id_token", "idToken")
        .ok_or_else(|| "OAuth 账号缺少 id_token".to_string())?;
    let access_token = nested_string_field(payload, tokens, "access_token", "accessToken")
        .ok_or_else(|| "OAuth 账号缺少 access_token".to_string())?;
    let refresh_token = nested_string_field(payload, tokens, "refresh_token", "refreshToken");
    let account_id = nested_string_field(payload, tokens, "account_id", "accountId")
        .or_else(|| account.account_id.clone());

    let mut token_map = serde_json::Map::new();
    token_map.insert("id_token".to_string(), Value::String(id_token));
    token_map.insert("access_token".to_string(), Value::String(access_token));
    if let Some(refresh_token) = refresh_token {
        token_map.insert("refresh_token".to_string(), Value::String(refresh_token));
    }
    if let Some(account_id) = account_id {
        token_map.insert("account_id".to_string(), Value::String(account_id));
    }

    let mut result = serde_json::Map::new();
    result.insert(
        "auth_mode".to_string(),
        Value::String("chatgpt".to_string()),
    );
    result.insert("OPENAI_API_KEY".to_string(), Value::Null);
    result.insert("tokens".to_string(), Value::Object(token_map));
    result.insert(
        "last_refresh".to_string(),
        payload
            .get("last_refresh")
            .cloned()
            .unwrap_or_else(|| Value::String(codex_last_refresh_now())),
    );
    Ok(Value::Object(result))
}

fn write_codex_auth(account: &ManagedAccount) -> Result<(), String> {
    let auth_payload = build_codex_auth_payload(account)?;
    let content = serde_json::to_string_pretty(&auth_payload)
        .map_err(|error| format!("序列化 Codex auth.json 失败: {error}"))?;
    let codex_home = codex_home_dir()?;
    write_string_atomic(&codex_home.join("auth.json"), &content)?;
    if auth_payload.get("tokens").is_some() {
        write_codex_keychain(&codex_home, &content)?;
    }
    Ok(())
}

fn build_codex_keychain_account(base_dir: &Path) -> String {
    let resolved_home = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(resolved_home.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    format!("cli|{}", &format!("{digest:x}")[..16])
}

#[cfg(target_os = "macos")]
fn write_codex_keychain(base_dir: &Path, secret: &str) -> Result<(), String> {
    let account = build_codex_keychain_account(base_dir);
    let output = Command::new("security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(account)
        .arg("-w")
        .arg(secret)
        .output()
        .map_err(|error| format!("执行 security 写入 Codex Keychain 失败: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "写入 Codex Keychain 失败: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(not(target_os = "macos"))]
fn write_codex_keychain(_base_dir: &Path, _secret: &str) -> Result<(), String> {
    Ok(())
}

fn build_gemini_oauth_payload(account: &ManagedAccount) -> Result<Value, String> {
    let payload = account
        .auth_payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| "该账号缺少可写入的 Gemini 凭证，请重新导入 oauth_creds.json".to_string())?;
    let token = payload.get("token").and_then(Value::as_object);
    let access_token = nested_string_field(payload, token, "access_token", "accessToken")
        .ok_or_else(|| "Gemini 账号缺少 access_token".to_string())?;
    let refresh_token = nested_string_field(payload, token, "refresh_token", "refreshToken");
    let id_token = nested_string_field(payload, token, "id_token", "idToken");
    let token_type = nested_string_field(payload, token, "token_type", "tokenType")
        .unwrap_or_else(|| "Bearer".to_string());
    let scope = nested_string_field(payload, token, "scope", "scope");
    let expiry_date = payload
        .get("expiry_date")
        .or_else(|| payload.get("expiryDate"))
        .or_else(|| token.and_then(|t| t.get("expires_at")))
        .or_else(|| token.and_then(|t| t.get("expiresAt")))
        .cloned();

    let mut result = serde_json::Map::new();
    result.insert("access_token".to_string(), Value::String(access_token));
    if let Some(refresh_token) = refresh_token {
        result.insert("refresh_token".to_string(), Value::String(refresh_token));
    }
    if let Some(id_token) = id_token {
        result.insert("id_token".to_string(), Value::String(id_token));
    }
    result.insert("token_type".to_string(), Value::String(token_type));
    if let Some(scope) = scope {
        result.insert("scope".to_string(), Value::String(scope));
    }
    if let Some(expiry_date) = expiry_date {
        result.insert("expiry_date".to_string(), expiry_date);
    }
    Ok(Value::Object(result))
}

fn write_gemini_active_account(email: &str) -> Result<(), String> {
    let path = gemini_home_dir()?.join("google_accounts.json");
    let mut value = if path.exists() {
        serde_json::from_str::<Value>(&read_to_string(&path)?)
            .unwrap_or_else(|_| serde_json::json!({ "old": [] }))
    } else {
        serde_json::json!({ "old": [] })
    };
    if !value.is_object() {
        value = serde_json::json!({ "old": [] });
    }
    let obj = value
        .as_object_mut()
        .ok_or_else(|| "Gemini google_accounts.json 根结构非法".to_string())?;
    if let Some(active) = obj
        .get("active")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    {
        if !active.eq_ignore_ascii_case(email) {
            let old = obj.entry("old").or_insert_with(|| Value::Array(Vec::new()));
            if let Some(arr) = old.as_array_mut() {
                if !arr
                    .iter()
                    .any(|item| item.as_str() == Some(active.as_str()))
                {
                    arr.push(Value::String(active));
                }
                arr.retain(|item| {
                    item.as_str()
                        .map(|old_email| !old_email.eq_ignore_ascii_case(email))
                        .unwrap_or(true)
                });
            }
        }
    }
    obj.insert("active".to_string(), Value::String(email.to_string()));
    let content = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("序列化 Gemini google_accounts.json 失败: {error}"))?;
    write_string_atomic(&path, &content)
}

fn write_gemini_selected_auth_type() -> Result<(), String> {
    let path = gemini_home_dir()?.join("settings.json");
    let mut value = if path.exists() {
        serde_json::from_str::<Value>(&read_to_string(&path)?)
            .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    } else {
        Value::Object(serde_json::Map::new())
    };
    if !value.is_object() {
        value = Value::Object(serde_json::Map::new());
    }
    let root = value
        .as_object_mut()
        .ok_or_else(|| "Gemini settings.json 根结构非法".to_string())?;
    let security = root
        .entry("security")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !security.is_object() {
        *security = Value::Object(serde_json::Map::new());
    }
    let security_obj = security
        .as_object_mut()
        .ok_or_else(|| "Gemini settings.json.security 结构非法".to_string())?;
    let auth = security_obj
        .entry("auth")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !auth.is_object() {
        *auth = Value::Object(serde_json::Map::new());
    }
    auth.as_object_mut()
        .ok_or_else(|| "Gemini settings.json.security.auth 结构非法".to_string())?
        .insert(
            "selectedType".to_string(),
            Value::String("oauth-personal".to_string()),
        );
    let content = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("序列化 Gemini settings.json 失败: {error}"))?;
    write_string_atomic(&path, &content)
}

fn clear_gemini_file_keychain() -> Result<(), String> {
    let path = gemini_home_dir()?.join(GEMINI_FILE_KEYCHAIN_FILE);
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path)
        .map_err(|error| format!("清理 Gemini file keychain 失败 {}: {error}", path.display()))
}

fn write_gemini_auth(account: &ManagedAccount) -> Result<(), String> {
    let oauth_payload = build_gemini_oauth_payload(account)?;
    let oauth_content = serde_json::to_string_pretty(&oauth_payload)
        .map_err(|error| format!("序列化 Gemini oauth_creds.json 失败: {error}"))?;
    write_string_atomic(&gemini_home_dir()?.join("oauth_creds.json"), &oauth_content)?;
    write_gemini_keychain(&oauth_payload)?;
    clear_gemini_file_keychain()?;
    write_gemini_active_account(&account.email)?;
    write_gemini_selected_auth_type()
}

#[cfg(target_os = "macos")]
fn write_gemini_keychain(oauth_payload: &Value) -> Result<(), String> {
    if !is_macos_default_keychain_available() {
        return Ok(());
    }
    let Some(obj) = oauth_payload.as_object() else {
        return Ok(());
    };
    let access_token = string_field(obj.get("access_token"))
        .ok_or_else(|| "Gemini Keychain 写入失败: access_token 为空".to_string())?;
    let token_type = string_field(obj.get("token_type")).unwrap_or_else(|| "Bearer".to_string());
    let mut token = serde_json::Map::new();
    token.insert("accessToken".to_string(), Value::String(access_token));
    token.insert("tokenType".to_string(), Value::String(token_type));
    if let Some(refresh_token) = string_field(obj.get("refresh_token")) {
        token.insert("refreshToken".to_string(), Value::String(refresh_token));
    }
    if let Some(scope) = string_field(obj.get("scope")) {
        token.insert("scope".to_string(), Value::String(scope));
    }
    if let Some(expiry_date) = obj.get("expiry_date").cloned() {
        token.insert("expiresAt".to_string(), expiry_date);
    }
    let payload = serde_json::json!({
        "serverName": GEMINI_KEYCHAIN_ACCOUNT,
        "token": token,
        "updatedAt": now_ts() * 1000,
    });
    let secret = serde_json::to_string(&payload)
        .map_err(|error| format!("序列化 Gemini Keychain 凭证失败: {error}"))?;
    let output = Command::new("security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(GEMINI_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(GEMINI_KEYCHAIN_ACCOUNT)
        .arg("-w")
        .arg(secret)
        .output()
        .map_err(|error| format!("执行 security 写入 Gemini Keychain 失败: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "写入 Gemini Keychain 失败: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(not(target_os = "macos"))]
fn write_gemini_keychain(_oauth_payload: &Value) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_macos_default_keychain_available() -> bool {
    Command::new("security")
        .arg("default-keychain")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn read_gemini_keychain() -> Result<Option<Value>, String> {
    let output = Command::new("security")
        .arg("find-generic-password")
        .arg("-s")
        .arg(GEMINI_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(GEMINI_KEYCHAIN_ACCOUNT)
        .arg("-w")
        .output()
        .map_err(|error| format!("执行 security 读取 Gemini Keychain 失败: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if secret.is_empty() {
        return Ok(None);
    }
    let parsed: Value = serde_json::from_str(&secret)
        .map_err(|error| format!("解析 Gemini Keychain 凭证失败: {error}"))?;
    let Some(token) = parsed.get("token").and_then(Value::as_object) else {
        return Ok(None);
    };
    let mut oauth = serde_json::Map::new();
    if let Some(access_token) = token.get("accessToken").and_then(Value::as_str) {
        oauth.insert(
            "access_token".to_string(),
            Value::String(access_token.to_string()),
        );
    }
    if let Some(refresh_token) = token.get("refreshToken").and_then(Value::as_str) {
        oauth.insert(
            "refresh_token".to_string(),
            Value::String(refresh_token.to_string()),
        );
    }
    if let Some(token_type) = token.get("tokenType").and_then(Value::as_str) {
        oauth.insert(
            "token_type".to_string(),
            Value::String(token_type.to_string()),
        );
    }
    if let Some(scope) = token.get("scope").and_then(Value::as_str) {
        oauth.insert("scope".to_string(), Value::String(scope.to_string()));
    }
    if let Some(expires_at) = token.get("expiresAt").cloned() {
        oauth.insert("expiry_date".to_string(), expires_at);
    }
    Ok(Some(Value::Object(oauth)))
}

#[cfg(not(target_os = "macos"))]
fn read_gemini_keychain() -> Result<Option<Value>, String> {
    Ok(None)
}

fn query_map_from_url(
    path_and_query: &str,
    port: u16,
) -> Result<(String, HashMap<String, String>), String> {
    let parsed = Url::parse(&format!("http://127.0.0.1:{port}{path_and_query}"))
        .map_err(|error| format!("解析 OAuth 回调失败: {error}"))?;
    let path = parsed.path().to_string();
    let params = parsed
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<HashMap<_, _>>();
    Ok((path, params))
}

fn respond_oauth_success(request: tiny_http::Request) {
    let html = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Authorization complete</title>
  <style>
    :root { color-scheme: dark; }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background: #0f0f0f;
      color: #ececf1;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    main {
      width: min(420px, calc(100vw - 40px));
      padding: 36px 30px 32px;
      border: 1px solid rgba(255,255,255,.12);
      border-radius: 14px;
      background: #171717;
      box-shadow: 0 24px 80px rgba(0,0,0,.42);
      text-align: center;
    }
    .mark {
      width: 42px;
      height: 42px;
      margin: 0 auto 22px;
      display: grid;
      place-items: center;
      border-radius: 50%;
      background: #10a37f;
      color: #04110d;
      font-size: 22px;
      font-weight: 800;
      line-height: 1;
    }
    h1 {
      margin: 0 0 10px;
      font-size: 22px;
      line-height: 1.25;
      font-weight: 650;
      letter-spacing: 0;
    }
    p {
      margin: 0;
      color: #b4b4b4;
      font-size: 14px;
      line-height: 1.65;
    }
  </style>
</head>
<body>
  <main>
    <div class="mark">✓</div>
    <h1>授权已完成</h1>
    <p>Super AI 已收到授权回调。<br>你可以关闭此页面并返回应用。</p>
  </main>
</body>
</html>"#;
    let mut response = Response::from_string(html);
    if let Ok(header) = Header::from_bytes("Content-Type", "text/html; charset=utf-8") {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn respond_oauth_redirect(request: tiny_http::Request, location: &str) {
    let mut response = Response::from_string(String::new()).with_status_code(StatusCode(301));
    if let Ok(header) = Header::from_bytes("Location", location) {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn notify_oauth_listener_cancel(port: u16) {
    if let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) {
        use std::io::Write;
        let _ = stream
            .write_all(b"GET /cancel HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        let _ = stream.flush();
    }
}

fn cancel_pending_oauth_for_provider(provider: &str) {
    let ports = OAUTH_PENDING
        .lock()
        .map(|mut pending| {
            let ids = pending
                .iter()
                .filter(|&(_, item)| item.provider == provider)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| pending.remove(&id).map(|item| item.port))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for port in ports {
        notify_oauth_listener_cancel(port);
    }
}

fn start_oauth_callback_listener(
    login_id: String,
    provider: String,
    port: u16,
    callback_path: String,
    success_redirect: Option<String>,
) {
    std::thread::spawn(move || {
        let server = match Server::http(format!("127.0.0.1:{port}")) {
            Ok(server) => server,
            Err(_) => return,
        };
        let started_at = now_ts();
        loop {
            if now_ts() - started_at > OAUTH_TIMEOUT_SECONDS {
                let _ = OAUTH_PENDING.lock().map(|mut pending| {
                    pending.remove(&login_id);
                });
                break;
            }

            let still_pending = OAUTH_PENDING
                .lock()
                .ok()
                .and_then(|pending| pending.get(&login_id).map(|item| item.provider == provider))
                .unwrap_or(false);
            if !still_pending {
                break;
            }

            let request = match server.recv_timeout(Duration::from_millis(500)) {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(_) => break,
            };
            let request_url = request.url().to_string();
            if request_url.starts_with("/cancel") {
                let _ = request.respond(Response::from_string("cancelled"));
                let _ = OAUTH_PENDING.lock().map(|mut pending| {
                    pending.remove(&login_id);
                });
                break;
            }

            let Ok((path, params)) = query_map_from_url(&request_url, port) else {
                let _ = request.respond(Response::from_string("Bad Request").with_status_code(400));
                continue;
            };
            if path != callback_path {
                let _ = request.respond(Response::from_string("Not Found").with_status_code(404));
                continue;
            }
            if let Some(error) = params.get("error") {
                let _ = request.respond(
                    Response::from_string(format!("OAuth error: {error}")).with_status_code(400),
                );
                break;
            }
            let code = params
                .get("code")
                .and_then(|value| normalize_non_empty(Some(value.as_str())));
            let state = params
                .get("state")
                .and_then(|value| normalize_non_empty(Some(value.as_str())));
            let Some(code) = code else {
                let _ =
                    request.respond(Response::from_string("Missing code").with_status_code(400));
                continue;
            };

            let accepted = OAUTH_PENDING
                .lock()
                .ok()
                .and_then(|mut pending| {
                    let item = pending.get_mut(&login_id)?;
                    if item.provider != provider || item.state != state.clone().unwrap_or_default()
                    {
                        return Some(false);
                    }
                    item.code = Some(code);
                    Some(true)
                })
                .unwrap_or(false);
            if !accepted {
                let _ =
                    request.respond(Response::from_string("State mismatch").with_status_code(400));
                continue;
            }
            if let Some(location) = success_redirect.as_deref() {
                respond_oauth_redirect(request, location);
            } else {
                respond_oauth_success(request);
            }
            break;
        }
    });
}

fn reserve_callback_port(preferred: Option<u16>) -> Result<u16, String> {
    let port = preferred.unwrap_or(0);
    let mut last_error = None;
    for attempt in 0..6 {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                let port = listener
                    .local_addr()
                    .map_err(|error| format!("读取 OAuth 回调端口失败: {error}"))?
                    .port();
                drop(listener);
                return Ok(port);
            }
            Err(error) if error.kind() == ErrorKind::AddrInUse && preferred.is_some() => {
                last_error = Some(error);
                if attempt < 5 {
                    std::thread::sleep(Duration::from_millis(120));
                    continue;
                }
            }
            Err(error) => return Err(format!("分配 OAuth 回调端口失败: {error}")),
        }
    }
    let detail = last_error
        .map(|error| format!(" ({error})"))
        .unwrap_or_default();
    Err(format!("OAuth 回调端口 {port} 已被占用。Codex OAuth 必须使用固定端口，请先退出正在占用该端口的应用后重试。{detail}"))
}

fn build_codex_oauth_url(
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<String, String> {
    let mut url = Url::parse(CODEX_OAUTH_AUTH_URL)
        .map_err(|error| format!("构建 Codex OAuth URL 失败: {error}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CODEX_OAUTH_CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", CODEX_OAUTH_SCOPES)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", "codex_vscode");
    Ok(url.to_string())
}

fn build_gemini_oauth_url(redirect_uri: &str, state: &str) -> Result<String, String> {
    let mut url = Url::parse(GEMINI_OAUTH_AUTH_URL)
        .map_err(|error| format!("构建 Gemini OAuth URL 失败: {error}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", GEMINI_OAUTH_CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("access_type", "offline")
        .append_pair(
            "scope",
            "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile",
        )
        .append_pair("state", state);
    Ok(url.to_string())
}

async fn exchange_codex_oauth_code(
    code: &str,
    code_verifier: &str,
    port: u16,
) -> Result<Value, String> {
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Codex OAuth 客户端失败: {error}"))?;
    let response = client
        .post(CODEX_OAUTH_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", CODEX_OAUTH_CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|error| format!("Codex OAuth token 请求失败: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 Codex OAuth token 响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Codex OAuth token 交换失败: status={status}, body_len={}",
            body.len()
        ));
    }
    let token_response: Value = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Codex OAuth token 响应失败: {error}"))?;
    let id_token = string_field(token_response.get("id_token"))
        .ok_or_else(|| "Codex OAuth 响应缺少 id_token".to_string())?;
    let access_token = string_field(token_response.get("access_token"))
        .ok_or_else(|| "Codex OAuth 响应缺少 access_token".to_string())?;
    let mut tokens = serde_json::Map::new();
    tokens.insert("id_token".to_string(), Value::String(id_token));
    tokens.insert("access_token".to_string(), Value::String(access_token));
    if let Some(refresh_token) = string_field(token_response.get("refresh_token")) {
        tokens.insert("refresh_token".to_string(), Value::String(refresh_token));
    }
    if let Some(jwt) = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(parse_jwt_payload)
    {
        if let Some(account_id) = jwt
            .get("https://api.openai.com/auth")
            .and_then(Value::as_object)
            .and_then(|auth| string_field(auth.get("chatgpt_account_id")))
        {
            tokens.insert("account_id".to_string(), Value::String(account_id));
        }
    }

    Ok(serde_json::json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": Value::Null,
        "tokens": Value::Object(tokens),
        "last_refresh": codex_last_refresh_now()
    }))
}

async fn exchange_gemini_oauth_code(code: &str, redirect_uri: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Gemini OAuth 客户端失败: {error}"))?;
    let response = client
        .post(GEMINI_OAUTH_TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[
            ("code", code),
            ("client_id", GEMINI_OAUTH_CLIENT_ID),
            ("client_secret", GEMINI_OAUTH_CLIENT_SECRET),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|error| format!("请求 Google OAuth token 失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Google OAuth token 交换失败: status={status}, body_len={}",
            body.len()
        ));
    }
    let payload = response
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|error| format!("解析 Google OAuth token 响应失败: {error}"))?;
    let access_token = payload.access_token.clone().ok_or_else(|| {
        format!(
            "Google OAuth 响应缺少 access_token: error={:?}, desc={:?}",
            payload.error, payload.error_description
        )
    })?;
    let user_info = fetch_google_userinfo(&access_token).await;
    let email = normalize_non_empty(user_info.as_ref().and_then(|info| info.email.as_deref()))
        .or_else(|| {
            payload
                .id_token
                .as_deref()
                .and_then(parse_jwt_payload)
                .and_then(|jwt| string_field(jwt.get("email")))
        })
        .unwrap_or_else(|| "unknown@gmail.com".to_string());
    let auth_id = normalize_non_empty(user_info.as_ref().and_then(|info| info.id.as_deref()))
        .or_else(|| {
            payload
                .id_token
                .as_deref()
                .and_then(parse_jwt_payload)
                .and_then(|jwt| string_field(jwt.get("sub")))
        });
    let name = normalize_non_empty(user_info.as_ref().and_then(|info| info.name.as_deref()))
        .or_else(|| {
            payload
                .id_token
                .as_deref()
                .and_then(parse_jwt_payload)
                .and_then(|jwt| string_field(jwt.get("name")))
        });
    let expiry_date = payload
        .expires_in
        .map(|seconds| now_ts_ms() + seconds.saturating_mul(1000));

    let mut result = serde_json::Map::new();
    result.insert("access_token".to_string(), Value::String(access_token));
    if let Some(refresh_token) = payload.refresh_token {
        result.insert("refresh_token".to_string(), Value::String(refresh_token));
    }
    if let Some(id_token) = payload.id_token {
        result.insert("id_token".to_string(), Value::String(id_token));
    }
    if let Some(token_type) = payload.token_type {
        result.insert("token_type".to_string(), Value::String(token_type));
    }
    if let Some(scope) = payload.scope {
        result.insert("scope".to_string(), Value::String(scope));
    }
    if let Some(expiry_date) = expiry_date {
        result.insert("expiry_date".to_string(), Value::Number(expiry_date.into()));
    }
    result.insert("email".to_string(), Value::String(email));
    if let Some(auth_id) = auth_id {
        result.insert("auth_id".to_string(), Value::String(auth_id));
    }
    if let Some(name) = name {
        result.insert("name".to_string(), Value::String(name));
    }
    result.insert(
        "selected_auth_type".to_string(),
        Value::String("oauth-personal".to_string()),
    );
    Ok(Value::Object(result))
}

async fn fetch_google_userinfo(access_token: &str) -> Option<GoogleUserInfoResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let response = client
        .get(GOOGLE_USERINFO_URL)
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<GoogleUserInfoResponse>().await.ok()
}

async fn refresh_gemini_access_token(refresh_token: &str) -> Result<OAuthTokenResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("创建 Gemini token 客户端失败: {error}"))?;
    let response = client
        .post(GEMINI_OAUTH_TOKEN_URL)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", GEMINI_OAUTH_CLIENT_ID),
            ("client_secret", GEMINI_OAUTH_CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| format!("刷新 Gemini access_token 请求失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "刷新 Gemini access_token 失败: status={status}, body_len={}",
            body.len()
        ));
    }
    response
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|error| format!("解析 Gemini access_token 刷新响应失败: {error}"))
}

async fn post_gemini_code_assist_json(
    access_token: &str,
    endpoint: &str,
    payload: &Value,
    action: &str,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("创建 Gemini API 客户端失败: {error}"))?;
    let response = client
        .post(endpoint)
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .header(CONTENT_TYPE, "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|error| format!("请求 Gemini {action} 失败: {error}"))?;
    if response.status().as_u16() == 401 {
        return Err("UNAUTHORIZED: Gemini access_token 已失效".to_string());
    }
    if response.status().is_success() {
        return response
            .json::<Value>()
            .await
            .map_err(|error| format!("解析 Gemini {action} 响应失败: {error}"));
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!(
        "请求 Gemini {action} 失败: status={status}, body_len={}",
        body.len()
    ))
}

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn list_accounts(app: tauri::AppHandle) -> Result<Vec<ManagedAccount>, String> {
    let _ = cleanup_expired_windsurf_and_handoff(&app);
    let conn = open_app_db(&app)?;
    encrypt_plain_windsurf_accounts(&conn)?;
    enforce_single_current_account(&conn)?;
    read_accounts_from_conn(&conn).map(accounts_for_frontend)
}

#[tauri::command]
fn clear_local_database(app: tauri::AppHandle) -> Result<(), String> {
    let conn = open_app_db(&app)?;
    conn.execute("DELETE FROM accounts", [])
        .map_err(|error| format!("清空账号数据库失败: {error}"))?;
    conn.execute("DELETE FROM public_usage_history", [])
        .map_err(|error| format!("清空用量历史失败: {error}"))?;
    schedule_windsurf_sync(app);
    Ok(())
}

#[tauri::command]
fn upsert_accounts(app: tauri::AppHandle, accounts: Vec<ManagedAccount>) -> Result<(), String> {
    // 前端拿到的账号 provider 是 "superai"（由 account_for_frontend 改写），
    // 这里翻译回内部协议字面量，否则 upsert_account 里 `provider == "windsurf"`
    // 的分支全部走不到，邮箱 / display_name / 加密包裹全部错位。
    let normalized: Vec<ManagedAccount> = accounts
        .into_iter()
        .map(|mut account| {
            account.provider = normalize_provider_from_frontend(&account.provider);
            account
        })
        .collect();
    upsert_accounts_into_db(&app, &normalized)
}

#[tauri::command]
#[allow(non_snake_case)]
async fn refresh_account(
    app: tauri::AppHandle,
    accountId: String,
) -> Result<ManagedAccount, String> {
    // 后台 cleanup 可能已经把这个账号删了；这种情况下不要抛 "Query returned no rows"
    // 的红色 toast，而是 emit 同款事件让前端 re-fetch 列表，悄悄把残留卡片刷掉。
    let mut account = {
        let conn = open_app_db(&app)?;
        match load_account_from_db(&conn, &accountId) {
            Ok(account) => account,
            Err(_) => {
                let _ = app.emit(
                    "accounts-expired-removed",
                    serde_json::json!({
                        "ids": [accountId.clone()],
                        "count": 1,
                    }),
                );
                return Err("账号已被自动清理".to_string());
            }
        }
    };
    let was_current = is_current_status(&account.status);

    match account.provider.as_str() {
        "codex" => {
            if let Err(error) = refresh_codex_account_remote(&mut account).await {
                mark_account_unavailable(&mut account, error);
            }
        }
        "gemini" => {
            if let Err(error) = refresh_gemini_account_remote(&mut account).await {
                mark_account_unavailable(&mut account, error);
            }
        }
        "windsurf" => {
            if let Err(error) = refresh_windsurf_account_remote(&mut account).await {
                mark_account_unavailable(&mut account, error);
            }
        }
        other => return Err(format!("不支持的账号类型: {other}")),
    }
    let just_exhausted = apply_public_usage_after_refresh(&mut account);
    if was_current && !public_usage_is_exhausted(&account) {
        mark_account_current(&mut account);
    }

    let written = upsert_existing_accounts_into_db(&app, &[account.clone()])?;
    if written.is_empty() {
        return Err("账号已被删除，刷新结果已丢弃".to_string());
    }
    if just_exhausted {
        emit_account_exhausted(&app, &account);
        schedule_windsurf_sync(app.clone());
    }
    Ok(account_for_frontend(&account))
}

#[tauri::command]
async fn refresh_provider_accounts(
    app: tauri::AppHandle,
    provider: String,
) -> Result<Vec<ManagedAccount>, String> {
    // 前端传进来的 "superai" 翻回内部协议名再 filter，否则 SuperAI 账号一个都
    // 匹配不上。
    let provider = normalize_provider_from_frontend(&provider);
    let mut accounts = {
        let conn = open_app_db(&app)?;
        read_accounts_from_conn(&conn)?
            .into_iter()
            .filter(|account| account.provider == provider)
            .collect::<Vec<_>>()
    };

    for account in &mut accounts {
        let was_current = is_current_status(&account.status);
        match account.provider.as_str() {
            "codex" => {
                if let Err(error) = refresh_codex_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            "gemini" => {
                if let Err(error) = refresh_gemini_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            "windsurf" => {
                if let Err(error) = refresh_windsurf_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            _ => {}
        }
        let just_exhausted = apply_public_usage_after_refresh(account);
        if was_current && !public_usage_is_exhausted(account) {
            mark_account_current(account);
        }
        if just_exhausted {
            emit_account_exhausted(&app, account);
        }
    }

    let any_exhausted = accounts.iter().any(public_usage_is_exhausted);
    let result = upsert_existing_accounts_into_db(&app, &accounts).map(accounts_for_frontend)?;
    if any_exhausted {
        schedule_windsurf_sync(app.clone());
    }
    Ok(result)
}

#[tauri::command]
async fn refresh_all_accounts(app: tauri::AppHandle) -> Result<Vec<ManagedAccount>, String> {
    let _ = cleanup_expired_windsurf_and_handoff(&app);
    let mut accounts = {
        let conn = open_app_db(&app)?;
        encrypt_plain_windsurf_accounts(&conn)?;
        read_accounts_from_conn(&conn)?
    };

    for account in &mut accounts {
        let was_current = is_current_status(&account.status);
        match account.provider.as_str() {
            "codex" => {
                if let Err(error) = refresh_codex_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            "gemini" => {
                if let Err(error) = refresh_gemini_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            "windsurf" => {
                if let Err(error) = refresh_windsurf_account_remote(account).await {
                    mark_account_unavailable(account, error);
                }
            }
            _ => {}
        }
        let just_exhausted = apply_public_usage_after_refresh(account);
        if was_current && !public_usage_is_exhausted(account) {
            mark_account_current(account);
        }
        if just_exhausted {
            emit_account_exhausted(&app, account);
        }
    }

    let any_exhausted = accounts.iter().any(public_usage_is_exhausted);
    let result = upsert_existing_accounts_into_db(&app, &accounts).map(accounts_for_frontend)?;
    if any_exhausted {
        schedule_windsurf_sync(app.clone());
    }
    Ok(result)
}

#[tauri::command]
#[allow(non_snake_case)]
fn delete_account(app: tauri::AppHandle, accountId: String) -> Result<Vec<ManagedAccount>, String> {
    let conn = open_app_db(&app)?;
    // 删之前先把账号读出来，给公开版 + batch_key 的 SuperAI 账号 stash 一份
    // 使用记录到 public_usage_history（24h 内重新导入会自动恢复）。
    if let Ok(account) = load_account_from_db(&conn, &accountId) {
        drop(conn);
        stash_public_usage_history(&app, &account);
    }
    let conn = open_app_db(&app)?;
    let deleted = conn
        .execute("DELETE FROM accounts WHERE id = ?1", params![accountId])
        .map_err(|error| format!("删除账号失败: {error}"))?;
    if deleted == 0 {
        return Err("账号不存在，可能已经被删除".to_string());
    }
    let accounts = read_accounts_from_conn(&conn)?;
    drop(conn);
    schedule_windsurf_sync(app);
    Ok(accounts_for_frontend(accounts))
}

#[tauri::command]
#[allow(non_snake_case)]
fn switch_account(app: tauri::AppHandle, accountId: String) -> Result<Vec<ManagedAccount>, String> {
    let conn = open_app_db(&app)?;
    let account = load_account_from_db(&conn, &accountId)?;
    // 兜底：SuperAI 账号有效期已过 / 公开版本地累计已耗尽 → 不允许启用，
    // 避免上游 422 / 用户误把已停用账号挂起。前端理应同步过滤，但万一不一致
    // 走到这里也得明确拦掉。
    if account.provider == "windsurf" {
        if let Some(expires_at) = windsurf_license_expires_at(&account) {
            if windsurf_license_expired_at(expires_at) {
                return Err("该账号有效期已过，无法启用，请删除后重新导入".to_string());
            }
        }
        if public_usage_is_exhausted(&account) {
            return Err("该账号本地累计额度已耗尽，无法启用".to_string());
        }
    }
    match account.provider.as_str() {
        "codex" => write_codex_auth(&account)?,
        "gemini" => write_gemini_auth(&account)?,
        "windsurf" => {
            activate_windsurf_account_for_api(&app, &account)?;
        }
        other => return Err(format!("不支持的账号类型: {other}")),
    }
    set_account_current_state(&conn, &account.provider, &account.id).map(accounts_for_frontend)
}

fn activate_windsurf_account_for_api(
    app: &tauri::AppHandle,
    account: &ManagedAccount,
) -> Result<(), String> {
    if !api_service::is_running_with_sidecar() {
        return Ok(());
    }

    match api_service::activate_account_by_email(&account.email) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            sync_superai_accounts_to_api(app.clone())?;
            api_service::activate_account_by_email(&account.email).map_err(|second_error| {
                format!("启用 API 账号失败: {second_error}; 同步前错误: {first_error}")
            })
        }
    }
}

#[tauri::command]
fn sync_api_service_active_account(app: tauri::AppHandle) -> Result<Vec<ManagedAccount>, String> {
    let Some(email) = api_service::last_used_account_email() else {
        return Ok(Vec::new());
    };
    // 幂等短路：sidecar 上次挑的还是这个号 → 我们已经把 "当前" 标签打过，
    // 不必再开 sqlite + AES 解密 + 全表 upsert。前端 setInterval 3s 也几乎零开销。
    if api_service::last_synced_active_email()
        .as_deref()
        .map(|prev| prev.eq_ignore_ascii_case(&email))
        .unwrap_or(false)
    {
        return Ok(Vec::new());
    }
    let conn = open_app_db(&app)?;
    let Some(account) = read_accounts_from_conn(&conn)?.into_iter().find(|account| {
        account.provider == "windsurf" && account.email.eq_ignore_ascii_case(&email)
    }) else {
        return Ok(Vec::new());
    };
    let result =
        set_account_current_state(&conn, "windsurf", &account.id).map(accounts_for_frontend)?;
    api_service::record_synced_active_email(email.to_ascii_lowercase());
    Ok(result)
}

#[tauri::command]
#[allow(non_snake_case)]
fn export_account(app: tauri::AppHandle, accountId: String) -> Result<String, String> {
    let conn = open_app_db(&app)?;
    let account = load_account_from_db(&conn, &accountId)?;
    let value = match account.provider.as_str() {
        "codex" => build_codex_auth_payload(&account)?,
        "gemini" => build_gemini_oauth_payload(&account)?,
        "windsurf" if is_public_build() => {
            return Err("公开版不允许导出 SuperAI 原始凭证".to_string());
        }
        "windsurf" => build_windsurf_payload(&account)?,
        _ => serde_json::to_value(&account).map_err(|error| format!("序列化账号失败: {error}"))?,
    };
    serde_json::to_string_pretty(&value).map_err(|error| format!("序列化导出内容失败: {error}"))
}

#[tauri::command]
#[allow(non_snake_case)]
fn export_public_superai_account(
    app: tauri::AppHandle,
    accountId: String,
) -> Result<String, String> {
    let conn = open_app_db(&app)?;
    let account = load_account_from_db(&conn, &accountId)?;
    public_windsurf_export_key(&account)
}

fn system_auto_launch_enabled(app: &tauri::AppHandle) -> Result<Option<bool>, String> {
    #[cfg(desktop)]
    {
        app.autolaunch()
            .is_enabled()
            .map(Some)
            .map_err(|error| format!("读取系统开机自启状态失败: {error}"))
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(None)
    }
}

fn apply_system_auto_launch(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(desktop)]
    {
        let manager = app.autolaunch();

        // 先检查当前状态：如果已经是目标状态，直接跳过 enable/disable，避免触发
        // Windows 平台 disable() 在注册表项不存在时抛 ERROR_FILE_NOT_FOUND (os
        // error 2) 这种"看似失败实际目标已达成"的情况。
        if let Ok(current) = manager.is_enabled() {
            if current == enabled {
                return Ok(());
            }
        }

        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };

        // enable/disable 返回错误时，仍然查一下实际状态：可能底层只是"项不存在"
        // 之类的良性错误，状态已经正确就当成功。
        match result {
            Ok(_) => {}
            Err(error) => {
                let actual = manager.is_enabled().unwrap_or(!enabled);
                if actual != enabled {
                    return Err(if enabled {
                        format!("启用系统开机自启失败: {error}")
                    } else {
                        format!("关闭系统开机自启失败: {error}")
                    });
                }
            }
        }

        let actual = manager
            .is_enabled()
            .map_err(|error| format!("校验系统开机自启状态失败: {error}"))?;
        if actual != enabled {
            return Err("系统开机自启状态未按预期写入".to_string());
        }
        Ok(())
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        let _ = enabled;
        Ok(())
    }
}

#[tauri::command]
fn load_settings(app: tauri::AppHandle) -> Result<Option<AppSettings>, String> {
    let conn = open_app_db(&app)?;
    let result = conn.query_row(
        "SELECT value_json FROM settings WHERE key = 'app'",
        [],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(value_json) => {
            let mut settings = serde_json::from_str::<AppSettings>(&value_json)
                .map_err(|error| format!("解析设置失败: {error}"))?;
            if let Some(enabled) = system_auto_launch_enabled(&app)? {
                settings.auto_launch = enabled;
            }
            Ok(Some(settings))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            if let Some(enabled) = system_auto_launch_enabled(&app)? {
                let mut settings = default_app_settings();
                settings.auto_launch = enabled;
                return Ok(Some(settings));
            }
            Ok(None)
        }
        Err(error) => Err(format!("读取设置失败: {error}")),
    }
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    apply_system_auto_launch(&app, settings.auto_launch)?;
    // 前端不维护旧 API 服务字段，从已有记录里继承，避免被默认值覆盖。
    let mut merged = settings;
    if let Ok(existing) = read_settings_record(&app) {
        // 前端不维护这两项，从已有记录里继承避免被默认值覆盖。
        merged.api_service_enabled = existing.api_service_enabled;
        merged.api_service_key = existing.api_service_key;
        // host/port/default_model 现在前端会管，但缺省值（空串/0）继续走旧值，方便前端先不发也能保留。
        if merged.api_service_host.is_empty() {
            merged.api_service_host = existing.api_service_host;
        }
        if merged.api_service_default_model.is_empty()
            && !existing.api_service_default_model.is_empty()
        {
            merged.api_service_default_model = existing.api_service_default_model;
        }
    }
    merged.api_service_default_model =
        effective_api_service_model(&merged.api_service_default_model);
    write_settings_record(&app, &merged)
}

#[tauri::command]
fn import_accounts_from_json(
    app: tauri::AppHandle,
    json_content: String,
    label: Option<String>,
) -> Result<ImportResult, String> {
    let result =
        parse_auth_json_content(&json_content, "paste", label.as_deref().unwrap_or("JSON"));
    persist_and_refresh_imported(app, result)
}

#[tauri::command]
fn import_codex_from_local(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let auth_path = codex_home_dir()?.join("auth.json");
    if !auth_path.exists() {
        return Err("未找到 ~/.codex/auth.json 文件".to_string());
    }

    let content = read_to_string(&auth_path)?;
    let mut result = parse_auth_json_content(&content, "local", "Codex 本机账号");
    if result.imported.is_empty() {
        result.failed = vec![ImportFailure {
            label: "Codex 本机账号".to_string(),
            reason: "auth.json 缺少可导入的 Codex 登录信息".to_string(),
        }];
        return Ok(result);
    }
    persist_and_refresh_imported(app, result)
}

#[tauri::command]
fn import_gemini_from_local(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let gemini_dir = home_dir()?.join(".gemini");
    let oauth_path = gemini_dir.join("oauth_creds.json");
    let google_accounts_path = gemini_dir.join("google_accounts.json");
    let settings_path = gemini_dir.join("settings.json");

    let mut oauth_value = if let Some(value) = read_gemini_keychain()? {
        value
    } else {
        if !oauth_path.exists() {
            return Err(format!(
                "未找到本机 Gemini 账号文件: {}",
                oauth_path.display()
            ));
        }
        let oauth_content = read_to_string(&oauth_path)?;
        serde_json::from_str(&oauth_content)
            .map_err(|e| format!("解析 oauth_creds.json 失败: {e}"))?
    };

    if let Some(obj) = oauth_value.as_object_mut() {
        if let Ok(raw) = fs::read_to_string(&google_accounts_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                if let Some(active) = v.get("active").and_then(Value::as_str) {
                    obj.insert("email".to_string(), Value::String(active.to_string()));
                }
            }
        }
        if let Ok(raw) = fs::read_to_string(&settings_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                if let Some(selected) = v
                    .get("selectedAuthType")
                    .and_then(Value::as_str)
                    .or_else(|| v.get("selected_auth_type").and_then(Value::as_str))
                {
                    obj.insert("plan_name".to_string(), Value::String(selected.to_string()));
                }
            }
        }
    }

    let result = parse_auth_json_content(&oauth_value.to_string(), "local", "Gemini 本机账号");
    persist_and_refresh_imported(app, result)
}

#[tauri::command]
fn start_codex_oauth() -> Result<OAuthStartResult, String> {
    cancel_pending_oauth_for_provider("codex");
    let port = reserve_callback_port(Some(CODEX_OAUTH_CALLBACK_PORT))?;
    let login_id = random_urlsafe_token(24);
    let state = random_urlsafe_token(24);
    let code_verifier = random_urlsafe_token(32);
    let challenge = code_challenge(&code_verifier);
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let auth_url = build_codex_oauth_url(&redirect_uri, &challenge, &state)?;
    OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .insert(
            login_id.clone(),
            OAuthPending {
                provider: "codex".to_string(),
                redirect_uri,
                state,
                code_verifier: Some(code_verifier),
                port,
                expires_at: now_ts() + OAUTH_TIMEOUT_SECONDS,
                code: None,
            },
        );
    start_oauth_callback_listener(
        login_id.clone(),
        "codex".to_string(),
        port,
        "/auth/callback".to_string(),
        None,
    );
    open_oauth_url(&auth_url)?;

    Ok(OAuthStartResult {
        login_id,
        provider: "codex".to_string(),
        command: auth_url.clone(),
        message: "已启动 Codex OAuth，完成浏览器授权后会自动添加。".to_string(),
        auth_url: Some(auth_url),
    })
}

#[tauri::command]
async fn complete_codex_oauth(
    app: tauri::AppHandle,
    login_id: String,
) -> Result<ImportResult, String> {
    let Some(pending) = oauth_pending_get(&login_id)? else {
        return Ok(ImportResult {
            imported: vec![],
            failed: vec![],
        });
    };
    if pending.provider != "codex" {
        return Err("无效的 Codex OAuth 会话".to_string());
    }
    if pending.expires_at <= now_ts() {
        oauth_pending_remove(&login_id);
        return Err("Codex OAuth 登录已超时，请重新发起授权".to_string());
    }
    let Some(code) = pending.code else {
        return Ok(ImportResult {
            imported: vec![],
            failed: vec![],
        });
    };
    let code_verifier = pending
        .code_verifier
        .ok_or_else(|| "Codex OAuth 会话缺少 code_verifier".to_string())?;
    let payload = exchange_codex_oauth_code(&code, &code_verifier, pending.port).await?;
    let result = parse_auth_json_content(&payload.to_string(), "oauth", "Codex OAuth");
    let result = persist_and_refresh_imported(app, result)?;
    oauth_pending_remove(&login_id);
    Ok(result)
}

#[tauri::command]
fn start_gemini_oauth() -> Result<OAuthStartResult, String> {
    cancel_pending_oauth_for_provider("gemini");
    let port = reserve_callback_port(None)?;
    let login_id = random_urlsafe_token(24);
    let state = random_urlsafe_token(24);
    let redirect_uri = format!("http://127.0.0.1:{port}{GEMINI_OAUTH_CALLBACK_PATH}");
    let auth_url = build_gemini_oauth_url(&redirect_uri, &state)?;
    OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .insert(
            login_id.clone(),
            OAuthPending {
                provider: "gemini".to_string(),
                redirect_uri,
                state,
                code_verifier: None,
                port,
                expires_at: now_ts() + OAUTH_TIMEOUT_SECONDS,
                code: None,
            },
        );
    start_oauth_callback_listener(
        login_id.clone(),
        "gemini".to_string(),
        port,
        GEMINI_OAUTH_CALLBACK_PATH.to_string(),
        Some("https://developers.google.com/gemini-code-assist/auth_success_gemini".to_string()),
    );
    open_oauth_url(&auth_url)?;

    Ok(OAuthStartResult {
        login_id,
        provider: "gemini".to_string(),
        command: auth_url.clone(),
        message: "已启动 Gemini OAuth，完成浏览器授权后会自动添加。".to_string(),
        auth_url: Some(auth_url),
    })
}

#[tauri::command]
async fn complete_gemini_oauth(
    app: tauri::AppHandle,
    login_id: String,
) -> Result<ImportResult, String> {
    let Some(pending) = oauth_pending_get(&login_id)? else {
        return Ok(ImportResult {
            imported: vec![],
            failed: vec![],
        });
    };
    if pending.provider != "gemini" {
        return Err("无效的 Gemini OAuth 会话".to_string());
    }
    if pending.expires_at <= now_ts() {
        oauth_pending_remove(&login_id);
        return Err("Gemini OAuth 登录已超时，请重新发起授权".to_string());
    }
    let Some(code) = pending.code else {
        return Ok(ImportResult {
            imported: vec![],
            failed: vec![],
        });
    };
    let payload = exchange_gemini_oauth_code(&code, &pending.redirect_uri).await?;
    let result = parse_auth_json_content(&payload.to_string(), "oauth", "Gemini OAuth");
    let result = persist_and_refresh_imported(app, result)?;
    oauth_pending_remove(&login_id);
    Ok(result)
}

fn read_settings_record(app: &tauri::AppHandle) -> Result<AppSettings, String> {
    let conn = open_app_db(app)?;
    let result = conn.query_row(
        "SELECT value_json FROM settings WHERE key = 'app'",
        [],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(value_json) => serde_json::from_str::<AppSettings>(&value_json)
            .map_err(|error| format!("解析设置失败: {error}")),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(default_app_settings()),
        Err(error) => Err(format!("读取设置失败: {error}")),
    }
}

fn write_settings_record(app: &tauri::AppHandle, settings: &AppSettings) -> Result<(), String> {
    let conn = open_app_db(app)?;
    let value_json =
        serde_json::to_string(settings).map_err(|error| format!("序列化设置失败: {error}"))?;
    conn.execute(
        r#"
      INSERT INTO settings (key, value_json, updated_at)
      VALUES ('app', ?1, ?2)
      ON CONFLICT(key) DO UPDATE SET
        value_json = excluded.value_json,
        updated_at = excluded.updated_at
      "#,
        params![value_json, now_ts()],
    )
    .map_err(|error| format!("保存设置失败: {error}"))?;
    Ok(())
}

fn ensure_api_service_key(
    app: &tauri::AppHandle,
    settings: &mut AppSettings,
) -> Result<(), String> {
    let key = settings.api_service_key.trim();
    if key.is_empty() || api_service::is_legacy_api_key(key) {
        settings.api_service_key = api_service::generate_api_key();
        write_settings_record(app, settings)?;
    }
    Ok(())
}

#[tauri::command]
fn get_api_service_status(
    app: tauri::AppHandle,
) -> Result<api_service::ApiServiceStatus, String> {
    let mut settings = read_settings_record(&app)?;
    ensure_api_service_key(&app, &mut settings)?;
    Ok(api_service::current_status(
        &settings.api_service_host,
        settings.api_service_port,
        &settings.api_service_key,
        &effective_api_service_model(&settings.api_service_default_model),
    ))
}

#[tauri::command]
async fn start_api_service(
    app: tauri::AppHandle,
) -> Result<api_service::ApiServiceStatus, String> {
    tauri::async_runtime::spawn_blocking(move || start_api_service_impl(app))
        .await
        .map_err(|error| format!("启动 API 服务任务失败: {error}"))?
}

fn start_api_service_impl(
    app: tauri::AppHandle,
) -> Result<api_service::ApiServiceStatus, String> {
    let mut settings = read_settings_record(&app)?;
    ensure_api_service_key(&app, &mut settings)?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("读取应用数据目录失败: {error}"))?;
    let status = api_service::start(
        &data_dir,
        &settings.api_service_host,
        settings.api_service_port,
        &settings.api_service_key,
        &effective_api_service_model(&settings.api_service_default_model),
    )?;
    // 把启用状态 + 实际端口持久化（持久化端口避免每次重启都换）。
    let mut needs_write = false;
    if !settings.api_service_enabled {
        settings.api_service_enabled = true;
        needs_write = true;
    }
    if let Some(actual) = status.actual_port {
        if settings.api_service_port != actual {
            settings.api_service_port = actual;
            needs_write = true;
        }
    }
    let effective_model = effective_api_service_model(&settings.api_service_default_model);
    if settings.api_service_default_model != effective_model {
        settings.api_service_default_model = effective_model;
        needs_write = true;
    }
    if needs_write {
        write_settings_record(&app, &settings)?;
    }
    // 启动命令要尽快返回给前端；账号同步里会调用 sidecar 的 dashboard
    // 能力刷新接口，LS 未 ready 时可能比较慢，放后台避免 UI 卡住。
    schedule_windsurf_sync(app.clone());
    Ok(status)
}

fn emit_api_service_status_changed(
    app: &tauri::AppHandle,
    status: &api_service::ApiServiceStatus,
) {
    let _ = app.emit("api-service-status-changed", status);
}

#[tauri::command]
fn list_api_service_models() -> Result<Vec<serde_json::Value>, String> {
    api_service::list_models()
}

#[tauri::command]
fn set_api_service_default_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
    let mut settings = read_settings_record(&app)?;
    settings.api_service_default_model = effective_api_service_model(&model);
    write_settings_record(&app, &settings)?;
    // 在跑就立刻热更，不在跑只持久化等下次启动。
    let _ = api_service::update_default_model(&settings.api_service_default_model);
    // 顺手把 ~/.codex/config.toml 的 model 行原地改写，
    // 这样 codex 重启后 TUI 顶部 `model:` 跟 SuperAI UI 一致。
    // 用户没点过"配置Codex"时该函数返回 false，不会擅自创建文件。
    if let Err(error) = rewrite_managed_model_line(&settings.api_service_default_model) {
        eprintln!("[SuperAI] 改写 codex config.toml model 行失败: {error}");
    }
    Ok(())
}

/// 把单个 SuperAI 账号转换成 sidecar `/auth/login` 期望的入参。
///
/// 上游 sidecar 的 `/auth/login` 只接受三种凭证：
///   - `{ api_key, label }`               — 已有 Codeium api_key 或 Devin sessionToken
///   - `{ token, label }`                 — windsurf.com show-auth-token 拿到的 ott$/JWT token
///   - `{ email, password, label }`       — 完整账号密码（走 Auth1 → PostAuth → sessionToken，目前唯一能拿到付费配额的路径）
///
/// 之前 v2.0.7 支持的 `refresh_token` 路径在上游 2.0.90 已移除（Firebase 路径已死）。
/// 持久化里只剩 refresh_token 的老账号不能同步进 sidecar，需要用户重新用 token 或邮箱密码导入。
fn windsurf_account_to_sidecar_payload(account: &ManagedAccount) -> Option<serde_json::Value> {
    if account.provider != "windsurf" {
        return None;
    }
    // 公开版账号本地累计已用满 → 不参与 sidecar 同步。reconcile_accounts
    // 拿到的 desired_emails 不再包含它，会主动 DELETE 到 sidecar /auth/accounts/:id。
    if public_usage_is_exhausted(account) {
        return None;
    }
    let label = if !account.email.is_empty() {
        account.email.clone()
    } else {
        account.id.clone()
    };
    if let Some(token) = windsurf_payload_string(account, "api_key") {
        return Some(serde_json::json!({ "api_key": token, "label": label }));
    }
    if let Some(token) = windsurf_payload_string(account, "id_token") {
        return Some(serde_json::json!({ "token": token, "label": label }));
    }
    if let Some(token) = windsurf_payload_string(account, "access_token") {
        return Some(serde_json::json!({ "token": token, "label": label }));
    }
    None
}

#[tauri::command]
fn sync_superai_accounts_to_api(app: tauri::AppHandle) -> Result<usize, String> {
    let accounts = {
        let conn = open_app_db(&app)?;
        read_accounts_from_conn(&conn)?
    };
    let payloads: Vec<serde_json::Value> = accounts
        .iter()
        .filter(|a| a.provider == "windsurf")
        .filter_map(windsurf_account_to_sidecar_payload)
        .collect();
    let count = payloads.len();
    api_service::reconcile_accounts(payloads)?;
    Ok(count)
}

/// 后台线程触发同步，避免阻塞 Tauri 命令。
fn schedule_windsurf_sync(app: tauri::AppHandle) {
    if !api_service::is_running_with_sidecar() {
        return;
    }
    std::thread::spawn(move || {
        if let Err(error) = sync_superai_accounts_to_api(app) {
            eprintln!("[SuperAI API] 后台同步失败: {error}");
        }
    });
}

#[tauri::command]
async fn stop_api_service(
    app: tauri::AppHandle,
) -> Result<api_service::ApiServiceStatus, String> {
    tauri::async_runtime::spawn_blocking(move || stop_api_service_impl(app))
        .await
        .map_err(|error| format!("停止 API 服务任务失败: {error}"))?
}

// ---------- 一键配置 Codex App ----------
//
// 直接改写 `~/.codex/config.toml`：生成一份干净的 SuperAI 配置，
// 内含：
//   - 顶层 `model_provider = "superai"` / `model = "<选中的模型>"`
//   - `[model_providers.superai]` 段，把 base_url、wire_api 写好，并通过
//     `requires_openai_auth = true` 让 codex 从 `~/.codex/auth.json` 的
//     `OPENAI_API_KEY` 字段读取 SuperAI key 当 bearer token。
//
// `config.toml` 完全由 SuperAI 接管：写入时**不**备份（我们覆盖整文件，
// 多次点"配置 Codex"也不会丢失任何"用户原配置"，因为第一次点的时候就已经覆盖了）；
// 恢复时直接删除 SuperAI 写的 config.toml，让 codex 回到自己的默认行为。
//
// `auth.json` 不一样：里面可能有 ChatGPT 登录态，所以只在**首次**写入前
// 备份到 `auth.json.superai-bak`，已有备份就跳过 —— 这样反复点配置不会
// 把真正的原 `auth.json` 覆盖掉。

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexAppSetupResult {
    config_path: String,
    base_url: String,
    model_id: String,
    /// 若动到了 `~/.codex/auth.json`，这里是原文件的备份路径。
    auth_backup_path: Option<String>,
    /// 为 true 表示我们把 auth.json 的 ChatGPT tokens 清空了（只保留 API key 模式），
    /// 这样官方 Codex 客户端不会再显示 ChatGPT 额度，引导用户到 SuperAI 查看。
    auth_neutralized: bool,
}

/// 判断这份 `config.toml` 是不是 SuperAI 写的。
///
/// 判定：同时包含顶层 `model_provider = "superai"` 行和 `[model_providers.superai]`
/// 段。两个都在才算我们写的；否则一律不动文件（恢复逻辑会留给用户自己处理）。
fn config_is_superai_owned(content: &str) -> bool {
    let has_top = content
        .lines()
        .any(|line| line.trim() == "model_provider = \"superai\"");
    let has_section = content
        .lines()
        .any(|line| line.trim() == "[model_providers.superai]");
    has_top && has_section
}

/// 转义 TOML basic string 里的字符。我们的 API key 是 `agt_superai_<hex>`，
/// 实际只会落在 ASCII 安全集合里，但兜底处理一下双引号 / 反斜杠 / 控制符。
fn escape_toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 写入 codex `~/.codex/config.toml` 的 SuperAI 配置。
///
/// 设计：
/// - `model` 字段直接用 SuperAI UI 当前选中的真实模型名（如 `claude-opus-4.7-medium`）。
///   这样 codex TUI 顶部那行 `model:` 跟 SuperAI UI 一致，不再误导。
/// - SuperAI 切模型时，前端调 `set_api_service_default_model`，后端会顺手
///   `rewrite_managed_model_line()` 把这行改掉，下次 codex 重启就显示新模型；
///   codex 进程没重启时，proxy 内存里 default_model 也已热更，请求立即生效。
///
/// 鉴权选择 `requires_openai_auth = true`（**不**设 `env_key`）：
///   - codex-rs 里 `env_key` 是"强制查环境变量"语义：设了之后，进程启动时
///     该 env var 不存在就直接报 `Missing environment variable: OPENAI_API_KEY`，
///     **不会**回落到 `auth.json`。所以不能用 env_key 引用 auth.json 字段。
///   - 不设 env_key + `requires_openai_auth = true` 时，codex 会走 OpenAI auth
///     流程，从 `~/.codex/auth.json` 读 `OPENAI_API_KEY` 当 bearer token；
///     桌面版 settings 面板也认这种形态（渲染成"OpenAI 兼容站点用 API key"）。
///   - 之前用 `experimental_bearer_token` + `http_headers.Authorization` 虽然
///     codex CLI 能跑，但桌面版 settings UI 不认这种鉴权形态，左下角面板和
///     历史会话视图会整个坏掉。
fn build_superai_managed_block(base_url: &str, model_id: &str, _api_key: &str) -> String {
    let url = escape_toml_basic_string(base_url);
    let model = escape_toml_basic_string(model_id);
    format!(
        "model_provider = \"superai\"\n\
model = \"{model}\"\n\
model_reasoning_effort = \"medium\"\n\
approval_policy = \"on-request\"\n\
sandbox_mode = \"workspace-write\"\n\
network_access = \"enabled\"\n\
model_context_window = 200000\n\
model_max_output_tokens = 32768\n\
disable_response_storage = true\n\
personality = \"pragmatic\"\n\
service_tier = \"fast\"\n\
\n\
[model_providers.superai]\n\
name = \"SuperAI\"\n\
base_url = \"{url}\"\n\
wire_api = \"responses\"\n\
requires_openai_auth = true\n\
",
    )
}

/// 当 SuperAI UI 切换模型时调用：原地把 SuperAI 配置里的
/// `model = "..."` 那一行改成新模型名。
///
/// - 文件不存在 / 不是 SuperAI 配置 / 没找到 model 行 → 一律不动文件，返回 false。
///   说明用户还没点"配置 Codex"，不该擅自创建文件。
/// - 改动成功返回 true。
///
fn rewrite_managed_model_line(model_id: &str) -> Result<bool, String> {
    let codex_home = match codex_home_dir() {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    let config_path = codex_home.join("config.toml");
    if !config_path.exists() {
        return Ok(false);
    }
    let existing = read_to_string(&config_path)?;
    if !config_is_superai_owned(&existing) {
        return Ok(false);
    }

    let escaped = escape_toml_basic_string(model_id);
    let mut new_lines: Vec<String> = Vec::with_capacity(existing.lines().count());
    let mut replaced = false;
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with("model = \"") {
            new_lines.push(format!("model = \"{escaped}\""));
            replaced = true;
        } else {
            new_lines.push(line.to_string());
        }
    }
    if !replaced {
        return Ok(false);
    }
    let mut next = new_lines.join("\n");
    if existing.ends_with('\n') && !next.ends_with('\n') {
        next.push('\n');
    }

    if next == existing {
        return Ok(false);
    }

    write_string_atomic(&config_path, &next)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600));
    }
    Ok(true)
}

/// 把 `~/.codex/auth.json` 里的 ChatGPT tokens 清掉，只保留我们的 API key，
/// 这样官方 Codex CLI / 桌面版不会再拿 ChatGPT access_token 发请求，
/// 也不会再从 ChatGPT 账号拉"已用额度 / 订阅套餐"面板。
///
/// 原文件做一次性备份到 `auth.json.superai-bak`（若已存在则跳过，保留最早那份）。
/// 返回 (backup_path, neutralized)。neutralized=true 表示真的写了新内容。
fn neutralize_codex_auth_json(
    codex_home: &Path,
    api_key: &str,
) -> Result<(Option<String>, bool), String> {
    let auth_path = codex_home.join("auth.json");
    let backup_path_buf = codex_home.join("auth.json.superai-bak");

    // 1) 一次性备份原文件。
    let backup_path = if auth_path.exists() && !backup_path_buf.exists() {
        fs::copy(&auth_path, &backup_path_buf)
            .map_err(|error| format!("备份 {} 失败: {error}", auth_path.display()))?;
        Some(backup_path_buf.display().to_string())
    } else if backup_path_buf.exists() {
        Some(backup_path_buf.display().to_string())
    } else {
        None
    };

    // 2) 判断是否需要改写：若已经是 "仅 API key" 的 SuperAI 形态，就不重复写。
    //
    // 历史教训：之前我们写 `{ OPENAI_API_KEY, tokens: null, last_refresh: null }`，
    // 想着"显式声明 ChatGPT 登录态被清空"。但 codex 桌面版的设置面板会去读
    // `tokens.id_token` 之类的子字段，遇到 `null` 而不是 missing 直接报错，
    // 左下角设置 + 历史会话视图整个挂掉。参考 ylscode / 第三方站点工作配置，
    // auth.json 就只放 `OPENAI_API_KEY` 一个字段，`tokens` / `last_refresh` 必须**缺失**
    // 而不是 null。codex CLI/app 见到没 tokens 自然走 OPENAI_API_KEY 鉴权路径，
    // 设置面板也会渲染成"OpenAI 兼容站点用 API key"的标准状态。
    let desired = serde_json::json!({
        "OPENAI_API_KEY": api_key,
    });
    if auth_path.exists() {
        if let Ok(existing) = fs::read_to_string(&auth_path) {
            if let Ok(existing_json) = serde_json::from_str::<serde_json::Value>(&existing) {
                if existing_json == desired {
                    return Ok((backup_path, false));
                }
            }
        }
    }

    // 3) 原子写入 + 600 权限。
    let payload = serde_json::to_string_pretty(&desired)
        .map_err(|error| format!("序列化 auth.json 失败: {error}"))?;
    write_string_atomic(&auth_path, &payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600));
    }
    Ok((backup_path, true))
}

#[tauri::command]
fn configure_codex_app(app: tauri::AppHandle) -> Result<CodexAppSetupResult, String> {
    let mut settings = read_settings_record(&app)?;
    ensure_api_service_key(&app, &mut settings)?;
    let status = api_service::current_status(
        &settings.api_service_host,
        settings.api_service_port,
        &settings.api_service_key,
        &effective_api_service_model(&settings.api_service_default_model),
    );
    if !status.running {
        return Err("API 服务未运行，请先启动服务再一键配置 Codex".to_string());
    }
    let base_url = status
        .address
        .clone()
        .ok_or_else(|| "API 服务未提供监听地址".to_string())?;
    let api_key = status.api_key.clone();
    if api_key.trim().is_empty() {
        return Err("API 服务密钥为空，无法配置 Codex".to_string());
    }
    let model_id = status.default_model.trim().to_string();
    if model_id.is_empty() {
        return Err("尚未选择默认模型，请先在 API 服务配置里挑一个".to_string());
    }

    let codex_home = codex_home_dir()?;
    fs::create_dir_all(&codex_home)
        .map_err(|error| format!("创建目录失败 {}: {error}", codex_home.display()))?;

    let config_path = codex_home.join("config.toml");

    // config.toml 完全由 SuperAI 接管：直接整文件覆盖，不做任何备份。
    // 多次点"配置 Codex"也不会丢东西 —— 真有过用户原配置，也只会在最早那一次被覆盖。
    let mut next = build_superai_managed_block(&base_url, &model_id, &api_key);
    if !next.ends_with('\n') {
        next.push('\n');
    }
    write_string_atomic(&config_path, &next)?;
    // config.toml 含明文 bearer key，仅当前用户可读。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600));
    }

    // 中和 auth.json：清掉 ChatGPT tokens，只保留 API key 模式，
    // 让官方 Codex 不再展示 ChatGPT 额度，也不再拿 ChatGPT access_token 发请求。
    // 这一步**会**做幂等备份（仅首次），避免反复点配置 Codex 把真原文件覆盖。
    let (auth_backup_path, auth_neutralized) = neutralize_codex_auth_json(&codex_home, &api_key)?;

    Ok(CodexAppSetupResult {
        config_path: config_path.display().to_string(),
        base_url,
        model_id,
        auth_backup_path,
        auth_neutralized,
    })
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAppRestoreResult {
    /// 实际生效的恢复动作，给前端展示用。
    pub steps: Vec<String>,
    /// 是否真的把 auth.json 从 `.superai-bak` 备份恢复回来了。
    pub auth_restored_from_backup: bool,
    /// 是否把 SuperAI 自己写的 config.toml 删掉，让 codex 回到默认。
    pub config_removed: bool,
}

/// 把 `~/.codex` 还原成 SuperAI 接管之前的样子。
///
/// 现在的策略很简单：
/// - **`config.toml`**：完全由 SuperAI 写入，没备份；恢复时只要这份文件是
///   SuperAI 自己写的（同时含顶层 `model_provider = "superai"` 和
///   `[model_providers.superai]`），直接删掉，codex 回到默认行为。
///   不是 SuperAI 写的就保留 —— 我们没碰过用户自己手写的内容。
/// - **`auth.json`**：有 `.superai-bak` 备份就 cp 回去；没备份就跳过（我们写的
///   "仅 OPENAI_API_KEY" 形态没法机械反推 ChatGPT 登录态）。
#[tauri::command]
fn restore_codex_app(_app: tauri::AppHandle) -> Result<CodexAppRestoreResult, String> {
    let codex_home = codex_home_dir()?;
    let config_path = codex_home.join("config.toml");
    let auth_path = codex_home.join("auth.json");
    let auth_bak = codex_home.join("auth.json.superai-bak");

    let mut steps: Vec<String> = Vec::new();
    let mut auth_restored_from_backup = false;
    let mut config_removed = false;

    // ---- config.toml：是 SuperAI 写的就删 ----
    if config_path.exists() {
        let existing = read_to_string(&config_path)?;
        if config_is_superai_owned(&existing) {
            fs::remove_file(&config_path)
                .map_err(|error| format!("删除 SuperAI 写的 config.toml 失败: {error}"))?;
            steps.push(format!(
                "已删除 SuperAI 写的 {}（codex 回到默认行为）",
                config_path.display(),
            ));
            config_removed = true;
        } else {
            steps.push(format!(
                "config.toml 非 SuperAI 接管，原样保留（{}）",
                config_path.display(),
            ));
        }
    } else {
        steps.push("没有 ~/.codex/config.toml，无需处理".to_string());
    }

    // ---- auth.json：有备份就 cp 回去 ----
    if auth_bak.exists() {
        fs::copy(&auth_bak, &auth_path).map_err(|error| format!("恢复 auth.json 失败: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600));
        }
        steps.push(format!(
            "已用 {} 覆盖回 {}",
            auth_bak.display(),
            auth_path.display(),
        ));
        auth_restored_from_backup = true;
    } else {
        steps.push(
            "未找到 auth.json.superai-bak（首次配置前可能无 ChatGPT 登录态），跳过 auth.json"
                .to_string(),
        );
    }

    Ok(CodexAppRestoreResult {
        steps,
        auth_restored_from_backup,
        config_removed,
    })
}

fn stop_api_service_impl(app: tauri::AppHandle) -> Result<api_service::ApiServiceStatus, String> {
    api_service::stop()?;
    let mut settings = read_settings_record(&app)?;
    ensure_api_service_key(&app, &mut settings)?;
    if settings.api_service_enabled {
        settings.api_service_enabled = false;
        write_settings_record(&app, &settings)?;
    }
    Ok(api_service::current_status(
        &settings.api_service_host,
        settings.api_service_port,
        &settings.api_service_key,
        &effective_api_service_model(&settings.api_service_default_model),
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(Vec::<&str>::new()),
        ))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            start_window_drag,
            list_accounts,
            clear_local_database,
            upsert_accounts,
            refresh_account,
            refresh_provider_accounts,
            refresh_all_accounts,
            delete_account,
            switch_account,
            export_account,
            export_public_superai_account,
            load_settings,
            save_settings,
            import_accounts_from_json,
            import_codex_from_local,
            import_gemini_from_local,
            start_codex_oauth,
            complete_codex_oauth,
            start_gemini_oauth,
            complete_gemini_oauth,
            add_superai_account_by_password,
            add_superai_accounts_by_batch_keys,
            add_superai_account_by_token,
            get_api_service_status,
            start_api_service,
            stop_api_service,
            sync_superai_accounts_to_api,
            sync_api_service_active_account,
            list_api_service_models,
            set_api_service_default_model,
            configure_codex_app,
            restore_codex_app,
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                hide_main_window(window);
            }
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(ActivationPolicy::Accessory);

            let tray_menu = Menu::with_items(
                app,
                &[
                    &MenuItemBuilder::with_id(TRAY_MENU_SHOW, "显示主窗口").build(app)?,
                    &MenuItemBuilder::with_id(TRAY_MENU_QUIT, "退出 SuperAI").build(app)?,
                ],
            )?;
            let tray_builder = TrayIconBuilder::with_id("superai-tray")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .tooltip("Super AI");
            let tray_builder = if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder.icon(icon)
            } else {
                tray_builder
            };
            let _tray = tray_builder.build(app)?;

            app.on_menu_event(|app, event| match event.id().as_ref() {
                TRAY_MENU_SHOW => show_main_window(app),
                TRAY_MENU_QUIT => app.exit(0),
                _ => {}
            });

            app.on_tray_icon_event(|app, event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                ) {
                    show_main_window(app);
                }
            });

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "windows")]
                let app_size = LogicalSize::new(1320.0, 740.0);
                #[cfg(not(target_os = "windows"))]
                let app_size = LogicalSize::new(1320.0, 760.0);
                #[cfg(target_os = "windows")]
                window.set_resizable(true)?;
                #[cfg(not(target_os = "windows"))]
                window.set_resizable(false)?;
                window.set_min_size(Some(app_size))?;
                window.set_size(app_size)?;
            }

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // 清理历史构建留下的旧 sidecar 数据子目录（superal-api 是早期临时
            // 命名，windsurfapi 是更早的上游默认名）。新版本只往 superai-api
            // 写，所以这两个旧目录可以安全删除，避免暴露在用户的 Application
            // Support 目录里。
            if let Ok(data_dir) = app.handle().path().app_data_dir() {
                for stale in ["superal-api", "windsurfapi"] {
                    let path = data_dir.join(stale);
                    if path.exists() {
                        if let Err(error) = std::fs::remove_dir_all(&path) {
                            eprintln!(
                                "[startup] 清理旧 sidecar 数据目录失败 {}: {error}",
                                path.display()
                            );
                        }
                    }
                }
            }

            let handle = app.handle().clone();

            // 启动时立刻扫一遍过期 SuperAI 账号；之后每分钟再跑一次。
            // 仅 provider == "windsurf" + license_expires_at 已过期的账号会被删，
            // 软停用（100% 已耗尽）账号保留不动，等到期再统一清掉。
            // 删到当前账号会自动从剩余账号里挑下一个接管。
            match cleanup_expired_windsurf_and_handoff(&handle) {
                Ok(removed) if removed > 0 => {
                    eprintln!("[startup] 已清理 {removed} 个过期 SuperAI 账号");
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[startup] 清理过期 SuperAI 账号失败: {error}");
                }
            }
            spawn_expired_windsurf_cleanup(handle.clone());

            if let Ok(mut settings) = read_settings_record(&handle) {
                let _ = ensure_api_service_key(&handle, &mut settings);
                if settings.api_service_enabled {
                    tauri::async_runtime::spawn_blocking(move || {
                        match start_api_service_impl(handle.clone()) {
                            Ok(status) => emit_api_service_status_changed(&handle, &status),
                            Err(error) => {
                                eprintln!("[SuperAI API] 自启失败: {error}");
                                let _ = handle.emit(
                                    "api-service-error",
                                    serde_json::json!({"phase": "auto_start", "message": error}),
                                );
                            }
                        }
                    });
                }
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|handle, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                show_main_window(handle);
            }
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                let _ = api_service::stop();
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_status_proto_quota_tags_keep_daily_and_weekly_distinct() {
        let field_map: HashMap<&str, &str> =
            windsurf_plan_status_proto_field_map().into_iter().collect();

        assert_eq!(
            field_map.get("daily_quota_remaining_percent"),
            Some(&"int_15")
        );
        assert_eq!(
            field_map.get("weekly_quota_remaining_percent"),
            Some(&"int_14")
        );
        assert_eq!(field_map.get("daily_quota_reset_at_unix"), Some(&"int_18"));
        assert_eq!(field_map.get("weekly_quota_reset_at_unix"), Some(&"int_17"));
    }
}
