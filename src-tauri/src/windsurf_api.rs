//! Windsurf 本地 API 服务（阶段 1 骨架）
//!
//! 暴露 OpenAI 兼容入口，外部工具可通过 `Authorization: Bearer agt_wsf_*`
//! 访问 `127.0.0.1:<port>/v1/...`。当前实现只做：
//!   - 服务起停 / 状态查询
//!   - Bearer 鉴权
//!   - `GET /v1/models` 返回固定模型列表
//!   - `POST /v1/chat/completions` 返回 501，等待阶段 3 接入 LS sidecar
//!
//! 阶段 2 起会接入 Windsurf Language Server sidecar，再把 chat 协议接通。

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server, StatusCode};

/// 默认监听主机：`0.0.0.0` 表示同时监听本机与局域网。
pub const DEFAULT_HOST: &str = "0.0.0.0";
/// 默认端口 `0` 表示由内核分配未占用端口。
pub const DEFAULT_PORT: u16 = 0;
/// 默认 API Key 前缀；首次启动会生成 `agt_wsf_<随机串>`。
pub const API_KEY_PREFIX: &str = "agt_wsf_";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfApiStatus {
    pub running: bool,
    pub bind_host: String,
    pub bind_port: u16,
    /// 实际监听的端口（自动选端口时与 `bind_port` 不同）。
    pub actual_port: Option<u16>,
    /// 拼好的 base URL，例如 `http://127.0.0.1:63721/v1`。
    pub address: Option<String>,
    pub api_key: String,
    pub last_error: Option<String>,
}

struct Runtime {
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    bind_host: String,
    bind_port: u16,
    actual_port: u16,
    api_key: String,
    last_error: Option<String>,
}

static RUNTIME: LazyLock<Mutex<Option<Runtime>>> = LazyLock::new(|| Mutex::new(None));

fn lock() -> std::sync::MutexGuard<'static, Option<Runtime>> {
    RUNTIME.lock().expect("Windsurf API 运行态锁失败")
}

/// 生成形如 `agt_wsf_xxxxxxxxxxxxxxxx` 的密钥。
pub fn generate_api_key() -> String {
    let bytes: [u8; 24] = rand::random();
    let token = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    format!("{API_KEY_PREFIX}{token}")
}

/// 构造对外展示的状态。
pub fn current_status(default_host: &str, default_port: u16, api_key: &str) -> WindsurfApiStatus {
    let guard = lock();
    if let Some(runtime) = guard.as_ref() {
        let address = build_address(&runtime.bind_host, runtime.actual_port);
        WindsurfApiStatus {
            running: true,
            bind_host: runtime.bind_host.clone(),
            bind_port: runtime.bind_port,
            actual_port: Some(runtime.actual_port),
            address: Some(address),
            api_key: runtime.api_key.clone(),
            last_error: runtime.last_error.clone(),
        }
    } else {
        WindsurfApiStatus {
            running: false,
            bind_host: default_host.to_string(),
            bind_port: default_port,
            actual_port: None,
            address: None,
            api_key: api_key.to_string(),
            last_error: None,
        }
    }
}

fn build_address(host: &str, port: u16) -> String {
    let visible = if host == "0.0.0.0" || host.is_empty() {
        "127.0.0.1"
    } else {
        host
    };
    format!("http://{visible}:{port}/v1")
}

/// 启动服务。重复调用会先停旧实例再启动新的。
pub fn start(host: &str, port: u16, api_key: &str) -> Result<WindsurfApiStatus, String> {
    if api_key.trim().is_empty() {
        return Err("API Key 为空，无法启动".to_string());
    }
    stop()?;

    let bind = format!("{host}:{port}");
    let server = Server::http(&bind).map_err(|error| format!("绑定 {bind} 失败: {error}"))?;
    let actual_port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "读取服务监听端口失败".to_string())?
        .port();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_for_thread = stop_flag.clone();
    let api_key_owned = api_key.to_string();
    let host_owned = host.to_string();

    let join = thread::Builder::new()
        .name("windsurf-api".into())
        .spawn(move || {
            run_server(server, stop_flag_for_thread, api_key_owned);
        })
        .map_err(|error| format!("创建服务线程失败: {error}"))?;

    *lock() = Some(Runtime {
        stop_flag,
        join: Some(join),
        bind_host: host_owned.clone(),
        bind_port: port,
        actual_port,
        api_key: api_key.to_string(),
        last_error: None,
    });

    Ok(WindsurfApiStatus {
        running: true,
        bind_host: host_owned.clone(),
        bind_port: port,
        actual_port: Some(actual_port),
        address: Some(build_address(&host_owned, actual_port)),
        api_key: api_key.to_string(),
        last_error: None,
    })
}

