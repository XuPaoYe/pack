import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getName, getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  BadgeCheck,
  CalendarDays,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleAlert,
  Clipboard,
  Copy,
  Cloud,
  Download,
  Eye,
  EyeOff,
  FileJson,
  FolderDown,
  Info,
  Laptop,
  LockKeyhole,
  Monitor,
  Moon,
  Plus,
  Power,
  RefreshCw,
  RotateCcw,
  RotateCw,
  Rocket,
  ScrollText,
  Search,
  SearchX,
  Server,
  Settings,
  Sun,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import "./App.css";
import logoUrl from "./assets/logo.svg";
import { useUpdater } from "./hooks/useUpdater";
import { useNotice } from "./hooks/useNotice";
import { useVirtualScrollbar } from "./hooks/useVirtualScrollbar";
import { ForceUpdateModal } from "./components/ForceUpdateModal";
import { AccountDetailsDialog } from "./components/AccountDetailsDialog";
import { NoticeToast } from "./components/NoticeToast";
import { AboutPanelBody } from "./components/AboutPanel";
import { SidebarAdCard } from "./components/SidebarAdCard";
import { noticeToneConfig } from "./components/noticeTone";
import { CodexIcon } from "./components/icons/CodexIcon";
import { GeminiIcon } from "./components/icons/GeminiIcon";
import { SuperaiIcon } from "./components/icons/SuperaiIcon";
import { AntigravityIcon } from "./components/icons/AntigravityIcon";
import { fallbackStatus, formatValidityText, resolvePlanBadge } from "./lib/accountPresentation";
import {
  loadAppLogs as loadAppLogsRaw,
  persistAppLogs,
  pruneAppLogs,
} from "./lib/appLogs";
import { parseAuthJson, type AccountState, type ImportFailure, type ManagedAccount, type Provider, type QuotaMetric } from "./lib/authParser";
import { formatDateTime, formatRelative, formatResetTime } from "./lib/time";

type ImportMode = "paste" | "file" | "local" | "oauth" | "batchKey" | "password";
type OAuthProvider = "codex" | "gemini" | "antigravity";
type ThemeMode = "system" | "light" | "dark";

const themeOptions: Array<{ key: ThemeMode; label: string; icon: typeof Monitor }> = [
  { key: "system", label: "跟随系统", icon: Monitor },
  { key: "light", label: "浅色", icon: Sun },
  { key: "dark", label: "深色", icon: Moon },
];

type BackendImportResult = {
  imported: ManagedAccount[];
  failed: ImportFailure[];
};

type AppSettings = {
  theme: ThemeMode;
  autoLaunch: boolean;
  maskSensitive: boolean;
  apiServiceEnabled: boolean;
  apiServiceHost: string;
  apiServicePort: number;
  apiServiceDefaultModel: string;
};

type ApiServiceModel = {
  id: string;
  owned_by?: string;
};

type OAuthStartResult = {
  login_id: string;
  provider: OAuthProvider;
  command: string;
  message: string;
};

type OAuthCallbackEvent = {
  provider?: OAuthProvider;
  loginId?: string;
};

type CompleteOAuthFn = (
  provider: OAuthProvider,
  loginId: string,
  options?: { silent?: boolean },
) => Promise<boolean>;

type ApiServiceStatus = {
  running: boolean;
  bindHost: string;
  bindPort: number;
  actualPort: number | null;
  address: string | null;
  apiKey: string;
  defaultModel: string;
  lastError: string | null;
};

type SwitchAccountResult = ManagedAccount[];
type ExportPreview = {
  payload: string;
  kind: "json" | "key";
  label: string;
  fileBase: string;
};

type KeyIssueModalState = {
  title: string;
  message: string;
};
const ACCOUNT_PAGE_SIZE = 12;
const ACTIVE_ACCOUNT_REFRESH_INTERVAL_MS = 15_000;
const API_SERVICE_PORT_MIN = 51000;
const API_SERVICE_PORT_MAX = 59999;
const DEFAULT_API_SERVICE_PORT = 51888;
const APP_NAME = [83, 117, 112, 101, 114, 32, 65, 73]
  .map((c) => String.fromCharCode(c))
  .join("");
const IS_PUBLIC_BUILD = import.meta.env.VITE_SUPERAI_PUBLIC_BUILD === "1";

const modeConfig: Record<
  ImportMode,
  {
    icon: typeof FileJson;
    title: string;
    desc: string;
  }
> = {
  paste: {
    icon: Clipboard,
    title: "粘贴凭证",
    desc: "粘贴 Auth.json 或账号 JSON。",
  },
  file: {
    icon: FileJson,
    title: "上传Json",
    desc: `支持 Auth.json、${APP_NAME} 等多种格式`,
  },
  local: {
    icon: Laptop,
    title: "读取本机",
    desc: "从本地已登录的会话中导入 Codex 账号",
  },
  oauth: {
    icon: Cloud,
    title: "OAuth授权",
    desc: "点击下方按钮，在浏览器中完成 OpenAI 账号 OAuth 授权。",
  },
  batchKey: {
    icon: Clipboard,
    title: "批量密钥",
    desc: `一行一个密钥，支持多个 ${APP_NAME} 账号一起导入。`,
  },
  password: {
    icon: LockKeyhole,
    title: "账号密码",
    desc: `输入 ${APP_NAME} 邮箱与密码导入单个账号。`,
  },
};

const importModeOrder: ImportMode[] = ["oauth", "paste", "local", "file"];
// Antigravity 没有标准本机凭证文件（Google IDE 把 token 存在 protobuf 编码的 vscdb 里），所以不提供本机导入。
const antigravityImportModeOrder: ImportMode[] = ["oauth", "paste", "file"];
const superaiImportModeOrder: ImportMode[] = ["batchKey", "password"];
const defaultImportMode: ImportMode = "oauth";
const defaultSuperaiImportMode: ImportMode = "batchKey";

function importModesForProvider(provider: Provider): ImportMode[] {
  if (provider === PROVIDER_SUPERAI) return superaiImportModeOrder;
  if (provider === "antigravity") return antigravityImportModeOrder;
  return importModeOrder;
}

function defaultImportModeForProvider(provider: Provider): ImportMode {
  return provider === PROVIDER_SUPERAI ? defaultSuperaiImportMode : defaultImportMode;
}

function providerLabel(provider: Provider) {
  if (provider === "codex") return "Codex";
  if (provider === "gemini") return "Gemini Cli";
  if (provider === "antigravity") return "Antigravity";
  return APP_NAME;
}

// 用 .map(...).join("") 形式构造，绕过 esbuild / vite 的常量折叠，让 dist 里
// 不出现上游品牌字面量。直接 String.fromCharCode(...) 会被构建器在编译期算成明文。
const __LEGACY_PROVIDER_NAME: string = [87, 105, 110, 100, 115, 117, 114, 102]
  .map((c) => String.fromCharCode(c))
  .join("");
const __LEGACY_PROVIDER_PROJECT: string = [119, 105, 110, 100, 115, 117, 114, 102, 97, 112, 105]
  .map((c) => String.fromCharCode(c))
  .join("");
const PROVIDER_SUPERAI = "superai" as const;
function sanitizeUserFacingText(text: string) {
  // 先替换更长的 lowercase 项目代号，再替换品牌名；顺序反了会留下大小写混用的形态。
  const next = text
    .replaceAll(__LEGACY_PROVIDER_PROJECT, "superai-sidecar")
    .replace(new RegExp(__LEGACY_PROVIDER_NAME + "API", "gi"), APP_NAME)
    .replaceAll(__LEGACY_PROVIDER_NAME, APP_NAME);
  return next;
}

function loadAppLogs() {
  return loadAppLogsRaw(sanitizeUserFacingText);
}

function normalizeUserError(error: unknown) {
  const raw = String(error ?? "").trim();
  const message = sanitizeUserFacingText(raw);
  if (
    message.includes("SuperAI 主密钥缺失")
    || message.includes("本地密钥文件已不存在")
  ) {
    return "本机加密密钥缺失，现有 SuperAI 账号无法读取。请恢复本机数据目录中的密钥文件，或重新导入这些账号。";
  }
  if (message.includes("SuperAI 主密钥损坏")) {
    return "本机加密密钥已损坏，现有 SuperAI 账号无法读取。请恢复正确的密钥文件，或删除后重新导入账号。";
  }
  if (
    message.includes("主密钥与本地数据不匹配")
    || message.includes("读取 SuperAI 加密账号失败")
    || message.includes("SuperAI v2 解密失败")
  ) {
    return "SuperAI 本地加密数据无法解密。通常是当前机器上的密钥文件与已有账号数据不匹配，或数据本身已损坏。";
  }
  return message || "未知错误";
}

function resolveKeyIssueModal(error: unknown): KeyIssueModalState | null {
  const raw = String(error ?? "").trim();
  const message = sanitizeUserFacingText(raw);
  if (
    message.includes("SuperAI 主密钥缺失")
    || message.includes("本地密钥文件已不存在")
  ) {
    return {
      title: "本地密钥缺失",
      message: "当前机器上用于解密 SuperAI 账号的本地密钥文件已经缺失，已有账号暂时无法读取。恢复原机器上的密钥文件后可继续使用；如果无法恢复，只能删除这些账号后重新导入。",
    };
  }
  if (message.includes("SuperAI 主密钥损坏")) {
    return {
      title: "本地密钥损坏",
      message: "当前机器上的 SuperAI 本地密钥文件已损坏，已有账号暂时无法读取。请恢复正确的密钥文件；如果无法恢复，只能删除这些账号后重新导入。",
    };
  }
  if (
    message.includes("主密钥与本地数据不匹配")
    || message.includes("读取 SuperAI 加密账号失败")
    || message.includes("SuperAI v2 解密失败")
  ) {
    return {
      title: "本地加密数据无法解密",
      message: "当前机器上的密钥文件与已有 SuperAI 账号数据不匹配，或本地加密数据本身已损坏。通常发生在更换机器、清理本地数据目录，或手动覆盖数据库之后。",
    };
  }
  return null;
}

function stateLabel(state: AccountState) {
  if (state === "available") return "可用";
  if (state === "warning") return "告警";
  if (state === "unknown") return "未知";
  return "不可用";
}

function isCurrentAccount(account: ManagedAccount) {
  return account.status?.state === "available" && account.status.label === "当前";
}

function accountTitle(account: ManagedAccount) {
  return account.email || account.displayName || account.accountId || account.id;
}

function accountCardTitle(account: ManagedAccount) {
  return accountTitle(account);
}

function accountDisplayLabel(account: ManagedAccount) {
  return accountCardTitle(account);
}

function accountNoticeLabel(account: ManagedAccount) {
  return account.accountId || account.email || accountTitle(account);
}

function accountActionLabel(account: ManagedAccount) {
  return `${providerLabel(account.provider)} ${accountNoticeLabel(account)}`;
}

function activationSuccessMessage(account: ManagedAccount) {
  const label = accountActionLabel(account);
  if (account.provider === "codex") {
    return `已启用 ${label}，请重启 Codex 相关产品`;
  }
  if (account.provider === "gemini") {
    return `已启用 ${label}，请重启 Gemini 相关产品`;
  }
  if (account.provider === "antigravity") {
    return `已启用 ${label}，请重启 Antigravity 相关产品`;
  }
  return `已启用 ${label}`;
}

function refreshFailureMessage(account: ManagedAccount) {
  const reason = account.status?.reason ?? account.quota?.error;
  return reason ? `刷新 ${accountActionLabel(account)} 失败：${reason}` : `刷新 ${accountActionLabel(account)} 失败`;
}

function batchDeleteDescription(accounts: ManagedAccount[]) {
  const providers = Array.from(new Set(accounts.map((account) => providerLabel(account.provider))));
  const scope = providers.length === 1 ? `${providers[0]} ` : "";
  return `共 ${scope}${accounts.length} 个账号将被移除`;
}

function isRefreshUnavailable(account: ManagedAccount) {
  return account.status?.state === "unavailable" || Boolean(account.quota?.error);
}

