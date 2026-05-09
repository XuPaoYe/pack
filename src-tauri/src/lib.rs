mod windsurf_api;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
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
use tauri::{LogicalSize, Manager};
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
const DEVIN_AUTH_BASE_URL: &str = "https://windsurf.com/_devin-auth";
const DEVIN_APP_AUTH_BASE_URL: &str = "https://app.devin.ai/api/auth1";
const WINDSURF_BACKEND_URL: &str = "https://web-backend.windsurf.com";

#[derive(Debug, Deserialize)]
struct DevinPasswordLoginResponse {
    #[serde(alias = "token", alias = "auth1Token", alias = "auth_token")]
    auth1_token: String,
    #[serde(default, alias = "user_id", alias = "userId", alias = "accountId")]
    account_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct WindsurfPostAuthResult {
    session_token: String,
    auth1_token: Option<String>,
    account_id: Option<String>,
    primary_org_id: Option<String>,
    orgs: Vec<WindsurfOrg>,
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
    #[serde(default)]
    windsurf_api_enabled: bool,
    #[serde(default = "default_windsurf_api_host")]
    windsurf_api_host: String,
    #[serde(default)]
    windsurf_api_port: u16,
    #[serde(default)]
    windsurf_api_key: String,
}

fn default_windsurf_api_host() -> String {
    windsurf_api::DEFAULT_HOST.to_string()
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
        windsurf_api_enabled: false,
        windsurf_api_host: default_windsurf_api_host(),
        windsurf_api_port: windsurf_api::DEFAULT_PORT,
        windsurf_api_key: String::new(),
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
      "#,
    )
    .map_err(|error| format!("初始化 SQLite 数据库失败: {error}"))?;
    conn.execute(
        "DELETE FROM accounts WHERE id IN ('codex_preview', 'gemini_preview')",
        [],
    )
    .map_err(|error| format!("清理演示账号失败: {error}"))?;
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
        } else if account
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
        let account = serde_json::from_str::<ManagedAccount>(&account_json)
            .map_err(|error| format!("解析账号记录失败: {error}"))?;
        accounts.push(account);
    }
    Ok(accounts)
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

fn enforce_single_current_account(conn: &Connection) -> Result<(), String> {
    let mut accounts = read_accounts_from_conn(conn)?;
    let mut current_ids = accounts
        .iter()
        .filter(|account| is_current_status(&account.status))
        .map(|account| (account.id.clone(), account.updated_at))
        .collect::<Vec<_>>();
    if current_ids.len() <= 1 {
        return Ok(());
    }

    current_ids.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let keep_id = current_ids[0].0.clone();
    for account in &mut accounts {
        if account.id != keep_id && is_current_status(&account.status) {
            mark_account_available(account);
            let account_json = serde_json::to_string(account)
                .map_err(|error| format!("序列化账号失败: {error}"))?;
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
    serde_json::from_str::<ManagedAccount>(&account_json)
        .map_err(|error| format!("解析账号记录失败: {error}"))
}

fn upsert_account(conn: &Connection, account: &ManagedAccount) -> Result<(), String> {
    let mut account_to_write = account.clone();
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
            if existing.id != account_to_write.id && is_current_status(&existing.status) {
                mark_account_available(existing);
                let existing_json = serde_json::to_string(existing)
                    .map_err(|error| format!("序列化账号失败: {error}"))?;
                conn.execute(
                    "UPDATE accounts SET account_json = ?1, updated_at = ?2 WHERE id = ?3",
                    params![existing_json, existing.updated_at, existing.id],
                )
                .map_err(|error| format!("清理当前账号状态失败: {error}"))?;
            }
        }
    }
    let account_json = serde_json::to_string(&account_to_write)
        .map_err(|error| format!("序列化账号失败: {error}"))?;
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
            account_to_write.provider,
            account_to_write.email,
            account_to_write.display_name,
            account_json,
            account_to_write.created_at,
            account_to_write.updated_at
        ],
    )
    .map_err(|error| format!("写入账号 SQLite 失败: {error}"))?;
    Ok(())
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

    if quota
        .map(|q| {
            q.metrics
                .iter()
                .any(|metric| metric.remaining_percent == Some(0))
        })
        .unwrap_or(false)
    {
        return AccountStatus {
            state: "unavailable".to_string(),
            label: "不可用".to_string(),
            reason: Some("至少一个额度窗口剩余 0%".to_string()),
            updated_at: quota.and_then(|q| q.last_updated),
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
        && auth1_token.is_none()
        && session_token.is_none()
    {
        return None;
    }

    let jwt = id_token.as_deref().and_then(parse_jwt_payload);
    let discriminator_token = session_token
        .clone()
        .or_else(|| auth1_token.clone())
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
            || session_token.is_some()
            || auth1_token.is_some(),
        has_refresh_token: refresh_token.is_some(),
        has_id_token: id_token.is_some(),
        expires_at,
    };
    let discriminator = local_id
        .clone()
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
                account.token_meta.has_access_token = true;
                return Ok(());
            }
            return Err("缺少 refresh_token".to_string());
        }
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Windsurf 客户端失败: {error}"))?;
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
        .map_err(|error| format!("Windsurf token 刷新请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 Windsurf 刷新响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!("Windsurf 刷新失败 ({status}): {text}"));
    }
    let payload: Value = serde_json::from_str(&text)
        .map_err(|error| format!("解析 Windsurf 刷新响应失败: {error}"))?;
    let id_token = string_field(payload.get("id_token"))
        .ok_or_else(|| "Windsurf 刷新响应缺少 id_token".to_string())?;
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
    Ok(())
}