/// 停止服务（幂等）。
pub fn stop() -> Result<(), String> {
    let runtime = { lock().take() };
    let Some(mut runtime) = runtime else {
        return Ok(());
    };
    runtime.stop_flag.store(true, Ordering::SeqCst);
    if let Some(handle) = runtime.join.take() {
        let _ = handle.join();
    }
    Ok(())
}

fn run_server(server: Server, stop_flag: Arc<AtomicBool>, api_key: String) {
    while !stop_flag.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(request)) => handle_request(request, &api_key),
            Ok(None) => continue,
            Err(_) => break,
        }
    }
}

fn handle_request(mut request: tiny_http::Request, api_key: &str) {
    // CORS 预检
    if matches!(request.method(), Method::Options) {
        let _ = request.respond(cors_response(204, b""));
        return;
    }

    let path = request.url().split('?').next().unwrap_or("").to_string();
    let method = request.method().clone();

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

    match (method, path.as_str()) {
        (Method::Get, "/v1/models") | (Method::Get, "/v1/models/") => {
            let _ = request.respond(json_response(200, &models_payload()));
        }
        (Method::Post, "/v1/chat/completions") | (Method::Post, "/v1/messages") => {
            // 占位：阶段 3 接入 LS sidecar 后再实现。
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let _ = request.respond(json_response(
                501,
                &json!({
                    "error": {
                        "message": "Windsurf 本地 API 服务尚未接入 Language Server，请等待后续版本。",
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

fn is_authorized(request: &tiny_http::Request, api_key: &str) -> bool {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| {
            let value = h.value.as_str();
            value.strip_prefix("Bearer ").map(str::to_string)
        })
        .is_some_and(|token| token.trim() == api_key)
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
            &b"GET, POST, OPTIONS"[..],
        )
        .expect("cors methods"),
        Header::from_bytes(
            &b"Access-Control-Allow-Headers"[..],
            &b"Authorization, Content-Type"[..],
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

fn models_payload() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ids = [
        "windsurf-swe-1",
        "windsurf-swe-1-lite",
        "claude-3-5-sonnet",
        "claude-3-7-sonnet",
        "claude-sonnet-4",
        "gpt-4o",
        "gpt-4.1",
        "gpt-5",
        "gemini-2.5-pro",
    ];
    let data: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "created": now,
                "owned_by": "windsurf",
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
        let key = "agt_wsf_test_key_12345";
        let status = start("127.0.0.1", 0, key).expect("start");
        assert!(status.running);
        let port = status.actual_port.expect("actual port");
        assert!(port > 0);
        let addr = format!("127.0.0.1:{port}");

        // 401 without auth
        let (code, _) = http_get(&addr, "/v1/models", None);
        assert_eq!(code, 401, "missing auth should be 401");

        // 401 with wrong key
        let (code, _) = http_get(&addr, "/v1/models", Some("wrong"));
        assert_eq!(code, 401, "wrong key should be 401");

        // 200 with right key
        let (code, body) = http_get(&addr, "/v1/models", Some(key));
        assert_eq!(code, 200);
        assert!(body.contains("\"object\":\"list\""), "body: {body}");
        assert!(body.contains("claude-sonnet-4"), "body: {body}");

        // 404 unknown path
        let (code, _) = http_get(&addr, "/nope", Some(key));
        assert_eq!(code, 404);

        // 501 chat completions placeholder
        let (code, body) =
            http_post(&addr, "/v1/chat/completions", key, r#"{"model":"x","messages":[]}"#);
        assert_eq!(code, 501);
        assert!(body.contains("not_implemented"), "body: {body}");

        // stop is idempotent
        stop().unwrap();
        stop().unwrap();
    }

    #[test]
    fn restart_picks_new_port() {
        let key = "agt_wsf_restart_test";
        let s1 = start("127.0.0.1", 0, key).unwrap();
        let p1 = s1.actual_port.unwrap();
        let s2 = start("127.0.0.1", 0, key).unwrap();
        let p2 = s2.actual_port.unwrap();
        assert!(p1 > 0 && p2 > 0);
        // 重启服务可成功，端口可能相同也可能不同；关键是 RUNTIME 已替换
        let cur = current_status("127.0.0.1", 0, key);
        assert!(cur.running);
        assert_eq!(cur.actual_port, Some(p2));
        stop().unwrap();
    }

    #[test]
    fn empty_key_rejected() {
        let err = start("127.0.0.1", 0, "").unwrap_err();
        assert!(err.contains("API Key"));
    }
}
