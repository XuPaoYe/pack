export type Provider = "codex" | "gemini";

export type ImportSource = "paste" | "file" | "local" | "oauth";

export type ManagedAccount = {
  id: string;
  provider: Provider;
  email: string;
  displayName?: string;
  plan?: string;
  accountId?: string;
  userId?: string;
  source: ImportSource;
  tokenMeta: {
    hasAccessToken: boolean;
    hasRefreshToken: boolean;
    hasIdToken: boolean;
    expiresAt?: number;
  };
  createdAt: number;
  updatedAt: number;
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
  const discriminator = accountId ?? userId ?? accessToken ?? apiKey ?? email;
  const now = Math.floor(Date.now() / 1000);

  return {
    id: stringField(value.id) ?? accountIdFor("codex", email, discriminator),
    provider: "codex",
    email: email.toLowerCase(),
    displayName: stringField(value.account_name) ?? stringField(value.name),
    plan,
    accountId,
    userId,
    source,
    tokenMeta: {
      hasAccessToken: Boolean(accessToken || apiKey),
      hasRefreshToken: Boolean(refreshToken),
      hasIdToken: Boolean(idToken),
      expiresAt: numberField(jwt?.exp),
    },
    createdAt: numberField(value.created_at) ?? now,
    updatedAt: now,
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
  const expiresAt =
    numberField(value.expiry_date) ??
    numberField(value.expiryDate) ??
    numberField(token?.expires_at) ??
    numberField(token?.expiresAt) ??
    numberField(jwt?.exp);
  const now = Math.floor(Date.now() / 1000);

  return {
    id: stringField(value.id) ?? accountIdFor("gemini", email, authId ?? accessToken ?? email),
    provider: "gemini",
    email: email.toLowerCase(),
    displayName: stringField(value.name),
    plan: stringField(value.plan_name) ?? stringField(value.planName) ?? stringField(value.tier_name),
    accountId: authId,
    userId: authId,
    source,
    tokenMeta: {
      hasAccessToken: Boolean(accessToken),
      hasRefreshToken: Boolean(refreshToken),
      hasIdToken: Boolean(idToken),
      expiresAt,
    },
    createdAt: numberField(value.created_at) ?? now,
    updatedAt: now,
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
    const account = parseCodex(item, source) ?? parseGemini(item, source);
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