fn build_windsurf_payload(account: &ManagedAccount) -> Result<Value, String> {
    let payload = account
        .auth_payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| "该账号缺少可导出的 Windsurf 凭证".to_string())?;
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
        .map_err(|error| format!("创建 Windsurf 客户端失败: {error}"))?;
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
        .map_err(|error| format!("Windsurf 登录请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 Windsurf 登录响应失败: {error}"))?;
    if !status.is_success() {
        if text.contains("INVALID_LOGIN_CREDENTIALS") || text.contains("INVALID_PASSWORD") {
            return Err("邮箱或密码错误".to_string());
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
        return Err(format!("Windsurf 登录失败 ({status}): {text}"));
    }
    serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("解析 Windsurf 登录响应失败: {error}"))
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
            .ok_or_else(|| "WindsurfPostAuth 响应 tag 解码失败".to_string())?;
        i += consumed;
        let field_no = (tag >> 3) as u32;
        let wire_type = (tag & 0x7) as u8;
        if wire_type == 2 {
            let (len, consumed_len) = decode_varint(bytes, i)
                .ok_or_else(|| "WindsurfPostAuth 响应长度解码失败".to_string())?;
            i += consumed_len;
            let end = i + len as usize;
            if end > bytes.len() {
                return Err("WindsurfPostAuth 响应长度越界".to_string());
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
                        .ok_or_else(|| "WindsurfPostAuth 响应 varint 跳过失败".to_string())?;
                    i += consumed_value;
                }
                1 => i += 8,
                5 => i += 4,
                _ => return Err(format!("WindsurfPostAuth 不支持的 wire type: {wire_type}")),
            }
        }
    }
    if result.session_token.is_empty() {
        return Err("WindsurfPostAuth 响应未包含 session_token".to_string());
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
    for (key, field) in [
        ("available_flex_credits", "int_4"),
        ("used_flow_credits", "int_5"),
        ("used_prompt_credits", "int_6"),
        ("used_flex_credits", "int_7"),
        ("available_prompt_credits", "int_8"),
        ("available_flow_credits", "int_9"),
        ("daily_quota_remaining_percent", "int_14"),
        ("weekly_quota_remaining_percent", "int_15"),
        ("overage_balance_micros", "int_16"),
        ("daily_quota_reset_at_unix", "int_17"),
        ("weekly_quota_reset_at_unix", "int_18"),
    ] {
        if let Some(value) = proto_i64(plan_status.get(field)) {
            result.insert(key.to_string(), Value::Number(value.into()));
        }
    }
    Ok(Value::Object(result))
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

async fn windsurf_get_current_user(account: &ManagedAccount) -> Result<Value, String> {
    let session_token = windsurf_payload_string(account, "session_token")
        .or_else(|| windsurf_payload_string(account, "id_token"))
        .ok_or_else(|| "缺少可用于查询 Windsurf 账号信息的 token".to_string())?;
    let auth1_token = windsurf_payload_string(account, "auth1_token");
    let account_id = windsurf_payload_string(account, "local_id");
    let primary_org_id = windsurf_payload_string(account, "primary_org_id");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Windsurf GetCurrentUser 客户端失败: {error}"))?;
    let url = format!(
        "{WINDSURF_BACKEND_URL}/exa.seat_management_pb.SeatManagementService/GetCurrentUser"
    );
    let mut body = Vec::with_capacity(session_token.len() + 8);
    encode_proto_string_field(&mut body, 1, &session_token);
    body.extend_from_slice(&[0x10, 0x01, 0x18, 0x01, 0x20, 0x01]);
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
        .map_err(|error| format!("GetCurrentUser 请求失败: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 GetCurrentUser 响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "GetCurrentUser 失败 ({status}): {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    extract_windsurf_current_user(&bytes)
}

async fn windsurf_get_plan_status(account: &ManagedAccount) -> Result<Value, String> {
    let session_token = windsurf_payload_string(account, "session_token")
        .or_else(|| windsurf_payload_string(account, "id_token"))
        .ok_or_else(|| "缺少可用于查询 Windsurf 套餐状态的 token".to_string())?;
    let auth1_token = windsurf_payload_string(account, "auth1_token");
    let account_id = windsurf_payload_string(account, "local_id");
    let primary_org_id = windsurf_payload_string(account, "primary_org_id");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Windsurf GetPlanStatus 客户端失败: {error}"))?;
    let url = format!(
        "{WINDSURF_BACKEND_URL}/exa.seat_management_pb.SeatManagementService/GetPlanStatus"
    );
    let mut body = Vec::with_capacity(session_token.len() + 8);
    encode_proto_string_field(&mut body, 1, &session_token);
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
        .map_err(|error| format!("GetPlanStatus 请求失败: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 GetPlanStatus 响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "GetPlanStatus 失败 ({status}): {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    extract_windsurf_plan_status(&bytes)
}

async fn enrich_windsurf_account_remote(account: &mut ManagedAccount) -> Result<(), String> {
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
        Err(last_error.unwrap_or_else(|| "未能获取 Windsurf 账号信息".to_string()))
    }
}

async fn devin_password_login_with_base(
    base_url: &str,
    email: &str,
    password: &str,
) -> Result<DevinPasswordLoginResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 Devin 客户端失败: {error}"))?;
    let url = format!("{base_url}/password/login");
    let response = client
        .post(&url)
        .json(&serde_json::json!({ "email": email, "password": password }))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/account/login")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-origin")
        .header(USER_AGENT, CODEX_API_USER_AGENT)
        .send()
        .await
        .map_err(|error| format!("Devin 登录请求失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 Devin 登录响应失败: {error}"))?;
    if !status.is_success() {
        let lower = text.to_lowercase();
        if lower.contains("invalid")
            && (lower.contains("password") || lower.contains("credentials"))
        {
            return Err("邮箱或密码错误".to_string());
        }
        if lower.contains("not found") || lower.contains("no such") {
            return Err("该邮箱未注册 Devin/Auth1 账号".to_string());
        }
        if lower.contains("too many") || lower.contains("rate") || status.as_u16() == 429 {
            return Err("尝试次数过多，请稍后再试".to_string());
        }
        return Err(format!("Devin 登录失败 ({status}): {text}"));
    }
    serde_json::from_str::<DevinPasswordLoginResponse>(&text)
        .map_err(|error| format!("解析 Devin 登录响应失败: {error}"))
}

async fn windsurf_post_auth(
    auth1_token: &str,
    org_id: Option<&str>,
) -> Result<WindsurfPostAuthResult, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建 WindsurfPostAuth 客户端失败: {error}"))?;
    let url = format!(
        "{WINDSURF_BACKEND_URL}/exa.seat_management_pb.SeatManagementService/WindsurfPostAuth"
    );
    let mut body = Vec::with_capacity(auth1_token.len() + org_id.unwrap_or("").len() + 4);
    encode_proto_string_field(&mut body, 1, auth1_token);
    if let Some(org_id) = org_id.filter(|value| !value.trim().is_empty()) {
        encode_proto_string_field(&mut body, 2, org_id);
    }
    let response = client
        .post(&url)
        .body(body)
        .header(ACCEPT, "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
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
        .map_err(|error| format!("WindsurfPostAuth 请求失败: {error}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 WindsurfPostAuth 响应失败: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "WindsurfPostAuth 失败 ({status}): {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    parse_windsurf_post_auth_response(&bytes)
}

async fn windsurf_devin_sign_in(email: &str, password: &str) -> Result<Value, String> {
    let login = match devin_password_login_with_base(DEVIN_AUTH_BASE_URL, email, password).await {
        Ok(value) => value,
        Err(bridge_error) => {
            match devin_password_login_with_base(DEVIN_APP_AUTH_BASE_URL, email, password).await {
                Ok(value) => value,
                Err(native_error) => {
                    return Err(format!(
                    "Devin/Auth1 登录失败；Windsurf 桥接: {bridge_error}；Devin 原生: {native_error}"
                ));
                }
            }
        }
    };
    let post_auth = windsurf_post_auth(&login.auth1_token, None).await?;
    let effective_auth1_token = post_auth
        .auth1_token
        .clone()
        .unwrap_or_else(|| login.auth1_token.clone());
    let resolved_email = login.email.clone().unwrap_or_else(|| email.to_string());
    let mut tokens_map = serde_json::Map::new();
    tokens_map.insert(
        "auth1_token".to_string(),
        Value::String(effective_auth1_token),
    );
    tokens_map.insert(
        "session_token".to_string(),
        Value::String(post_auth.session_token),
    );
    if let Some(account_id) = post_auth.account_id.clone().or(login.account_id.clone()) {
        tokens_map.insert("local_id".to_string(), Value::String(account_id));
    }
    if let Some(primary_org_id) = post_auth
        .primary_org_id
        .clone()
        .or_else(|| post_auth.orgs.first().map(|org| org.id.clone()))
    {
        tokens_map.insert("primary_org_id".to_string(), Value::String(primary_org_id));
    }
    let mut payload = serde_json::Map::new();
    payload.insert(
        "provider".to_string(),
        Value::String("windsurf".to_string()),
    );
    payload.insert("email".to_string(), Value::String(resolved_email));
    payload.insert("tokens".to_string(), Value::Object(tokens_map));
    Ok(Value::Object(payload))
}

#[tauri::command]
#[allow(non_snake_case)]
async fn add_windsurf_account_by_password(
    app: tauri::AppHandle,
    email: String,
    password: String,
) -> Result<ManagedAccount, String> {
    let trimmed_email = email.trim();
    let trimmed_password = password.trim();
    if trimmed_email.is_empty() || trimmed_password.is_empty() {
        return Err("邮箱和密码不能为空".to_string());
    }

    let signin = match windsurf_firebase_sign_in(trimmed_email, trimmed_password).await {
        Ok(value) => value,
        Err(firebase_error) => {
            match windsurf_devin_sign_in(trimmed_email, trimmed_password).await {
                Ok(value) => {
                    let mut account = parse_windsurf_account(&value, "password")
                        .ok_or_else(|| "构建 Devin/Auth1 Windsurf 账号记录失败".to_string())?;
                    let _ = enrich_windsurf_account_remote(&mut account).await;
                    upsert_accounts_into_db(&app, std::slice::from_ref(&account))?;
                    return Ok(account);
                }
                Err(devin_error) => {
                    return Err(format!(
                        "Firebase 登录失败：{firebase_error}；Devin/Auth1 登录失败：{devin_error}"
                    ));
                }
            }
        }
    };
    let id_token = string_field(signin.get("idToken"))
        .ok_or_else(|| "Windsurf 登录响应缺少 idToken".to_string())?;
    let refresh_token = string_field(signin.get("refreshToken"))
        .ok_or_else(|| "Windsurf 登录响应缺少 refreshToken".to_string())?;
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
    if let Some(name) = final_display_name.clone() {
        payload.insert("display_name".to_string(), Value::String(name));
    }
    payload.insert("tokens".to_string(), Value::Object(tokens_map));

    let account = parse_windsurf_account(&Value::Object(payload), "password")
        .ok_or_else(|| "构建 Windsurf 账号记录失败".to_string())?;
    upsert_accounts_into_db(&app, std::slice::from_ref(&account))?;
    Ok(account)
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
    Ok(result)
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
                reason: "未识别到 Codex / Gemini / Windsurf 凭证字段".to_string(),
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
    let conn = open_app_db(&app)?;
    enforce_single_current_account(&conn)?;
    read_accounts_from_conn(&conn)
}

#[tauri::command]
fn upsert_accounts(app: tauri::AppHandle, accounts: Vec<ManagedAccount>) -> Result<(), String> {
    upsert_accounts_into_db(&app, &accounts)
}

#[tauri::command]
#[allow(non_snake_case)]
async fn refresh_account(
    app: tauri::AppHandle,
    accountId: String,
) -> Result<ManagedAccount, String> {
    let mut account = {
        let conn = open_app_db(&app)?;
        load_account_from_db(&conn, &accountId)?
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
    if was_current {
        mark_account_current(&mut account);
    }

    let written = upsert_existing_accounts_into_db(&app, &[account.clone()])?;
    if written.is_empty() {
        return Err("账号已被删除，刷新结果已丢弃".to_string());
    }
    Ok(account)
}

#[tauri::command]
async fn refresh_provider_accounts(
    app: tauri::AppHandle,
    provider: String,
) -> Result<Vec<ManagedAccount>, String> {
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
        if was_current {
            mark_account_current(account);
        }
    }

    upsert_existing_accounts_into_db(&app, &accounts)
}

#[tauri::command]
async fn refresh_all_accounts(app: tauri::AppHandle) -> Result<Vec<ManagedAccount>, String> {
    let mut accounts = {
        let conn = open_app_db(&app)?;
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
            _ => {}
        }
        if was_current {
            mark_account_current(account);
        }
    }

    upsert_existing_accounts_into_db(&app, &accounts)
}

#[tauri::command]
#[allow(non_snake_case)]
fn delete_account(app: tauri::AppHandle, accountId: String) -> Result<Vec<ManagedAccount>, String> {
    let conn = open_app_db(&app)?;
    let deleted = conn
        .execute("DELETE FROM accounts WHERE id = ?1", params![accountId])
        .map_err(|error| format!("删除账号失败: {error}"))?;
    if deleted == 0 {
        return Err("账号不存在，可能已经被删除".to_string());
    }
    read_accounts_from_conn(&conn)
}

#[tauri::command]
#[allow(non_snake_case)]
fn switch_account(app: tauri::AppHandle, accountId: String) -> Result<Vec<ManagedAccount>, String> {
    let conn = open_app_db(&app)?;
    let account = load_account_from_db(&conn, &accountId)?;
    match account.provider.as_str() {
        "codex" => write_codex_auth(&account)?,
        "gemini" => write_gemini_auth(&account)?,
        "windsurf" => {
            // Windsurf 是 Web 端账号，没有本机配置文件需要写入；仅在数据库中标记为当前。
        }
        other => return Err(format!("不支持的账号类型: {other}")),
    }
    set_account_current_state(&conn, &account.provider, &account.id)
}

#[tauri::command]
#[allow(non_snake_case)]
fn export_account(app: tauri::AppHandle, accountId: String) -> Result<String, String> {
    let conn = open_app_db(&app)?;
    let account = load_account_from_db(&conn, &accountId)?;
    let value = match account.provider.as_str() {
        "codex" => build_codex_auth_payload(&account)?,
        "gemini" => build_gemini_oauth_payload(&account)?,
        "windsurf" => build_windsurf_payload(&account)?,
        _ => serde_json::to_value(&account).map_err(|error| format!("序列化账号失败: {error}"))?,
    };
    serde_json::to_string_pretty(&value).map_err(|error| format!("序列化导出内容失败: {error}"))
}

fn system_auto_launch_enabled(app: &tauri::AppHandle) -> Result<Option<bool>, String> {
    #[cfg(desktop)]
    {
        app
            .autolaunch()
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
        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        result.map_err(|error| {
            if enabled {
                format!("启用系统开机自启失败: {error}")
            } else {
                format!("关闭系统开机自启失败: {error}")
            }
        })?;
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
    // 前端不维护 windsurf_api_* 字段，从已有记录里继承，避免被默认值覆盖。
    let mut merged = settings;
    if let Ok(existing) = read_settings_record(&app) {
        merged.windsurf_api_enabled = existing.windsurf_api_enabled;
        merged.windsurf_api_host = existing.windsurf_api_host;
        merged.windsurf_api_port = existing.windsurf_api_port;
        merged.windsurf_api_key = existing.windsurf_api_key;
    }
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
        return Ok(ImportResult { imported: vec![], failed: vec![] });
    };
    if pending.provider != "codex" {
        return Err("无效的 Codex OAuth 会话".to_string());
    }
    if pending.expires_at <= now_ts() {
        oauth_pending_remove(&login_id);
        return Err("Codex OAuth 登录已超时，请重新发起授权".to_string());
    }
    let Some(code) = pending.code else {
        return Ok(ImportResult { imported: vec![], failed: vec![] });
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
        return Ok(ImportResult { imported: vec![], failed: vec![] });
    };
    if pending.provider != "gemini" {
        return Err("无效的 Gemini OAuth 会话".to_string());
    }
    if pending.expires_at <= now_ts() {
        oauth_pending_remove(&login_id);
        return Err("Gemini OAuth 登录已超时，请重新发起授权".to_string());
    }
    let Some(code) = pending.code else {
        return Ok(ImportResult { imported: vec![], failed: vec![] });
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

fn ensure_windsurf_api_key(app: &tauri::AppHandle, settings: &mut AppSettings) -> Result<(), String> {
    if settings.windsurf_api_key.trim().is_empty() {
        settings.windsurf_api_key = windsurf_api::generate_api_key();
        write_settings_record(app, settings)?;
    }
    Ok(())
}

#[tauri::command]
fn get_windsurf_api_status(
    app: tauri::AppHandle,
) -> Result<windsurf_api::WindsurfApiStatus, String> {
    let mut settings = read_settings_record(&app)?;
    ensure_windsurf_api_key(&app, &mut settings)?;
    Ok(windsurf_api::current_status(
        &settings.windsurf_api_host,
        settings.windsurf_api_port,
        &settings.windsurf_api_key,
    ))
}

#[tauri::command]
fn start_windsurf_api(
    app: tauri::AppHandle,
) -> Result<windsurf_api::WindsurfApiStatus, String> {
    let mut settings = read_settings_record(&app)?;
    ensure_windsurf_api_key(&app, &mut settings)?;
    let status = windsurf_api::start(
        &settings.windsurf_api_host,
        settings.windsurf_api_port,
        &settings.windsurf_api_key,
    )?;
    if !settings.windsurf_api_enabled {
        settings.windsurf_api_enabled = true;
        write_settings_record(&app, &settings)?;
    }
    Ok(status)
}

#[tauri::command]
fn stop_windsurf_api(
    app: tauri::AppHandle,
) -> Result<windsurf_api::WindsurfApiStatus, String> {
    windsurf_api::stop()?;
    let mut settings = read_settings_record(&app)?;
    ensure_windsurf_api_key(&app, &mut settings)?;
    if settings.windsurf_api_enabled {
        settings.windsurf_api_enabled = false;
        write_settings_record(&app, &settings)?;
    }
    Ok(windsurf_api::current_status(
        &settings.windsurf_api_host,
        settings.windsurf_api_port,
        &settings.windsurf_api_key,
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
            upsert_accounts,
            refresh_account,
            refresh_provider_accounts,
            refresh_all_accounts,
            delete_account,
            switch_account,
            export_account,
            load_settings,
            save_settings,
            import_accounts_from_json,
            import_codex_from_local,
            import_gemini_from_local,
            start_codex_oauth,
            complete_codex_oauth,
            start_gemini_oauth,
            complete_gemini_oauth,
            add_windsurf_account_by_password,
            get_windsurf_api_status,
            start_windsurf_api,
            stop_windsurf_api,
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let app_size = LogicalSize::new(1240.0, 820.0);
                window.set_resizable(false)?;
                window.set_min_size(Some(app_size))?;
                window.set_max_size(Some(app_size))?;
                window.set_size(app_size)?;
            }

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let handle = app.handle().clone();
            if let Ok(mut settings) = read_settings_record(&handle) {
                let _ = ensure_windsurf_api_key(&handle, &mut settings);
                if settings.windsurf_api_enabled {
                    if let Err(error) = windsurf_api::start(
                        &settings.windsurf_api_host,
                        settings.windsurf_api_port,
                        &settings.windsurf_api_key,
                    ) {
                        eprintln!("[Windsurf API] 自启失败: {error}");
                    }
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
