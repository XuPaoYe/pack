import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
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
  EyeOff,
  ExternalLink,
  FileJson,
  FolderDown,
  KeyRound,
  Laptop,
  LockKeyhole,
  Monitor,
  Moon,
  Plus,
  Info,
  Power,
  RefreshCw,
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
import { CodexIcon } from "./components/icons/CodexIcon";
import { GeminiIcon } from "./components/icons/GeminiIcon";
import { SuperaiIcon } from "./components/icons/SuperaiIcon";
import { fallbackStatus, formatValidityText, resolvePlanBadge } from "./lib/accountPresentation";
import {
  createLogId,
  loadAppLogs as loadAppLogsRaw,
  persistAppLogs,
  pruneAppLogs,
  type AppLogEntry,
  type NoticeTone,
} from "./lib/appLogs";
import { parseAuthJson, type AccountState, type ImportFailure, type ManagedAccount, type Provider } from "./lib/authParser";
import { formatDateTime, formatRelative, formatResetTime } from "./lib/time";

type ImportMode = "paste" | "file" | "local" | "oauth" | "batchKey";
type OAuthProvider = "codex" | "gemini";
type ThemeMode = "system" | "light" | "dark";

const IS_PUBLIC_BUILD = import.meta.env.VITE_SUPERAI_PUBLIC_BUILD === "1";

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
type Notice = { tone: NoticeTone; text: string };
type ExportPreview = {
  payload: string;
  kind: "json" | "key";
  label: string;
  fileBase: string;
};
type ForceUpdateState = {
  update: Update;
  phase: "ready" | "downloading" | "installing" | "error";
  version: string;
  currentVersion: string;
  downloadedBytes: number;
  totalBytes: number | null;
  error?: string;
};

const NOTICE_TIMEOUT_MS = 7000;
const ACCOUNT_PAGE_SIZE = 12;
const ACTIVE_ACCOUNT_REFRESH_INTERVAL_MS = 15_000;
const API_ACTIVE_ACCOUNT_SYNC_INTERVAL_MS = 3_000;

const noticeToneConfig: Record<NoticeTone, { icon: typeof Info; label: string }> = {
  success: { icon: BadgeCheck, label: "成功" },
  error: { icon: CircleAlert, label: "错误" },
  info: { icon: Info, label: "提示" },
};

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
    desc: "支持 Auth.json、SuperAI 等多种格式",
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
    icon: KeyRound,
    title: "批量密钥",
    desc: "一行一个密钥，支持多个 SuperAI 账号一起导入。",
  },
};

const importModeOrder: ImportMode[] = ["oauth", "paste", "local", "file"];
const superaiImportModeOrder: ImportMode[] = ["batchKey"];
const defaultImportMode: ImportMode = "oauth";
const defaultSuperaiImportMode: ImportMode = "batchKey";

function importModesForProvider(provider: Provider): ImportMode[] {
  return provider === PROVIDER_WSF ? superaiImportModeOrder : importModeOrder;
}

function defaultImportModeForProvider(provider: Provider): ImportMode {
  return provider === PROVIDER_WSF ? defaultSuperaiImportMode : defaultImportMode;
}

function providerLabel(provider: Provider) {
  if (provider === "codex") return "Codex";
  if (provider === "gemini") return "Gemini Cli";
  return "SuperAI";
}

// 用 .map(...).join("") 形式构造，绕过 esbuild / vite 的常量折叠，让 dist 里
// 不出现 Windsurf / windsurf 字面量。直接 String.fromCharCode(87,105,...) 会
// 被构建器在编译期算成 "Windsurf"，反而塞进 bundle。
const __WSF: string = [87, 105, 110, 100, 115, 117, 114, 102]
  .map((c) => String.fromCharCode(c))
  .join("");
const __WSFAPI: string = [119, 105, 110, 100, 115, 117, 114, 102, 97, 112, 105]
  .map((c) => String.fromCharCode(c))
  .join("");
const PROVIDER_WSF = __WSFAPI.slice(0, 8) as "windsurf"; // DB 里存的 provider 值