function sortAccountsForView(items: ManagedAccount[]) {
  return [...items].sort((a, b) => {
    const currentDelta = Number(isCurrentAccount(b)) - Number(isCurrentAccount(a));
    if (currentDelta !== 0) return currentDelta;
    const createdDelta = b.createdAt - a.createdAt;
    if (createdDelta !== 0) return createdDelta;
    return a.id.localeCompare(b.id);
  });
}

function AccountStateCorner({ account }: { account: ManagedAccount }) {
  const status = account.status ?? fallbackStatus(account);
  const isCurrent = isCurrentAccount(account);
  return (
    <span className={clsx("state-corner", status.state, isCurrent && "current")} title={isCurrent ? "当前启用账号" : (status.reason ?? stateLabel(status.state))}>
      {isCurrent ? "启用" : status.label || stateLabel(status.state)}
    </span>
  );
}

function AccountPlanBadge({ account }: { account: ManagedAccount }) {
  const badge = resolvePlanBadge(account);
  return <span className={clsx("pill", "plan", badge.tone)}>{badge.label}</span>;
}

function localizeQuotaLabel(label: string): string {
  const upper = label.toUpperCase();
  if (upper === "DAILY") return "日限";
  if (upper === "WEEKLY") return "周限";
  if (/^\d+[HD]$/.test(upper)) return upper;
  return label;
}

function parseApiServicePort(value: string): number | null {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const port = Number(trimmed);
  if (!Number.isInteger(port)) return null;
  if (port < API_SERVICE_PORT_MIN || port > API_SERVICE_PORT_MAX) return null;
  return port;
}

function parseApiServiceHost(value: string): string | null {
  const trimmed = value.trim();
  const parts = trimmed.split(".");
  if (parts.length !== 4) return null;
  const octets: number[] = [];
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) return null;
    const octet = Number(part);
    if (!Number.isInteger(octet) || octet < 0 || octet > 255) return null;
    octets.push(octet);
  }
  return octets.join(".");
}

function pickAntigravitySummaryMetrics(metrics: QuotaMetric[]): QuotaMetric[] {
  if (metrics.length === 0) return metrics;
  const pickLowest = (predicate: (label: string, name: string) => boolean, fallbackLabel: string, fallbackKey: string): QuotaMetric | undefined => {
    const matches = metrics.filter((metric) => {
      const display = (metric.displayName ?? metric.label ?? "").toLowerCase();
      const name = (metric.modelName ?? "").toLowerCase();
      return predicate(display, name);
    });
    if (matches.length === 0) return undefined;
    const best = matches.reduce((acc, metric) => {
      const remaining = metric.remainingPercent ?? 101;
      const accRemaining = acc.remainingPercent ?? 101;
      return remaining < accRemaining ? metric : acc;
    });
    return { ...best, key: fallbackKey, label: fallbackLabel };
  };
  const gemini31Pro = pickLowest(
    (label, name) => name.includes("gemini-3.1-pro") || label.includes("gemini 3.1 pro"),
    "Gemini 3.1 Pro",
    "antigravity-summary-gemini-3.1-pro",
  );
  const gemini3Flash = pickLowest(
    (label, name) =>
      name.includes("gemini-3-flash") ||
      name.includes("gemini-3.0-flash") ||
      name.includes("gemini-3.1-flash") ||
      label.includes("gemini 3 flash") ||
      label.includes("gemini 3.0 flash") ||
      label.includes("gemini 3.1 flash"),
    "Gemini 3 Flash",
    "antigravity-summary-gemini-3-flash",
  );
  const claude = pickLowest(
    (label, name) => name.startsWith("claude") || label.includes("claude"),
    "Claude",
    "antigravity-summary-claude",
  );
  const picked = [gemini31Pro, gemini3Flash, claude].filter((m): m is QuotaMetric => Boolean(m));
  return picked.length > 0 ? picked : metrics.slice(0, 3);
}

function QuotaMeters({ account }: { account: ManagedAccount }) {
  const isUnavailable = account.status?.state === "unavailable";
  const rawMetrics =
    account.quota?.metrics?.length
      ? account.quota.metrics
      : [
          { key: "quota-primary", label: "状态", remainingPercent: undefined, state: "unknown" as AccountState },
        ];
  let metrics = rawMetrics;
  if (account.provider === "antigravity" && account.quota?.metrics?.length) {
    metrics = pickAntigravitySummaryMetrics(account.quota.metrics);
  }

  return (
    <div className="quota-meters">
      {metrics.slice(0, 3).map((metric) => {
        const remaining = metric.remainingPercent;
        const state = isUnavailable ? "unavailable" : (metric.state ?? (remaining === undefined ? "unknown" : remaining <= 0 ? "unavailable" : remaining <= 15 ? "warning" : "available"));
        const resetText = isUnavailable ? "--" : (formatResetTime(metric.resetAt) ?? "--");
        const meterTitle = metric.detail ?? account.quota?.error ?? metric.label;
        return (
          <div className={clsx("quota-meter", state)} key={metric.key} title={meterTitle}>
            <div className="quota-meter-head">
              <span>{localizeQuotaLabel(metric.label)}</span>
              <time className="quota-meter-reset">{resetText}</time>
              <div className="quota-meter-value">
                <strong>{remaining === undefined ? "N/A" : `${remaining}%`}</strong>
              </div>
            </div>
            <div className={clsx("quota-track", state)}>
              <i style={{ width: `${remaining ?? 0}%` }} />
            </div>
          </div>
        );
      })}
    </div>
  );
}

function ValidityMeter({ account }: { account: ManagedAccount }) {
  const validity = formatValidityText(account);
  if (!validity.detail) {
    return <div className="validity-line placeholder" aria-hidden="true" />;
  }
  return (
    <div className={clsx("validity-line", validity.expired && "expired")} title={validity.title}>
      <CalendarDays size={15} strokeWidth={1.9} />
      <span>
        {validity.label} <strong>{validity.detail}</strong>
      </span>
      {validity.title && <time>{validity.title}</time>}
    </div>
  );
}

function mergeAccounts(current: ManagedAccount[], next: ManagedAccount[]) {
  const map = new Map(current.map((account) => [account.id, account]));
  for (const account of next) map.set(account.id, account);
  return sortAccountsForView([...map.values()]);
}

type EffortKey = "minimal" | "low" | "medium" | "high" | "xhigh";

const EFFORT_LABELS: Record<EffortKey, string> = {
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "XHigh",
};

type ModelFamily = {
  key: string;
  label: string;
  aliases?: string[];
  protocol: "openai" | "messages";
  /** 该家族支持的 effort 选项；空数组表示无 effort 概念。 */
  efforts: EffortKey[];
  /** 默认 effort；若 efforts 为空可省略。 */
  defaultEffort?: EffortKey;
  knownAvailable?: boolean;
  /** 标记为免费档位，下拉里多挂一个 Free 角标。 */
  free?: boolean;
  /**
   * 返回最终展示/保存用模型 id；不可用时返回 null。
   * effort 为 null 表示无 effort（如 codex 单一变体）。
   */
  resolveId: (effort: EffortKey | null) => string | null;
};

const MODEL_FAMILIES: ModelFamily[] = [
  {
    key: "claude-opus-4.6",
    label: "Claude Opus 4.6",
    aliases: ["Claude Opus 4.6", "claude-opus-4-6"],
    protocol: "messages",
    efforts: [],
    resolveId: () => "claude-opus-4.6",
  },
  {
    key: "claude-opus-4.7",
    label: "Claude Opus 4.7",
    aliases: ["Claude Opus 4.7"],
    protocol: "messages",
    efforts: ["low", "medium", "high", "xhigh"],
    defaultEffort: "medium",
    resolveId: (effort) => (effort ? `claude-opus-4.7-${effort}` : "claude-opus-4.7-medium"),
  },
  {
    key: "gemini-3.0-flash",
    label: "Gemini 3.0 Flash",
    aliases: ["Gemini 3.0 Flash", "gemini-3-0-flash"],
    protocol: "openai",
    // 上游模型表提供 minimal / low / medium / high
    // 四个档位；bare `gemini-3.0-flash` 等同 medium。
    efforts: ["minimal", "low", "medium", "high"],
    defaultEffort: "medium",
    knownAvailable: true,
    resolveId: (effort) => {
      if (!effort || effort === "medium") return "gemini-3.0-flash";
      if (effort === "xhigh") return null;
      return `gemini-3.0-flash-${effort}`;
    },
  },
  {
    key: "gemini-3.1-pro",
    label: "Gemini 3.1 Pro",
    aliases: ["Gemini 3.1 Pro", "gemini-3-1-pro"],
    protocol: "openai",
    // 上游只暴露 low / high 两档，没有 medium/xhigh/minimal。
    efforts: ["low", "high"],
    defaultEffort: "low",
    knownAvailable: true,
    resolveId: (effort) => {
      if (effort === "high") return "gemini-3.1-pro-high";
      return "gemini-3.1-pro-low";
    },
  },
  {
    key: "gemini-3.5-flash",
    label: "Gemini 3.5 Flash",
    aliases: ["Gemini 3.5 Flash", "gemini-3-5-flash"],
    protocol: "openai",
    efforts: ["minimal", "low", "medium", "high"],
    defaultEffort: "medium",
    knownAvailable: true,
    resolveId: (effort) => {
      if (!effort || effort === "medium") return "gemini-3.5-flash";
      if (effort === "xhigh") return null;
      return `gemini-3.5-flash-${effort}`;
    },
  },
  {
    key: "grok-3",
    label: "XAI Grok-3",
    aliases: ["XAI Grok-3", "Grok-3", "grok-3"],
    protocol: "openai",
    efforts: [],
    resolveId: () => "grok-3",
  },
  {
    key: "grok-3-mini-thinking",
    label: "XAI Grok-3 mini Thinking",
    aliases: [
      "XAI Grok-3 mini Thinking",
      "Grok-3 mini Thinking",
      "grok-3-mini-thinking",
    ],
    protocol: "openai",
    efforts: [],
    resolveId: () => "grok-3-mini-thinking",
  },
];

const FALLBACK_FAMILY_KEY = "claude-opus-4.7";

type ApiModelPref = {
  family: string;
  effort: EffortKey | null;
};

function defaultPref(): ApiModelPref {
  // 用模型家族自身的 defaultEffort，避免首次打开配置时推理强度无高亮。
  const fam = MODEL_FAMILIES.find((f) => f.key === FALLBACK_FAMILY_KEY);
  const effort = fam && fam.efforts.length > 0 ? fam.defaultEffort ?? fam.efforts[0] : null;
  return { family: FALLBACK_FAMILY_KEY, effort: effort ?? null };
}

const API_PREF_STORAGE_KEY = "super-ai:api-service-pref";

function protocolShortLabel(protocol: ModelFamily["protocol"]): string {
  return protocol === "messages" ? "Anthropic" : "OpenAI";
}

function loadApiPref(): ApiModelPref {
  if (typeof window === "undefined") return defaultPref();
  try {
    const raw = window.localStorage.getItem(API_PREF_STORAGE_KEY);
    if (!raw) return defaultPref();
    const parsed = JSON.parse(raw) as Partial<ApiModelPref>;
    const fam = MODEL_FAMILIES.find((f) => f.key === parsed.family);
    if (!fam) {
      // 之前选过的模型已被下架时——
      // 直接在这里把 localStorage 重置成默认值，避免下次启动还读到脏数据。
      const fallback = defaultPref();
      try {
        window.localStorage.setItem(API_PREF_STORAGE_KEY, JSON.stringify(fallback));
      } catch {
        /* ignore */
      }
      return fallback;
    }
    const effort = parsed.effort && fam.efforts.includes(parsed.effort)
      ? parsed.effort
      : fam.defaultEffort ?? null;
    return {
      family: fam.key,
      effort: fam.efforts.length === 0 ? null : effort,
    };
  } catch {
    return defaultPref();
  }
}

function persistApiPref(pref: ApiModelPref) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(API_PREF_STORAGE_KEY, JSON.stringify(pref));
  } catch {
    /* localStorage 不可用就放弃 */
  }
}

