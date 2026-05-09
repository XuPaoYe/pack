import type { AccountState, ManagedAccount } from "./authParser";
import { formatDateTime, normalizeUnixSeconds, nowUnixSeconds } from "./time";

export type PlanBadge = {
  label: string;
  tone: "plus" | "team" | "enterprise" | "pro" | "ultra" | "free" | "unknown";
};

export type ValidityText = {
  label: string;
  detail: string;
  title?: string;
  expired?: boolean;
};

function normalizePlanKey(value?: string) {
  return (value || "").trim().toLowerCase();
}

function resolveCodexPlanBadge(account: ManagedAccount): PlanBadge {
  const raw = normalizePlanKey(account.planType || account.plan);
  const authFile = normalizePlanKey(account.authFilePlanType);
  if (!raw) {
    return { label: account.plan || "未知", tone: "unknown" };
  }
  if (raw.includes("enterprise")) return { label: "Enterprise", tone: "enterprise" };
  if (raw.includes("team") || raw.includes("business") || raw.includes("edu")) return { label: "Team", tone: "team" };
  if (raw.includes("plus")) return { label: "Plus", tone: "plus" };
  if (raw.includes("pro")) {
    if (authFile.includes("5x") || authFile.includes("prolite") || authFile.includes("pro-lite") || authFile.includes("pro-5x")) {
      return { label: "Pro 5x", tone: "pro" };
    }
    if (authFile.includes("20x") || authFile.includes("promax") || authFile.includes("pro-max") || authFile.includes("pro-20x")) {
      return { label: "Pro 20x", tone: "pro" };
    }
    return { label: "Pro 20x", tone: "pro" };
  }
  if (raw.includes("free")) return { label: "Free", tone: "free" };
  return { label: account.plan || raw, tone: "unknown" };
}

function resolveGeminiPlanBadge(account: ManagedAccount): PlanBadge {
  const raw = normalizePlanKey(account.plan || account.planType);
  if (!raw) return { label: "未知", tone: "unknown" };
  if (raw.includes("ultra")) return { label: "ULTRA", tone: "ultra" };
  if (raw === "standard-tier") return { label: "FREE", tone: "free" };
  if (raw.includes("pro") || raw.includes("premium")) return { label: "PRO", tone: "pro" };
  if (raw === "free-tier" || raw.includes("free")) return { label: "FREE", tone: "free" };
  return { label: account.plan || account.planType || "未知", tone: "unknown" };
}

function resolveWindsurfPlanBadge(account: ManagedAccount): PlanBadge {
  const raw = normalizePlanKey(account.plan || account.planType);
  if (!raw) return { label: "SuperAl", tone: "unknown" };
  if (raw.includes("enterprise")) return { label: "Enterprise", tone: "enterprise" };
  if (raw.includes("team")) return { label: "Team", tone: "team" };
  if (raw.includes("pro")) return { label: "Pro", tone: "pro" };
  if (raw.includes("free")) return { label: "Free", tone: "free" };
  return { label: account.plan || "SuperAl", tone: "unknown" };
}

export function resolvePlanBadge(account: ManagedAccount): PlanBadge {
  if (account.provider === "gemini") return resolveGeminiPlanBadge(account);
  if (account.provider === "windsurf") return resolveWindsurfPlanBadge(account);
  return resolveCodexPlanBadge(account);
}

export function resolveValidityUntil(account: ManagedAccount) {
  if (account.provider === "gemini") return undefined;
  return normalizeUnixSeconds(account.subscriptionActiveUntil);
}

export function formatValidityText(account: ManagedAccount): ValidityText {
  if ((account.status ?? fallbackStatus(account)).state === "unavailable") {
    return { label: "有效期", detail: "--", title: account.status?.reason };
  }
  const until = resolveValidityUntil(account);
  if (until === undefined || until <= 0) return { label: "有效期", detail: "--" };
  const remaining = until - nowUnixSeconds();
  if (remaining <= 0) {
    return { label: "有效期", detail: "已过期", expired: true, title: formatDateTime(until) };
  }
  const days = Math.ceil(remaining / 86400);
  const hours = Math.ceil(remaining / 3600);
  return {
    label: "有效期",
    detail: days >= 1 ? `${days}天` : `${hours}小时`,
    title: formatDateTime(until),
  };
}

export function fallbackStatus(account: ManagedAccount): { state: AccountState; label: string; reason?: string } {
  const expiresAt = normalizeUnixSeconds(account.tokenMeta.expiresAt);
  if (!account.tokenMeta.hasAccessToken) return { state: "unavailable", label: "不可用", reason: "缺少 access token" };
  if (expiresAt && expiresAt <= nowUnixSeconds() && !account.tokenMeta.hasRefreshToken) {
    return { state: "unavailable", label: "不可用", reason: "本地 token 已过期" };
  }
  return { state: "available", label: "可用" };
}
