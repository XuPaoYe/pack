import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  AlertTriangle,
  BadgeCheck,
  Bot,
  Clipboard,
  Cloud,
  Clock3,
  Download,
  ExternalLink,
  FileJson,
  Fingerprint,
  FolderDown,
  Laptop,
  LockKeyhole,
  Plus,
  Power,
  RefreshCw,
  Search,
  Settings,
  ShieldCheck,
  Trash2,
  Upload,
} from "lucide-react";
import "./App.css";
import logoUrl from "./assets/logo.svg";
import { parseAuthJson, type AccountState, type ImportFailure, type ManagedAccount, type Provider } from "./lib/authParser";

type ImportMode = "paste" | "file" | "local" | "oauth";
type OAuthProvider = "codex" | "gemini";

const autoRefreshIntervalSeconds = 300;

type BackendImportResult = {
  imported: ManagedAccount[];
  failed: ImportFailure[];
};

type OAuthStartResult = {
  login_id: string;
  provider: OAuthProvider;
  command: string;
  message: string;
};

const seedAccounts: ManagedAccount[] = [
  {
    id: "codex_preview",
    provider: "codex",
    email: "personal@example.com",
    displayName: "Personal Codex",
    plan: "Plus",
    accountId: "acct_preview",
    userId: "user_preview",
    source: "local",
    tokenMeta: {
      hasAccessToken: true,
      hasRefreshToken: true,
      hasIdToken: true,
      expiresAt: Math.floor(Date.now() / 1000) + 3600 * 24,
    },
    accountName: "Personal Codex",
    planType: "plus",
    subscriptionActiveUntil: Math.floor(Date.now() / 1000) + 3600 * 24 * 30,
    status: {
      state: "available",
      label: "可用",
      updatedAt: Math.floor(Date.now() / 1000) - 900,
    },
    quota: {
      metrics: [
        {
          key: "codex-5h",
          label: "5H",
          remainingPercent: 68,
          resetAt: Math.floor(Date.now() / 1000) + 3600 * 2,
          state: "available",
        },
        {
          key: "codex-weekly",
          label: "WEEKLY",
          remainingPercent: 24,
          resetAt: Math.floor(Date.now() / 1000) + 3600 * 28,
          state: "warning",
        },
      ],
      lastUpdated: Math.floor(Date.now() / 1000) - 900,
    },
    createdAt: Math.floor(Date.now() / 1000) - 3600 * 48,
    updatedAt: Math.floor(Date.now() / 1000) - 900,
  },
  {
    id: "gemini_preview",
    provider: "gemini",
    email: "work@example.com",
    displayName: "Workspace Gemini",
    plan: "Gemini Code Assist",
    accountId: "google_preview",
    userId: "google_preview",
    source: "oauth",
    tokenMeta: {
      hasAccessToken: true,
      hasRefreshToken: true,
      hasIdToken: false,
    },
    accountName: "Workspace Gemini",
    status: {
      state: "unavailable",
      label: "不可用",
      reason: "最近一次额度查询返回 403",
      updatedAt: Math.floor(Date.now() / 1000) - 420,
    },
    quota: {
      metrics: [
        {
          key: "gemini-pro",
          label: "PRO",
          remainingPercent: 0,
          detail: "403",
          state: "unavailable",
        },
        {
          key: "gemini-flash",
          label: "FLASH",
          remainingPercent: 42,
          state: "available",
        },
      ],
      error: "最近一次额度查询返回 403",
      lastUpdated: Math.floor(Date.now() / 1000) - 420,
    },
    createdAt: Math.floor(Date.now() / 1000) - 3600 * 12,
    updatedAt: Math.floor(Date.now() / 1000) - 420,
  },
];

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
    title: "粘贴 auth.json",
    desc: "适合从 Codex auth.json、Gemini oauth_creds.json 或导出数组快速导入。",
  },
  file: {
    icon: FileJson,
    title: "导入 JSON 文件",
    desc: "支持单文件、多文件，后续桌面版会接入系统文件选择器。",
  },
  local: {
    icon: Laptop,
    title: "读取本机账号",
    desc: "桌面后端读取 ~/.codex/auth.json 与 ~/.gemini，本原型先展示流程。",
  },
  oauth: {
    icon: Cloud,
    title: "OAuth 授权",
    desc: "通过本地 Callback 完成授权；Codex 与 Gemini 将分别走官方 OAuth。",
  },
};

