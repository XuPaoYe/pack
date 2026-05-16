import { normalizeUnixSeconds, nowUnixSeconds } from "./time";

// 后端 IPC 出口已经把内部协议名改写成 "superai"，前端只需要跟 "superai" 比较。
// aud 校验路径仍需要上游协议字面量，所以单独从 charcode 构造，绕过 esbuild 常量折叠。
const PROVIDER_SUPERAI = "superai" as const;
const __SUPERAI_AUD_PROTOCOL = [119, 105, 110, 100, 115, 117, 114, 102]
  .map((c) => String.fromCharCode(c))
  .join("");
const EXAFUNCTION_SUPERAI_AUD = "exafunction-" + __SUPERAI_AUD_PROTOCOL;

export type Provider = "codex" | "gemini" | "superai";

export type ImportSource = "paste" | "file" | "local" | "oauth";

export type ManagedAccount = {
  id: string;
  provider: Provider;
  email: string;
  displayName?: string;
  accountName?: string;
  organizationId?: string;
  plan?: string;
  planType?: string;
  authFilePlanType?: string;
  subscriptionActiveUntil?: number | string;
  accountId?: string;
  userId?: string;
  source: ImportSource;
  tokenMeta: {
    hasAccessToken: boolean;
    hasRefreshToken: boolean;
    hasIdToken: boolean;
    expiresAt?: number;
  };
  status?: AccountStatus;
  quota?: AccountQuota;
  createdAt: number;
  updatedAt: number;
  authPayload?: unknown;
};

export type AccountState = "available" | "warning" | "unavailable" | "unknown";

export type AccountStatus = {
  state: AccountState;
  label: string;
  reason?: string;
  updatedAt?: number;
};

export type QuotaMetric = {
  key: string;
  label: string;
  remainingPercent?: number;
  resetAt?: number | string;
  detail?: string;
  state?: AccountState;
};

export type AccountQuota = {
  metrics: QuotaMetric[];
  lastUpdated?: number;
  error?: string;
  isForbidden?: boolean;
};

export type ImportFailure = {
  label: string;
  reason: string;
};

export type ImportResult = {
  imported: ManagedAccount[];
  failed: ImportFailure[];
};

type JsonObject = Record<string, unknown>;

