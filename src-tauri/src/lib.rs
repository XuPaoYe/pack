use serde::{Deserialize, Serialize};
use serde_json::Value;
use base64::Engine;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{LogicalSize, Manager};

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
  plan: Option<String>,
  account_id: Option<String>,
  user_id: Option<String>,
  source: String,
  token_meta: TokenMeta,
  status: Option<AccountStatus>,
  quota: Option<AccountQuota>,
  created_at: i64,
  updated_at: i64,
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
  let decoded = base64::engine::general_purpose::STANDARD.decode(padded).ok()?;
  serde_json::from_slice::<Value>(&decoded).ok()
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
      raw
        .and_then(|r| r.get("totalPercentUsed"))
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

fn derive_status(obj: &serde_json::Map<String, Value>, token_meta: &TokenMeta, quota: Option<&AccountQuota>) -> AccountStatus {
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

  if token_meta.expires_at.is_some_and(|expires_at| expires_at <= now) {
    return AccountStatus {
      state: "unavailable".to_string(),
      label: "已过期".to_string(),
      reason: Some("本地 token 已过期".to_string()),
      updated_at: None,
    };
  }

  if quota
    .map(|q| q.metrics.iter().any(|metric| metric.remaining_percent == Some(0)))
    .unwrap_or(false)
  {
    return AccountStatus {
      state: "unavailable".to_string(),
      label: "额度耗尽".to_string(),
      reason: Some("至少一个额度窗口剩余 0%".to_string()),
      updated_at: quota.and_then(|q| q.last_updated),
    };
  }

  if let Some(error) = quota.and_then(|q| q.error.clone()) {
    return AccountStatus {
      state: "warning".to_string(),
      label: "查询失败".to_string(),
      reason: Some(error),
      updated_at: quota.and_then(|q| q.last_updated),
    };
  }

  if token_meta.expires_at.is_some_and(|expires_at| expires_at - now < 3600) || !token_meta.has_refresh_token {
    return AccountStatus {
      state: "warning".to_string(),
      label: "需关注".to_string(),
      reason: if token_meta.expires_at.is_some_and(|expires_at| expires_at - now < 3600) {
        Some("token 即将过期".to_string())
      } else {
        Some("缺少 refresh token".to_string())
      },
      updated_at: None,
    };
  }

  if let Some(status) = raw_status {
    if !matches!(status.as_str(), "active" | "available" | "ok") {
      return AccountStatus {
        state: "warning".to_string(),
        label: status,
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

  let id_token = string_field(tokens.and_then(|t| t.get("id_token")).or_else(|| obj.get("id_token")));
  let access_token =
    string_field(tokens.and_then(|t| t.get("access_token")).or_else(|| obj.get("access_token")));
  let refresh_token = string_field(
    tokens
      .and_then(|t| t.get("refresh_token"))
      .or_else(|| obj.get("refresh_token")),
  );
  let api_key = string_field(obj.get("OPENAI_API_KEY"));
  let auth_mode = string_field(obj.get("auth_mode")).unwrap_or_default().to_lowercase();

  if id_token.is_none() && access_token.is_none() && api_key.is_none() && auth_mode != "apikey" {
    return None;
  }

  let jwt = id_token.as_deref().and_then(parse_jwt_payload);
  let profile = jwt
    .as_ref()
    .and_then(|j| j.get("https://api.openai.com/profile"))
    .and_then(Value::as_object);
  let auth = jwt
    .as_ref()
    .and_then(|j| j.get("https://api.openai.com/auth"))
    .and_then(Value::as_object);

  let email = string_field(obj.get("email"))
    .or_else(|| profile.and_then(|p| string_field(p.get("email"))))
    .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("email"))))
    .or_else(|| {
      api_key
        .as_ref()
        .map(|k| format!("api-key-{}@local", &stable_hash(k)[..6]))
    })?;

  let account_id = string_field(tokens.and_then(|t| t.get("account_id")).or_else(|| obj.get("account_id")))
    .or_else(|| auth.and_then(|a| string_field(a.get("chatgpt_account_id"))));
  let user_id = string_field(obj.get("user_id"))
    .or_else(|| auth.and_then(|a| string_field(a.get("chatgpt_user_id"))))
    .or_else(|| jwt.as_ref().and_then(|j| string_field(j.get("sub"))));
  let plan = string_field(obj.get("plan_type"))
    .or_else(|| auth.and_then(|a| string_field(a.get("chatgpt_plan_type"))))
    .or_else(|| api_key.as_ref().map(|_| "API Key".to_string()));
  let discriminator = account_id
    .clone()
    .or(user_id.clone())
    .or(access_token.clone())
    .or(api_key.clone())
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
      .unwrap_or_else(|| format!("codex_{}", stable_hash(&format!("{}::{discriminator}", email.to_lowercase())))),
    provider: "codex".to_string(),
    email: email.to_lowercase(),
    display_name: string_field(obj.get("account_name")).or_else(|| string_field(obj.get("name"))),
    plan,
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
  })
}

