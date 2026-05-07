export function nowUnixSeconds() {
  return Math.floor(Date.now() / 1000);
}

export function normalizeUnixSeconds(value?: number | string | null) {
  if (value === undefined || value === null) return undefined;
  if (typeof value === "number") {
    if (!Number.isFinite(value) || value <= 0) return undefined;
    return value > 1e12 ? Math.floor(value / 1000) : Math.floor(value);
  }

  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const numeric = Number(trimmed);
  if (Number.isFinite(numeric) && /^\d+(\.\d+)?$/.test(trimmed)) {
    return numeric > 1e12 ? Math.floor(numeric / 1000) : Math.floor(numeric);
  }

  const parsed = Date.parse(trimmed);
  if (Number.isNaN(parsed)) return undefined;
  return Math.floor(parsed / 1000);
}

export function formatDateTime(timestamp?: number | string | null) {
  const normalized = normalizeUnixSeconds(timestamp);
  if (normalized === undefined) return undefined;
  const date = new Date(normalized * 1000);
  if (Number.isNaN(date.getTime())) return undefined;
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function formatRelative(timestamp?: number | string | null) {
  const normalized = normalizeUnixSeconds(timestamp);
  if (normalized === undefined) return "--";
  const diff = Math.max(0, nowUnixSeconds() - normalized);
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

export function formatResetTime(resetAt?: number | string | null) {
  const normalized = normalizeUnixSeconds(resetAt);
  if (normalized === undefined) {
    return typeof resetAt === "string" ? resetAt.trim() || undefined : undefined;
  }

  const dateTime = formatDateTime(normalized);
  if (!dateTime) return undefined;
  const exact = dateTime.slice(5).replace("-", "/");
  const diff = normalized - nowUnixSeconds();
  if (diff <= 0) return exact;
  const minutes = Math.floor(diff / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  const relative =
    days > 0
      ? `${days}d ${hours % 24}h`
      : hours > 0
        ? `${hours}h ${minutes % 60}m`
        : `${Math.max(1, minutes)}m`;
  return `${relative} (${exact})`;
}