function providerLabel(provider: Provider) {
  return provider === "codex" ? "Codex" : "Gemini Cli";
}

function providerClass(provider: Provider) {
  return provider === "codex" ? "codex" : "gemini";
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

function formatReset(resetAt?: number | string) {
  if (resetAt === undefined) return undefined;
  if (typeof resetAt === "string") return resetAt;
  const diff = resetAt - Math.floor(Date.now() / 1000);
  if (diff <= 0) return "已重置";
  if (diff < 3600) return `${Math.ceil(diff / 60)}m`;
  if (diff < 86400) return `${Math.ceil(diff / 3600)}h`;
  return `${Math.ceil(diff / 86400)}d`;
}

function formatCountdown(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60);
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  const remainingSeconds = total % 60;
  if (hours > 0) return `${hours}:${String(remainingMinutes).padStart(2, "0")}:${String(remainingSeconds).padStart(2, "0")}`;
  return `${String(remainingMinutes).padStart(2, "0")}:${String(remainingSeconds).padStart(2, "0")}`;
}

function stateLabel(state: AccountState) {
  if (state === "available") return "可用";
  if (state === "warning") return "需关注";
  if (state === "unavailable") return "不可用";
  return "未知";
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
    const parsed = Date.parse(account.subscriptionActiveUntil);
    if (!Number.isNaN(parsed)) return Math.floor(parsed / 1000);
    const numeric = Number(account.subscriptionActiveUntil);
    if (Number.isFinite(numeric)) return numeric > 1e12 ? Math.floor(numeric / 1000) : numeric;
  }
  return account.tokenMeta.expiresAt;
}

function formatValidityText(account: ManagedAccount) {
  const until = resolveValidityUntil(account);
  if (until === undefined) return { label: "有效期", detail: "未读到有效期信息" };
  const now = Math.floor(Date.now() / 1000);
  const remaining = until - now;
  if (remaining <= 0) {
    return { label: "有效期", detail: formatDateTime(until) ?? "已过期", expired: true };
  }
  return {
    label: "有效期",
    detail: formatDateTime(until) ?? "可用",
  };
}

function fallbackStatus(account: ManagedAccount) {
  const now = Math.floor(Date.now() / 1000);
  if (!account.tokenMeta.hasAccessToken) return { state: "unavailable" as const, label: "不可用", reason: "缺少 access token" };
  if (account.tokenMeta.expiresAt && account.tokenMeta.expiresAt <= now) {
    return { state: "unavailable" as const, label: "已过期", reason: "本地 token 已过期" };
  }
  if (!account.tokenMeta.hasRefreshToken) return { state: "warning" as const, label: "需关注", reason: "缺少 refresh token" };
  return { state: "available" as const, label: "可用" };
}

function AccountStatusBadge({ account }: { account: ManagedAccount }) {
  const status = account.status ?? fallbackStatus(account);
  const Icon = status.state === "available" ? ShieldCheck : AlertTriangle;
  return (
    <span className={clsx("status-badge", status.state)} title={status.reason ?? stateLabel(status.state)}>
      <Icon size={15} />
      {status.label}
    </span>
  );
}

function AccountPlanBadge({ account }: { account: ManagedAccount }) {
  const badge = resolveCodexPlanBadge(account);
  return <span className={clsx("pill", "plan", badge.tone)}>{badge.label}</span>;
}

function QuotaMeters({ account }: { account: ManagedAccount }) {
  const metrics = account.quota?.metrics ?? [];
  if (account.quota?.error && metrics.length === 0) {
    return (
      <div className="quota-empty warning">
        <AlertTriangle size={15} />
        <span>额度查询失败</span>
      </div>
    );
  }

  if (metrics.length === 0) {
    return <div className="quota-empty">暂无配额数据</div>;
  }

  return (
    <div className="quota-meters">
      {metrics.slice(0, 3).map((metric) => {
        const remaining = metric.remainingPercent;
        const state = metric.state ?? (remaining === undefined ? "unknown" : remaining <= 0 ? "unavailable" : remaining <= 15 ? "warning" : "available");
        return (
          <div className="quota-meter" key={metric.key} title={metric.detail ?? account.quota?.error ?? metric.label}>
            <div className="quota-meter-head">
              <span>{metric.label}</span>
              <strong>{remaining === undefined ? "N/A" : `${remaining}%`}</strong>
            </div>
            <div className={clsx("quota-track", state)}>
              <i style={{ width: `${remaining ?? 0}%` }} />
            </div>
            <small>{formatReset(metric.resetAt) ?? metric.detail ?? (state === "unavailable" ? "不可用" : "可用")}</small>
          </div>
        );
      })}
    </div>
  );
}