fn parse_gemini_account(value: &Value, source: &str) -> Option<ManagedAccount> {
  let obj = value.as_object()?;
  let token = obj.get("token").and_then(Value::as_object);

  let access_token =
    string_field(obj.get("access_token")).or_else(|| token.and_then(|t| string_field(t.get("access_token"))));
  let refresh_token = string_field(obj.get("refresh_token"))
    .or_else(|| token.and_then(|t| string_field(t.get("refresh_token"))));
  let id_token = string_field(obj.get("id_token")).or_else(|| token.and_then(|t| string_field(t.get("id_token"))));

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
    id: string_field(obj.get("id"))
      .unwrap_or_else(|| format!("gemini_{}", stable_hash(&format!("{}::{}", email.to_lowercase(), auth_id.clone().unwrap_or_else(|| access_token.clone().unwrap_or(email.clone())))))),
    provider: "gemini".to_string(),
    email: email.to_lowercase(),
    display_name: string_field(obj.get("name")),
    plan: string_field(obj.get("plan_name")).or_else(|| string_field(obj.get("tier_name"))),
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
  })
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
    if let Some(account) = parse_codex_account(item, source).or_else(|| parse_gemini_account(item, source)) {
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

fn launch_oauth_command(command_text: &str) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  {
    let script = format!("tell application \"Terminal\" to do script \"{command_text}\"");
    Command::new("osascript")
      .arg("-e")
      .arg(script)
      .spawn()
      .map_err(|e| format!("启动终端失败: {e}"))?;
    return Ok(());
  }

  #[cfg(target_os = "windows")]
  {
    Command::new("cmd")
      .args(["/C", "start", "cmd", "/K", command_text])
      .spawn()
      .map_err(|e| format!("启动终端失败: {e}"))?;
    return Ok(());
  }

  #[cfg(target_os = "linux")]
  {
    let candidates = [
      ("x-terminal-emulator", vec!["-e", "sh", "-lc", command_text]),
      ("gnome-terminal", vec!["--", "sh", "-lc", command_text]),
      ("konsole", vec!["-e", "sh", "-lc", command_text]),
    ];
    for (bin, args) in candidates {
      if Command::new(bin).args(args).spawn().is_ok() {
        return Ok(());
      }
    }
    return Err("未找到可用终端，请手动执行登录命令".to_string());
  }

  #[allow(unreachable_code)]
  Err("当前系统暂不支持自动启动 OAuth 终端".to_string())
}

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
  window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn import_accounts_from_json(json_content: String, label: Option<String>) -> Result<ImportResult, String> {
  Ok(parse_auth_json_content(
    &json_content,
    "paste",
    label.as_deref().unwrap_or("JSON"),
  ))
}

#[tauri::command]
fn import_codex_from_local() -> Result<ImportResult, String> {
  let path = home_dir()?.join(".codex").join("auth.json");
  if !path.exists() {
    return Err(format!("未找到本机 Codex 账号文件: {}", path.display()));
  }
  let content = read_to_string(&path)?;
  Ok(parse_auth_json_content(&content, "local", "Codex 本机账号"))
}

#[tauri::command]
fn import_gemini_from_local() -> Result<ImportResult, String> {
  let gemini_dir = home_dir()?.join(".gemini");
  let oauth_path = gemini_dir.join("oauth_creds.json");
  let google_accounts_path = gemini_dir.join("google_accounts.json");
  let settings_path = gemini_dir.join("settings.json");

  if !oauth_path.exists() {
    return Err(format!(
      "未找到本机 Gemini 账号文件: {}",
      oauth_path.display()
    ));
  }

  let oauth_content = read_to_string(&oauth_path)?;
  let mut oauth_value: Value =
    serde_json::from_str(&oauth_content).map_err(|e| format!("解析 oauth_creds.json 失败: {e}"))?;

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

  Ok(parse_auth_json_content(
    &oauth_value.to_string(),
    "local",
    "Gemini 本机账号",
  ))
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
    message: "已打开终端执行 `codex login`，完成后回到本应用点击“完成导入”。".to_string(),
  })
}

#[tauri::command]
fn complete_codex_oauth(login_id: String) -> Result<ImportResult, String> {
  let pending = OAUTH_PENDING
    .lock()
    .map_err(|_| "OAuth 状态锁失败".to_string())?
    .remove(&login_id);
  if pending.as_ref().map(|p| p.provider.as_str()) != Some("codex") {
    return Err("无效或已过期的 Codex OAuth 会话".to_string());
  }
  import_codex_from_local()
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
    message: "已打开终端执行 `gemini auth login`，完成后回到本应用点击“完成导入”。".to_string(),
  })
}

#[tauri::command]
fn complete_gemini_oauth(login_id: String) -> Result<ImportResult, String> {
  let pending = OAUTH_PENDING
    .lock()
    .map_err(|_| "OAuth 状态锁失败".to_string())?
    .remove(&login_id);
  if pending.as_ref().map(|p| p.provider.as_str()) != Some("gemini") {
    return Err("无效或已过期的 Gemini OAuth 会话".to_string());
  }
  import_gemini_from_local()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![
      start_window_drag,
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
        let app_size = LogicalSize::new(1180.0, 760.0);
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
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
