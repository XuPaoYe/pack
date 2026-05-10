// 应用内通知日志（toast 历史）的 localStorage 持久化与清理。
// 与 React 解耦的纯函数模块，App.tsx 通过 useState(loadAppLogs) + persistAppLogs 接入。

export type NoticeTone = "success" | "error" | "info";

export type AppLogEntry = {
  id: string;
  tone: NoticeTone;
  text: string;
  createdAt: number;
};

const APP_LOG_STORAGE_KEY = "super-ai:app-logs";
const APP_LOG_RETENTION_MS = 3 * 24 * 60 * 60 * 1000;
const APP_LOG_LIMIT = 300;

function isAppLogEntry(value: unknown): value is AppLogEntry {
  if (!value || typeof value !== "object") return false;
  const item = value as Partial<AppLogEntry>;
  return (
    typeof item.id === "string" &&
    typeof item.text === "string" &&
    typeof item.createdAt === "number" &&
    (item.tone === "success" || item.tone === "error" || item.tone === "info")
  );
}

export function pruneAppLogs(logs: AppLogEntry[]): AppLogEntry[] {
  const cutoff = Date.now() - APP_LOG_RETENTION_MS;
  return logs
    .filter((log) => log.createdAt >= cutoff)
    .sort((a, b) => b.createdAt - a.createdAt)
    .slice(0, APP_LOG_LIMIT);
}

/**
 * 从 localStorage 读取并清洗历史日志。
 * sanitize 用于把脱敏 / 重命名规则应用到 text 上（由调用方注入，避免本模块依赖 UI 常量）。
 */
export function loadAppLogs(sanitize: (text: string) => string = (t) => t): AppLogEntry[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(APP_LOG_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return pruneAppLogs(
      parsed.filter(isAppLogEntry).map((log) => ({
        ...log,
        text: sanitize(log.text),
      })),
    );
  } catch {
    return [];
  }
}

export function persistAppLogs(logs: AppLogEntry[]): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(APP_LOG_STORAGE_KEY, JSON.stringify(pruneAppLogs(logs)));
  } catch {
    // Local logging is best effort only.
  }
}

export function createLogId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