function ValidityMeter({ account }: { account: ManagedAccount }) {
  const validity = formatValidityText(account);
  return validity.detail ? (
    <div className={clsx("validity-line", validity.expired && "expired")}>
      <span>{validity.label}</span>
      <strong>{validity.detail}</strong>
    </div>
  ) : (
    <div className="validity-line">
      <span>{validity.label}</span>
      <strong>未读到有效期信息</strong>
    </div>
  );
}

function serializeAccount(account: ManagedAccount) {
  return JSON.stringify(account, null, 2);
}

function mergeAccounts(current: ManagedAccount[], next: ManagedAccount[]) {
  const map = new Map(current.map((account) => [account.id, account]));
  for (const account of next) map.set(account.id, account);
  return [...map.values()].sort((a, b) => b.updatedAt - a.updatedAt);
}

function App() {
  const [accounts, setAccounts] = useState<ManagedAccount[]>(seedAccounts);
  const [activeProvider, setActiveProvider] = useState<Provider>("codex");
  const [mode, setMode] = useState<ImportMode>("paste");
  const [pasteValue, setPasteValue] = useState("");
  const [failures, setFailures] = useState<ImportFailure[]>([]);
  const [query, setQuery] = useState("");
  const [notice, setNotice] = useState("原型已就绪：粘贴 JSON 或选择文件即可测试解析。");
  const [isImportModalOpen, setIsImportModalOpen] = useState(false);
  const [isBusy, setIsBusy] = useState(false);
  const [isBatchDetecting, setIsBatchDetecting] = useState(false);
  const [lastBatchDetectAt, setLastBatchDetectAt] = useState<number>(() => Math.floor(Date.now() / 1000));
  const [autoRefreshRemaining, setAutoRefreshRemaining] = useState(autoRefreshIntervalSeconds);
  const [pendingOAuth, setPendingOAuth] = useState<Partial<Record<OAuthProvider, string>>>({});
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const appWindow = useMemo(() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  }, []);

  const filteredAccounts = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return accounts.filter((account) => {
      if (account.provider !== activeProvider) return false;
      if (!normalizedQuery) return true;
      return [account.email, account.displayName, account.plan, account.accountId]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(normalizedQuery));
    });
  }, [accounts, activeProvider, query]);

  const counts = useMemo(
    () => ({
      codex: accounts.filter((account) => account.provider === "codex").length,
      gemini: accounts.filter((account) => account.provider === "gemini").length,
    }),
    [accounts],
  );

  const applyImportResult = (result: BackendImportResult, successText: string, emptyText: string) => {
    const importedForCurrentProvider = result.imported.filter((account) => account.provider === activeProvider);
    const skippedCount = result.imported.length - importedForCurrentProvider.length;
    if (result.imported.length > 0 && importedForCurrentProvider.length > 0) {
      if (importedForCurrentProvider.length > 0) {
        setAccounts((current) => mergeAccounts(current, importedForCurrentProvider));
      }
      const baseNotice = successText.replace("{count}", String(importedForCurrentProvider.length));
      setNotice(skippedCount > 0 ? `${baseNotice}（已跳过 ${skippedCount} 个非当前平台账号）` : baseNotice);
    } else if (result.imported.length > 0 && importedForCurrentProvider.length === 0) {
      setNotice(`没有导入 ${providerLabel(activeProvider)} 账号（已识别到 ${skippedCount} 个其他平台账号并跳过）。`);
    } else {
      setNotice(emptyText);
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
      applyImportResult(result, "导入成功：{count} 个账号已加入本地列表。", "没有导入账号，检查 JSON 是否包含 Codex/Gemini 凭证字段。");
    } finally {
      setIsBusy(false);
    }
  };

  const handleFileImport = async (files: FileList | null) => {
    if (!files?.length) return;
    setIsBusy(true);
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
          "文件导入完成：新增或更新 {count} 个账号。",
          "文件读取完成，但没有识别到可导入账号。",
        );
      } else {
        setNotice("文件读取完成，但没有识别到可导入账号。");
        setFailures(allFailures);
      }
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
        `读取本机 ${providerLabel(provider)} 成功：{count} 个账号已加入列表。`,
        `未从本机读取到 ${providerLabel(provider)} 账号。`,
      );
    } catch (error) {
      setNotice(`读取本机 ${providerLabel(provider)} 失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
  };

  const handleOAuthStart = async (provider: OAuthProvider) => {
    setIsBusy(true);
    try {
      const command = provider === "codex" ? "start_codex_oauth" : "start_gemini_oauth";
      const result = await invoke<OAuthStartResult>(command);
      setPendingOAuth((current) => ({ ...current, [provider]: result.login_id }));
      setNotice(result.message);
    } catch (error) {
      setNotice(`${providerLabel(provider)} OAuth 启动失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
  };

  const handleOAuthComplete = async (provider: OAuthProvider) => {
    const loginId = pendingOAuth[provider];
    if (!loginId) {
      setNotice(`请先启动 ${providerLabel(provider)} OAuth。`);
      return;
    }
    setIsBusy(true);
    try {
      const command = provider === "codex" ? "complete_codex_oauth" : "complete_gemini_oauth";
      const result = await invoke<BackendImportResult>(command, { loginId });
      applyImportResult(
        result,
        `${providerLabel(provider)} OAuth 完成：导入 {count} 个账号。`,
        `${providerLabel(provider)} OAuth 已完成，但未读取到账号。`,
      );
      setPendingOAuth((current) => ({ ...current, [provider]: undefined }));
    } catch (error) {
      setNotice(`${providerLabel(provider)} OAuth 完成失败：${String(error)}`);
    } finally {
      setIsBusy(false);
    }
  };

  const selectedMode = modeConfig[mode];
  const ModeIcon = selectedMode.icon;
  const startWindowDrag = (event: React.MouseEvent<HTMLElement>) => {
    if (event.target instanceof HTMLElement && event.target.closest("button, input, textarea, select, a, label")) return;
    if (event.button !== 0) return;
    event.preventDefault();
    void invoke("start_window_drag").catch(() => {
      void appWindow?.startDragging().catch(() => {
        setNotice("窗口拖拽未启动：请拖动窗口顶部空白区域。");
      });
    });
  };
  const handleAddAccount = () => {
    setMode("paste");
    setIsImportModalOpen(true);
  };
  const handleSettings = () => {
    setNotice("设置页稍后接入：这里会放主题、隐私模式、本地路径和 OAuth 参数。");
  };
  const updateAccount = (accountId: string, updater: (account: ManagedAccount) => ManagedAccount) => {
    setAccounts((current) => current.map((account) => (account.id === accountId ? updater(account) : account)));
  };
  const handleToggleAccount = (account: ManagedAccount) => {
    const now = Math.floor(Date.now() / 1000);
    const nextStatus =
      account.status?.state === "unavailable"
        ? { state: "available" as const, label: "可用", updatedAt: now }
        : { state: "unavailable" as const, label: "不可用", reason: "已在本地停用", updatedAt: now };
    updateAccount(account.id, (current) => ({
      ...current,
      status: nextStatus,
      updatedAt: now,
    }));
    setNotice(`${providerLabel(account.provider)} 账号 ${account.email} 已${nextStatus.state === "available" ? "启用" : "停用"}。`);
  };
  const handleRefreshAccount = (account: ManagedAccount) => {
    const now = Math.floor(Date.now() / 1000);
    updateAccount(account.id, (current) => ({
      ...current,
      updatedAt: now,
      status: current.status
        ? {
            ...current.status,
            updatedAt: now,
          }
        : {
            state: "available",
            label: "可用",
            updatedAt: now,
          },
      quota: current.quota
        ? {
            ...current.quota,
            lastUpdated: now,
          }
        : current.quota,
    }));
    setNotice(`已刷新 ${providerLabel(account.provider)} 账号：${account.email}`);
  };
  const handleExportAccount = async (account: ManagedAccount) => {
    const payload = serializeAccount(account);
    const blob = new Blob([payload], { type: "application/json;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${account.provider}-${account.email.replace(/[^a-z0-9._-]+/gi, "_")}.json`;
    a.click();
    URL.revokeObjectURL(url);
    setNotice(`已导出 ${providerLabel(account.provider)} 账号：${account.email}`);
  };
  const handleDeleteAccount = (account: ManagedAccount) => {
    const confirmed = window.confirm(`删除 ${account.email}？这会从当前列表移除本地预览。`);
    if (!confirmed) return;
    setAccounts((current) => current.filter((item) => item.id !== account.id));
    setNotice(`已删除 ${providerLabel(account.provider)} 账号：${account.email}`);
  };
  const handleOpenStore = (event: React.MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    setNotice("正在打开 Super Store AI 权益页面...");
    void openUrl("https://ai.talentisan.cn/");
  };

  const handleBatchDetect = async (reason: "startup" | "manual" | "timer" = "manual") => {
    const now = Math.floor(Date.now() / 1000);
    setIsBatchDetecting(true);
    await new Promise((resolve) => window.setTimeout(resolve, 150));
    let accountCount = 0;
    setAccounts((current) => {
      accountCount = current.length;
      return current
        .map((account): ManagedAccount => ({
          ...account,
          status: account.status
            ? { ...account.status, updatedAt: now }
            : {
                state: account.tokenMeta.hasAccessToken ? "available" : "unavailable",
                label: account.tokenMeta.hasAccessToken ? "可用" : "不可用",
                reason: account.tokenMeta.hasAccessToken ? undefined : "缺少 access token",
                updatedAt: now,
              },
          quota: account.quota ? { ...account.quota, lastUpdated: now } : account.quota,
          updatedAt: now,
        }))
        .sort((a, b) => b.updatedAt - a.updatedAt);
    });
    setLastBatchDetectAt(now);
    setAutoRefreshRemaining(autoRefreshIntervalSeconds);
    setIsBatchDetecting(false);
    const prefix =
      reason === "startup" ? "已启动批量检测" : reason === "timer" ? "定时批量检测完成" : "已完成批量检测";
    setNotice(`${prefix}：当前管理 ${accountCount || accounts.length} 个账号。`);
  };

  useEffect(() => {
    const startupTimer = window.setTimeout(() => {
      void handleBatchDetect("startup");
    }, 0);
    const timer = window.setInterval(() => {
      setAutoRefreshRemaining((current) => {
        if (current <= 1) {
          void handleBatchDetect("timer");
          return autoRefreshIntervalSeconds;
        }
        return current - 1;
      });
    }, 1000);
    return () => {
      window.clearTimeout(startupTimer);
      window.clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <main className="shell">
      <div className="global-drag-region" data-tauri-drag-region onMouseDown={startWindowDrag} />
      <div className="top-edge-drag-region" data-tauri-drag-region onMouseDown={startWindowDrag} />
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
          <button onClick={handleSettings}>
            <Settings size={18} />
            设置
          </button>
          <p>所有凭证只在本机处理。桌面版会把敏感读写放进 Rust 后端。</p>
        </div>
      </aside>

      <section className="workspace">
        <header className="topbar" data-tauri-drag-region onMouseDown={startWindowDrag}>
          <div className="title-drag drag-surface" data-tauri-drag-region onMouseDown={startWindowDrag}>
            <p className="eyebrow">Account Control Center</p>
            <h1>Super AI 账号管理</h1>
          </div>
          <div className="topbar-actions">
            <label className="search">
              <Search size={18} />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索邮箱、计划或账号 ID" />
            </label>
            <button className="primary" onClick={handleAddAccount}>
              <Plus size={18} />
              添加账号
            </button>
          </div>
        </header>

        <section className="status-strip">
          <div className="status-copy">
            <ShieldCheck size={20} />
            <span>{notice}</span>
          </div>
          <div className="status-actions">
            <span className="countdown-pill" title={formatDateTime(lastBatchDetectAt)}>
              <Clock3 size={15} />
              自动检测 {formatCountdown(autoRefreshRemaining)}
            </span>
            <button onClick={() => void handleBatchDetect("manual")} disabled={isBatchDetecting}>
              <RefreshCw size={18} />
              {isBatchDetecting ? "检测中..." : "立即检测"}
            </button>
          </div>
        </section>

        <section className="content-grid">
          <div className="accounts-panel">
            <div className="panel-head">
              <div>
                <h2>账号列表</h2>
                <p>
                  {providerLabel(activeProvider)} · {filteredAccounts.length} 个匹配项
                </p>
              </div>
            </div>

            <div className="account-list card-mode">
              {filteredAccounts.map((account) => (
                <article className={clsx("account-row", account.status?.state === "unavailable" && "disabled")} key={account.id}>
                  <div className="account-provider">
                    <div className={clsx("provider-dot", providerClass(account.provider))}>
                      {account.provider === "codex" ? <Bot size={24} /> : <Fingerprint size={24} />}
                    </div>
                  </div>
                  <div className="account-main">
                    <div className="account-title">
                      <strong>{account.displayName || account.email}</strong>
                      <AccountPlanBadge account={account} />
                      <AccountStatusBadge account={account} />
                      {account.tokenMeta.hasRefreshToken && (
                        <span className="pill muted">
                          <BadgeCheck size={15} />
                          Refresh
                        </span>
                      )}
                    </div>
                    <div className="account-subtitle">
                      <span>用户名：{account.accountName || account.displayName || account.userId || account.email.split("@")[0]}</span>
                      <span>邮箱：{account.email}</span>
                      {account.organizationId && <span>组织：{account.organizationId}</span>}
                    </div>
                  </div>
                  <div className="account-meta">
                    <div>
                      <span>最后检测</span>
                      <small>{formatRelative(account.updatedAt)}</small>
                    </div>
                  </div>
                  <ValidityMeter account={account} />
                  <QuotaMeters account={account} />
                  <div className="account-actions">
                    <button
                      className="icon-button"
                      aria-label={account.status?.state === "unavailable" ? "启用账号" : "停用账号"}
                      title={account.status?.state === "unavailable" ? "启用" : "停用"}
                      onClick={() => handleToggleAccount(account)}
                    >
                      <Power size={20} />
                    </button>
                    <button className="icon-button" aria-label="刷新账号" title="刷新" onClick={() => handleRefreshAccount(account)}>
                      <RefreshCw size={20} />
                    </button>
                    <button className="icon-button" aria-label="导出账号" title="导出" onClick={() => void handleExportAccount(account)}>
                      <Download size={20} />
                    </button>
                    <button className="icon-button danger" aria-label="删除账号" title="删除" onClick={() => handleDeleteAccount(account)}>
                      <Trash2 size={20} />
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </div>
        </section>
      </section>

      {isImportModalOpen && (
        <div className="modal-overlay" onMouseDown={() => setIsImportModalOpen(false)}>
          <aside className="import-panel modal-content" onMouseDown={(e) => e.stopPropagation()}>
            <div className="panel-head">
              <div>
                <h2>导入</h2>
                <p>选择导入方式添加账号</p>
              </div>
            </div>

            <div className="mode-grid">
              {(Object.keys(modeConfig) as ImportMode[]).map((key) => {
                const item = modeConfig[key];
                const Icon = item.icon;
                return (
                  <button key={key} className={clsx("mode-button", mode === key && "active")} onClick={() => setMode(key)}>
                    <Icon size={20} />
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
                    {isBusy ? "处理中..." : "解析并导入"}
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
                  <button className="drop-zone" onClick={() => fileInputRef.current?.click()} disabled={isBusy}>
                    <Upload size={28} />
                    <strong>选择 JSON 文件</strong>
                    <span>支持 auth.json、oauth_creds.json、导出数组</span>
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
                  <button className="wide" onClick={() => handleLocalImport(activeProvider)} disabled={isBusy}>
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
                      启动 {providerLabel(activeProvider)} 终端 OAuth 登录
                    </p>
                  </div>
                  <button className="wide" onClick={() => handleOAuthStart(activeProvider)} disabled={isBusy}>
                    <LockKeyhole size={20} />
                    启动 {providerLabel(activeProvider)} OAuth
                  </button>
                  <button className="wide" onClick={() => handleOAuthComplete(activeProvider)} disabled={isBusy}>
                    <BadgeCheck size={20} />
                    完成 {providerLabel(activeProvider)} 导入
                  </button>
                </div>
              )}
            </div>

            {failures.length > 0 && (
              <div className="failure-list">
                <strong>未导入项</strong>
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
    </main>
  );
}

export default App;
