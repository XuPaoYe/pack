import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  BadgeCheck,
  Bot,
  ChevronRight,
  Clipboard,
  Cloud,
  ExternalLink,
  FileJson,
  Fingerprint,
  FolderDown,
  Laptop,
  LockKeyhole,
  Plus,
  RefreshCw,
  Search,
  Settings,
  ShieldCheck,
  Upload,
} from "lucide-react";
import "./App.css";
import logoUrl from "./assets/logo.svg";
import { parseAuthJson, type ImportFailure, type ManagedAccount, type Provider } from "./lib/authParser";

type ImportMode = "paste" | "file" | "local" | "oauth";
type ViewMode = "list" | "card";
type OAuthProvider = "codex" | "gemini";

const viewModeStorageKey = "super-ai.account-view-mode";

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

function mergeAccounts(current: ManagedAccount[], next: ManagedAccount[]) {
  const map = new Map(current.map((account) => [account.id, account]));
  for (const account of next) map.set(account.id, account);
  return [...map.values()].sort((a, b) => b.updatedAt - a.updatedAt);
}

function readStoredViewMode(): ViewMode {
  try {
    const stored = window.localStorage.getItem(viewModeStorageKey);
    return stored === "card" || stored === "list" ? stored : "list";
  } catch {
    return "list";
  }
}

function App() {
  const [accounts, setAccounts] = useState<ManagedAccount[]>(seedAccounts);
  const [activeProvider, setActiveProvider] = useState<Provider>("codex");
  const [mode, setMode] = useState<ImportMode>("paste");
  const [viewMode, setViewMode] = useState<ViewMode>(() => readStoredViewMode());
  const [pasteValue, setPasteValue] = useState("");
  const [failures, setFailures] = useState<ImportFailure[]>([]);
  const [query, setQuery] = useState("");
  const [notice, setNotice] = useState("原型已就绪：粘贴 JSON 或选择文件即可测试解析。");
  const [isImportModalOpen, setIsImportModalOpen] = useState(false);
  const [isBusy, setIsBusy] = useState(false);
  const [pendingOAuth, setPendingOAuth] = useState<Partial<Record<OAuthProvider, string>>>({});
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const appWindow = useMemo(() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(viewModeStorageKey, viewMode);
    } catch {
      // Ignore storage failures; the view still works for the current session.
    }
  }, [viewMode]);

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
  const handleRefresh = () => {
    setNotice(`已刷新界面状态：当前管理 ${accounts.length} 个账号。`);
  };
  const handleAddAccount = () => {
    setMode("paste");
    setIsImportModalOpen(true);
  };
  const handleSettings = () => {
    setNotice("设置页稍后接入：这里会放主题、隐私模式、本地路径和 OAuth 参数。");
  };
  const handleAccountOpen = (account: ManagedAccount) => {
    setNotice(`已选中 ${providerLabel(account.provider)} 账号：${account.email}`);
  };
  const handleOpenStore = (event: React.MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    setNotice("正在打开 Super Store AI 权益页面...");
    void openUrl("https://ai.talentisan.cn/");
  };

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
            <Bot size={17} />
            <span>Codex</span>
            <b>{counts.codex}</b>
          </button>
          <button className={clsx(activeProvider === "gemini" && "active")} onClick={() => setActiveProvider("gemini")}>
            <Fingerprint size={17} />
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
            <ExternalLink size={16} />
          </a>
          <button onClick={handleSettings}>
            <Settings size={17} />
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
              <Search size={16} />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索邮箱、计划或账号 ID" />
            </label>
            <button className="primary" onClick={handleAddAccount}>
              <Plus size={17} />
              添加账号
            </button>
          </div>
        </header>

        <section className="status-strip">
          <div>
            <ShieldCheck size={18} />
            <span>{notice}</span>
          </div>
          <button onClick={handleRefresh}>
            <RefreshCw size={16} />
            刷新状态
          </button>
        </section>

        <section className="content-grid">
          <div className="accounts-panel">
            <div className="panel-head">
              <div>
                <h2>账号</h2>
                <p>
                  {providerLabel(activeProvider)} · {filteredAccounts.length} 个匹配项
                </p>
              </div>
              <div className="segmented">
                <button className={clsx(viewMode === "list" && "active")} onClick={() => setViewMode("list")}>
                  列表
                </button>
                <button className={clsx(viewMode === "card" && "active")} onClick={() => setViewMode("card")}>
                  卡片
                </button>
              </div>
            </div>

            <div className={clsx("account-list", viewMode === "card" && "card-mode")}>
              {filteredAccounts.map((account) => (
                <article className="account-row" key={account.id}>
                  <div className="account-provider">
                    <div className={clsx("provider-dot", providerClass(account.provider))}>
                      {account.provider === "codex" ? <Bot size={18} /> : <Fingerprint size={18} />}
                    </div>
                  </div>
                  <div className="account-main">
                    <div className="account-title">
                      <strong>{account.displayName || account.email}</strong>
                      <span className={clsx("pill", providerClass(account.provider))}>{providerLabel(account.provider)}</span>
                      {account.tokenMeta.hasRefreshToken && (
                        <span className="pill muted">
                          <BadgeCheck size={13} />
                          Refresh
                        </span>
                      )}
                    </div>
                    <p>{account.email}</p>
                  </div>
                  <div className="account-meta">
                    <div>
                      <span>{account.plan || "Unknown Plan"}</span>
                      <small>{formatRelative(account.updatedAt)}</small>
                    </div>
                  </div>
                  <button className="icon-button" aria-label="Open Account" onClick={() => handleAccountOpen(account)}>
                    <ChevronRight size={18} />
                  </button>
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
                    <Icon size={17} />
                    <span>{item.title}</span>
                  </button>
                );
              })}
            </div>

            <div className="import-body">
              <div className="import-title">
                <ModeIcon size={20} />
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
                    <Clipboard size={17} />
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
                    <Upload size={22} />
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
                    <FolderDown size={17} />
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
                    <LockKeyhole size={17} />
                    启动 {providerLabel(activeProvider)} OAuth
                  </button>
                  <button className="wide" onClick={() => handleOAuthComplete(activeProvider)} disabled={isBusy}>
                    <BadgeCheck size={17} />
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