/** 把 pref 翻译成当前 UI 配置使用的 model id，并判断是否在列表中可用。 */
function resolveModelId(pref: ApiModelPref): string | null {
  const family = MODEL_FAMILIES.find((f) => f.key === pref.family);
  if (!family) return null;
  return family.resolveId(pref.effort);
}

function normalizeModelName(value: string): string {
  return value
    .toLowerCase()
    .replace(/\bthinking\b/g, "")
    .replace(/\bcodex\b/g, "codex")
    .replace(/[^a-z0-9]+/g, "");
}

function modelTokens(family: ModelFamily): string[] {
  return [
    family.key,
    family.label,
    ...(family.aliases ?? []),
    family.resolveId(null),
    ...family.efforts.map((effort) => family.resolveId(effort)),
  ].filter((value): value is string => Boolean(value));
}

/** 该家族在当前可选模型列表中可用（默认 effort 的变体存在）。 */
function isFamilyAvailable(family: ModelFamily, available: Set<string>): boolean {
  if (family.knownAvailable) return true;
  if (available.size === 0) return false;
  const availableNames = Array.from(available);
  const normalizedAvailable = availableNames.map(normalizeModelName);
  return modelTokens(family).some((token) => {
    const normalizedToken = normalizeModelName(token);
    return normalizedAvailable.some(
      (model) => model === normalizedToken || model.includes(normalizedToken) || normalizedToken.includes(model),
    );
  });
}

