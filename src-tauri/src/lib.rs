use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{LogicalSize, Manager};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const GEMINI_KEYCHAIN_SERVICE: &str = "gemini-cli-oauth";
const GEMINI_KEYCHAIN_ACCOUNT: &str = "main-account";
const CODEX_ACCOUNT_CHECK_URL: &str = "https://chatgpt.com/backend-api/accounts/check/v4-2023-04-27";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_API_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

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
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
struct OAuthPending {
    provider: String,
}

static OAUTH_PENDING: LazyLock<Mutex<HashMap<String, OAuthPending>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
        if account.provider != provider {
            continue;
        }
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
            account.status = Some(AccountStatus {
                state: "available".to_string(),
                label: "可用".to_string(),
                reason: None,
                updated_at: Some(now),
            });
            account.updated_at = now;
        }
    }

    for account in accounts.iter().filter(|account| account.provider == provider) {
        upsert_account(conn, account)?;
    }

    Ok(accounts
        .into_iter()
        .filter(|account| account.provider == provider)
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

fn persist_local_import_as_current(
    app: &tauri::AppHandle,
    result: &mut ImportResult,
) -> Result<(), String> {
    if result.imported.is_empty() {
        return Ok(());
    }

    let current_id = result.imported[0].id.clone();
    let provider = result.imported[0].provider.clone();
    let conn = open_app_db(app)?;
    for account in &result.imported {
        upsert_account(&conn, account)?;
    }
    result.imported = set_account_current_state(&conn, &provider, &current_id)?;
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
    let account_json =
        serde_json::to_string(account).map_err(|error| format!("序列化账号失败: {error}"))?;
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
            account.id,
            account.provider,
            account.email,
            account.display_name,
            account_json,
            account.created_at,
            account.updated_at
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
    tx.commit()
        .map_err(|error| format!("提交 SQLite 事务失败: {error}"))?;
    Ok(())
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
            label: "WEEKLY".to_string(),
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
    let mut metrics = Vec::new();

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

    if token_meta.expires_at.is_some_and(|expires_at| expires_at <= now)
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
        let token_account_id = string_field(tokens.and_then(|t| t.get("account_id")).or_else(|| obj.get("account_id")))?;
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
        .or_else(|| auth.and_then(|a| a.get("chatgpt_subscription_active_until")).cloned());
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
        id: string_field(obj.get("id")).unwrap_or_else(|| {
            format!(
                "codex_{}",
                stable_hash(&discriminator)
            )
        }),
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

fn codex_access_token(account: &ManagedAccount) -> Option<String> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    let tokens = payload.get("tokens").and_then(Value::as_object);
    string_field(tokens.and_then(|t| t.get("access_token")).or_else(|| payload.get("access_token")))
}

fn codex_subscription_until_from_payload(account: &ManagedAccount) -> Option<Value> {
    let payload = account.auth_payload.as_ref()?.as_object()?;
    payload
        .get("subscription_active_until")
        .or_else(|| payload.get("subscriptionActiveUntil"))
        .cloned()
        .or_else(|| {
            let tokens = payload.get("tokens").and_then(Value::as_object);
            let id_token = string_field(tokens.and_then(|t| t.get("id_token")).or_else(|| payload.get("id_token")))?;
            let jwt = parse_jwt_payload(&id_token)?;
            jwt.get("https://api.openai.com/auth")
                .and_then(Value::as_object)
                .and_then(|auth| auth.get("chatgpt_subscription_active_until"))
                .cloned()
        })
}

fn parse_codex_account_profile(payload: &Value, account: &ManagedAccount) -> (Option<String>, Option<String>) {
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

fn usage_window_metric(key: &str, fallback_label: &str, window: Option<&Value>) -> Option<QuotaMetric> {
    let window = window?.as_object()?;
    let used = number_field(window.get("used_percent")).unwrap_or(0).clamp(0, 100);
    let remaining = 100 - used;
    let window_minutes = number_field(window.get("limit_window_seconds")).map(|seconds| (seconds + 59) / 60);
    let reset_at = number_field(window.get("reset_at")).or_else(|| {
        number_field(window.get("reset_after_seconds")).map(|seconds| now_ts() + seconds)
    });
    Some(QuotaMetric {
        key: key.to_string(),
        label: window_minutes
            .map(|minutes| if minutes >= 1440 { format!("{}d", minutes / 1440) } else { format!("{}h", (minutes + 59) / 60) })
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
    if let Some(metric) = usage_window_metric("primary", "5h", rate_limit.and_then(|r| r.get("primary_window"))) {
        metrics.push(metric);
    }
    if let Some(metric) = usage_window_metric("secondary", "Weekly", rate_limit.and_then(|r| r.get("secondary_window"))) {
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
    if account.provider != "codex" || account.token_meta.has_access_token == false {
        return Ok(());
    }
    let access_token = codex_access_token(account).ok_or_else(|| "缺少 access token".to_string())?;
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

fn fallback_status_refreshed(account: &ManagedAccount) -> AccountStatus {
    let empty = serde_json::Map::new();
    let obj = account
        .auth_payload
        .as_ref()
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    derive_status(
        obj,
        &account.token_meta,
        account.quota.as_ref(),
    )
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
        if account.provider == "codex" {
            if let Err(error) = refresh_codex_account_remote(account).await {
                mark_account_unavailable(account, error);
            }
        }
    }
}

fn refresh_imported_accounts_in_background(app: tauri::AppHandle, accounts: Vec<ManagedAccount>) {
    tauri::async_runtime::spawn(async move {
        let mut refreshed = accounts;
        refresh_imported_accounts(&mut refreshed).await;
        let _ = upsert_accounts_into_db(&app, &refreshed);
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
        if let Some(account) =
            parse_codex_account(item, source).or_else(|| parse_gemini_account(item, source))
        {
            imported.push(account);
        } else {
            failed.push(ImportFailure {
                label: item_label,
                reason: "未识别到 Codex 或 Gemini 凭证字段".to_string(),
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
    fs::rename(&tmp_path, path)
        .map_err(|error| format!("替换文件失败 {}: {error}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
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
    result.insert("auth_mode".to_string(), Value::String("chatgpt".to_string()));
    result.insert("OPENAI_API_KEY".to_string(), Value::Null);
    result.insert("tokens".to_string(), Value::Object(token_map));
    result.insert(
        "last_refresh".to_string(),
        payload
            .get("last_refresh")
            .cloned()
            .unwrap_or_else(|| Value::String(now_ts().to_string())),
    );
    Ok(Value::Object(result))
}

fn write_codex_auth(account: &ManagedAccount) -> Result<(), String> {
    let auth_payload = build_codex_auth_payload(account)?;
    let content = serde_json::to_string_pretty(&auth_payload)
        .map_err(|error| format!("序列化 Codex auth.json 失败: {error}"))?;
    let codex_home = codex_home_dir()?;
    write_string_atomic(&codex_home.join("auth.json"), &content)?;
    let _ = write_codex_keychain(&codex_home, &content);
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
    let token_type =
        nested_string_field(payload, token, "token_type", "tokenType").unwrap_or_else(|| "Bearer".to_string());
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
            let old = obj
                .entry("old")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(arr) = old.as_array_mut() {
                if !arr.iter().any(|item| item.as_str() == Some(active.as_str())) {
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

fn write_gemini_auth(account: &ManagedAccount) -> Result<(), String> {
    let oauth_payload = build_gemini_oauth_payload(account)?;
    let oauth_content = serde_json::to_string_pretty(&oauth_payload)
        .map_err(|error| format!("序列化 Gemini oauth_creds.json 失败: {error}"))?;
    write_string_atomic(&gemini_home_dir()?.join("oauth_creds.json"), &oauth_content)?;
    write_gemini_keychain(&oauth_payload)?;
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
        oauth.insert("access_token".to_string(), Value::String(access_token.to_string()));
    }
    if let Some(refresh_token) = token.get("refreshToken").and_then(Value::as_str) {
        oauth.insert("refresh_token".to_string(), Value::String(refresh_token.to_string()));
    }
    if let Some(token_type) = token.get("tokenType").and_then(Value::as_str) {
        oauth.insert("token_type".to_string(), Value::String(token_type.to_string()));
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

fn launch_oauth_command(command_text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        Command::new(shell)
            .args(["-lc", command_text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动 OAuth 失败: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            .args(["/C", command_text])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动 OAuth 失败: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("sh")
            .args(["-lc", command_text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动 OAuth 失败: {e}"))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持自动启动 OAuth".to_string())
}

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn list_accounts(app: tauri::AppHandle) -> Result<Vec<ManagedAccount>, String> {
    let conn = open_app_db(&app)?;
    read_accounts_from_conn(&conn)
}

#[tauri::command]
fn upsert_accounts(app: tauri::AppHandle, accounts: Vec<ManagedAccount>) -> Result<(), String> {
    upsert_accounts_into_db(&app, &accounts)
}

#[tauri::command]
#[allow(non_snake_case)]
async fn refresh_account(app: tauri::AppHandle, accountId: String) -> Result<ManagedAccount, String> {
    let mut account = {
        let conn = open_app_db(&app)?;
        load_account_from_db(&conn, &accountId)?
    };

    match account.provider.as_str() {
        "codex" => {
            if let Err(error) = refresh_codex_account_remote(&mut account).await {
                mark_account_unavailable(&mut account, error);
            }
        }
        "gemini" => {
            account.updated_at = now_ts();
        }
        other => return Err(format!("不支持的账号类型: {other}")),
    }

    let conn = open_app_db(&app)?;
    upsert_account(&conn, &account)?;
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
        if account.provider == "codex" {
            if let Err(error) = refresh_codex_account_remote(account).await {
                mark_account_unavailable(account, error);
            }
        } else {
            account.updated_at = now_ts();
        }
    }

    upsert_accounts_into_db(&app, &accounts)?;
    Ok(accounts)
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
        _ => serde_json::to_value(&account).map_err(|error| format!("序列化账号失败: {error}"))?,
    };
    serde_json::to_string_pretty(&value).map_err(|error| format!("序列化导出内容失败: {error}"))
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
        Ok(value_json) => serde_json::from_str::<AppSettings>(&value_json)
            .map(Some)
            .map_err(|error| format!("解析设置失败: {error}")),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(format!("读取设置失败: {error}")),
    }
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let conn = open_app_db(&app)?;
    let value_json =
        serde_json::to_string(&settings).map_err(|error| format!("序列化设置失败: {error}"))?;
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

#[tauri::command]
fn import_accounts_from_json(
    app: tauri::AppHandle,
    json_content: String,
    label: Option<String>,
) -> Result<ImportResult, String> {
    let result =
        parse_auth_json_content(&json_content, "paste", label.as_deref().unwrap_or("JSON"));
    upsert_accounts_into_db(&app, &result.imported)?;
    refresh_imported_accounts_in_background(app, result.imported.clone());
    Ok(result)
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
    persist_local_import_as_current(&app, &mut result)?;
    refresh_imported_accounts_in_background(app, result.imported.clone());
    Ok(result)
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
        serde_json::from_str(&oauth_content).map_err(|e| format!("解析 oauth_creds.json 失败: {e}"))?
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

    let mut result = parse_auth_json_content(&oauth_value.to_string(), "local", "Gemini 本机账号");
    persist_local_import_as_current(&app, &mut result)?;
    Ok(result)
}

#[tauri::command]
fn start_codex_oauth() -> Result<OAuthStartResult, String> {
    let login_id = format!("codex-{}-{}", now_ts(), stable_hash("codex"));
    let command = "codex login".to_string();
    launch_oauth_command(&command)?;
    OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .insert(
            login_id.clone(),
            OAuthPending {
                provider: "codex".to_string(),
            },
        );

    Ok(OAuthStartResult {
        login_id,
        provider: "codex".to_string(),
        command,
        message: "已启动 Codex OAuth，完成浏览器授权后会自动添加。".to_string(),
    })
}

#[tauri::command]
async fn complete_codex_oauth(app: tauri::AppHandle, login_id: String) -> Result<ImportResult, String> {
    let is_pending = OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .get(&login_id)
        .map(|p| p.provider.as_str() == "codex")
        .unwrap_or(false);
    if !is_pending {
        return Err("无效或已过期的 Codex OAuth 会话".to_string());
    }
    let result = import_codex_from_local(app)?;
    if !result.imported.is_empty() {
        OAUTH_PENDING
            .lock()
            .map_err(|_| "OAuth 状态锁失败".to_string())?
            .remove(&login_id);
    }
    Ok(result)
}

#[tauri::command]
fn start_gemini_oauth() -> Result<OAuthStartResult, String> {
    let login_id = format!("gemini-{}-{}", now_ts(), stable_hash("gemini"));
    let command = "gemini auth login".to_string();
    launch_oauth_command(&command)?;
    OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .insert(
            login_id.clone(),
            OAuthPending {
                provider: "gemini".to_string(),
            },
        );

    Ok(OAuthStartResult {
        login_id,
        provider: "gemini".to_string(),
        command,
        message: "已启动 Gemini OAuth，完成浏览器授权后会自动添加。".to_string(),
    })
}

#[tauri::command]
fn complete_gemini_oauth(app: tauri::AppHandle, login_id: String) -> Result<ImportResult, String> {
    let is_pending = OAUTH_PENDING
        .lock()
        .map_err(|_| "OAuth 状态锁失败".to_string())?
        .get(&login_id)
        .map(|p| p.provider.as_str() == "gemini")
        .unwrap_or(false);
    if !is_pending {
        return Err("无效或已过期的 Gemini OAuth 会话".to_string());
    }
    let result = import_gemini_from_local(app)?;
    if !result.imported.is_empty() {
        OAUTH_PENDING
            .lock()
            .map_err(|_| "OAuth 状态锁失败".to_string())?
            .remove(&login_id);
    }
    Ok(result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            start_window_drag,
            list_accounts,
            upsert_accounts,
            refresh_account,
            refresh_provider_accounts,
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
            complete_gemini_oauth
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let min_size = LogicalSize::new(1180.0, 760.0);
                let app_size = LogicalSize::new(1240.0, 820.0);
                window.set_min_size(Some(min_size))?;
                window.set_size(app_size)?;
            }

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