function sanitizeUserFacingText(text: string) {
  let next = text
    .replaceAll(__WSF, "SuperAI")
    .replaceAll(__WSFAPI, "superai-sidecar");
  if (IS_PUBLIC_BUILD) {
    next = next
      .replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, "[account]")
      .replace(/(password|pwd|api[_-]?key|session[_-]?token|auth1[_-]?token|refresh[_-]?token|access[_-]?token|id[_-]?token)(["'\s:=]+)([^"',\s}]+)/gi, "$1$2[secret]")
      .replace(/\b(auth1_[A-Za-z0-9._-]+)/g, "[secret]")
      .replace(/\b(devin-session-token\$[A-Za-z0-9._-]+)/g, "[secret]")
      .replace(/\b(eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)?)\b/g, "[secret]");
  }
  return next;
}

function loadAppLogs() {
  return loadAppLogsRaw(sanitizeUserFacingText);
}

function stateLabel(state: AccountState) {
  if (state === "available") return "可用";
  return "不可用";
}

function isCurrentAccount(account: ManagedAccount) {
  return account.status?.state === "available" && account.status.label === "当前";
}

function accountTitle(account: ManagedAccount) {
  return account.email || account.displayName || account.accountId || account.id;
}

function publicAccountCode(account: ManagedAccount, prefix: string) {
  const input = `${account.provider}:${account.id}:${account.email}:${account.accountId ?? ""}`;
  let hash = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${prefix}-${(hash >>> 0).toString(36).toUpperCase().padStart(7, "0").slice(0, 7)}`;
}

function shouldHideAccountDetails(account: ManagedAccount) {
  return IS_PUBLIC_BUILD && account.provider === PROVIDER_WSF;
}

function accountDisplayLabel(account: ManagedAccount) {
  return shouldHideAccountDetails(account) ? publicAccountCode(account, "SUPERAI") : account.email;
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
    <span className={clsx("state-corner", status.state, isCurrent && "current")} title={shouldHideAccountDetails(account) ? undefined : isCurrent ? "当前启用账号" : (status.reason ?? stateLabel(status.state))}>
      {isCurrent ? "启用" : stateLabel(status.state)}
    </span>
  );
}

function AccountPlanBadge({ account }: { account: ManagedAccount }) {
  if (shouldHideAccountDetails(account)) {
    return <span className="pill plan unknown">{publicAccountCode(account, "TIER")}</span>;
  }

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

function QuotaMeters({ account }: { account: ManagedAccount }) {
  const isUnavailable = account.status?.state === "unavailable";
  const rawMetrics =
    account.quota?.metrics?.length
      ? account.quota.metrics
      : [
          { key: "codex-5h", label: "5H", remainingPercent: 0 },
          { key: "codex-weekly", label: "周限", remainingPercent: 0 },
        ];
  const metrics =
    shouldHideAccountDetails(account)
      ? rawMetrics.filter((metric) => metric.key === "superai-daily" || localizeQuotaLabel(metric.label) === "日限")
      : rawMetrics;

  return (
    <div className="quota-meters">
      {metrics.slice(0, 3).map((metric) => {
        const remaining = isUnavailable ? 0 : metric.remainingPercent;
        const state = isUnavailable ? "unavailable" : (metric.state ?? (remaining === undefined ? "unknown" : remaining <= 0 ? "unavailable" : remaining <= 15 ? "warning" : "available"));
        const shouldHideReset = shouldHideAccountDetails(account);
        const resetText = shouldHideReset ? "" : isUnavailable ? "--" : (formatResetTime(metric.resetAt) ?? "--");
        const meterTitle = shouldHideAccountDetails(account) ? localizeQuotaLabel(metric.label) : metric.detail ?? account.quota?.error ?? metric.label;
        return (
          <div className={clsx("quota-meter", state)} key={metric.key} title={meterTitle}>
            <div className="quota-meter-head">
              <span>{localizeQuotaLabel(metric.label)}</span>
              <time className={clsx("quota-meter-reset", shouldHideReset && "hidden")}>{resetText}</time>
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
  return validity.detail ? (
    <div className={clsx("validity-line", validity.expired && "expired")} title={validity.title}>
      <CalendarDays size={15} strokeWidth={1.9} />
      <span>
        {validity.label} <strong>{validity.detail}</strong>
      </span>
      {validity.title && <time>{validity.title}</time>}
    </div>
  ) : (
    <div className="validity-line">
      <span>{validity.label} --</span>
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
  /** 该家族支持的 effort 选项；空数组表示无 effort 概念。 */
  efforts: EffortKey[];
  /** 默认 effort；若 efforts 为空可省略。 */
  defaultEffort?: EffortKey;
  knownAvailable?: boolean;
  /**
   * 返回最终 sidecar 模型 id；不可用时返回 null。
   * effort 为 null 表示无 effort（如 codex 单一变体）。
   */
  resolveId: (effort: EffortKey | null) => string | null;
};

const MODEL_FAMILIES: ModelFamily[] = [
  {
    key: "gpt-5.3-codex",
    label: "GPT-5.3-Codex",
    aliases: [
      "GPT-5.3 Codex",
      "GPT-5.3-Codex",
      "gpt-5.3-codex-low",
      "gpt-5.3-codex-high",
      "gpt-5.3-codex-xhigh",
    ],
    efforts: ["low", "medium", "high", "xhigh"],
    defaultEffort: "medium",
    resolveId: (effort) => {
      if (effort === "low") return "gpt-5.3-codex-low";
      if (effort === "high") return "gpt-5.3-codex-high";
      if (effort === "xhigh") return "gpt-5.3-codex-xhigh";
      return "gpt-5.3-codex";
    },
  },
  {
    key: "gpt-5.4",
    label: "GPT-5.4",
    aliases: ["GPT-5.4"],
    efforts: ["low", "medium", "high", "xhigh"],
    defaultEffort: "medium",
    resolveId: (effort) => (effort ? `gpt-5.4-${effort}` : "gpt-5.4-medium"),
  },
  {
    key: "gpt-5.5",
    label: "GPT-5.5",
    aliases: ["GPT-5.5", "GPT-5.5 Low Thinking", "GPT-5.5 Medium Thinking", "GPT-5.5 High Thinking"],
    efforts: ["low", "medium", "high", "xhigh"],
    defaultEffort: "medium",
    knownAvailable: true,
    resolveId: (effort) => (effort ? `gpt-5.5-${effort}` : "gpt-5.5-medium"),
  },
  {
    key: "claude-opus-4.6",
    label: "Claude Opus 4.6",
    aliases: ["Claude Opus 4.6", "claude-opus-4-6"],
    efforts: [],
    resolveId: () => "claude-opus-4.6",
  },
  {
    key: "claude-opus-4.7",
    label: "Claude Opus 4.7",
    aliases: ["Claude Opus 4.7"],
    efforts: ["low", "medium", "high", "xhigh"],
    defaultEffort: "medium",
    resolveId: (effort) => (effort ? `claude-opus-4.7-${effort}` : "claude-opus-4.7-medium"),
  },
  {
    key: "kimi-k2-6",
    label: "Kimi K2.6",
    aliases: ["Kimi K2.6", "kimi-k2.6", "kimi-k2-6"],
    efforts: [],
    knownAvailable: true,
    resolveId: () => "kimi-k2-6",
  },
  {
    key: "glm-5.1",
    label: "GLM 5.1",
    aliases: ["GLM 5.1", "glm-5-1"],
    efforts: [],
    knownAvailable: true,
    resolveId: () => "glm-5.1",
  },
  {
    key: "gemini-2.5-pro",
    label: "Gemini 2.5 Pro",
    aliases: ["Gemini 2.5 Pro"],
    efforts: [],
    knownAvailable: true,
    resolveId: () => "gemini-2.5-pro",
  },
  {
    key: "gemini-3.0-flash",
    label: "Gemini 3.0 Flash",
    aliases: ["Gemini 3.0 Flash", "gemini-3-0-flash"],
    // 上游 vendor/windsurfapi/src/models.js:155-158 提供 minimal / low / medium / high
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
    // 上游只暴露 low / high 两档，没有 medium/xhigh/minimal。
    efforts: ["low", "high"],
    defaultEffort: "low",
    knownAvailable: true,
    resolveId: (effort) => {
      if (effort === "high") return "gemini-3.1-pro-high";
      return "gemini-3.1-pro-low";
    },
  },
];

const PUBLIC_MODEL_FAMILY_KEYS = new Set([
  "gpt-5.3-codex",
  "gpt-5.4",
  "gpt-5.5",
  "claude-opus-4.6",
  "claude-opus-4.7",
  "gemini-2.5-pro",
  "gemini-3.1-pro",
  "gemini-3.0-flash",
]);

function modelFamiliesForBuild() {
  return IS_PUBLIC_BUILD
    ? MODEL_FAMILIES.filter((family) => PUBLIC_MODEL_FAMILY_KEYS.has(family.key))
    : MODEL_FAMILIES;
}

const FALLBACK_FAMILY_KEY = "gpt-5.3-codex";

type ApiModelPref = {
  family: string;
  effort: EffortKey | null;
};

function defaultPref(): ApiModelPref {
  return { family: FALLBACK_FAMILY_KEY, effort: null };
}

const API_PREF_STORAGE_KEY = "super-ai:api-service-pref";

function loadApiPref(): ApiModelPref {
  if (typeof window === "undefined") return defaultPref();
  try {
    const raw = window.localStorage.getItem(API_PREF_STORAGE_KEY);
    if (!raw) return defaultPref();
    const parsed = JSON.parse(raw) as Partial<ApiModelPref>;
    const fam = modelFamiliesForBuild().find((f) => f.key === parsed.family);
    if (!fam) {
      // 之前选过的模型已被下架（例如旧的 deepseek-v4）——
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

/** 把 pref 翻译成 sidecar 的 model id，并判断是否在 sidecar 真实可用。 */
function resolveModelId(pref: ApiModelPref): string | null {
  const family = modelFamiliesForBuild().find((f) => f.key === pref.family);
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

/** 该家族在当前 sidecar 实际可用（默认 effort 的变体存在）。 */
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
  const families = modelFamiliesForBuild();
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
                  {family.label}
                  {!available && <em>暂未上线</em>}
                </span>
                {selected && available && <Check size={14} />}
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
  pref,
  onToggleService,
  onCopy,
  onOpenConfig,
}: {
  status: ApiServiceStatus | null;
  busy: boolean;
  pref: ApiModelPref;
  onToggleService: () => void;
  onCopy: (text: string, label: string) => void;
  onOpenConfig: () => void;
}) {
  const running = Boolean(status?.running);
  const address = status?.address ?? "—";
  const apiKey = status?.apiKey ?? "";
  const families = modelFamiliesForBuild();
  const family = families.find((f) => f.key === pref.family) ?? families[0];
  const modelSummary = [
    family.label,
    pref.effort ? EFFORT_LABELS[pref.effort] : null,
  ].filter(Boolean).join(" · ");

  return (
    <article className={clsx("account-row superai-api-card", running && "running")}>
      <div className="superai-api-head">
        <div className="superai-api-icon">
          <Server size={22} strokeWidth={1.8} />
        </div>
        <div className="superai-api-title">
          <strong>API 服务</strong>
          <span>支持本机与局域网调用</span>
        </div>
        <div className={clsx("superai-api-status-dot", running && "running")} />
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
        <button
          type="button"
          className={clsx("superai-api-toggle", running ? "off" : "on")}
          onClick={onToggleService}
          disabled={busy}
        >
          <Power size={14} />
          {running ? "停止服务" : "启动服务"}
        </button>

        <p className="superai-api-hint">
          {running
            ? "API 服务运行中。账号池会按 SuperAI 账号可用额度自动轮询请求。"
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
}: {
  running: boolean;
  models: ApiServiceModel[];
  pref: ApiModelPref;
  onChangeFamily: (family: string) => void;
  onChangeEffort: (effort: EffortKey) => void;
}) {
  const families = modelFamiliesForBuild();
  const family = families.find((f) => f.key === pref.family) ?? families[0];
  const showEffort = family.efforts.length > 0;

  return (
    <div className="api-config-body">
      <section className="api-config-row">
        <div className="api-config-copy">
          <strong>模型</strong>
          <p>选择 API 服务默认使用的模型家族</p>
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

    </div>
  );
}

function NoticeToast({ notice, onClose }: { notice: Notice; onClose: () => void }) {
  const config = noticeToneConfig[notice.tone];
  const Icon = config.icon;

  return (
    <div className="toast" data-tone={notice.tone} role="status" aria-live="polite">
      <Icon className="toast-icon" size={16} aria-hidden="true" />
      <span className="toast-text">
        <b>{config.label}</b>
        {sanitizeUserFacingText(notice.text)}
      </span>
      <button onClick={onClose} aria-label="关闭提示">×</button>
    </div>
  );
}

function formatBytes(bytes: number) {
  if (bytes <= 0) return "0 KB";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function ForceUpdateModal({ state, onInstall }: { state: ForceUpdateState; onInstall: () => void }) {
  const progressPercent = state.totalBytes ? Math.min(100, Math.round((state.downloadedBytes / state.totalBytes) * 100)) : 0;
  const isWorking = state.phase === "downloading" || state.phase === "installing";

  return (
    <div className="modal-overlay force-update-overlay">
      <aside className="force-update-panel modal-content" role="alertdialog" aria-modal="true" aria-labelledby="force-update-title">
        <div className="force-update-icon">
          <Download size={24} strokeWidth={2.1} />
        </div>
        <div className="force-update-copy">
          <h2 id="force-update-title">发现新版本</h2>
          <p>
            Super AI {state.version} 已可用，当前版本 {state.currentVersion}。必须升级后才能继续使用。
          </p>
        </div>
        {(state.phase === "downloading" || state.phase === "installing") && (
          <div className="force-update-progress" aria-label="升级进度">
            <div>
              <span>{state.phase === "installing" ? "正在安装" : "正在下载"}</span>
              <b>{state.totalBytes ? `${progressPercent}%` : formatBytes(state.downloadedBytes)}</b>
            </div>
            <i>
              <span style={{ width: state.totalBytes ? `${progressPercent}%` : "35%" }} />
            </i>
          </div>
        )}
        {state.phase === "error" && <p className="force-update-error">{state.error ?? "升级失败，请重试。"}</p>}
        <button className="primary force-update-button" onClick={onInstall} disabled={isWorking}>
          {state.phase === "error" ? <RotateCw size={18} /> : <Download size={18} />}
          {state.phase === "error" ? "重试升级" : isWorking ? "升级中" : "立即升级"}
        </button>
      </aside>
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
  const [activeProvider, setActiveProvider] = useState<Provider>(PROVIDER_WSF);
  const [mode, setMode] = useState<ImportMode>(defaultImportMode);
  const [pasteValue, setPasteValue] = useState("");
  const [superaiBatchKeys, setSuperaiBatchKeys] = useState("");
  const [query, setQuery] = useState("");
  const [accountPage, setAccountPage] = useState(1);
  const [isImportModalOpen, setIsImportModalOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isLogsOpen, setIsLogsOpen] = useState(false);
  const [isApiConfigOpen, setIsApiConfigOpen] = useState(false);
  const [exportPreview, setExportPreview] = useState<ExportPreview | null>(null);
  const [selectedExportIds, setSelectedExportIds] = useState<Set<string>>(() => new Set());
  const [pendingDeleteAccount, setPendingDeleteAccount] = useState<ManagedAccount | null>(null);
  const [pendingBatchDelete, setPendingBatchDelete] = useState<ManagedAccount[] | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [isDeletingAccount, setIsDeletingAccount] = useState(false);
  const [isFileImporting, setIsFileImporting] = useState(false);
  const [refreshingAccountIds, setRefreshingAccountIds] = useState<Set<string>>(() => new Set());
  const [refreshingProviders, setRefreshingProviders] = useState<Set<Provider>>(() => new Set());
  const [pendingOAuth, setPendingOAuth] = useState<Partial<Record<OAuthProvider, string>>>({});
  const [isSettingsLoaded, setIsSettingsLoaded] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [appLogs, setAppLogs] = useState<AppLogEntry[]>(loadAppLogs);
  const [forceUpdate, setForceUpdate] = useState<ForceUpdateState | null>(null);
  const [apiService, setApiService] = useState<ApiServiceStatus | null>(null);
  const [isApiServiceBusy, setIsApiServiceBusy] = useState(false);
  const [settings, setSettings] = useState<AppSettings>({
    theme: "system",
    autoLaunch: false,
    maskSensitive: false,
    apiServiceHost: "0.0.0.0",
    apiServicePort: 0,
    apiServiceDefaultModel: "gpt-5.3-codex",
  });
  const [apiServiceModels, setApiServiceModels] = useState<ApiServiceModel[]>([]);
  const [apiPref, setApiPrefState] = useState<ApiModelPref>(loadApiPref);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const accountListRef = useRef<HTMLDivElement | null>(null);
  const oauthPollTimers = useRef<Partial<Record<OAuthProvider, number>>>({});
  const noticeTimer = useRef<number | null>(null);
  const hasStartedStartupRefresh = useRef(false);
  const activeAccountRefreshInFlight = useRef<string | null>(null);
  const [accountScrollbar, setAccountScrollbar] = useState({
    visible: false,
    top: 0,
    height: 0,
  });
  const appWindow = useMemo(() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  }, []);

  const installForceUpdate = useCallback(async () => {
    if (!forceUpdate) return;
    let downloadedBytes = 0;

    try {
      setForceUpdate((current) =>
        current
          ? {
              ...current,
              phase: "downloading",
              downloadedBytes: 0,
              totalBytes: null,
              error: undefined,
            }
          : current,
      );

      const handleDownloadEvent = (event: DownloadEvent) => {
        if (event.event === "Started") {
          downloadedBytes = 0;
          setForceUpdate((current) =>
            current
              ? {
                  ...current,
                  phase: "downloading",
                  downloadedBytes: 0,
                  totalBytes: event.data.contentLength ?? null,
                }
              : current,
          );
          return;
        }

        if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          setForceUpdate((current) =>
            current
              ? {
                  ...current,
                  downloadedBytes,
                }
              : current,
          );
          return;
        }

        setForceUpdate((current) =>
          current
            ? {
                ...current,
                phase: "installing",
                downloadedBytes: current.totalBytes ?? current.downloadedBytes,
              }
            : current,
        );
      };

      await forceUpdate.update.downloadAndInstall(handleDownloadEvent);
      await relaunch();
    } catch (error) {
      setForceUpdate((current) =>
        current
          ? {
              ...current,
              phase: "error",
              error: `升级失败：${String(error)}`,
            }
          : current,
      );
    }
  }, [forceUpdate]);

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
    () => refreshingProviders.has(activeProvider) || filteredAccounts.some((account) => refreshingAccountIds.has(account.id)),
    [activeProvider, filteredAccounts, refreshingAccountIds, refreshingProviders],
  );

  const counts = useMemo(
    () => ({
      codex: accounts.filter((account) => account.provider === "codex").length,
      gemini: accounts.filter((account) => account.provider === "gemini").length,
      [PROVIDER_WSF]: accounts.filter((account) => account.provider === PROVIDER_WSF).length,
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

  const closeNotice = useCallback(() => {
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = null;
    setNotice(null);
  }, []);

  const appendAppLog = useCallback((tone: NoticeTone, text: string) => {
    const sanitizedText = sanitizeUserFacingText(text);
    setAppLogs((current) =>
      pruneAppLogs([
        {
          id: createLogId(),
          tone,
          text: sanitizedText,
          createdAt: Date.now(),
        },
        ...current,
      ]),
    );
  }, []);

  const showNotice = useCallback((tone: NoticeTone, text: string) => {
    const sanitizedText = sanitizeUserFacingText(text);
    appendAppLog(tone, sanitizedText);
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    setNotice({ tone, text: sanitizedText });
    noticeTimer.current = window.setTimeout(() => {
      setNotice(null);
      noticeTimer.current = null;
    }, NOTICE_TIMEOUT_MS);
  }, [appendAppLog]);

  useEffect(() => {
    if (import.meta.env.DEV || !isTauri()) return;
    let isCancelled = false;

    async function checkForUpdateOnLaunch() {
      try {
        const update = await check();
        if (!update || isCancelled) return;
        setForceUpdate({
          update,
          phase: "ready",
          version: update.version,
          currentVersion: update.currentVersion,
          downloadedBytes: 0,
          totalBytes: null,
        });
      } catch (error) {
        if (!isCancelled) {
          const message = String(error);
          const isUpdaterUnconfigured =
            message.includes("plugins > updater doesn't exist") ||
            (message.includes("updater") && message.includes("configuration"));
          if (isUpdaterUnconfigured) {
            appendAppLog("info", "远程升级未配置，已跳过启动更新检测。");
          } else {
            showNotice("error", `检测更新失败：${message}`);
          }
        }
      }
    }

    void checkForUpdateOnLaunch();

    return () => {
      isCancelled = true;
    };
  }, [appendAppLog, showNotice]);

  const reloadAccountsSoon = useCallback((delay = 1800) => {
    window.setTimeout(() => {
      void invoke<ManagedAccount[]>("list_accounts")
        .then((storedAccounts) => setAccounts(storedAccounts))
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
        showNotice("error", `启动检查账号状态失败：${String(error)}`);
      })
      .finally(() => {
        setRefreshingAccountIds((current) => {
          const next = new Set(current);
          startupAccountIds.forEach((accountId) => next.delete(accountId));
          return next;
        });
      });
  }, [reloadAccountsSoon, showNotice]);

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

  const clearOAuthPoll = (provider: OAuthProvider) => {
    const timer = oauthPollTimers.current[provider];
    if (timer) window.clearTimeout(timer);
    delete oauthPollTimers.current[provider];
  };

  const closeImportModal = () => {
    setIsImportModalOpen(false);
    setMode(defaultImportModeForProvider(activeProvider));
    setPasteValue("");
    setSuperaiBatchKeys("");
  };

  const applyImportResult = (
    result: BackendImportResult,
    options: { closeModal?: boolean; successText?: string } = {},
  ) => {
    if (result.imported.length > 0) {
      setAccounts((current) => mergeAccounts(current, result.imported));
      void invoke("upsert_accounts", { accounts: result.imported })
        .catch(() => undefined)
        .finally(() => refreshImportedAccountStatus(result.imported));
      if (options.closeModal ?? true) {
        closeImportModal();
        setPasteValue("");
      }
      setAccountPage(1);
      showNotice("success", options.successText ?? `已添加 ${result.imported.length} 个账号`);
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
      });
      return result;
    } catch {
      return parseAuthJson(content, "paste", label);
    }
  };

  const handleFileImport = async (files: FileList | null) => {
    if (!files?.length) return;
    setIsBusy(true);
    setIsFileImporting(true);
    const allFailures: ImportFailure[] = [];
    const allImported: ManagedAccount[] = [];
    try {
      for (const file of Array.from(files)) {
        const content = await file.text();
        const result = await parseWithBackend(content, file.name);
        allImported.push(...result.imported);
        allFailures.push(...result.failed);
      }
      if (allImported.length > 0) {
        applyImportResult(
          { imported: allImported, failed: allFailures },
        );
      } else {
        showNotice("error", allFailures[0]?.reason ?? "没有发现可添加的账号");
      }
    } finally {
      setIsFileImporting(false);
      setIsBusy(false);
    }
  };

  const handlePasteImport = async () => {
    setIsBusy(true);
    try {
      const result = await parseWithBackend(pasteValue, "粘贴内容");
      applyImportResult(result);
    } finally {
      setIsBusy(false);
    }
  };

  const handleLocalImport = async (provider: OAuthProvider) => {
    setIsBusy(true);
    try {
      const command = provider === "codex" ? "import_codex_from_local" : "import_gemini_from_local";
      const result = await invoke<BackendImportResult>(command);
      applyImportResult(
        result,
      );
    } catch (error) {
      showNotice("error", `读取本机 ${providerLabel(provider)} 失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
  };

  const completeOAuth = async (
    provider: OAuthProvider,
    loginId: string,
    options: { silent?: boolean } = {},
  ) => {
    const command = provider === "codex" ? "complete_codex_oauth" : "complete_gemini_oauth";
    try {
      const result = await invoke<BackendImportResult>(command, { loginId });
      if (result.imported.length === 0) {
        if (!options.silent) applyImportResult(result, { closeModal: false });
        return false;
      }
      applyImportResult(
        result,
        { successText: `${providerLabel(provider)} OAuth 登录成功，已添加 ${result.imported.length} 个账号` },
      );
      setPendingOAuth((current) => ({ ...current, [provider]: undefined }));
      clearOAuthPoll(provider);
      return true;
    } catch (error) {
      if (!options.silent) {
        showNotice("error", `${providerLabel(provider)} OAuth 完成失败：${String(error)}`);
      }
      return false;
    }
  };

  const scheduleOAuthPoll = (provider: OAuthProvider, loginId: string, attempt = 0) => {
    clearOAuthPoll(provider);
    if (attempt >= 90) {
      showNotice("info", `${providerLabel(provider)} OAuth 仍在等待完成，可再次在浏览器中打开授权。`);
      return;
    }
    oauthPollTimers.current[provider] = window.setTimeout(() => {
      void completeOAuth(provider, loginId, { silent: true }).then((isComplete) => {
        if (!isComplete) scheduleOAuthPoll(provider, loginId, attempt + 1);
      });
    }, 2000);
  };

  const handleOAuthStart = async (provider: OAuthProvider) => {
    clearOAuthPoll(provider);
    setIsBusy(true);
    try {
      const command = provider === "codex" ? "start_codex_oauth" : "start_gemini_oauth";
      const result = await invoke<OAuthStartResult>(command);
      setPendingOAuth((current) => ({ ...current, [provider]: result.login_id }));
      scheduleOAuthPoll(provider, result.login_id);
      showNotice("info", result.message);
    } catch (error) {
      showNotice("error", `${providerLabel(provider)} OAuth 启动失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
  };

  const selectedMode = modeConfig[mode];
  const ModeIcon = selectedMode.icon;
  const isActiveProviderOAuthPending =
    activeProvider !== PROVIDER_WSF && Boolean(pendingOAuth[activeProvider as OAuthProvider]);
  const oauthAccountLabel = activeProvider === "codex" ? "OpenAI" : "Gemini";
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
    setMode(defaultImportModeForProvider(activeProvider));
    setIsImportModalOpen(true);
  };

  const handleSuperaiBatchKeyImport = async () => {
    const keys = superaiBatchKeys
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
    if (keys.length === 0) {
      showNotice("error", "请粘贴 SuperAI 批量密钥");
      return;
    }
    setIsBusy(true);
    try {
      const result = await invoke<BackendImportResult>("add_superai_accounts_by_batch_keys", { keys });
      applyImportResult(result, {
        closeModal: result.imported.length > 0,
        successText: `已添加 ${result.imported.length} 个 SuperAI 账号`,
      });
      if (result.imported.length > 0) {
        setSuperaiBatchKeys("");
      }
    } catch (error) {
      showNotice("error", `批量导入失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
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
  const updateSetting = <Key extends keyof typeof settings>(key: Key, value: (typeof settings)[Key]) => {
    setSettings((current) => ({ ...current, [key]: value }));
  };
  const handleToggleAccount = async (account: ManagedAccount) => {
    if (isCurrentAccount(account)) return;
    if ((account.status ?? fallbackStatus(account)).state === "unavailable") return;
    setIsBusy(true);
    try {
      const providerAccounts = await invoke<SwitchAccountResult>("switch_account", { accountId: account.id });
      setAccounts((current) =>
        sortAccountsForView(current.map((item) => providerAccounts.find((changed) => changed.id === item.id) ?? item)),
      );
      setAccountPage(1);
      showNotice("success", `已启用 ${accountDisplayLabel(account)}`);
    } catch (error) {
      showNotice("error", `启用账号失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
  };

  const handleRefreshAccount = async (account: ManagedAccount) => {
    setIsBusy(true);
    setRefreshingAccountIds((current) => new Set(current).add(account.id));
    try {
      const refreshed = await invoke<ManagedAccount>("refresh_account", { accountId: account.id });
      setAccounts((current) => sortAccountsForView(current.map((item) => (item.id === refreshed.id ? refreshed : item))));
      showNotice("success", `已刷新 ${accountDisplayLabel(refreshed)}`);
    } catch (error) {
      showNotice("error", `刷新账号失败：${String(error)}`);
    } finally {
      setRefreshingAccountIds((current) => {
        const next = new Set(current);
        next.delete(account.id);
        return next;
      });
      setIsBusy(false);
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
      showNotice("success", `已刷新 ${providerLabel(provider)} 账号`);
    } catch (error) {
      showNotice("error", `刷新 ${providerLabel(provider)} 失败：${String(error)}`);
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
    let payload: string;
    const isPublicKeyExport = shouldHideAccountDetails(account);
    try {
      payload = isPublicKeyExport
        ? await invoke<string>("export_public_superai_account", { accountId: account.id })
        : await invoke<string>("export_account", { accountId: account.id });
    } catch (error) {
      showNotice("error", `导出账号失败：${String(error)}`);
      return;
    }
    setExportPreview({
      payload,
      kind: isPublicKeyExport ? "key" : "json",
      label: isPublicKeyExport ? "SuperAI 密钥" : accountDisplayLabel(account),
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
    const selectedAccounts = filteredAccounts.filter((account) => selectedExportIds.has(account.id));
    if (selectedAccounts.length === 0) {
      showNotice("error", "请选择要导出的账号");
      return;
    }
    try {
      const exported = await Promise.all(
        selectedAccounts.map(async (account) => {
          const isPublicKeyExport = shouldHideAccountDetails(account);
          const payload = isPublicKeyExport
            ? await invoke<string>("export_public_superai_account", { accountId: account.id })
            : await invoke<string>("export_account", { accountId: account.id });
          return { payload, kind: isPublicKeyExport ? ("key" as const) : ("json" as const) };
        }),
      );
      const isKeyExport = exported.every((item) => item.kind === "key");
      const payload = isKeyExport
        ? exported.map((item) => item.payload.trim()).filter(Boolean).join("\n")
        : JSON.stringify(exported.map((item) => JSON.parse(item.payload)), null, 2);
      setExportPreview({
        payload,
        kind: isKeyExport ? "key" : "json",
        label: isKeyExport ? `SuperAI 密钥 · ${exported.length} 个账号` : `${providerLabel(activeProvider)} · ${exported.length} 个账号`,
        fileBase: `${activeProvider}-${exported.length}-accounts`,
      });
      setSelectedExportIds(new Set());
    } catch (error) {
      showNotice("error", `批量导出失败：${String(error)}`);
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
    if (!pendingBatchDelete || isBusy) return;
    const targets = pendingBatchDelete;
    setIsBusy(true);
    try {
      let nextAccounts = accounts;
      for (const account of targets) {
        nextAccounts = await invoke<ManagedAccount[]>("delete_account", { accountId: account.id });
      }
      setAccounts(nextAccounts);
      setSelectedExportIds(new Set());
      setPendingBatchDelete(null);
      showNotice("success", `已删除 ${targets.length} 个账号`);
    } catch (error) {
      showNotice("error", `批量删除失败：${String(error)}`);
    } finally {
      setIsBusy(false);
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
      setAccounts(nextAccounts);
      setPendingDeleteAccount(null);
      showNotice("success", `已删除 ${accountDisplayLabel(account)}`);
    } catch (error) {
      showNotice("error", `删除账号失败：${String(error)}`);
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
        setAccounts(storedAccounts);
        refreshAllAccountsOnLaunch(storedAccounts);
      })
      .catch(() => undefined);

    void invoke<AppSettings | null>("load_settings")
      .then((storedSettings) => {
        if (storedSettings) {
          setSettings(storedSettings);
        }
      })
      .catch(() => undefined)
      .finally(() => setIsSettingsLoaded(true));

    return () => {
      Object.values(oauthPollTimers.current).forEach((timer) => {
        if (timer) window.clearTimeout(timer);
      });
      oauthPollTimers.current = {};
      if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    };
  }, [refreshAllAccountsOnLaunch]);

  useEffect(() => {
    if (!isSettingsLoaded) return;
    void invoke("save_settings", { settings }).catch((error) => {
      showNotice("error", `保存设置失败：${String(error)}`);
    });
  }, [isSettingsLoaded, settings, showNotice]);

  useEffect(() => {
    persistAppLogs(appLogs);
  }, [appLogs]);

  useEffect(() => {
    if (!isTauri()) return;
    invoke<ApiServiceStatus>("get_api_service_status")
      .then((status) => setApiService(status))
      .catch(() => undefined);
  }, []);

  // 服务在跑就拉一次 sidecar 的模型清单。
  useEffect(() => {
    if (!isTauri() || !apiService?.running) {
      setApiServiceModels([]);
      return;
    }
    let cancelled = false;
    invoke<ApiServiceModel[]>("list_api_service_models")
      .then((list) => {
        if (!cancelled) setApiServiceModels(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setApiServiceModels([]);
      });
    return () => {
      cancelled = true;
    };
  }, [apiService?.running, apiService?.actualPort]);

  useEffect(() => {
    if (!isTauri() || !apiService?.running) return undefined;
    let cancelled = false;

    const syncActiveApiAccount = () => {
      void invoke<SwitchAccountResult>("sync_api_service_active_account")
        .then((changedAccounts) => {
          if (cancelled || changedAccounts.length === 0) return;
          setAccounts((current) =>
            sortAccountsForView(current.map((item) => changedAccounts.find((changed) => changed.id === item.id) ?? item)),
          );
        })
        .catch(() => undefined);
    };

    syncActiveApiAccount();
    const timer = window.setInterval(syncActiveApiAccount, API_ACTIVE_ACCOUNT_SYNC_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [apiService?.running, apiService?.actualPort]);

  // 自启失败的事件 → toast。
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | null = null;
    void listen<{ phase?: string; message?: string }>("api-service-error", (event) => {
      const message = event.payload?.message ?? "未知错误";
      showNotice("error", `SuperAI API 服务异常：${message}`);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [showNotice]);

  const toggleApiService = useCallback(async () => {
    if (isApiServiceBusy) return;
    setIsApiServiceBusy(true);
    try {
      const command = apiService?.running ? "stop_api_service" : "start_api_service";
      const status = await invoke<ApiServiceStatus>(command);
      setApiService(status);
      showNotice(
        "success",
        status.running ? `已启动 API 服务${status.address ? "：" + status.address : ""}` : "已停用 API 服务",
      );
    } catch (error) {
      showNotice("error", `操作 API 服务失败：${String(error)}`);
    } finally {
      setIsApiServiceBusy(false);
    }
  }, [isApiServiceBusy, showNotice, apiService?.running]);

  const applyApiPref = useCallback(
    (updater: (prev: ApiModelPref) => ApiModelPref) => {
      setApiPrefState((prev) => {
        const next = updater(prev);
        persistApiPref(next);
        const modelId = resolveModelId(next) ?? "";
        if (isTauri()) {
          invoke("set_api_service_default_model", { model: modelId })
            .then(() => {
              setApiService((status) => status ? { ...status, defaultModel: modelId } : status);
            })
            .catch((error) => {
              showNotice("error", `设置默认模型失败：${String(error)}`);
            });
        }
        return next;
      });
    },
    [showNotice],
  );

  const handleChangeFamily = useCallback(
    (familyKey: string) => {
      applyApiPref((prev) => {
        const fam = modelFamiliesForBuild().find((f) => f.key === familyKey);
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

  // 服务启动后，确保 sidecar 用的 default_model 和 UI 当前选择一致。
  useEffect(() => {
    if (!isTauri() || !apiService?.running) return;
    const modelId = resolveModelId(apiPref) ?? "";
    invoke("set_api_service_default_model", { model: modelId }).catch(() => undefined);
  }, [apiService?.running, apiPref]);

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
        (isImportModalOpen || isSettingsOpen || isLogsOpen || isApiConfigOpen || exportPreview || pendingDeleteAccount || pendingBatchDelete || forceUpdate) && "modal-active",
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
            <strong>Super AI</strong>
            <span>账号管理工具</span>
          </div>
        </div>

        <nav className="nav-list" aria-label="Providers">
          <button className={clsx(activeProvider === PROVIDER_WSF && "active")} onClick={() => handleProviderChange(PROVIDER_WSF)}>
            <SuperaiIcon className="provider-nav-icon superai" />
            <span>SuperAI</span>
            <b>{counts[PROVIDER_WSF]}</b>
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
        </nav>

        <div className="sidebar-footer">
          <a className="ad-card" href="https://ai.talentisan.cn/" onClick={handleOpenStore}>
            <div>
              <span>AI 权益补给站</span>
              <strong>购买 AI 到 Super Store</strong>
            </div>
            <ExternalLink size={18} />
          </a>
          <div className="sidebar-divider" />
          <button onClick={handleLogs}>
            <ScrollText size={18} />
            日志
          </button>
          <button onClick={handleSettings}>
            <Settings size={18} />
            设置
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
                  disabled={isBusy || selectedExportIds.size === 0}
                  title={selectedExportIds.size === 0 ? "请先勾选账号" : `导出选中的 ${selectedExportIds.size} 个账号`}
                >
                  <Download size={18} />
                  导出{selectedExportIds.size > 0 ? ` (${selectedExportIds.size})` : ""}
                </button>
                <button
                  className="secondary danger-action"
                  onClick={() => handleBatchDelete()}
                  disabled={isBusy || selectedExportIds.size === 0}
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
                  {activeProvider === PROVIDER_WSF && (
                    <ApiServiceCard
                      status={apiService}
                      busy={isApiServiceBusy}
                      pref={apiPref}
                      onToggleService={() => void toggleApiService()}
                      onCopy={(text, label) => void copyApiServiceText(text, label)}
                      onOpenConfig={() => setIsApiConfigOpen(true)}
                    />
                  )}
                  {filteredAccounts.length === 0 && activeProvider !== PROVIDER_WSF && (
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
                          {shouldHideAccountDetails(account) ? (
                            <strong>{publicAccountCode(account, "SUPERAI")}</strong>
                          ) : (
                            <strong>{accountTitle(account)}</strong>
                          )}
                          <AccountPlanBadge account={account} />
                        </div>
                        <div className="account-subtitle">
                          {shouldHideAccountDetails(account) ? (
                            <>
                              <span>
                                <b>名称</b>
                                {publicAccountCode(account, "USER")}
                              </span>
                              <span>
                                <b>账号</b>
                                {publicAccountCode(account, "ACCT")}
                              </span>
                            </>
                          ) : (
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
                          )}
                          {!shouldHideAccountDetails(account) && account.organizationId && (
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
                          disabled={isCurrentAccount(account) || (account.status ?? fallbackStatus(account)).state === "unavailable"}
                        >
                          <BadgeCheck size={15} strokeWidth={1.75} />
                        </button>
                        <button className="icon-button" aria-label="刷新账号" title="刷新" onClick={() => handleRefreshAccount(account)} disabled={refreshingAccountIds.has(account.id)}>
                          <RefreshCw size={15} strokeWidth={1.75} className={clsx(refreshingAccountIds.has(account.id) && "spin")} />
                        </button>
                        <button className="icon-button" aria-label="导出账号" title="导出" onClick={() => void handleExportAccount(account)}>
                          <Download size={15} strokeWidth={1.75} />
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
        <NoticeToast notice={notice} onClose={closeNotice} />
      )}

      {forceUpdate && (
        <ForceUpdateModal state={forceUpdate} onInstall={() => void installForceUpdate()} />
      )}

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

              {mode === "paste" && activeProvider !== PROVIDER_WSF && (
                <>
                  <textarea
                    value={pasteValue}
                    onChange={(event) => setPasteValue(event.target.value)}
                    spellCheck={false}
                    placeholder={'{\n  "tokens": {\n    "id_token": "...",\n    "access_token": "...",\n    "refresh_token": "..."\n  }\n}'}
                  />
                  <button className="wide primary" onClick={handlePasteImport} disabled={!pasteValue.trim() || isBusy}>
                    <Clipboard size={20} />
                    {isBusy ? "处理中..." : "解析并添加"}
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
                  <button className={clsx("drop-zone", isFileImporting && "loading")} onClick={() => fileInputRef.current?.click()} disabled={isBusy}>
                    <Upload size={28} />
                    <strong>{isFileImporting ? "正在导入 JSON..." : "选择 JSON 文件"}</strong>
                    <span>{isFileImporting ? "正在解析并刷新账号信息" : "支持 auth.json、oauth_creds.json、导出数组"}</span>
                  </button>
                </>
              )}

              {mode === "local" && activeProvider !== PROVIDER_WSF && (
                <>
                  <button className="drop-zone local-import-button" onClick={() => handleLocalImport(activeProvider as OAuthProvider)} disabled={isBusy}>
                    <FolderDown size={28} />
                    <strong>{isBusy ? "正在读取本机账号..." : `读取 ${providerLabel(activeProvider)} 本机账号`}</strong>
                    <span>{providerLabel(activeProvider)} 本机凭证只在当前设备处理</span>
                  </button>
                </>
              )}

              {mode === "oauth" && activeProvider !== PROVIDER_WSF && (
                <div className="oauth-flow">
                  <button className={clsx("drop-zone", isActiveProviderOAuthPending && "oauth-pending")} onClick={() => handleOAuthStart(activeProvider as OAuthProvider)}>
                    <LockKeyhole size={28} />
                    <strong>{isActiveProviderOAuthPending ? "重新打开授权" : "在浏览器中打开"}</strong>
                    <span>{isActiveProviderOAuthPending ? "浏览器关闭或卡住时可重新发起" : `${oauthAccountLabel} OAuth 授权将在浏览器中完成`}</span>
                  </button>
                </div>
              )}

              {mode === "batchKey" && activeProvider === PROVIDER_WSF && (
                <>
                  <textarea
                    value={superaiBatchKeys}
                    onChange={(event) => setSuperaiBatchKeys(event.target.value)}
                    placeholder="一行一个批量密钥"
                    spellCheck={false}
                    disabled={isBusy}
                  />
                  <button
                    className="wide primary"
                    onClick={() => void handleSuperaiBatchKeyImport()}
                    disabled={!superaiBatchKeys.trim() || isBusy}
                  >
                    <KeyRound size={20} />
                    {isBusy ? "导入中..." : "批量导入"}
                  </button>
                </>
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
            running={Boolean(apiService?.running)}
            models={apiServiceModels}
            pref={apiPref}
            onChangeFamily={handleChangeFamily}
            onChangeEffort={handleChangeEffort}
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
                    <p>登录系统后自动启动 Super AI</p>
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
                    <p>地址 0.0.0.0 同时监听本机与局域网；端口 0 表示首次启动随机分配，之后会保持。</p>
                  </div>
                </div>
                <div className="setting-inline-fields">
                  <label className="setting-inline-field">
                    <span>地址</span>
                    <input
                      type="text"
                      value={settings.apiServiceHost}
                      onChange={(event) => updateSetting("apiServiceHost", event.target.value)}
                      placeholder="0.0.0.0"
                    />
                  </label>
                  <label className="setting-inline-field">
                    <span>端口</span>
                    <input
                      type="number"
                      min={0}
                      max={65535}
                      value={settings.apiServicePort}
                      onChange={(event) => {
                        const next = Number(event.target.value);
                        updateSetting("apiServicePort", Number.isFinite(next) ? next : 0);
                      }}
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
              <div className="log-list">
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
            )}
            <p className="log-note">日志只存储在本机，超过 3 天会自动清理。</p>
          </div>
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
              <p>{accountDisplayLabel(pendingDeleteAccount)}</p>
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
            if (!isBusy) setPendingBatchDelete(null);
          })}
        >
          <aside className="confirm-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="confirm-icon danger">
              <Trash2 size={22} />
            </div>
            <div className="confirm-copy">
              <h2>批量删除账号</h2>
              <p>共 {pendingBatchDelete.length} 个账号将被移除</p>
            </div>
            <div className="confirm-actions">
              <button className="secondary" onClick={() => setPendingBatchDelete(null)} disabled={isBusy}>
                取消
              </button>
              <button className="danger-button" onClick={() => void confirmBatchDelete()} disabled={isBusy}>
                {isBusy ? "删除中..." : `删除 ${pendingBatchDelete.length} 个`}
              </button>
            </div>
          </aside>
        </div>
      )}
    </main>
  );
}

export default App;