function ModelSelect({
  familyKey,
  availableModels,
  disabled,
  loading,
  emptyHint,
  onChange,
}: {
  familyKey: string;
  availableModels: ApiServiceModel[];
  disabled: boolean;
  loading: boolean;
  emptyHint: string;
  onChange: (familyKey: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return undefined;
    const handle = (event: MouseEvent) => {
      if (!containerRef.current) return;
      if (!containerRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", handle, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", handle, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  const availableSet = useMemo(
    () => new Set(availableModels.map((m) => m.id)),
    [availableModels],
  );
  const families = MODEL_FAMILIES;
  const selectedFamily = families.find((f) => f.key === familyKey);
  const triggerLabel = disabled
    ? selectedFamily?.label ?? emptyHint
    : loading
    ? "加载模型…"
    : selectedFamily?.label ?? "选择模型";

  const handlePick = (key: string, available: boolean) => {
    if (!available) return;
    onChange(key);
    setOpen(false);
  };

  return (
    <div
      ref={containerRef}
      className={clsx("model-select", open && "open", disabled && "disabled")}
    >
      <button
        type="button"
        className="model-select-trigger"
        disabled={disabled || loading}
        onClick={() => setOpen((prev) => !prev)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className="model-select-value" title={triggerLabel}>
          {triggerLabel}
        </span>
        <ChevronDown size={14} className="model-select-caret" />
      </button>
      {open && !disabled && (
        <div className="model-select-popover" role="listbox">
          {families.map((family) => {
            const available = isFamilyAvailable(family, availableSet);
            const selected = family.key === familyKey;
            return (
              <button
                type="button"
                key={family.key}
                className={clsx(
                  "model-select-option",
                  selected && "selected",
                  !available && "unavailable",
                )}
                disabled={!available}
                onClick={() => handlePick(family.key, available)}
                title={available ? family.label : "暂未上线"}
              >
                <span className="model-select-option-main">
                  <span className="model-select-option-text">
                    <span>{family.label}</span>
                  </span>
                </span>
                <span className="model-select-option-side">
                  <span className="model-select-option-badges">
                    <em className={clsx("model-badge-protocol", family.protocol === "messages" && "messages")}>
                      {protocolShortLabel(family.protocol)}
                    </em>
                    {family.free && <em className="model-badge-free">Free</em>}
                    {!available && <em>暂未上线</em>}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function EffortSegments({
  options,
  value,
  disabled,
  onChange,
}: {
  options: EffortKey[];
  value: EffortKey | null;
  disabled: boolean;
  onChange: (effort: EffortKey) => void;
}) {
  return (
    <div className={clsx("effort-segments", disabled && "disabled")} role="radiogroup">
      {options.map((effort) => (
        <button
          key={effort}
          type="button"
          role="radio"
          aria-checked={value === effort}
          className={clsx("effort-segment", value === effort && "active")}
          disabled={disabled}
          onClick={() => onChange(effort)}
        >
          {EFFORT_LABELS[effort]}
        </button>
      ))}
    </div>
  );
}

function ApiServiceCard({
  status,
  busy,
  starting,
  pref,
  onToggleService,
  onCopy,
  onOpenConfig,
}: {
  status: ApiServiceStatus | null;
  busy: boolean;
  starting: boolean;
  pref: ApiModelPref;
  onToggleService: () => void;
  onCopy: (text: string, label: string) => void;
  onOpenConfig: () => void;
}) {
  const running = Boolean(status?.running);
  const stateClass = starting ? "starting" : running ? "running" : "idle";
  const address = status?.address ?? "—";
  const apiKey = status?.apiKey ?? "";
  const families = MODEL_FAMILIES;
  const family = families.find((f) => f.key === pref.family) ?? families[0];
  const modelSummary = [
    family.label,
    protocolShortLabel(family.protocol),
    pref.effort ? EFFORT_LABELS[pref.effort] : null,
  ].filter(Boolean).join(" · ");

  return (
    <article className={clsx("account-row superai-api-card", stateClass)}>
      <div className="superai-api-head">
        <div className="superai-api-icon">
          <Server size={22} strokeWidth={1.8} />
        </div>
        <div className="superai-api-title">
          <strong>API 服务</strong>
          <span>支持本机与局域网调用</span>
        </div>
        <div className={clsx("superai-api-status-dot", stateClass)} />
      </div>

      <dl className="superai-api-grid">
        <dt>地址</dt>
        <dd>
          <code>{address}</code>
          <button
            type="button"
            className="icon-btn"
            onClick={() => onCopy(address === "—" ? "" : address, "地址")}
            disabled={!status?.address}
            aria-label="复制地址"
          >
            <Copy size={14} />
          </button>
        </dd>
        <dt>密钥</dt>
        <dd>
          <code>{apiKey || "—"}</code>
          <button
            type="button"
            className="icon-btn"
            onClick={() => onCopy(apiKey, "密钥")}
            disabled={!apiKey}
            aria-label="复制密钥"
          >
            <Copy size={14} />
          </button>
        </dd>
        <dt>配置</dt>
        <dd>
          <code>{modelSummary}</code>
          <button
            type="button"
            className="icon-btn"
            onClick={onOpenConfig}
            aria-label="打开配置"
            title="打开配置"
          >
            <Settings size={14} />
          </button>
        </dd>
      </dl>

      {status?.lastError && (
        <p className="api-service-error">
          <CircleAlert size={14} />
          {sanitizeUserFacingText(status.lastError)}
        </p>
      )}

      <div className="superai-api-footer">
        <div className="superai-api-actions">
          <button
            type="button"
            className={clsx("superai-api-toggle", starting ? "on" : running ? "off" : "on")}
            onClick={onToggleService}
            disabled={busy || starting}
          >
            <Power size={14} />
            {starting ? "启动中" : running ? "停止服务" : "启动服务"}
          </button>
        </div>

        <p className="superai-api-hint">
          {starting
            ? "API 服务正在后台启动，请稍候，启动完成后会自动刷新状态。"
            : running
            ? "API 服务运行中。你可以使用上方地址和密钥让外部 IDE 或工具接入。"
            : "启动 API 服务后，你可以通过上方地址和密钥在 IDE 或其他工具中调用。"}
        </p>
      </div>
    </article>
  );
}

function ApiServiceConfigPanel({
  running,
  models,
  pref,
  onChangeFamily,
  onChangeEffort,
  onConfigureClaude,
  onRestoreClaude,
  configuringClaude,
  restoringClaude,
}: {
  running: boolean;
  models: ApiServiceModel[];
  pref: ApiModelPref;
  onChangeFamily: (family: string) => void;
  onChangeEffort: (effort: EffortKey) => void;
  onConfigureClaude: () => void;
  onRestoreClaude: () => void;
  configuringClaude: boolean;
  restoringClaude: boolean;
}) {
  const families = MODEL_FAMILIES;
  const family = families.find((f) => f.key === pref.family) ?? families[0];
  const showEffort = family.efforts.length > 0;

  return (
    <div className="api-config-body">
      <section className="api-config-row">
        <div className="api-config-copy">
          <strong>模型</strong>
          <p>选择 API 服务默认使用的模型家族。</p>
        </div>
        <ModelSelect
          familyKey={family.key}
          availableModels={models}
          disabled={!running}
          loading={running && models.length === 0}
          emptyHint="服务未启动"
          onChange={onChangeFamily}
        />
      </section>

      <section className="api-config-row">
        <div className="api-config-copy">
          <strong>推理强度</strong>
          <p>{showEffort ? "控制模型思考深度和响应成本" : "当前模型没有推理强度选项"}</p>
        </div>
        {showEffort ? (
          <EffortSegments
            options={family.efforts}
            value={pref.effort}
            disabled={!running}
            onChange={onChangeEffort}
          />
        ) : (
          <div className="effort-segments unsupported" aria-disabled="true">
            <span className="effort-segment placeholder">不支持</span>
          </div>
        )}
      </section>

      <section className="api-config-row">
        <div className="api-config-copy">
          <strong>Claude 配置</strong>
          <p>把当前地址和密钥写入 ~/.claude/settings.json，并切到 Claude 推荐模型。</p>
        </div>
        <div className="api-config-actions">
          <button
            type="button"
            className="superai-api-secondary"
            onClick={onConfigureClaude}
            disabled={!running || configuringClaude || restoringClaude}
            title={running ? "把当前 API 服务根地址、密钥同步到 ~/.claude/settings.json" : "请先启动 API 服务"}
          >
            <CodexIcon size={14} />
            {configuringClaude ? "配置中…" : "配置 Claude"}
          </button>
          <button
            type="button"
            className="superai-api-secondary"
            onClick={onRestoreClaude}
            disabled={configuringClaude || restoringClaude}
            title={`从 .superai-bak 恢复，没有备份则尽量移除 ${APP_NAME} 痕迹`}
          >
            <RotateCcw size={14} />
            {restoringClaude ? "恢复中…" : "恢复 Claude"}
          </button>
        </div>
      </section>

    </div>
  );
}

function AppModal({
  title,
  description,
  closeLabel,
  className,
  onClose,
  children,
}: {
  title: string;
  description: string;
  closeLabel: string;
  className?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <div className="modal-overlay">
      <aside className={clsx("app-modal", className, "modal-content")} onMouseDown={(event) => event.stopPropagation()}>
        <div className="panel-head">
          <div>
            <h2>{title}</h2>
            <p>{description}</p>
          </div>
          <button className="modal-close-button" onClick={onClose} aria-label={closeLabel}>
            <X size={22} strokeWidth={2.2} />
          </button>
        </div>
        {children}
      </aside>
    </div>
  );
}

function App() {
  const [accounts, setAccounts] = useState<ManagedAccount[]>([]);
  const [activeProvider, setActiveProvider] = useState<Provider>(PROVIDER_SUPERAI);
  const [mode, setMode] = useState<ImportMode>(defaultImportMode);
  const [pasteValue, setPasteValue] = useState("");
  const [superaiBatchKeys, setSuperaiBatchKeys] = useState("");
  const [superaiPasswordEmail, setSuperaiPasswordEmail] = useState("");
  const [superaiPasswordPwd, setSuperaiPasswordPwd] = useState("");
  const [query, setQuery] = useState("");
  const [accountPage, setAccountPage] = useState(1);
  const [isImportModalOpen, setIsImportModalOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isLogsOpen, setIsLogsOpen] = useState(false);
  const [isAboutOpen, setIsAboutOpen] = useState(false);
  const [aboutInfo, setAboutInfo] = useState<{ name: string; version: string } | null>(null);
  const [detailsAccountId, setDetailsAccountId] = useState<string | null>(null);
  const [isApiConfigOpen, setIsApiConfigOpen] = useState(false);
  const [exportPreview, setExportPreview] = useState<ExportPreview | null>(null);
  const [keyIssueModal, setKeyIssueModal] = useState<KeyIssueModalState | null>(null);
  const [selectedExportIds, setSelectedExportIds] = useState<Set<string>>(() => new Set());
  const [pendingDeleteAccount, setPendingDeleteAccount] = useState<ManagedAccount | null>(null);
  const [pendingBatchDelete, setPendingBatchDelete] = useState<ManagedAccount[] | null>(null);
  const [isAccountBusy, setIsAccountBusy] = useState(false);
  const [isExportBusy, setIsExportBusy] = useState(false);
  const [isImportBusy, setIsImportBusy] = useState(false);
  const [isDeletingAccount, setIsDeletingAccount] = useState(false);
  const [switchingAccountId, setSwitchingAccountId] = useState<string | null>(null);
  const [isFileImporting, setIsFileImporting] = useState(false);
  const [refreshingAccountIds, setRefreshingAccountIds] = useState<Set<string>>(() => new Set());
  const [refreshingProviders, setRefreshingProviders] = useState<Set<Provider>>(() => new Set());
  const [pendingOAuth, setPendingOAuth] = useState<Partial<Record<OAuthProvider, string>>>({});
  const [isSettingsLoaded, setIsSettingsLoaded] = useState(false);
  const isApiServiceStarting = false;
  const isApiServiceBusy = false;
  const [settings, setSettings] = useState<AppSettings>({
    theme: "system",
    autoLaunch: false,
    maskSensitive: false,
    apiServiceEnabled: false,
    apiServiceHost: "0.0.0.0",
    apiServicePort: DEFAULT_API_SERVICE_PORT,
    apiServiceDefaultModel: "claude-opus-4.7-medium",
  });
  const [apiServiceHostInput, setApiServiceHostInput] = useState("0.0.0.0");
  const [apiServicePortInput, setApiServicePortInput] = useState(String(DEFAULT_API_SERVICE_PORT));
  const apiServiceModels = useMemo<ApiServiceModel[]>(() => [], []);
  const [apiPref, setApiPrefState] = useState<ApiModelPref>(loadApiPref);
  const [isConfiguringClaude] = useState(false);
  const [isRestoringClaude] = useState(false);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const accountListRef = useRef<HTMLDivElement | null>(null);
  const pendingOAuthRef = useRef(pendingOAuth);
  const completeOAuthRef = useRef<CompleteOAuthFn | null>(null);
  const hasStartedStartupRefresh = useRef(false);
  const activeAccountRefreshInFlight = useRef<string | null>(null);
  const [accountScrollbar, setAccountScrollbar] = useState({
    visible: false,
    top: 0,
    height: 0,
  });
  const {
    ref: logListRef,
    state: logScrollbar,
    update: updateLogScrollbar,
  } = useVirtualScrollbar<HTMLDivElement>(isLogsOpen);
  const appWindow = useMemo(() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  }, []);
  const apiService = useMemo<ApiServiceStatus>(() => ({
    running: false,
    bindHost: settings.apiServiceHost,
    bindPort: settings.apiServicePort,
    actualPort: null,
    address: null,
    apiKey: "",
    defaultModel: resolveModelId(apiPref) ?? "",
    lastError: null,
  }), [settings.apiServiceHost, settings.apiServicePort, apiPref]);
  const apiServiceRunning = Boolean(apiService.running);

  useEffect(() => {
    pendingOAuthRef.current = pendingOAuth;
  }, [pendingOAuth]);

  const cancelPendingOAuth = useCallback(async (provider: OAuthProvider, loginId?: string) => {
    try {
      await invoke("cancel_oauth", {
        provider,
        loginId: loginId ?? null,
      });
    } catch {
      // 关闭弹窗时取消 OAuth 只是清理动作，不阻断 UI。
    } finally {
      setPendingOAuth((current) => {
        if (!current[provider]) return current;
        return { ...current, [provider]: undefined };
      });
    }
  }, []);

  const filteredAccounts = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return sortAccountsForView(accounts.filter((account) => {
      if (account.provider !== activeProvider) return false;
      if (!normalizedQuery) return true;
      return [account.email, account.displayName, account.plan, account.accountId]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(normalizedQuery));
    }));
  }, [accounts, activeProvider, query]);

  const totalAccountPages = Math.max(1, Math.ceil(filteredAccounts.length / ACCOUNT_PAGE_SIZE));
  const currentAccountPage = Math.min(accountPage, totalAccountPages);
  const shouldShowAccountPagination = filteredAccounts.length > ACCOUNT_PAGE_SIZE;
  const pagedAccounts = useMemo(() => {
    const start = (currentAccountPage - 1) * ACCOUNT_PAGE_SIZE;
    return filteredAccounts.slice(start, start + ACCOUNT_PAGE_SIZE);
  }, [filteredAccounts, currentAccountPage]);
  const isActiveProviderRefreshing = useMemo(
    () => refreshingProviders.has(activeProvider),
    [activeProvider, refreshingProviders],
  );

  const counts = useMemo(
    () => ({
      codex: accounts.filter((account) => account.provider === "codex").length,
      gemini: accounts.filter((account) => account.provider === "gemini").length,
      antigravity: accounts.filter((account) => account.provider === "antigravity").length,
      [PROVIDER_SUPERAI]: 0,
    }),
    [accounts],
  );
  const activeAccount = useMemo(
    () => accounts.find((account) => isCurrentAccount(account)) ?? null,
    [accounts],
  );

  const updateAccountScrollbar = useCallback(() => {
    const element = accountListRef.current;
    if (!element) {
      setAccountScrollbar({ visible: false, top: 0, height: 0 });
      return;
    }
    const { clientHeight, scrollHeight, scrollTop } = element;
    const visible = scrollHeight > clientHeight + 1;
    if (!visible) {
      setAccountScrollbar({ visible: false, top: 0, height: 0 });
      return;
    }
    const trackInset = 8;
    const trackHeight = Math.max(0, clientHeight - trackInset * 2);
    const height = Math.max(54, Math.round((clientHeight / scrollHeight) * trackHeight));
    const maxScrollTop = Math.max(1, scrollHeight - clientHeight);
    const maxThumbTop = Math.max(0, trackHeight - height);
    const top = trackInset + Math.round((scrollTop / maxScrollTop) * maxThumbTop);
    setAccountScrollbar((current) => {
      if (current.visible === visible && current.top === top && current.height === height) return current;
      return { visible, top, height };
    });
  }, []);

  useEffect(() => {
    updateAccountScrollbar();
    const element = accountListRef.current;
    const resizeObserver = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateAccountScrollbar);
    if (element) resizeObserver?.observe(element);
    window.addEventListener("resize", updateAccountScrollbar);
    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", updateAccountScrollbar);
    };
  }, [pagedAccounts.length, activeProvider, query, currentAccountPage, updateAccountScrollbar]);

  const { notice, closeNotice, showNotice, appendAppLog, appLogs, setAppLogs } = useNotice({
    sanitize: sanitizeUserFacingText,
    initialLogs: loadAppLogs(),
  });

  const showNormalizedError = useCallback((prefix: string, error: unknown) => {
    const keyIssue = resolveKeyIssueModal(error);
    if (keyIssue) {
      setKeyIssueModal(keyIssue);
    }
    showNotice("error", `${prefix}：${normalizeUserError(error)}`);
  }, [showNotice]);

  const prepareForUpdateInstall = useCallback(async () => {
    if (!isTauri()) return;
  }, []);

  const { forceUpdate, installForceUpdate } = useUpdater({
    appendAppLog,
    showError: (message) => showNotice("error", message),
    sanitize: sanitizeUserFacingText,
    beforeInstall: prepareForUpdateInstall,
  });

  const reloadAccountsSoon = useCallback((delay = 1800) => {
    window.setTimeout(() => {
      void invoke<ManagedAccount[]>("list_accounts")
        .then((storedAccounts) => setAccounts(sortAccountsForView(storedAccounts)))
        .catch(() => undefined);
    }, delay);
  }, []);

  const refreshAllAccountsOnLaunch = useCallback((storedAccounts: ManagedAccount[]) => {
    if (hasStartedStartupRefresh.current || storedAccounts.length === 0) return;
    hasStartedStartupRefresh.current = true;
    const startupAccountIds = Array.from(new Set(storedAccounts.map((account) => account.id)));

    setRefreshingAccountIds((current) => {
      const next = new Set(current);
      startupAccountIds.forEach((accountId) => next.add(accountId));
      return next;
    });

    void invoke<ManagedAccount[]>("refresh_all_accounts")
      .then((refreshedAccounts) => {
        const refreshedById = new Map(refreshedAccounts.map((account) => [account.id, account]));
        setAccounts((current) =>
          sortAccountsForView(
            (current.length > 0 ? current : refreshedAccounts).map((account) => refreshedById.get(account.id) ?? account),
          ),
        );
        showNotice("success", `启动检查完成，已刷新 ${refreshedAccounts.length} 个账号状态`);
      })
      .catch((error) => {
        reloadAccountsSoon(1200);
        showNormalizedError("启动检查账号状态失败", error);
      })
      .finally(() => {
        setRefreshingAccountIds((current) => {
          const next = new Set(current);
          startupAccountIds.forEach((accountId) => next.delete(accountId));
          return next;
        });
      });
  }, [reloadAccountsSoon, showNormalizedError, showNotice]);

  const refreshActiveAccountSilently = useCallback(async () => {
    if (!activeAccount) return;
    if (activeAccountRefreshInFlight.current) return;
    if (refreshingAccountIds.has(activeAccount.id)) return;

    activeAccountRefreshInFlight.current = activeAccount.id;
    try {
      const refreshed = await invoke<ManagedAccount>("refresh_account", { accountId: activeAccount.id });
      setAccounts((current) => sortAccountsForView(current.map((item) => (item.id === refreshed.id ? refreshed : item))));
    } catch {
      reloadAccountsSoon(1200);
    } finally {
      activeAccountRefreshInFlight.current = null;
    }
  }, [activeAccount, refreshingAccountIds, reloadAccountsSoon]);

  const refreshImportedAccountStatus = (importedAccounts: ManagedAccount[]) => {
    const accountIds = Array.from(new Set(importedAccounts.map((account) => account.id)));
    if (accountIds.length === 0) return;

    void (async () => {
      setRefreshingAccountIds((current) => {
        const next = new Set(current);
        accountIds.forEach((accountId) => next.add(accountId));
        return next;
      });
      for (const accountId of accountIds) {
        try {
          const refreshed = await invoke<ManagedAccount>("refresh_account", { accountId });
          setAccounts((current) => sortAccountsForView(current.map((item) => (item.id === refreshed.id ? refreshed : item))));
        } catch {
          reloadAccountsSoon(1200);
        } finally {
          setRefreshingAccountIds((current) => {
            const next = new Set(current);
            next.delete(accountId);
            return next;
          });
        }
      }
      reloadAccountsSoon(500);
    })();
  };

  const closeImportModal = () => {
    if (isImportBusy) return;
    const provider = activeProvider as OAuthProvider;
    const loginId = pendingOAuthRef.current[provider];
    if (loginId) void cancelPendingOAuth(provider, loginId);
    setIsImportModalOpen(false);
    setMode(defaultImportModeForProvider(activeProvider));
    setPasteValue("");
    setSuperaiBatchKeys("");
    setSuperaiPasswordEmail("");
    setSuperaiPasswordPwd("");
  };

  const applyImportResult = (
    result: BackendImportResult,
    options: { closeModal?: boolean; successText?: string; accountsToPersist?: ManagedAccount[] } = {},
  ) => {
    if (result.imported.length > 0) {
      setAccounts((current) => mergeAccounts(current, result.imported));
      const accountsToPersist = options.accountsToPersist ?? result.imported;
      if (accountsToPersist.length > 0) {
        void invoke("upsert_accounts", { accounts: accountsToPersist })
          .catch(() => undefined)
          .finally(() => refreshImportedAccountStatus(result.imported));
      } else {
        refreshImportedAccountStatus(result.imported);
        reloadAccountsSoon(300);
      }
      if (options.closeModal ?? true) {
        closeImportModal();
        setPasteValue("");
      }
      setAccountPage(1);
      const successText = options.successText ?? `已添加 ${result.imported.length} 个账号`;
      if (result.failed.length > 0) {
        appendAppLog("error", `导入时另有 ${result.failed.length} 项失败：${result.failed[0]?.reason ?? "未知错误"}`);
      }
      showNotice(
        result.failed.length > 0 ? "info" : "success",
        result.failed.length > 0 ? `${successText}，另有 ${result.failed.length} 项失败` : successText,
      );
    } else if (result.failed.length > 0) {
      showNotice("error", result.failed[0]?.reason ?? "未添加账号");
    } else {
      showNotice("info", "没有发现可添加的账号");
    }
  };

  const parseWithBackend = async (content: string, label: string) => {
    try {
      const result = await invoke<BackendImportResult>("import_accounts_from_json", {
        jsonContent: content,
        label,
        providerHint: activeProvider,
      });
      return { result, accountsToPersist: [] };
    } catch (backendError) {
      try {
        const result = parseAuthJson(content, "paste", label);
        return { result, accountsToPersist: result.imported };
      } catch {
        throw backendError;
      }
    }
  };

  const handleFileImport = async (files: FileList | null) => {
    if (!files?.length) return;
    if (isImportBusy) return;
    setIsImportBusy(true);
    setIsFileImporting(true);
    const allFailures: ImportFailure[] = [];
    const allImported: ManagedAccount[] = [];
    const accountsToPersist: ManagedAccount[] = [];
    try {
      for (const file of Array.from(files)) {
        const content = await file.text();
        const parsed = await parseWithBackend(content, file.name);
        const { result } = parsed;
        allImported.push(...result.imported);
        allFailures.push(...result.failed);
        accountsToPersist.push(...parsed.accountsToPersist);
      }
      if (allImported.length > 0) {
        applyImportResult(
          { imported: allImported, failed: allFailures },
          { accountsToPersist },
        );
      } else {
        showNotice("error", allFailures[0]?.reason ?? "没有发现可添加的账号");
      }
    } catch (error) {
      showNormalizedError("导入文件失败", error);
    } finally {
      setIsFileImporting(false);
      setIsImportBusy(false);
    }
  };

  const handlePasteImport = async () => {
    if (isImportBusy) return;
    setIsImportBusy(true);
    try {
      const { result, accountsToPersist } = await parseWithBackend(pasteValue, "粘贴内容");
      applyImportResult(result, { accountsToPersist });
    } catch (error) {
      showNormalizedError("解析粘贴内容失败", error);
    } finally {
      setIsImportBusy(false);
    }
  };

  const handleLocalImport = async (provider: OAuthProvider) => {
    if (isImportBusy) return;
    if (provider === "antigravity") {
      showNotice("error", "Antigravity 暂不支持本机导入，请使用 OAuth 授权或粘贴/上传 JSON");
      return;
    }
    setIsImportBusy(true);
    try {
      const command = provider === "codex" ? "import_codex_from_local" : "import_gemini_from_local";
      const result = await invoke<BackendImportResult>(command);
      applyImportResult(
        result,
        { accountsToPersist: [] },
      );
    } catch (error) {
      showNormalizedError(`读取本机 ${providerLabel(provider)} 失败`, error);
    } finally {
      setIsImportBusy(false);
    }
  };

  const completeOAuth = async (
    provider: OAuthProvider,
    loginId: string,
    options: { silent?: boolean } = {},
  ) => {
    const command =
      provider === "codex" ? "complete_codex_oauth"
      : provider === "gemini" ? "complete_gemini_oauth"
      : "complete_antigravity_oauth";
    try {
      const result = await invoke<BackendImportResult>(command, { loginId });
      if (result.imported.length === 0) {
        if (!options.silent && result.failed.length > 0) {
          applyImportResult(result, { closeModal: false });
        }
        return false;
      }
      applyImportResult(
        result,
        { successText: `${providerLabel(provider)} OAuth 登录成功，已添加 ${result.imported.length} 个账号`, accountsToPersist: [] },
      );
      setPendingOAuth((current) => ({ ...current, [provider]: undefined }));
      return true;
    } catch (error) {
      if (!options.silent) {
        showNormalizedError(`${providerLabel(provider)} OAuth 完成失败`, error);
        setPendingOAuth((current) => ({ ...current, [provider]: undefined }));
      }
      return false;
    }
  };

  const handleOAuthStart = async (provider: OAuthProvider) => {
    if (isImportBusy) return;
    setIsImportBusy(true);
    try {
      const command =
        provider === "codex" ? "start_codex_oauth"
        : provider === "gemini" ? "start_gemini_oauth"
        : "start_antigravity_oauth";
      const result = await invoke<OAuthStartResult>(command);
      setPendingOAuth((current) => ({ ...current, [provider]: result.login_id }));
      showNotice("info", result.message);
    } catch (error) {
      showNormalizedError(`${providerLabel(provider)} OAuth 启动失败`, error);
    } finally {
      setIsImportBusy(false);
    }
  };

  const selectedMode = modeConfig[mode];
  const ModeIcon = selectedMode.icon;
  const isActiveProviderOAuthPending =
    activeProvider !== PROVIDER_SUPERAI && Boolean(pendingOAuth[activeProvider as OAuthProvider]);
  const oauthAccountLabel =
    activeProvider === "codex" ? "OpenAI"
    : activeProvider === "gemini" ? "Gemini"
    : activeProvider === "antigravity" ? "Antigravity"
    : "OpenAI";
  const localImportDesc =
    activeProvider === "codex" ? "从本地已登录的会话中导入 Codex 账号" : "从本地已登录的会话中导入 Gemini Cli 账号";
  const selectedModeDesc =
    mode === "oauth"
      ? `点击下方按钮，在浏览器中完成 ${oauthAccountLabel} 账号 OAuth 授权。`
      : mode === "local"
        ? localImportDesc
        : selectedMode.desc;
  const isInteractiveDragTarget = (target: EventTarget | null) =>
    target instanceof HTMLElement &&
    Boolean(target.closest("button, input, textarea, select, a, label, [role='button'], [contenteditable='true']"));

  const requestWindowDrag = () => {
    void invoke("start_window_drag").catch(() => {
      void appWindow?.startDragging().catch(() => {
        console.info("窗口拖拽未启动：请拖动窗口顶部空白区域。");
      });
    });
  };

  const startWindowDrag = (event: React.MouseEvent<HTMLElement>) => {
    if (isInteractiveDragTarget(event.target)) return;
    if (event.button !== 0) return;
    event.preventDefault();
    requestWindowDrag();
  };
  const handleShellTopDrag = (event: React.MouseEvent<HTMLElement>) => {
    if (event.button !== 0 || event.clientY > 70) return;
    if (event.clientX < 280 && window.innerWidth > 980) return;
    if (isInteractiveDragTarget(event.target)) return;
    if (event.target instanceof HTMLElement && event.target.closest(".modal-content")) return;
    event.preventDefault();
    event.stopPropagation();
    requestWindowDrag();
  };
  const handleModalBackdropMouseDown = (
    event: React.MouseEvent<HTMLDivElement>,
    close: () => void,
  ) => {
    if (event.target !== event.currentTarget) return;
    close();
  };
  const handleAddAccount = () => {
    if (isImportBusy) return;
    setMode(defaultImportModeForProvider(activeProvider));
    setIsImportModalOpen(true);
  };

  const handleSuperaiPasswordImport = async () => {
    showNotice("info", "SuperAI 账号导入界面已保留，但内部接入已移除。");
  };

  const handleSuperaiBatchKeyImport = async () => {
    showNotice("info", "SuperAI 批量密钥导入界面已保留，但内部接入已移除。");
  };

  const handleProviderChange = (provider: Provider) => {
    setActiveProvider(provider);
    setSelectedExportIds(new Set());
    setAccountPage(1);
  };
  const handleSearchChange = (value: string) => {
    setQuery(value);
    setSelectedExportIds(new Set());
    setAccountPage(1);
  };
  const handleSettings = () => {
    setIsSettingsOpen(true);
  };
  const handleLogs = () => {
    setAppLogs((current) => pruneAppLogs(current));
    setIsLogsOpen(true);
  };
  const handleAbout = async () => {
    if (!aboutInfo) {
      try {
        const [name, version] = await Promise.all([getName(), getVersion()]);
        setAboutInfo({ name, version });
      } catch (error) {
        setAboutInfo({ name: APP_NAME, version: "unknown" });
        showNormalizedError("读取版本信息失败", error);
      }
    }
    setIsAboutOpen(true);
  };
  const updateSetting = <Key extends keyof typeof settings>(key: Key, value: (typeof settings)[Key]) => {
    setSettings((current) => ({ ...current, [key]: value }));
  };
  const updateApiServiceHost = (value: string) => {
    const ipText = value.replace(/[^\d.]/g, "").slice(0, 15);
    setApiServiceHostInput(ipText);
    const next = parseApiServiceHost(ipText);
    if (next !== null) updateSetting("apiServiceHost", next);
  };
  const commitApiServiceHost = () => {
    const next = parseApiServiceHost(apiServiceHostInput);
    if (next !== null) {
      setApiServiceHostInput(next);
      updateSetting("apiServiceHost", next);
      return;
    }
    setApiServiceHostInput(settings.apiServiceHost);
  };
  const updateApiServicePort = (value: string) => {
    const digitsOnly = value.replace(/\D/g, "").slice(0, 5);
    setApiServicePortInput(digitsOnly);
    const next = parseApiServicePort(digitsOnly);
    if (next !== null) updateSetting("apiServicePort", next);
  };
  const commitApiServicePort = () => {
    const next = parseApiServicePort(apiServicePortInput);
    if (next !== null) {
      setApiServicePortInput(String(next));
      updateSetting("apiServicePort", next);
      return;
    }
    setApiServicePortInput(String(settings.apiServicePort));
  };
  const handleToggleAccount = async (account: ManagedAccount) => {
    if (isCurrentAccount(account)) return;
    if ((account.status ?? fallbackStatus(account)).state === "unavailable") return;
    if (switchingAccountId) return;
    setSwitchingAccountId(account.id);
    try {
      const providerAccounts = await invoke<SwitchAccountResult>("switch_account", { accountId: account.id });
      setAccounts((current) =>
        sortAccountsForView(current.map((item) => providerAccounts.find((changed) => changed.id === item.id) ?? item)),
      );
      setAccountPage(1);
      showNotice("success", activationSuccessMessage(account));
    } catch (error) {
      showNormalizedError("启用账号失败", error);
    } finally {
      setSwitchingAccountId(null);
    }
  };

  const handleRefreshAccount = async (account: ManagedAccount) => {
    if (refreshingAccountIds.has(account.id) || refreshingProviders.has(account.provider)) return;
    setRefreshingAccountIds((current) => new Set(current).add(account.id));
    try {
      const refreshed = await invoke<ManagedAccount>("refresh_account", { accountId: account.id });
      setAccounts((current) => sortAccountsForView(current.map((item) => (item.id === refreshed.id ? refreshed : item))));
      if (isRefreshUnavailable(refreshed)) {
        showNotice("error", refreshFailureMessage(refreshed));
      } else {
        showNotice("success", `已刷新 ${accountActionLabel(refreshed)}`);
      }
    } catch (error) {
      showNormalizedError("刷新账号失败", error);
    } finally {
      setRefreshingAccountIds((current) => {
        const next = new Set(current);
        next.delete(account.id);
        return next;
      });
    }
  };
  const handleRefreshVisibleAccounts = async () => {
    const provider = activeProvider;
    if (refreshingProviders.has(provider)) return;
    setRefreshingProviders((current) => {
      const next = new Set(current);
      next.add(provider);
      return next;
    });
    try {
      const refreshedAccounts = await invoke<ManagedAccount[]>("refresh_provider_accounts", { provider });
      setAccounts((current) =>
        sortAccountsForView(current.map((item) => refreshedAccounts.find((changed) => changed.id === item.id) ?? item)),
      );
      const failedAccounts = refreshedAccounts.filter(isRefreshUnavailable);
      if (failedAccounts.length > 0) {
        showNotice(
          "error",
          failedAccounts.length === 1
            ? refreshFailureMessage(failedAccounts[0])
            : `已刷新 ${providerLabel(provider)} 账号，其中 ${failedAccounts.length} 个失败。首个原因：${
                failedAccounts[0].status?.reason ?? failedAccounts[0].quota?.error ?? "未知错误"
              }`,
        );
      } else {
        showNotice("success", `已刷新 ${providerLabel(provider)} 账号`);
      }
    } catch (error) {
      showNormalizedError(`刷新 ${providerLabel(provider)} 失败`, error);
    } finally {
      setRefreshingProviders((current) => {
        const next = new Set(current);
        next.delete(provider);
        return next;
      });
    }
  };
  const exportFileBase = (label: string) => label.replace(/[^a-z0-9._-]+/gi, "_");
  const handleExportAccount = async (account: ManagedAccount) => {
    if (isExportBusy) return;
    setIsExportBusy(true);
    let payload: string;
    try {
      payload = await invoke<string>("export_account", { accountId: account.id });
    } catch (error) {
      showNormalizedError("导出账号失败", error);
      return;
    } finally {
      setIsExportBusy(false);
    }
    setExportPreview({
      payload,
      kind: "json",
      label: accountDisplayLabel(account),
      fileBase: `${account.provider}-${exportFileBase(accountDisplayLabel(account))}`,
    });
  };
  const toggleExportSelection = (accountId: string) => {
    setSelectedExportIds((current) => {
      const next = new Set(current);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  };
  const handleBatchExport = async () => {
    if (isExportBusy) return;
    const selectedAccounts = filteredAccounts.filter((account) => selectedExportIds.has(account.id));
    if (selectedAccounts.length === 0) {
      showNotice("error", "请选择要导出的账号");
      return;
    }
    setIsExportBusy(true);
    try {
      const exported = await Promise.all(
        selectedAccounts.map(async (account) => {
          const payload = await invoke<string>("export_account", { accountId: account.id });
          return { payload, kind: "json" as const };
        }),
      );
      const payload = JSON.stringify(exported.map((item) => JSON.parse(item.payload)), null, 2);
      setExportPreview({
        payload,
        kind: "json",
        label: `${providerLabel(activeProvider)} · ${exported.length} 个账号`,
        fileBase: `${activeProvider}-${exported.length}-accounts`,
      });
      setSelectedExportIds(new Set());
    } catch (error) {
      showNormalizedError("批量导出失败", error);
    } finally {
      setIsExportBusy(false);
    }
  };
  const handleBatchDelete = () => {
    const selectedAccounts = filteredAccounts.filter((account) => selectedExportIds.has(account.id));
    if (selectedAccounts.length === 0) {
      showNotice("error", "请先勾选要删除的账号");
      return;
    }
    setPendingBatchDelete(selectedAccounts);
  };
  const confirmBatchDelete = async () => {
    if (!pendingBatchDelete || isAccountBusy) return;
    const targets = pendingBatchDelete;
    setIsAccountBusy(true);
    try {
      let nextAccounts = accounts;
      for (const account of targets) {
        nextAccounts = await invoke<ManagedAccount[]>("delete_account", { accountId: account.id });
      }
      setAccounts(sortAccountsForView(nextAccounts));
      setSelectedExportIds(new Set());
      setPendingBatchDelete(null);
      const providers = Array.from(new Set(targets.map((account) => providerLabel(account.provider))));
      const scope = providers.length === 1 ? `${providers[0]} ` : "";
      showNotice("success", `已删除 ${scope}${targets.length} 个账号`);
    } catch (error) {
      showNormalizedError("批量删除失败", error);
    } finally {
      setIsAccountBusy(false);
    }
  };
  const downloadExportPreview = (preview: ExportPreview) => {
    const isKey = preview.kind === "key";
    const blob = new Blob([preview.payload], { type: isKey ? "text/plain;charset=utf-8" : "application/json;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${preview.fileBase}.${isKey ? "txt" : "json"}`;
    a.click();
    URL.revokeObjectURL(url);
    showNotice("success", `已下载 ${preview.label}`);
  };
  const copyExportPreview = async (preview: ExportPreview) => {
    try {
      await navigator.clipboard.writeText(preview.payload);
      showNotice("success", preview.kind === "key" ? "账号密钥已复制" : "账号 JSON 已复制");
    } catch (error) {
      showNotice("error", `复制失败：${String(error)}`);
    }
  };
  const handleDeleteAccount = (account: ManagedAccount) => {
    setPendingDeleteAccount(account);
  };

  const confirmDeleteAccount = async () => {
    if (!pendingDeleteAccount || isDeletingAccount) return;
    const account = pendingDeleteAccount;
    setIsDeletingAccount(true);
    try {
      const nextAccounts = await invoke<ManagedAccount[]>("delete_account", { accountId: account.id });
      setAccounts(sortAccountsForView(nextAccounts));
      setPendingDeleteAccount(null);
      showNotice("success", `已删除 ${accountActionLabel(account)}`);
    } catch (error) {
      showNormalizedError("删除账号失败", error);
    } finally {
      setIsDeletingAccount(false);
    }
  };
  const handleOpenStore = (event: React.MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    void openUrl("https://ai.talentisan.cn/");
  };

  useEffect(() => {
    void invoke<ManagedAccount[]>("list_accounts")
      .then((storedAccounts) => {
        setAccounts(sortAccountsForView(storedAccounts));
        refreshAllAccountsOnLaunch(storedAccounts);
      })
      .catch((error) => {
        showNormalizedError("读取账号列表失败", error);
      });

    void invoke<AppSettings | null>("load_settings")
      .then((storedSettings) => {
        if (storedSettings) {
          setSettings(storedSettings);
          setApiServiceHostInput(storedSettings.apiServiceHost);
          setApiServicePortInput(String(storedSettings.apiServicePort));
        }
      })
      .catch(() => undefined)
      .finally(() => setIsSettingsLoaded(true));

    return undefined;
  }, [refreshAllAccountsOnLaunch, showNotice, showNormalizedError]);

  useEffect(() => {
    if (!isSettingsLoaded) return;
    void invoke("save_settings", { settings }).catch((error) => {
      showNormalizedError("保存设置失败", error);
    });
  }, [isSettingsLoaded, settings, showNormalizedError]);

  useEffect(() => {
    persistAppLogs(appLogs);
  }, [appLogs]);

  useEffect(() => {
    if (!isTauri() || !apiServiceRunning || apiServiceModels.length === 0) return;
    const availableSet = new Set(apiServiceModels.map((model) => model.id));
    const selectedFamily = MODEL_FAMILIES.find((family) => family.key === apiPref.family);
    if (selectedFamily && isFamilyAvailable(selectedFamily, availableSet)) return;

    const fallbackFamily = MODEL_FAMILIES.find((family) => isFamilyAvailable(family, availableSet));
    if (!fallbackFamily) return;
    const timer = window.setTimeout(() => {
      setApiPrefState((prev) => {
        if (prev.family === fallbackFamily.key) return prev;
        const next = {
          family: fallbackFamily.key,
          effort: fallbackFamily.efforts.length === 0
            ? null
            : fallbackFamily.defaultEffort ?? fallbackFamily.efforts[0],
        };
        persistApiPref(next);
        return next;
      });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [apiPref.family, apiServiceModels, apiServiceRunning, showNormalizedError]);

  useEffect(() => {
    completeOAuthRef.current = completeOAuth;
  });

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    void listen<OAuthCallbackEvent>("oauth-callback-received", (event) => {
      const provider = event.payload?.provider;
      const loginId = event.payload?.loginId;
      if (!provider || !loginId) return;
      if (pendingOAuthRef.current[provider] !== loginId) return;
      showNotice("info", `${providerLabel(provider)} 已收到授权回调，正在完成登录…`);
      const runComplete = completeOAuthRef.current;
      if (!runComplete) return;
      void runComplete(provider, loginId, { silent: false });
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [showNotice]);

  const toggleApiService = useCallback(async () => {
    if (isApiServiceBusy || isApiServiceStarting) return;
    showNotice("info", "API 服务目前仅保留 UI 壳子，后端能力已移除。");
  }, [isApiServiceBusy, isApiServiceStarting, showNotice]);

  const applyApiPref = useCallback(
    (updater: (prev: ApiModelPref) => ApiModelPref) => {
      setApiPrefState((prev) => {
        const next = updater(prev);
        persistApiPref(next);
        return next;
      });
    },
    [],
  );

  const handleChangeFamily = useCallback(
    (familyKey: string) => {
      applyApiPref((prev) => {
        const fam = MODEL_FAMILIES.find((f) => f.key === familyKey);
        if (!fam) return prev;
        return {
          family: fam.key,
          effort: fam.efforts.length === 0 ? null : fam.defaultEffort ?? fam.efforts[0],
        };
      });
    },
    [applyApiPref],
  );

  const handleChangeEffort = useCallback(
    (effort: EffortKey) => {
      applyApiPref((prev) => ({ ...prev, effort }));
    },
    [applyApiPref],
  );

  const configureClaudeApp = useCallback(async () => {
    showNotice("info", "API 服务目前仅保留 UI 壳子，Claude 配置能力暂未接入。");
  }, [showNotice]);

  const restoreClaudeApp = useCallback(async () => {
    showNotice("info", "API 服务目前仅保留 UI 壳子，Claude 恢复能力暂未接入。");
  }, [showNotice]);

  const copyApiServiceText = useCallback(
    async (text: string, label: string) => {
      if (!text) return;
      try {
        await navigator.clipboard.writeText(text);
        showNotice("success", `已复制${label}`);
      } catch (error) {
        showNotice("error", `复制${label}失败：${String(error)}`);
      }
    },
    [showNotice],
  );

  useEffect(() => {
    if (!activeAccount) return undefined;
    const timer = window.setInterval(() => {
      void refreshActiveAccountSilently();
    }, ACTIVE_ACCOUNT_REFRESH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [activeAccount, refreshActiveAccountSilently]);

  useEffect(() => {
    const root = document.documentElement;
    if (settings.theme === "system") {
      delete root.dataset.theme;
    } else {
      root.dataset.theme = settings.theme;
    }
  }, [settings.theme]);

  return (
    <main
      className={clsx(
        "shell",
        settings.maskSensitive && "privacy-mask",
        (isImportModalOpen || isSettingsOpen || isLogsOpen || isAboutOpen || isApiConfigOpen || exportPreview || pendingDeleteAccount || pendingBatchDelete || forceUpdate || keyIssueModal) && "modal-active",
      )}
      onMouseDownCapture={handleShellTopDrag}
    >
      <div className="global-drag-region" data-tauri-drag-region onMouseDown={startWindowDrag} />
      <div className="top-edge-drag-region" />
      <aside className="sidebar">
        <div className="window-strip drag-surface" data-tauri-drag-region onMouseDown={startWindowDrag} />
        <div className="brand">
          <div className="brand-mark">
            <img src={logoUrl} alt="" />
          </div>
          <div>
            <strong>{APP_NAME}</strong>
            <span>账号管理工具</span>
          </div>
        </div>

        <nav className="nav-list" aria-label="Providers">
          <button className={clsx(activeProvider === PROVIDER_SUPERAI && "active")} onClick={() => handleProviderChange(PROVIDER_SUPERAI)}>
            <SuperaiIcon className="provider-nav-icon superai" />
            <span>{APP_NAME}</span>
            <b>{counts[PROVIDER_SUPERAI]}</b>
          </button>
          <button className={clsx(activeProvider === "codex" && "active")} onClick={() => handleProviderChange("codex")}>
            <CodexIcon className="provider-nav-icon codex" />
            <span>Codex</span>
            <b>{counts.codex}</b>
          </button>
          <button className={clsx(activeProvider === "gemini" && "active")} onClick={() => handleProviderChange("gemini")}>
            <GeminiIcon className="provider-nav-icon gemini" />
            <span>Gemini Cli</span>
            <b>{counts.gemini}</b>
          </button>
          <button className={clsx(activeProvider === "antigravity" && "active")} onClick={() => handleProviderChange("antigravity")}>
            <AntigravityIcon className="provider-nav-icon antigravity" />
            <span>Antigravity</span>
            <b>{counts.antigravity}</b>
          </button>
        </nav>

        <div className="sidebar-footer">
          <SidebarAdCard onOpenStore={handleOpenStore} />
          <div className="sidebar-divider" />
          <button onClick={handleLogs}>
            <ScrollText size={18} />
            日志
          </button>
          <button onClick={handleSettings}>
            <Settings size={18} />
            设置
          </button>
          <button onClick={handleAbout}>
            <Info size={18} />
            关于
          </button>
        </div>
      </aside>

      <section className="workspace">
        <section className="content-grid">
          <div className={clsx("accounts-panel", shouldShowAccountPagination && "has-pagination")}>
            <div className="accounts-toolbar">
              <div className="toolbar-meta">
                  {providerLabel(activeProvider)} · {filteredAccounts.length} 个匹配项
              </div>
              <div className="panel-actions">
                <label className="search">
                  <Search size={18} />
                  <input value={query} onChange={(event) => handleSearchChange(event.target.value)} placeholder="搜索邮箱、计划或账号 ID" />
                </label>
                <button className="primary" onClick={handleAddAccount}>
                  <Plus size={18} />
                  添加
                </button>
                <button className="secondary refresh-all" onClick={handleRefreshVisibleAccounts} disabled={isActiveProviderRefreshing}>
                  <RotateCw size={18} className={clsx(isActiveProviderRefreshing && "spin")} />
                  刷新
                </button>
                <button
                  className="secondary"
                  onClick={() => void handleBatchExport()}
                  disabled={isAccountBusy || isExportBusy || selectedExportIds.size === 0}
                  title={selectedExportIds.size === 0 ? "请先勾选账号" : `导出选中的 ${selectedExportIds.size} 个账号`}
                >
                  <Download size={18} className={clsx(isExportBusy && "spin")} />
                  {isExportBusy ? "生成中" : `导出${selectedExportIds.size > 0 ? ` (${selectedExportIds.size})` : ""}`}
                </button>
                <button
                  className="secondary danger-action"
                  onClick={() => handleBatchDelete()}
                  disabled={isAccountBusy || selectedExportIds.size === 0}
                  title={selectedExportIds.size === 0 ? "请先勾选账号" : `删除选中的 ${selectedExportIds.size} 个账号`}
                >
                  <Trash2 size={18} />
                  删除{selectedExportIds.size > 0 ? ` (${selectedExportIds.size})` : ""}
                </button>
              </div>
            </div>
            <div className="account-scroll-frame">
              <div className="account-scroll-shell" ref={accountListRef} onScroll={updateAccountScrollbar}>
                <div className="account-list card-mode">
                  {activeProvider === PROVIDER_SUPERAI && (
                    <ApiServiceCard
                      status={apiService}
                      busy={isApiServiceBusy}
                      starting={isApiServiceStarting}
                      pref={apiPref}
                      onToggleService={() => void toggleApiService()}
                      onCopy={(text, label) => void copyApiServiceText(text, label)}
                      onOpenConfig={() => setIsApiConfigOpen(true)}
                    />
                  )}
                  {filteredAccounts.length === 0 && activeProvider !== PROVIDER_SUPERAI && (
                    <div className="empty-state">
                      <SearchX size={48} strokeWidth={1.55} />
                      <strong>暂无账号</strong>
                    </div>
                  )}
                  {pagedAccounts.map((account) => (
                    <article
                      className={clsx(
                        "account-row",
                        account.status?.state === "unavailable" && "disabled",
                        isCurrentAccount(account) && "current",
                        "selectable",
                        selectedExportIds.has(account.id) && "selected",
                      )}
                      key={account.id}
                      onClick={() => {
                        toggleExportSelection(account.id);
                      }}
                    >
                      {selectedExportIds.has(account.id) && (
                        <span className="card-selected-mark" aria-hidden="true">
                          <Check size={24} strokeWidth={2.25} />
                        </span>
                      )}
                      <AccountStateCorner account={account} />
                      <div className="account-main">
                        <div className="account-title">
                          <strong>{accountCardTitle(account)}</strong>
                          <AccountPlanBadge account={account} />
                        </div>
                        <div className="account-subtitle">
                          <>
                            <span>
                              <b>名称</b>
                              {account.email}
                            </span>
                            {account.accountId && (
                              <span>
                                <b>账号</b>
                                {account.accountId}
                              </span>
                            )}
                          </>
                          {account.organizationId && (
                            <span>
                              <b>组织</b>
                              {account.organizationId}
                            </span>
                          )}
                        </div>
                      </div>
                      <QuotaMeters account={account} />
                      <ValidityMeter account={account} />
                      <div className="account-footer">
                        <time className="account-stamp">{account.status?.state === "unavailable" ? "--" : formatRelative(account.updatedAt)}</time>
                        <div className="account-actions" onClick={(event) => event.stopPropagation()}>
                        <button
                          className="icon-button"
                          aria-label={isCurrentAccount(account) ? "当前账号" : "设为当前账号"}
                          title={isCurrentAccount(account) ? "当前账号" : "设为当前"}
                          onClick={() => handleToggleAccount(account)}
                          disabled={
                            isCurrentAccount(account) ||
                            switchingAccountId !== null ||
                            (account.status ?? fallbackStatus(account)).state === "unavailable"
                          }
                        >
                          <BadgeCheck
                            size={15}
                            strokeWidth={1.75}
                            className={clsx(switchingAccountId === account.id && "spin")}
                          />
                        </button>
                        {account.provider === "antigravity" && (
                          <button
                            className="icon-button"
                            aria-label="查看账号明细"
                            title="查看明细"
                            onClick={() => setDetailsAccountId(account.id)}
                          >
                            <Eye size={15} strokeWidth={1.75} />
                          </button>
                        )}
                        <button
                          className="icon-button"
                          aria-label="刷新账号"
                          title="刷新"
                          onClick={() => handleRefreshAccount(account)}
                          disabled={
                            refreshingAccountIds.has(account.id) ||
                            refreshingProviders.has(account.provider)
                          }
                        >
                          <RefreshCw size={15} strokeWidth={1.75} className={clsx(refreshingAccountIds.has(account.id) && "spin")} />
                        </button>
                        <button
                          className="icon-button"
                          aria-label={isExportBusy ? "正在生成导出" : "导出账号"}
                          title={isExportBusy ? "正在生成导出" : "导出"}
                          onClick={() => void handleExportAccount(account)}
                          disabled={isExportBusy}
                        >
                          <Download size={15} strokeWidth={1.75} className={clsx(isExportBusy && "spin")} />
                        </button>
                        <button className="icon-button danger" aria-label="删除账号" title="删除" onClick={() => handleDeleteAccount(account)}>
                          <Trash2 size={15} strokeWidth={1.75} />
                        </button>
                        </div>
                      </div>
                    </article>
                  ))}
                </div>
              </div>
              <div className={clsx("account-scrollbar", accountScrollbar.visible && "visible")} aria-hidden="true">
                <i style={{ height: accountScrollbar.height, transform: `translateY(${accountScrollbar.top}px)` }} />
              </div>
            </div>
            {shouldShowAccountPagination && (
              <div className="account-pagination" aria-label="账号分页">
                <span>
                  第 {currentAccountPage} / {totalAccountPages} 页 · 每页 {ACCOUNT_PAGE_SIZE} 个
                </span>
                <div>
                  <button
                    className="secondary pagination-button"
                    onClick={() => setAccountPage((page) => Math.max(1, Math.min(page, totalAccountPages) - 1))}
                    disabled={currentAccountPage <= 1}
                  >
                    <ChevronLeft size={16} />
                    上一页
                  </button>
                  <button
                    className="secondary pagination-button"
                    onClick={() => setAccountPage((page) => Math.min(totalAccountPages, Math.max(page, 1) + 1))}
                    disabled={currentAccountPage >= totalAccountPages}
                  >
                    下一页
                    <ChevronRight size={16} />
                  </button>
                </div>
              </div>
            )}
          </div>
        </section>
      </section>

      {notice && (
        <NoticeToast notice={notice} onClose={closeNotice} sanitize={sanitizeUserFacingText} />
      )}

      {forceUpdate && (
        <ForceUpdateModal
          state={forceUpdate}
          onInstall={() => void installForceUpdate()}
          onViewLogs={handleLogs}
        />
      )}

      {keyIssueModal && (
        <AppModal
          title={keyIssueModal.title}
          description="本地加密密钥异常"
          closeLabel="关闭密钥异常提示"
          className="settings-panel"
          onClose={() => setKeyIssueModal(null)}
        >
          <div className="settings-body">
            <section className="setting-block">
              <div className="setting-copy">
                <CircleAlert size={18} />
                <div>
                  <strong>{keyIssueModal.title}</strong>
                  <p>{keyIssueModal.message}</p>
                </div>
              </div>
            </section>

            <section className="setting-block">
              <div className="setting-copy">
                <Info size={18} />
                <div>
                  <strong>处理方式</strong>
                  <p>如果你有原机器上的数据目录，请优先恢复本机主密钥文件及其 `.bak` 备份。若无法恢复，就删除受影响的 SuperAI 账号后重新导入。</p>
                </div>
              </div>
            </section>

            <div className="api-config-actions">
              <button type="button" className="superai-api-secondary" onClick={handleLogs}>
                <ScrollText size={14} />
                打开日志
              </button>
              <button type="button" className="primary" onClick={() => setKeyIssueModal(null)}>
                知道了
              </button>
            </div>
          </div>
        </AppModal>
      )}

      {detailsAccountId && (() => {
        const target = accounts.find((account) => account.id === detailsAccountId);
        if (!target) return null;
        return (
          <AccountDetailsDialog
            account={target}
            onClose={() => setDetailsAccountId(null)}
          />
        );
      })()}

      {isImportModalOpen && (
        <AppModal
          title="添加账号"
          description="选择添加方式"
          closeLabel="关闭添加账号"
          className="import-panel"
          onClose={closeImportModal}
        >
            <div className="mode-grid">
              {importModesForProvider(activeProvider).map((key) => {
                const item = modeConfig[key];
                const Icon = item.icon;
                return (
                  <button key={key} className={clsx("mode-button", mode === key && "active")} onClick={() => setMode(key)}>
                    <Icon size={16} />
                    <span>{item.title}</span>
                  </button>
                );
              })}
            </div>

            <div className="import-body">
              <div className="import-title">
                <ModeIcon size={24} />
                <div>
                  <strong>{selectedMode.title}</strong>
                  <p>{selectedModeDesc}</p>
                </div>
              </div>

              {mode === "paste" && activeProvider !== PROVIDER_SUPERAI && (
                <>
                  <textarea
                    value={pasteValue}
                    onChange={(event) => setPasteValue(event.target.value)}
                    spellCheck={false}
                    placeholder={'{\n  "tokens": {\n    "id_token": "...",\n    "access_token": "...",\n    "refresh_token": "..."\n  }\n}'}
                  />
                  <button className="wide primary" onClick={handlePasteImport} disabled={!pasteValue.trim() || isImportBusy}>
                    <Clipboard size={20} />
                    {isImportBusy ? "处理中..." : "解析并添加"}
                  </button>
                </>
              )}

              {mode === "file" && (
                <>
                  <input
                    ref={fileInputRef}
                    type="file"
                    accept="application/json,.json"
                    multiple
                    hidden
                    onChange={(event) => handleFileImport(event.target.files)}
                  />
                  <button className={clsx("drop-zone", isFileImporting && "loading")} onClick={() => fileInputRef.current?.click()} disabled={isImportBusy}>
                    <Upload size={28} />
                    <strong>{isFileImporting ? "正在导入 JSON..." : "选择 JSON 文件"}</strong>
                    <span>{isFileImporting ? "正在解析并刷新账号信息" : "支持 auth.json、oauth_creds.json、导出数组"}</span>
                  </button>
                </>
              )}

              {mode === "local" && activeProvider !== PROVIDER_SUPERAI && (
                <>
                  <button className="drop-zone local-import-button" onClick={() => handleLocalImport(activeProvider as OAuthProvider)} disabled={isImportBusy}>
                    <FolderDown size={28} />
                    <strong>{isImportBusy ? "正在读取本机账号..." : `读取 ${providerLabel(activeProvider)} 本机账号`}</strong>
                    <span>{providerLabel(activeProvider)} 本机凭证只在当前设备处理</span>
                  </button>
                </>
              )}

              {mode === "oauth" && activeProvider !== PROVIDER_SUPERAI && (
                <div className="oauth-flow">
                  <button className={clsx("drop-zone", isActiveProviderOAuthPending && "oauth-pending")} onClick={() => handleOAuthStart(activeProvider as OAuthProvider)}>
                    <LockKeyhole size={28} />
                    <strong>{isActiveProviderOAuthPending ? "重新打开授权" : "在浏览器中打开"}</strong>
                    <span>{isActiveProviderOAuthPending ? "浏览器关闭或卡住时可重新发起" : `${oauthAccountLabel} OAuth 授权将在浏览器中完成`}</span>
                  </button>
                </div>
              )}

              {mode === "batchKey" && activeProvider === PROVIDER_SUPERAI && (
                <>
                  <textarea
                    value={superaiBatchKeys}
                    onChange={(event) => setSuperaiBatchKeys(event.target.value)}
                    placeholder="一行一个批量密钥"
                    spellCheck={false}
                    disabled={isImportBusy}
                  />
                  <button
                    className="wide primary"
                    onClick={() => void handleSuperaiBatchKeyImport()}
                    disabled={!superaiBatchKeys.trim() || isImportBusy}
                  >
                    <Clipboard size={20} />
                    {isImportBusy ? "导入中..." : "批量导入"}
                  </button>
                </>
              )}

              {mode === "password" && activeProvider === PROVIDER_SUPERAI && (
                <div className="superai-password-form">
                  <label className="field">
                    <span>邮箱</span>
                    <input
                      type="email"
                      value={superaiPasswordEmail}
                      onChange={(event) => setSuperaiPasswordEmail(event.target.value)}
                      placeholder="name@example.com"
                      autoComplete="off"
                      spellCheck={false}
                      disabled={isImportBusy}
                    />
                  </label>
                  <label className="field">
                    <span>密码</span>
                    <input
                      type="password"
                      value={superaiPasswordPwd}
                      onChange={(event) => setSuperaiPasswordPwd(event.target.value)}
                      placeholder={`${APP_NAME} 登录密码`}
                      autoComplete="new-password"
                      spellCheck={false}
                      disabled={isImportBusy}
                    />
                  </label>
                  <p className="superai-password-tip">界面已保留；内部 superai 接入已移除，当前不会真的导入账号。</p>
                  <button
                    className="wide primary"
                    onClick={() => void handleSuperaiPasswordImport()}
                    disabled={!superaiPasswordEmail.trim() || !superaiPasswordPwd || isImportBusy}
                  >
                    <LockKeyhole size={20} />
                    {isImportBusy ? "导入中..." : "登录并导入"}
                  </button>
                </div>
              )}

            </div>

        </AppModal>
      )}

      {isApiConfigOpen && (
        <AppModal
          title="API 服务配置"
          description="调整默认模型与推理强度"
          closeLabel="关闭 API 服务配置"
          className="api-config-panel"
          onClose={() => setIsApiConfigOpen(false)}
        >
          <ApiServiceConfigPanel
            running={Boolean(apiService?.running) || isApiServiceStarting}
            models={apiServiceModels}
            pref={apiPref}
            onChangeFamily={handleChangeFamily}
            onChangeEffort={handleChangeEffort}
            onConfigureClaude={() => void configureClaudeApp()}
            onRestoreClaude={() => void restoreClaudeApp()}
            configuringClaude={isConfiguringClaude}
            restoringClaude={isRestoringClaude}
          />
        </AppModal>
      )}

      {isSettingsOpen && (
        <AppModal
          title="设置"
          description="外观、启动和本地隐私"
          closeLabel="关闭设置"
          className="settings-panel"
          onClose={() => setIsSettingsOpen(false)}
        >
            <div className="settings-body">
              <section className="setting-block">
                <div className="setting-copy">
                  <Settings size={18} />
                  <div>
                    <strong>主题</strong>
                    <p>选择桌面窗口的显示方式</p>
                  </div>
                </div>
                <div className="theme-segmented" role="group" aria-label="主题">
                  {themeOptions.map((option) => {
                    const Icon = option.icon;
                    return (
                      <button
                        key={option.key}
                        className={clsx(settings.theme === option.key && "active")}
                        onClick={() => updateSetting("theme", option.key)}
                      >
                        <Icon size={16} />
                        {option.label}
                      </button>
                    );
                  })}
                </div>
              </section>

              <section className="setting-row">
                <div className="setting-copy">
                  <Rocket size={18} />
                  <div>
                    <strong>开机自启</strong>
                    <p>登录系统后自动启动 {APP_NAME}</p>
                  </div>
                </div>
                <button
                  className={clsx("switch", settings.autoLaunch && "active")}
                  role="switch"
                  aria-checked={settings.autoLaunch}
                  onClick={() => updateSetting("autoLaunch", !settings.autoLaunch)}
                >
                  <i />
                </button>
              </section>

              <section className="setting-row api-listen-setting">
                <div className="setting-copy">
                  <Server size={18} />
                  <div>
                    <strong>API 服务监听</strong>
                    <p>
                      {apiService?.running || isApiServiceStarting
                        ? "API 服务运行中时不允许修改端口；请先停止服务。"
                        : "地址 0.0.0.0 同时监听本机与局域网；端口限制在 51000-59999。"}
                    </p>
                  </div>
                </div>
                <div className="setting-inline-fields">
                  <label className="setting-inline-field">
                    <span>地址</span>
                    <input
                      type="text"
                      inputMode="decimal"
                      pattern="[0-9.]*"
                      maxLength={15}
                      value={apiServiceHostInput}
                      onChange={(event) => updateApiServiceHost(event.target.value)}
                      onBlur={commitApiServiceHost}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") event.currentTarget.blur();
                      }}
                      aria-invalid={parseApiServiceHost(apiServiceHostInput) === null}
                      placeholder="0.0.0.0"
                      disabled={Boolean(apiService?.running) || isApiServiceStarting}
                    />
                  </label>
                  <label className="setting-inline-field">
                    <span>端口</span>
                    <input
                      type="text"
                      inputMode="numeric"
                      pattern="[0-9]*"
                      maxLength={5}
                      value={apiServicePortInput}
                      onChange={(event) => updateApiServicePort(event.target.value)}
                      onBlur={commitApiServicePort}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") event.currentTarget.blur();
                      }}
                      aria-invalid={parseApiServicePort(apiServicePortInput) === null}
                      placeholder={String(DEFAULT_API_SERVICE_PORT)}
                      disabled={Boolean(apiService?.running) || isApiServiceStarting}
                    />
                  </label>
                </div>
              </section>

              <section className="setting-row">
                <div className="setting-copy">
                  <EyeOff size={18} />
                  <div>
                    <strong>隐私遮罩</strong>
                    <p>隐藏卡片里的邮箱和账号 ID，适合录屏或截图</p>
                  </div>
                </div>
                <button
                  className={clsx("switch", settings.maskSensitive && "active")}
                  role="switch"
                  aria-checked={settings.maskSensitive}
                  onClick={() => updateSetting("maskSensitive", !settings.maskSensitive)}
                >
                  <i />
                </button>
              </section>
            </div>
        </AppModal>
      )}

      {isLogsOpen && (
        <AppModal
          title="日志"
          description="仅保留最近 3 天的本机操作记录"
          closeLabel="关闭日志"
          className="logs-panel"
          onClose={() => setIsLogsOpen(false)}
        >
          <div className="logs-body">
            {appLogs.length === 0 ? (
              <div className="logs-empty">
                <ScrollText size={42} strokeWidth={1.4} />
                <strong>暂无日志</strong>
                <p>成功、错误和提示信息会自动记录在这里。</p>
              </div>
            ) : (
              <div className="logs-list-wrap">
                <div className="log-list" ref={logListRef} onScroll={updateLogScrollbar}>
                  {appLogs.map((log) => (
                    <article className="log-row" key={log.id}>
                      <span className="log-dot" data-tone={log.tone} />
                      <div>
                        <div className="log-meta">
                          <strong>{noticeToneConfig[log.tone].label}</strong>
                          <time>{formatDateTime(Math.floor(log.createdAt / 1000))}</time>
                        </div>
                        <p>{sanitizeUserFacingText(log.text)}</p>
                      </div>
                    </article>
                  ))}
                </div>
                <div className={clsx("account-scrollbar", "in-dialog", logScrollbar.visible && "visible")} aria-hidden="true">
                  <i style={{ height: logScrollbar.height, transform: `translateY(${logScrollbar.top}px)` }} />
                </div>
              </div>
            )}
            <p className="log-note">日志只存储在本机，超过 3 天会自动清理。</p>
          </div>
        </AppModal>
      )}

      {isAboutOpen && (
        <AppModal
          title="关于"
          description="构建与运行环境信息"
          closeLabel="关闭关于"
          className="about-panel"
          onClose={() => setIsAboutOpen(false)}
        >
          <AboutPanelBody
            appName={APP_NAME}
            isPublicBuild={IS_PUBLIC_BUILD}
            aboutInfo={aboutInfo}
            onOpenStore={handleOpenStore}
          />
        </AppModal>
      )}

      {exportPreview && (
        <AppModal
          title="导出账号"
          description={exportPreview.label}
          closeLabel="关闭导出账号"
          className="export-panel"
          onClose={() => setExportPreview(null)}
        >
          <div className="export-body">
            <textarea className="export-json" value={exportPreview.payload} readOnly spellCheck={false} />
            <div className="export-actions">
              <button className="secondary" onClick={() => void copyExportPreview(exportPreview)}>
                <Copy size={16} />
                复制
              </button>
              <button className="primary" onClick={() => downloadExportPreview(exportPreview)}>
                <Download size={16} />
                下载
              </button>
            </div>
          </div>
        </AppModal>
      )}

      {pendingDeleteAccount && (
        <div
          className="modal-overlay"
          onMouseDown={(event) => handleModalBackdropMouseDown(event, () => {
            if (!isDeletingAccount) setPendingDeleteAccount(null);
          })}
        >
          <aside className="confirm-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="confirm-icon danger">
              <Trash2 size={22} />
            </div>
            <div className="confirm-copy">
              <h2>删除账号</h2>
              <p>{accountActionLabel(pendingDeleteAccount)}</p>
            </div>
            <div className="confirm-actions">
              <button className="secondary" onClick={() => setPendingDeleteAccount(null)} disabled={isDeletingAccount}>
                取消
              </button>
              <button className="danger-button" onClick={() => void confirmDeleteAccount()} disabled={isDeletingAccount}>
                {isDeletingAccount ? "删除中..." : "删除"}
              </button>
            </div>
          </aside>
        </div>
      )}

      {pendingBatchDelete && (
        <div
          className="modal-overlay"
          onMouseDown={(event) => handleModalBackdropMouseDown(event, () => {
            if (!isAccountBusy) setPendingBatchDelete(null);
          })}
        >
          <aside className="confirm-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="confirm-icon danger">
              <Trash2 size={22} />
            </div>
            <div className="confirm-copy">
              <h2>批量删除账号</h2>
              <p>{batchDeleteDescription(pendingBatchDelete)}</p>
            </div>
            <div className="confirm-actions">
              <button className="secondary" onClick={() => setPendingBatchDelete(null)} disabled={isAccountBusy}>
                取消
              </button>
              <button className="danger-button" onClick={() => void confirmBatchDelete()} disabled={isAccountBusy}>
                {isAccountBusy ? "删除中..." : `删除 ${pendingBatchDelete.length} 个`}
              </button>
            </div>
          </aside>
        </div>
      )}
    </main>
  );
}

export default App;
