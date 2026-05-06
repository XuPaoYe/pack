import { useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  BadgeCheck,
  ChevronRight,
  Clipboard,
  Cloud,
  Database,
  ExternalLink,
  FileJson,
  Fingerprint,
  FolderDown,
  KeyRound,
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
    desc: "通过本地 callback 完成授权；Codex 与 Gemini 将分别走官方 OAuth。",
  },
};

function providerLabel(provider: Provider) {
  return provider === "codex" ? "Codex" : "Gemini";
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

function App() {
  const [accounts, setAccounts] = useState<ManagedAccount[]>(seedAccounts);
  const [activeProvider, setActiveProvider] = useState<Provider | "all">("all");
  const [mode, setMode] = useState<ImportMode>("paste");
  const [viewMode, setViewMode] = useState<"list" | "card">("list");
  const [pasteValue, setPasteValue] = useState("");
  const [failures, setFailures] = useState<ImportFailure[]>([]);
  const [query, setQuery] = useState("");
  const [notice, setNotice] = useState("原型已就绪：粘贴 JSON 或选择文件即可测试解析。");
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
      if (activeProvider !== "all" && account.provider !== activeProvider) return false;
      if (!normalizedQuery) return true;
      return [account.email, account.displayName, account.plan, account.accountId]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(normalizedQuery));
    });
  }, [accounts, activeProvider, query]);

  const counts = useMemo(
    () => ({
      all: accounts.length,
      codex: accounts.filter((account) => account.provider === "codex").length,
      gemini: accounts.filter((account) => account.provider === "gemini").length,
    }),
    [accounts],
  );

  const importContent = (content: string, label: string, source: "paste" | "file") => {
    const result = parseAuthJson(content, source, label);
    if (result.imported.length > 0) {
      setAccounts((current) => mergeAccounts(current, result.imported));
      setNotice(`导入成功：${result.imported.length} 个账号已加入本地列表。`);
    } else {
      setNotice("没有导入账号，检查 JSON 是否包含 Codex/Gemini 凭证字段。");
    }
    setFailures(result.failed);
  };

  const handlePasteImport = () => {
    importContent(pasteValue, "粘贴内容", "paste");
  };

  const handleFileImport = async (files: FileList | null) => {
    if (!files?.length) return;
    const allFailures: ImportFailure[] = [];
    const allImported: ManagedAccount[] = [];
    for (const file of Array.from(files)) {
      const content = await file.text();
      const result = parseAuthJson(content, "file", file.name);
      allImported.push(...result.imported);
      allFailures.push(...result.failed);
    }
    if (allImported.length > 0) {
      setAccounts((current) => mergeAccounts(current, allImported));
      setNotice(`文件导入完成：新增或更新 ${allImported.length} 个账号。`);
    } else {
      setNotice("文件读取完成，但没有识别到可导入账号。");
    }
    setFailures(allFailures);
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
    setNotice("请选择一种导入方式：粘贴 auth.json、导入 JSON 文件、读取本机账号或 OAuth 授权。");
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
          <button className={clsx(activeProvider === "all" && "active")} onClick={() => setActiveProvider("all")}>
            <Database size={17} />
            <span>全部账号</span>
            <b>{counts.all}</b>
          </button>
          <button className={clsx(activeProvider === "codex" && "active")} onClick={() => setActiveProvider("codex")}>
            <KeyRound size={17} />
            <span>Codex</span>
            <b>{counts.codex}</b>
          </button>
          <button className={clsx(activeProvider === "gemini" && "active")} onClick={() => setActiveProvider("gemini")}>
            <Fingerprint size={17} />
            <span>Gemini</span>
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
            <p className="eyebrow">Account control center</p>
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
                <p>{filteredAccounts.length} 个匹配项</p>
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
                  <div className={clsx("provider-dot", providerClass(account.provider))}>
                    {account.provider === "codex" ? <KeyRound size={18} /> : <Fingerprint size={18} />}
                  </div>
                  <div className="account-main">
                    <div className="account-title">
                      <strong>{account.displayName || account.email}</strong>
                      <span className={clsx("pill", providerClass(account.provider))}>{providerLabel(account.provider)}</span>
                      {account.tokenMeta.hasRefreshToken && (
                        <span className="pill muted">
                          <BadgeCheck size={13} />
                          refresh
                        </span>
                      )}
                    </div>
                    <p>{account.email}</p>
                  </div>
                  <div className="account-meta">
                    <span>{account.plan || "Unknown plan"}</span>
                    <small>{formatRelative(account.updatedAt)}</small>
                  </div>
                  <button className="icon-button" aria-label="Open account" onClick={() => handleAccountOpen(account)}>
                    <ChevronRight size={18} />
                  </button>
                </article>
              ))}
            </div>
          </div>

          <aside className="import-panel">
            <div className="panel-head">
              <div>
                <h2>导入</h2>
                <p>四种入口先并起来</p>
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
                  <button className="wide primary" onClick={handlePasteImport} disabled={!pasteValue.trim()}>
                    <Clipboard size={17} />
                    解析并导入
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
                  <button className="drop-zone" onClick={() => fileInputRef.current?.click()}>
                    <Upload size={22} />
                    <strong>选择 JSON 文件</strong>
                    <span>支持 auth.json、oauth_creds.json、导出数组</span>
                  </button>
                </>
              )}

              {mode === "local" && (
                <div className="coming-card">
                  <FolderDown size={24} />
                  <strong>下一步接入 Tauri 后端</strong>
                  <p>读取 ~/.codex/auth.json、~/.gemini/oauth_creds.json，并在 macOS/Windows 做路径兼容。</p>
                </div>
              )}

              {mode === "oauth" && (
                <div className="oauth-flow">
                  <div>
                    <span>1</span>
                    <p>启动本地 callback 监听</p>
                  </div>
                  <div>
                    <span>2</span>
                    <p>打开 Codex 或 Gemini 授权页</p>
                  </div>
                  <div>
                    <span>3</span>
                    <p>交换 token 并保存账号</p>
                  </div>
                  <button className="wide">
                    <LockKeyhole size={17} />
                    等待 Rust 后端接入
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
        </section>
      </section>
    </main>
  );
}

export default App;
