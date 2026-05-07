import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  BadgeCheck,
  Bot,
  CalendarDays,
  CircleAlert,
  CirclePlay,
  Clipboard,
  Cloud,
  EyeOff,
  ExternalLink,
  FileDown,
  FileJson,
  Fingerprint,
  FolderDown,
  Laptop,
  LockKeyhole,
  Monitor,
  Moon,
  Plus,
  Info,
  RefreshCcw,
  RotateCw,
  Rocket,
  ScrollText,
  Search,
  Settings,
  Sun,
  Trash,
  Upload,
  X,
} from "lucide-react";
import "./App.css";
import logoUrl from "./assets/logo.svg";
import { parseAuthJson, type AccountState, type ImportFailure, type ManagedAccount, type Provider } from "./lib/authParser";

type ImportMode = "paste" | "file" | "local" | "oauth";
type OAuthProvider = "codex" | "gemini";
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
};

type OAuthStartResult = {
  login_id: string;
  provider: OAuthProvider;
  command: string;
  message: string;
};

type SwitchAccountResult = ManagedAccount[];
type NoticeTone = "success" | "error" | "info";
type Notice = { tone: NoticeTone; text: string };

const NOTICE_TIMEOUT_MS = 7000;

const noticeToneConfig: Record<NoticeTone, { icon: typeof Info; label: string }> = {
  success: { icon: BadgeCheck, label: "成功" },
  error: { icon: CircleAlert, label: "错误" },
  info: { icon: Info, label: "提示" },
};

const modeConfig: Record<
  ImportMode,
  {
    icon: typeof Clipboard;
    title: string;
    desc: string;
  }
> = {
  paste: {
    icon: Clipboard,
    title: "粘贴凭证",
    desc: "适合从 Codex auth.json、Gemini oauth_creds.json 或导出数组快速添加。",
  },
  file: {
    icon: FileJson,
    title: "上传Json",
    desc: "支持 auth.json、oauth_creds.json 和导出数组。",
  },
  local: {
    icon: Laptop,
    title: "读取本机",
    desc: "桌面后端读取 ~/.codex/auth.json 与 ~/.gemini，本原型先展示流程。",
  },
  oauth: {
    icon: Cloud,
    title: "OAuth授权",
    desc: "通过本地 Callback 完成授权；Codex 与 Gemini 将分别走官方 OAuth。",
  },
};

const importModeOrder: ImportMode[] = ["paste", "local", "file", "oauth"];

function providerLabel(provider: Provider) {
  return provider === "codex" ? "Codex" : "Gemini Cli";
}