const textEncoder = new TextEncoder();

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringField(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function numberField(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function timestampField(value: unknown): number | undefined {
  return typeof value === "number" || typeof value === "string" ? normalizeUnixSeconds(value) : undefined;
}

function boolField(value: unknown): boolean | undefined {
  if (typeof value === "boolean") return value;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (["true", "1", "yes"].includes(normalized)) return true;
    if (["false", "0", "no"].includes(normalized)) return false;
  }
  return undefined;
}

function percentField(value: unknown): number | undefined {
  const parsed = numberField(value);
  if (parsed === undefined) return undefined;
  return Math.max(0, Math.min(100, Math.round(parsed)));
}

function decodeBase64Url(input: string): string | null {
  try {
    const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
    return decodeURIComponent(
      atob(padded)
        .split("")
        .map((ch) => `%${ch.charCodeAt(0).toString(16).padStart(2, "0")}`)
        .join(""),
    );
  } catch {
    return null;
  }
}

function parseJwtPayload(token?: string): JsonObject | null {
  if (!token) return null;
  const [, payload] = token.split(".");
  if (!payload) return null;
  const decoded = decodeBase64Url(payload);
  if (!decoded) return null;
  try {
    const parsed = JSON.parse(decoded);
    return isObject(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function stableHash(value: string): string {
  let hash = 2166136261;
  for (const byte of textEncoder.encode(value)) {
    hash ^= byte;
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function accountIdFor(provider: Provider, email: string, discriminator: string): string {
  return `${provider}_${stableHash(`${email.toLowerCase()}::${discriminator}`)}`;
}

function quotaState(remainingPercent?: number): AccountState {
  if (remainingPercent === undefined) return "unknown";
  if (remainingPercent <= 0) return "unavailable";
  if (remainingPercent <= 15) return "warning";
  return "available";
}

function parseCodexQuota(value: JsonObject): AccountQuota | undefined {
  const quota = isObject(value.quota) ? value.quota : undefined;
  const quotaError = isObject(value.quota_error) ? value.quota_error : undefined;
  const metrics: QuotaMetric[] = [];
  const hourly = percentField(quota?.hourly_percentage ?? value.hourly_percentage);
  const weekly = percentField(quota?.weekly_percentage ?? value.weekly_percentage);

  if (hourly !== undefined) {
    metrics.push({
      key: "codex-5h",
      label: "5H",
      remainingPercent: hourly,
      resetAt: numberField(quota?.hourly_reset_time ?? value.hourly_reset_time),
      state: quotaState(hourly),
    });
  }

  if (weekly !== undefined) {
    metrics.push({
      key: "codex-weekly",
      label: "周限",
      remainingPercent: weekly,
      resetAt: numberField(quota?.weekly_reset_time ?? value.weekly_reset_time),
      state: quotaState(weekly),
    });
  }

  const error = stringField(quotaError?.message) ?? stringField(value.quota_query_last_error);
  const isForbidden = boolField(quota?.is_forbidden ?? value.is_forbidden) ?? false;
  if (!metrics.length && !error && !isForbidden) return undefined;

  return {
    metrics,
    lastUpdated: numberField(value.usage_updated_at ?? quota?.last_updated),
    error,
    isForbidden,
  };
}

function parseGeminiQuota(value: JsonObject): AccountQuota | undefined {
  const raw = isObject(value.gemini_usage_raw) ? value.gemini_usage_raw : undefined;
  const models = Array.isArray(raw?.models) ? raw.models : Array.isArray(value.models) ? value.models : [];
  const metrics: QuotaMetric[] = [];

  for (const item of models) {
    if (!isObject(item)) continue;
    const remainingPercent = percentField(item.percentage ?? item.remainingPercent ?? item.remaining_percent);
    const name = stringField(item.display_name) ?? stringField(item.displayName) ?? stringField(item.name);
    if (!name && remainingPercent === undefined) continue;
    metrics.push({
      key: `gemini-${metrics.length}`,
      label: name ?? `MODEL ${metrics.length + 1}`,
      remainingPercent,
      resetAt: stringField(item.reset_time) ?? stringField(item.resetTime),
      state: quotaState(remainingPercent),
    });
  }

  const totalPercentUsed = percentField(raw?.totalPercentUsed ?? raw?.total_percent_used ?? value.totalPercentUsed);
  if (!metrics.length && totalPercentUsed !== undefined) {
    const remainingPercent = 100 - totalPercentUsed;
    metrics.push({
      key: "gemini-total",
      label: "TOTAL",
      remainingPercent,
      state: quotaState(remainingPercent),
    });
  }

  const error = stringField(value.quota_query_last_error);
  if (!metrics.length && !error) return undefined;

  return {
    metrics,
    lastUpdated: numberField(value.usage_updated_at),
    error,
  };
}

function deriveStatus(value: JsonObject, tokenMeta: ManagedAccount["tokenMeta"], quota?: AccountQuota): AccountStatus {
  const now = nowUnixSeconds();
  const rawStatus = stringField(value.status)?.toLowerCase();
  const statusReason =
    stringField(value.status_reason) ??
    stringField(value.reauth_reason) ??
    quota?.error;

  if (boolField(value.requires_reauth) || rawStatus === "unavailable" || rawStatus === "disabled" || quota?.isForbidden) {
    return {
      state: "unavailable",
      label: "不可用",
      reason: statusReason ?? "账号需要重新授权或访问被拒绝",
      updatedAt: numberField(value.usage_updated_at),
    };
  }

  if (!tokenMeta.hasAccessToken) {
    return { state: "unavailable", label: "不可用", reason: "缺少 access token" };
  }

  const expiresAt = normalizeUnixSeconds(tokenMeta.expiresAt);
  if (expiresAt && expiresAt <= now && !tokenMeta.hasRefreshToken) {
    return { state: "unavailable", label: "不可用", reason: "本地 token 已过期" };
  }

  if (quota?.error) {
    return { state: "unavailable", label: "不可用", reason: quota.error, updatedAt: quota.lastUpdated };
  }

  if (rawStatus && !["active", "available", "ok"].includes(rawStatus)) {
    return { state: "unavailable", label: "不可用", reason: statusReason ?? rawStatus };
  }

  return { state: "available", label: "可用", updatedAt: quota?.lastUpdated };
}

function asItems(parsed: unknown): unknown[] {
  if (Array.isArray(parsed)) return parsed;
  if (isObject(parsed) && Array.isArray(parsed.accounts)) return parsed.accounts;
  return [parsed];
}

function parseCodex(value: unknown, source: ImportSource): ManagedAccount | null {
  if (!isObject(value)) return null;

  const authFileTokens = isObject(value.tokens) ? value.tokens : undefined;
  const accountTokens = isObject(value.tokens) ? value.tokens : undefined;
  const idToken =
    stringField(accountTokens?.id_token) ??
    stringField(accountTokens?.idToken) ??
    stringField(value.id_token) ??
    stringField(value.idToken);
  const accessToken =
    stringField(accountTokens?.access_token) ??
    stringField(accountTokens?.accessToken) ??
    stringField(value.access_token) ??
    stringField(value.accessToken);
  const refreshToken =
    stringField(accountTokens?.refresh_token) ??
    stringField(accountTokens?.refreshToken) ??
    stringField(value.refresh_token) ??
    stringField(value.refreshToken);
  const authMode = stringField(value.auth_mode)?.toLowerCase();
  const apiKey = stringField(value.OPENAI_API_KEY);

  if (!idToken && !accessToken && !apiKey && authMode !== "apikey") return null;

  const jwt = parseJwtPayload(idToken);
  const openaiAuth = isObject(jwt?.["https://api.openai.com/auth"])
    ? (jwt["https://api.openai.com/auth"] as JsonObject)
    : undefined;
  const profile = isObject(jwt?.["https://api.openai.com/profile"])
    ? (jwt["https://api.openai.com/profile"] as JsonObject)
    : undefined;

  const email =
    stringField(value.email) ??
    stringField(profile?.email) ??
    stringField(jwt?.email) ??
    (apiKey ? `api-key-${stableHash(apiKey).slice(0, 6)}@local` : undefined);
  if (!email) return null;

  const accountId =
    stringField(value.account_id) ??
    stringField(value.accountId) ??
    stringField(authFileTokens?.account_id) ??
    stringField(openaiAuth?.chatgpt_account_id) ??
    stringField(openaiAuth?.account_id);
  const userId =
    stringField(value.user_id) ??
    stringField(value.userId) ??
    stringField(openaiAuth?.chatgpt_user_id) ??
    stringField(openaiAuth?.user_id) ??
    stringField(jwt?.sub);
  const plan =
    stringField(value.plan_type) ??
    stringField(value.planType) ??
    stringField(value.auth_file_plan_type) ??
    stringField(openaiAuth?.chatgpt_plan_type) ??
    (apiKey ? "API Key" : undefined);
  const planType =
    stringField(value.plan_type) ??
    stringField(value.planType) ??
    stringField(openaiAuth?.chatgpt_plan_type);
  const authFilePlanType =
    stringField(value.auth_file_plan_type) ??
    stringField(value.authFilePlanType);
  const subscriptionActiveUntil =
    timestampField(value.subscription_active_until) ??
    timestampField(value.subscriptionActiveUntil) ??
    timestampField(value.subscription_until);
  const accountName = stringField(value.account_name) ?? stringField(value.accountName) ?? stringField(value.name);
  const organizationId = stringField(value.organization_id) ?? stringField(value.organizationId);
  const discriminator = accountId ?? userId ?? accessToken ?? apiKey ?? email;
  const now = nowUnixSeconds();
  const tokenMeta = {
    hasAccessToken: Boolean(accessToken || apiKey),
    hasRefreshToken: Boolean(refreshToken),
    hasIdToken: Boolean(idToken),
    expiresAt: numberField(jwt?.exp),
  };
  const quota = parseCodexQuota(value);

  return {
    id: stringField(value.id) ?? accountIdFor("codex", email, discriminator),
    provider: "codex",
    email: email.toLowerCase(),
    displayName: accountName,
    accountName,
    organizationId,
    plan,
    planType,
    authFilePlanType,
    subscriptionActiveUntil,
    accountId,
    userId,
    source,
    tokenMeta,
    status: deriveStatus(value, tokenMeta, quota),
    quota,
    createdAt: numberField(value.created_at) ?? now,
    updatedAt: numberField(value.last_used) ?? numberField(value.updated_at) ?? now,
  };
}

function looksLikeSuperAI(value: JsonObject): boolean {
  const explicit = stringField(value.provider)?.toLowerCase();
  if (explicit === PROVIDER_SUPERAI) return true;
  const tokens = isObject(value.tokens) ? value.tokens : undefined;
  const auth1Token =
    stringField(value.auth1_token) ??
    stringField(value.auth1Token) ??
    stringField(value.devin_auth1_token) ??
    stringField(value.devinAuth1Token) ??
    stringField(tokens?.auth1_token) ??
    stringField(tokens?.auth1Token) ??
    stringField(tokens?.devin_auth1_token) ??
    stringField(tokens?.devinAuth1Token);
  const sessionToken =
    stringField(value.session_token) ??
    stringField(value.sessionToken) ??
    stringField(value.devin_session_token) ??
    stringField(value.devinSessionToken) ??
    stringField(tokens?.session_token) ??
    stringField(tokens?.sessionToken) ??
    stringField(tokens?.devin_session_token) ??
    stringField(tokens?.devinSessionToken);
  if (auth1Token?.startsWith("auth1_") || sessionToken?.startsWith("devin-session-token$")) {
    return true;
  }
  if (
    stringField(value.local_id) ??
    stringField(value.localId) ??
    stringField(tokens?.local_id) ??
    stringField(tokens?.localId)
  ) {
    return true;
  }
  const idToken =
    stringField(value.id_token) ??
    stringField(value.idToken) ??
    stringField(tokens?.id_token) ??
    stringField(tokens?.idToken);
  const refreshToken =
    stringField(value.refresh_token) ??
    stringField(value.refreshToken) ??
    stringField(tokens?.refresh_token) ??
    stringField(tokens?.refreshToken);
  const jwt = parseJwtPayload(idToken);
  if (jwt) {
    const aud = stringField(jwt.aud) ?? "";
    const iss = stringField(jwt.iss) ?? "";
    if (aud.includes(EXAFUNCTION_SUPERAI_AUD) || iss.includes(EXAFUNCTION_SUPERAI_AUD)) return true;
    const firebase = isObject(jwt.firebase) ? jwt.firebase : undefined;
    const signInProvider = stringField(firebase?.sign_in_provider);
    if (signInProvider === "password" && refreshToken) return true;
  }
  return false;
}

function parseSuperAI(value: unknown, source: ImportSource): ManagedAccount | null {
  if (!isObject(value)) return null;
  if (!looksLikeSuperAI(value)) return null;

  const tokens = isObject(value.tokens) ? value.tokens : undefined;
  const idToken =
    stringField(value.id_token) ??
    stringField(value.idToken) ??
    stringField(tokens?.id_token) ??
    stringField(tokens?.idToken);
  const refreshToken =
    stringField(value.refresh_token) ??
    stringField(value.refreshToken) ??
    stringField(tokens?.refresh_token) ??
    stringField(tokens?.refreshToken);
  const accessToken =
    stringField(value.access_token) ??
    stringField(value.accessToken) ??
    stringField(tokens?.access_token) ??
    stringField(tokens?.accessToken) ??
    idToken;
  const apiKey =
    stringField(value.api_key) ??
    stringField(value.apiKey) ??
    stringField(tokens?.api_key) ??
    stringField(tokens?.apiKey);
  const auth1Token =
    stringField(value.auth1_token) ??
    stringField(value.auth1Token) ??
    stringField(value.devin_auth1_token) ??
    stringField(value.devinAuth1Token) ??
    stringField(tokens?.auth1_token) ??
    stringField(tokens?.auth1Token) ??
    stringField(tokens?.devin_auth1_token) ??
    stringField(tokens?.devinAuth1Token);
  const sessionToken =
    stringField(value.session_token) ??
    stringField(value.sessionToken) ??
    stringField(value.devin_session_token) ??
    stringField(value.devinSessionToken) ??
    stringField(tokens?.session_token) ??
    stringField(tokens?.sessionToken) ??
    stringField(tokens?.devin_session_token) ??
    stringField(tokens?.devinSessionToken);

  if (!idToken && !refreshToken && !apiKey && !auth1Token && !sessionToken) return null;

  const jwt = parseJwtPayload(idToken);
  const discriminatorToken = apiKey ?? sessionToken ?? auth1Token ?? accessToken ?? refreshToken ?? idToken ?? PROVIDER_SUPERAI;
  const email =
    stringField(value.email) ??
    stringField(value.account) ??
    stringField(value.active) ??
    stringField(jwt?.email) ??
    `${PROVIDER_SUPERAI}-${stableHash(discriminatorToken).slice(0, 8)}@local`;

  const localId =
    stringField(value.local_id) ??
    stringField(value.localId) ??
    stringField(tokens?.local_id) ??
    stringField(tokens?.localId) ??
    stringField(jwt?.user_id) ??
    stringField(jwt?.sub);
  const displayName =
    stringField(value.display_name) ??
    stringField(value.displayName) ??
    stringField(value.name) ??
    stringField(jwt?.name);
  const expiresAt =
    numberField(value.expires_at) ??
    numberField(value.expiresAt) ??
    numberField(tokens?.expires_at) ??
    numberField(tokens?.expiresAt) ??
    numberField(jwt?.exp);

  const now = nowUnixSeconds();
  const tokenMeta = {
    hasAccessToken: Boolean(accessToken || apiKey || sessionToken || auth1Token),
    hasRefreshToken: Boolean(refreshToken),
    hasIdToken: Boolean(idToken),
    expiresAt,
  };
  const discriminator = localId ?? apiKey ?? sessionToken ?? auth1Token ?? email;
  const plan =
    stringField(value.plan) ??
    stringField(value.plan_type) ??
    stringField(value.planType);

  return {
    id: stringField(value.id) ?? accountIdFor(PROVIDER_SUPERAI, email, discriminator),
    provider: PROVIDER_SUPERAI,
    email: email.toLowerCase(),
    displayName,
    plan,
    planType: plan,
    subscriptionActiveUntil: expiresAt,
    accountId: localId,
    userId: localId,
    source,
    tokenMeta,
    status: deriveStatus(value, tokenMeta),
    createdAt: numberField(value.created_at) ?? now,
    updatedAt: numberField(value.last_used) ?? numberField(value.updated_at) ?? now,
    authPayload: {
      provider: PROVIDER_SUPERAI,
      email: email.toLowerCase(),
      display_name: displayName,
      tokens: {
        id_token: idToken,
        refresh_token: refreshToken,
        access_token: accessToken,
        api_key: apiKey,
        auth1_token: auth1Token,
        session_token: sessionToken,
        local_id: localId,
        expires_at: expiresAt,
      },
    },
  };
}

function parseGemini(value: unknown, source: ImportSource): ManagedAccount | null {
  if (!isObject(value)) return null;

  const token = isObject(value.token) ? value.token : undefined;
  const accessToken =
    stringField(value.access_token) ??
    stringField(value.accessToken) ??
    stringField(token?.access_token) ??
    stringField(token?.accessToken);
  const refreshToken =
    stringField(value.refresh_token) ??
    stringField(value.refreshToken) ??
    stringField(token?.refresh_token) ??
    stringField(token?.refreshToken);
  const idToken =
    stringField(value.id_token) ??
    stringField(value.idToken) ??
    stringField(token?.id_token) ??
    stringField(token?.idToken);

  if (!accessToken && !refreshToken && !idToken) return null;

  const jwt = parseJwtPayload(idToken);
  const email =
    stringField(value.email) ??
    stringField(value.active) ??
    stringField(jwt?.email) ??
    stringField(value.account);
  if (!email) return null;

  const authId = stringField(value.auth_id) ?? stringField(value.authId) ?? stringField(jwt?.sub);
  const planType = stringField(value.selected_auth_type) ?? stringField(value.selectedAuthType);
  const expiresAt =
    numberField(value.expiry_date) ??
    numberField(value.expiryDate) ??
    numberField(token?.expires_at) ??
    numberField(token?.expiresAt) ??
    numberField(jwt?.exp);
  const now = nowUnixSeconds();
  const tokenMeta = {
    hasAccessToken: Boolean(accessToken),
    hasRefreshToken: Boolean(refreshToken),
    hasIdToken: Boolean(idToken),
    expiresAt,
  };
  const quota = parseGeminiQuota(value);

  return {
    id: stringField(value.id) ?? accountIdFor("gemini", email, authId ?? accessToken ?? email),
    provider: "gemini",
    email: email.toLowerCase(),
    displayName: stringField(value.name),
    plan: stringField(value.plan_name) ?? stringField(value.planName) ?? stringField(value.tier_name),
    planType,
    subscriptionActiveUntil: expiresAt,
    accountId: authId,
    userId: authId,
    source,
    tokenMeta,
    status: deriveStatus(value, tokenMeta, quota),
    quota,
    createdAt: numberField(value.created_at) ?? now,
    updatedAt: numberField(value.last_used) ?? numberField(value.updated_at) ?? now,
  };
}

export function parseAuthJson(content: string, source: ImportSource, label = "JSON"): ImportResult {
  let parsed: unknown;
  try {
    parsed = JSON.parse(content);
  } catch {
    return { imported: [], failed: [{ label, reason: "JSON 格式无效" }] };
  }

  const imported: ManagedAccount[] = [];
  const failed: ImportFailure[] = [];
  const items = asItems(parsed);

  items.forEach((item, index) => {
    const itemLabel = `${label}${items.length > 1 ? ` #${index + 1}` : ""}`;
    const account = parseCodex(item, source) ?? parseSuperAI(item, source) ?? parseGemini(item, source);
    if (account) {
      imported.push(account);
    } else {
      failed.push({ label: itemLabel, reason: "未识别到 Codex 或 Gemini 凭证字段" });
    }
  });

  const deduped = new Map<string, ManagedAccount>();
  for (const account of imported) deduped.set(account.id, account);
  return { imported: [...deduped.values()], failed };
}