function formatRelative(timestamp: number) {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

function formatDateTime(timestamp?: number | string) {
  if (timestamp === undefined) return undefined;
  const date = typeof timestamp === "number" ? new Date(timestamp * 1000) : new Date(timestamp);
  if (Number.isNaN(date.getTime())) return undefined;
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function formatResetTime(resetAt?: number | string) {
  if (resetAt === undefined) return undefined;
  if (typeof resetAt === "string") return resetAt;
  const dateTime = formatDateTime(resetAt);
  if (!dateTime) return undefined;
  const exact = dateTime.slice(5).replace("-", "/");
  const diff = resetAt - Math.floor(Date.now() / 1000);
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

function stateLabel(state: AccountState) {
  if (state === "available") return "可用";
  return "不可用";
}

function isCurrentAccount(account: ManagedAccount) {
  return account.status?.state === "available" && account.status.label === "当前";
}

function sortAccountsForView(items: ManagedAccount[]) {
  const providerRank: Record<Provider, number> = { codex: 0, gemini: 1 };
  return [...items].sort((a, b) => {
    const providerDelta = providerRank[a.provider] - providerRank[b.provider];
    if (providerDelta !== 0) return providerDelta;
    const emailDelta = a.email.localeCompare(b.email, undefined, { sensitivity: "base" });
    if (emailDelta !== 0) return emailDelta;
    const aName = a.accountName || a.displayName || a.accountId || a.id;
    const bName = b.accountName || b.displayName || b.accountId || b.id;
    const nameDelta = aName.localeCompare(bName, undefined, { sensitivity: "base" });
    if (nameDelta !== 0) return nameDelta;
    return a.id.localeCompare(b.id);
  });
}

function normalizePlanKey(value?: string) {
  return (value || "").trim().toLowerCase();
}

function resolveCodexPlanBadge(account: ManagedAccount) {
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

function resolveValidityUntil(account: ManagedAccount) {
  if (typeof account.subscriptionActiveUntil === "number") return account.subscriptionActiveUntil;
  if (typeof account.subscriptionActiveUntil === "string") {
    const numeric = Number(account.subscriptionActiveUntil);
    if (Number.isFinite(numeric)) return numeric > 1e12 ? Math.floor(numeric / 1000) : numeric;
    const parsed = Date.parse(account.subscriptionActiveUntil);
    if (!Number.isNaN(parsed)) return Math.floor(parsed / 1000);
  }
  return undefined;
}

function formatValidityText(account: ManagedAccount) {
  if ((account.status ?? fallbackStatus(account)).state === "unavailable") {
    return { label: "有效期", detail: "--", title: account.status?.reason };
  }
  const until = resolveValidityUntil(account);
  if (until === undefined || until <= 0) return { label: "有效期", detail: "未知" };
  const now = Math.floor(Date.now() / 1000);
  const remaining = until - now;
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

function fallbackStatus(account: ManagedAccount) {
  const now = Math.floor(Date.now() / 1000);
  if (!account.tokenMeta.hasAccessToken) return { state: "unavailable" as const, label: "不可用", reason: "缺少 access token" };
  if (account.tokenMeta.expiresAt && account.tokenMeta.expiresAt <= now && !account.tokenMeta.hasRefreshToken) {
    return { state: "unavailable" as const, label: "不可用", reason: "本地 token 已过期" };
  }
  return { state: "available" as const, label: "可用" };
}

function AccountStateCorner({ account }: { account: ManagedAccount }) {
  const status = account.status ?? fallbackStatus(account);
  return (
    <span className={clsx("state-corner", status.state, isCurrentAccount(account) && "current")} title={status.reason ?? stateLabel(status.state)}>
      {stateLabel(status.state)}
    </span>
  );
}

function AccountPlanBadge({ account }: { account: ManagedAccount }) {
  const badge = resolveCodexPlanBadge(account);
  return <span className={clsx("pill", "plan", badge.tone)}>{badge.label}</span>;
}

function QuotaMeters({ account }: { account: ManagedAccount }) {
  const isUnavailable = account.status?.state === "unavailable";
  const metrics =
    account.quota?.metrics?.length
      ? account.quota.metrics
      : [
          { key: "codex-5h", label: "5H", remainingPercent: 0 },
          { key: "codex-weekly", label: "WEEKLY", remainingPercent: 0 },
        ];

  return (
    <div className="quota-meters">
      {metrics.slice(0, 3).map((metric) => {
        const remaining = isUnavailable ? 0 : metric.remainingPercent;
        const state = isUnavailable ? "unavailable" : (metric.state ?? (remaining === undefined ? "unknown" : remaining <= 0 ? "unavailable" : remaining <= 15 ? "warning" : "available"));
        const resetText = isUnavailable ? "--" : (formatResetTime(metric.resetAt) ?? "--");
        return (
          <div className={clsx("quota-meter", state)} key={metric.key} title={metric.detail ?? account.quota?.error ?? metric.label}>
            <div className="quota-meter-head">
              <span>{metric.label}</span>
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
      <span>{validity.label} 未知</span>
    </div>
  );
}

function mergeAccounts(current: ManagedAccount[], next: ManagedAccount[]) {
  const map = new Map(current.map((account) => [account.id, account]));
  for (const account of next) map.set(account.id, account);
  return sortAccountsForView([...map.values()]);
}

function NoticeToast({ notice, onClose }: { notice: Notice; onClose: () => void }) {
  const config = noticeToneConfig[notice.tone];
  const Icon = config.icon;

  return (
    <div className="toast" data-tone={notice.tone} role="status" aria-live="polite">
      <Icon className="toast-icon" size={16} aria-hidden="true" />
      <span className="toast-text">
        <b>{config.label}</b>
        {notice.text}
      </span>
      <button onClick={onClose} aria-label="关闭提示">×</button>
    </div>
  );
}

function App() {
  const [accounts, setAccounts] = useState<ManagedAccount[]>([]);
  const [activeProvider, setActiveProvider] = useState<Provider>("codex");
  const [mode, setMode] = useState<ImportMode>("paste");
  const [pasteValue, setPasteValue] = useState("");
  const [failures, setFailures] = useState<ImportFailure[]>([]);
  const [query, setQuery] = useState("");
  const [isImportModalOpen, setIsImportModalOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [pendingDeleteAccount, setPendingDeleteAccount] = useState<ManagedAccount | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [isFileImporting, setIsFileImporting] = useState(false);
  const [refreshingAccountIds, setRefreshingAccountIds] = useState<Set<string>>(() => new Set());
  const [refreshingProvider, setRefreshingProvider] = useState<Provider | null>(null);
  const [pendingOAuth, setPendingOAuth] = useState<Partial<Record<OAuthProvider, string>>>({});
  const [isSettingsLoaded, setIsSettingsLoaded] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [settings, setSettings] = useState<AppSettings>({
    theme: "system",
    autoLaunch: false,
    maskSensitive: false,
  });
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const accountListRef = useRef<HTMLDivElement | null>(null);
  const oauthPollTimers = useRef<Partial<Record<OAuthProvider, number>>>({});
  const noticeTimer = useRef<number | null>(null);
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

  const counts = useMemo(
    () => ({
      codex: accounts.filter((account) => account.provider === "codex").length,
      gemini: accounts.filter((account) => account.provider === "gemini").length,
    }),
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
  }, [filteredAccounts.length, activeProvider, query, updateAccountScrollbar]);

  const closeNotice = useCallback(() => {
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = null;
    setNotice(null);
  }, []);

  const showNotice = useCallback((tone: NoticeTone, text: string) => {
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    setNotice({ tone, text });
    noticeTimer.current = window.setTimeout(() => {
      setNotice(null);
      noticeTimer.current = null;
    }, NOTICE_TIMEOUT_MS);
  }, []);

  const reloadAccountsSoon = (delay = 1800) => {
    window.setTimeout(() => {
      void invoke<ManagedAccount[]>("list_accounts")
        .then((storedAccounts) => setAccounts(storedAccounts))
        .catch(() => undefined);
    }, delay);
  };

  const clearOAuthPoll = (provider: OAuthProvider) => {
    const timer = oauthPollTimers.current[provider];
    if (timer) window.clearTimeout(timer);
    delete oauthPollTimers.current[provider];
  };

  const applyImportResult = (
    result: BackendImportResult,
    options: { closeModal?: boolean; successText?: string } = {},
  ) => {
    if (result.imported.length > 0) {
      setAccounts((current) => mergeAccounts(current, result.imported));
      void invoke("upsert_accounts", { accounts: result.imported }).catch(() => undefined);
      if (options.closeModal ?? true) {
        setIsImportModalOpen(false);
        setPasteValue("");
      }
      showNotice("success", options.successText ?? `已添加 ${result.imported.length} 个账号`);
      reloadAccountsSoon();
    } else if (result.failed.length > 0) {
      showNotice("error", result.failed[0]?.reason ?? "未添加账号");
    } else {
      showNotice("info", "没有发现可添加的账号");
    }
    setFailures(result.failed);
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

  const handlePasteImport = async () => {
    setIsBusy(true);
    try {
      const result = await parseWithBackend(pasteValue, "粘贴内容");
      applyImportResult(result);
    } finally {
      setIsBusy(false);
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
        setFailures(allFailures);
        showNotice("error", allFailures[0]?.reason ?? "没有发现可添加的账号");
      }
    } finally {
      setIsFileImporting(false);
      setIsBusy(false);
    }
  };

  const handleLocalImport = async (provider: OAuthProvider) => {
    setIsBusy(true);
    setFailures([]);
    try {
      const command = provider === "codex" ? "import_codex_from_local" : "import_gemini_from_local";
      const result = await invoke<BackendImportResult>(command);
      applyImportResult(
        result,
      );
    } catch (error) {
      setFailures([]);
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
      showNotice("info", `${providerLabel(provider)} OAuth 仍在等待完成，可点击“完成添加”重试。`);
      return;
    }
    oauthPollTimers.current[provider] = window.setTimeout(() => {
      void completeOAuth(provider, loginId, { silent: true }).then((isComplete) => {
        if (!isComplete) scheduleOAuthPoll(provider, loginId, attempt + 1);
      });
    }, 2000);
  };

  const handleOAuthStart = async (provider: OAuthProvider) => {
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

  const handleOAuthComplete = async (provider: OAuthProvider) => {
    const loginId = pendingOAuth[provider];
    if (!loginId) {
      showNotice("info", `请先启动 ${providerLabel(provider)} OAuth。`);
      return;
    }
    setIsBusy(true);
    try {
      await completeOAuth(provider, loginId);
    } finally {
      setIsBusy(false);
    }
  };

  const selectedMode = modeConfig[mode];
  const ModeIcon = selectedMode.icon;
  const isActiveProviderOAuthPending = Boolean(pendingOAuth[activeProvider]);
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
    setMode("paste");
    setIsImportModalOpen(true);
  };
  const handleSettings = () => {
    setIsSettingsOpen(true);
  };
  const handleLogs = () => {
    showNotice("info", "日志功能待接入。");
  };
  const updateSetting = <Key extends keyof typeof settings>(key: Key, value: (typeof settings)[Key]) => {
    setSettings((current) => ({ ...current, [key]: value }));
  };
  const handleToggleAccount = async (account: ManagedAccount) => {
    if (isCurrentAccount(account)) return;
    setIsBusy(true);
    try {
      const providerAccounts = await invoke<SwitchAccountResult>("switch_account", { accountId: account.id });
      setAccounts((current) =>
        sortAccountsForView(current.map((item) => providerAccounts.find((changed) => changed.id === item.id) ?? item)),
      );
      showNotice("success", `已启用 ${account.email}`);
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
      showNotice("success", `已刷新 ${refreshed.email}`);
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
    setIsBusy(true);
    setRefreshingProvider(activeProvider);
    try {
      const refreshedAccounts = await invoke<ManagedAccount[]>("refresh_provider_accounts", { provider: activeProvider });
      setAccounts((current) =>
        sortAccountsForView(current.map((item) => refreshedAccounts.find((changed) => changed.id === item.id) ?? item)),
      );
      showNotice("success", `已刷新 ${providerLabel(activeProvider)} 账号`);
    } catch (error) {
      showNotice("error", `刷新 ${providerLabel(activeProvider)} 失败：${String(error)}`);
    } finally {
      setRefreshingProvider(null);
      setIsBusy(false);
    }
  };
  const handleExportAccount = async (account: ManagedAccount) => {
    let payload: string;
    try {
      payload = await invoke<string>("export_account", { accountId: account.id });
    } catch (error) {
      showNotice("error", `导出账号失败：${String(error)}`);
      return;
    }
    const blob = new Blob([payload], { type: "application/json;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${account.provider}-${account.email.replace(/[^a-z0-9._-]+/gi, "_")}.json`;
    a.click();
    URL.revokeObjectURL(url);
    showNotice("success", `已导出 ${account.email}`);
  };
  const handleDeleteAccount = (account: ManagedAccount) => {
    setPendingDeleteAccount(account);
  };

  const confirmDeleteAccount = async () => {
    if (!pendingDeleteAccount) return;
    const account = pendingDeleteAccount;
    setIsBusy(true);
    try {
      const nextAccounts = await invoke<ManagedAccount[]>("delete_account", { accountId: account.id });
      setAccounts(nextAccounts);
      setPendingDeleteAccount(null);
      showNotice("success", `已删除 ${account.email}`);
    } catch (error) {
      showNotice("error", `删除账号失败：${String(error)}`);
    } finally {
      setIsBusy(false);
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
  }, []);

  useEffect(() => {
    if (!isSettingsLoaded) return;
    void invoke("save_settings", { settings }).catch(() => undefined);
  }, [isSettingsLoaded, settings]);

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
      className={clsx("shell", settings.maskSensitive && "privacy-mask", (isImportModalOpen || isSettingsOpen || pendingDeleteAccount) && "modal-active")}
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
            <span>Codex / Gemini</span>
          </div>
        </div>

        <nav className="nav-list" aria-label="Providers">
            <button className={clsx(activeProvider === "codex" && "active")} onClick={() => setActiveProvider("codex")}>
            <Bot size={20} />
            <span>Codex</span>
            <b>{counts.codex}</b>
          </button>
          <button className={clsx(activeProvider === "gemini" && "active")} onClick={() => setActiveProvider("gemini")}>
            <Fingerprint size={20} />
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
          <div className="accounts-panel">
            <div className="accounts-toolbar">
              <div className="toolbar-meta">
                  {providerLabel(activeProvider)} · {filteredAccounts.length} 个匹配项
              </div>
              <div className="panel-actions">
                <label className="search">
                  <Search size={18} />
                  <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索邮箱、计划或账号 ID" />
                </label>
                <button className="primary" onClick={handleAddAccount}>
                  <Plus size={18} />
                  添加账号
                </button>
                <button className="secondary refresh-all" onClick={handleRefreshVisibleAccounts} disabled={isBusy}>
                  <RotateCw size={18} className={clsx(refreshingProvider === activeProvider && "spin")} />
                  刷新账号
                </button>
              </div>
            </div>
            <div className="account-scroll-shell">
              <div className="account-list card-mode" ref={accountListRef} onScroll={updateAccountScrollbar}>
                {filteredAccounts.length === 0 && (
                  <div className="empty-state">
                    <FileJson size={54} strokeWidth={1.35} />
                    <strong>暂无账号</strong>
                  </div>
                )}
                {filteredAccounts.map((account) => (
                  <article className={clsx("account-row", account.status?.state === "unavailable" && "disabled", isCurrentAccount(account) && "current")} key={account.id}>
                    <AccountStateCorner account={account} />
                    <div className="account-main">
                      <div className="account-title">
                        <strong>{account.displayName || account.email}</strong>
                        <AccountPlanBadge account={account} />
                      </div>
                      <div className="account-subtitle">
                        <span>
                          <b>邮箱</b>
                          {account.email}
                        </span>
                        {account.accountId && (
                          <span>
                            <b>账号</b>
                            {account.accountId}
                          </span>
                        )}
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
                      <div className="account-actions">
                      <button
                        className="icon-button"
                        aria-label={isCurrentAccount(account) ? "停用当前账号" : "设为当前账号"}
                        title={isCurrentAccount(account) ? "停用" : "设为当前"}
                        onClick={() => handleToggleAccount(account)}
                      >
                        <CirclePlay size={15} strokeWidth={1.75} />
                      </button>
                      <button className="icon-button" aria-label="刷新账号" title="刷新" onClick={() => handleRefreshAccount(account)} disabled={refreshingAccountIds.has(account.id)}>
                        <RefreshCcw size={15} strokeWidth={1.75} className={clsx(refreshingAccountIds.has(account.id) && "spin")} />
                      </button>
                      <button className="icon-button" aria-label="导出账号" title="导出" onClick={() => void handleExportAccount(account)}>
                        <FileDown size={15} strokeWidth={1.75} />
                      </button>
                      <button className="icon-button danger" aria-label="删除账号" title="删除" onClick={() => handleDeleteAccount(account)}>
                        <Trash size={15} strokeWidth={1.75} />
                      </button>
                      </div>
                    </div>
                  </article>
                ))}
              </div>
              <div className={clsx("account-scrollbar", accountScrollbar.visible && "visible")} aria-hidden="true">
                <i style={{ height: accountScrollbar.height, transform: `translateY(${accountScrollbar.top}px)` }} />
              </div>
            </div>
          </div>
        </section>
      </section>

      {notice && (
        <NoticeToast notice={notice} onClose={closeNotice} />
      )}

      {isImportModalOpen && (
        <div className="modal-overlay">
          <aside className="import-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="panel-head">
              <div>
                <h2>添加账号</h2>
                <p>选择添加方式</p>
              </div>
              <button className="modal-close-button" onClick={() => setIsImportModalOpen(false)} aria-label="关闭添加账号">
                <X size={22} strokeWidth={2.2} />
              </button>
            </div>

            <div className="mode-grid">
              {importModeOrder.map((key) => {
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
                  <p>{selectedMode.desc}</p>
                </div>
              </div>

              {mode === "paste" && (
                <>
                  <textarea
                    value={pasteValue}
                    onChange={(event) => setPasteValue(event.target.value)}
                    spellCheck={false}
                    placeholder={'{\n  "tokens": {\n    "id_token": "eyJ...",\n    "access_token": "eyJ...",\n    "refresh_token": "rt_..."\n  }\n}'}
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
                    <Upload size={28} className={clsx(isFileImporting && "spin")} />
                    <strong>{isFileImporting ? "正在导入 JSON..." : "选择 JSON 文件"}</strong>
                    <span>{isFileImporting ? "正在解析并刷新账号信息" : "支持 auth.json、oauth_creds.json、导出数组"}</span>
                  </button>
                </>
              )}

              {mode === "local" && (
                <div className="oauth-flow">
                  <div>
                    <span>1</span>
                    <p>
                      读取本机 {providerLabel(activeProvider)} 账号配置
                    </p>
                  </div>
                  <button className="wide primary" onClick={() => handleLocalImport(activeProvider)} disabled={isBusy}>
                    <FolderDown size={20} />
                    读取 {providerLabel(activeProvider)} 本机账号
                  </button>
                </div>
              )}

              {mode === "oauth" && (
                <div className="oauth-flow">
                  <div>
                    <span>1</span>
                    <p>
                      启动 {providerLabel(activeProvider)} OAuth 登录
                    </p>
                  </div>
                  <button className="wide primary" onClick={() => handleOAuthStart(activeProvider)} disabled={isBusy || isActiveProviderOAuthPending}>
                    <LockKeyhole size={20} />
                    {isActiveProviderOAuthPending ? "等待授权完成..." : `启动 ${providerLabel(activeProvider)} OAuth`}
                  </button>
                  <button className="wide" onClick={() => handleOAuthComplete(activeProvider)} disabled={isBusy || !isActiveProviderOAuthPending}>
                    <BadgeCheck size={20} />
                    我已完成授权，立即添加
                  </button>
                </div>
              )}
            </div>

            {failures.length > 0 && (
              <div className="failure-list">
                <strong>未添加项</strong>
                {failures.slice(0, 4).map((failure) => (
                  <p key={`${failure.label}:${failure.reason}`}>
                    {failure.label}: {failure.reason}
                  </p>
                ))}
              </div>
            )}
          </aside>
        </div>
      )}

      {isSettingsOpen && (
        <div className="modal-overlay" onMouseDown={(event) => handleModalBackdropMouseDown(event, () => setIsSettingsOpen(false))}>
          <aside className="settings-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="panel-head">
              <div>
                <h2>设置</h2>
                <p>外观、启动和本地隐私</p>
              </div>
            </div>

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

              <p className="setting-note">开机自启会在接入 Tauri 后端后写入系统登录项；当前面板已保留配置入口。</p>
            </div>
          </aside>
        </div>
      )}

      {pendingDeleteAccount && (
        <div className="modal-overlay" onMouseDown={(event) => handleModalBackdropMouseDown(event, () => setPendingDeleteAccount(null))}>
          <aside className="confirm-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="confirm-icon danger">
              <Trash size={22} />
            </div>
            <div className="confirm-copy">
              <h2>删除账号</h2>
              <p>{pendingDeleteAccount.email}</p>
              <span>只会从 Super AI 的账号库移除，不会删除本机 Codex/Gemini 当前配置。</span>
            </div>
            <div className="confirm-actions">
              <button className="secondary" onClick={() => setPendingDeleteAccount(null)} disabled={isBusy}>
                取消
              </button>
              <button className="danger-button" onClick={() => void confirmDeleteAccount()} disabled={isBusy}>
                {isBusy ? "删除中..." : "删除"}
              </button>
            </div>
          </aside>
        </div>
      )}
    </main>
  );
}

export default App;
