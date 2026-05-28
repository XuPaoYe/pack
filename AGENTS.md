# AGENTS.md

This file is for AI coding agents working on Super AI.

## Project Goal

Super AI is a macOS / Windows desktop app for managing local AI accounts, focused only on:

- Codex / GPT-related local auth
- Gemini CLI / Gemini Code Assist local auth

The app should feel like a polished native desktop utility, not a web dashboard. Keep the UI quiet, dense, precise, and trustworthy.

## Current Stack

- React + TypeScript + Vite frontend
- `lucide-react` icons
- Plain CSS with system fonts and native-feeling layout
- Tauri 2 + Rust desktop shell（业务命令在 `src-tauri/src/lib.rs`）

Current frontend entry points:

- `src/App.tsx`
- `src/App.css`
- `src/index.css`
- `src/lib/authParser.ts`（仅 UI 预览解析；权威 parser 在 Rust 后端）

## Important Context

This repo was started after reviewing two reference projects in the parent directory:

- `../cockpit-tools-main`
  - Tauri 2 + React + Rust desktop app
  - Strong reference for UI patterns, OAuth flow, local account management, Codex/Gemini modules
- `../codex-auth-0.3.0-alpha.5`
  - Zig CLI for Codex auth switching
  - Strong reference for Codex `auth.json` parsing, account identity, registry logic

Do not copy huge unrelated parts from either project. Extract ideas and narrow implementation to Codex + Gemini + SuperAI。

## Product Scope

支持的 provider：

- **codex**：本地 `~/.codex/auth.json`、JSON 粘贴、OAuth
- **gemini**：本地 `~/.gemini/oauth_creds.json` / `google_accounts.json` / `settings.json`、JSON 粘贴、OAuth
- **superai**：保留左侧 tab、右侧页面、导入弹窗和 API 服务卡片的 UI 壳；内部旧接入已移除

核心账号操作（已实现）：

- 列表 / 搜索 / 过滤
- 导入 / 更新 / 导出（公开版只导出 batch_key，完全版导出原始凭证）
- 切换：写回 codex/gemini 本地配置；SuperAI 当前仅保留 UI 壳
- 刷新 token / 配额（Codex / Gemini / Antigravity）

Avoid expanding into more providers. This app is intentionally not a full clone of Cockpit Tools.

## UI Direction

Target feel:

- Raycast / Linear / macOS Settings style
- Native-looking desktop shell
- Compact information density
- Calm colors, strong hierarchy, clean spacing
- Icons for actions where possible
- No marketing landing page
- No oversized hero section
- No decorative gradient/orb background

Use system fonts. Keep border radius modest, usually `7px` to `10px`.

When adding screens, prefer:

- Sidebar navigation
- Toolbar actions
- Tables/lists for accounts
- Modal or side panel for import flows
- Segmented controls for modes
- Icon buttons for obvious operations

## Auth Parsing Rules

权威 parser 在 Rust 后端（`@/src-tauri/src/lib.rs` 的 `parse_*_payload` / `apply_*_user_status_*` 系列）。`@/src/lib/authParser.ts` 只做 UI 预览：粘贴框实时显示"识别到 N 个账号"，不做最终入库决策。

前端不应成为 security boundary：本地文件读取、token 校验、refresh、加密落盘全部在 Rust。前端拿到的 `ManagedAccount` 已经过 `account_for_frontend` 把 `auth_payload` 抹掉。

## Security Expectations

- 不要把本地 auth 文件 / token 发送到任何第三方服务
- 不要把原始 token 写日志
- UI 默认 mask sensitive 字段；公开版进一步走 `sanitizeUserFacingText`
- 凭证持久化走 SQLite + AES（详见后面"与构建模式无关"清单的 "DB 加密"项）；不依赖 macOS Keychain / Windows Credential Manager
- 写凭证文件用原子写法（temp + rename）+ 限制权限

## Engineering Rules

- Keep changes focused.
- Do not introduce large UI frameworks unless there is a clear payoff.
- Do not add Redux; local state or a small store is enough for now.
- Prefer boring TypeScript types over clever abstractions.
- Keep provider-specific logic separated from shared UI.
- Do not hardcode user-specific absolute paths in source code.
- Do not commit generated `dist/` output unless explicitly requested.
- Keep README user-facing and AGENTS.md agent-facing.

## Release / GitHub Actions Rules

- GitHub release workflow must keep all four packaging targets unless the user explicitly says otherwise: macOS arm64, macOS x64, Windows x64, Windows arm64.
- Manual `workflow_dispatch` release runs must always provide a concrete `release_version` such as `v1.1.2` and a short `release_note`; do not run a nameless workflow that only shows `release` in the Actions list.
- Action run names should be readable at a glance, e.g. `Release v1.1.2: public build latest code`, so later debugging can identify which version and purpose produced the artifacts.
- If packaging is triggered from a branch instead of a tag, mention the source branch/ref in the final status and confirm the commit SHA that was built.
- When fixing release workflow failures, keep changes scoped to CI/build scripts and do not remove target platforms as a workaround unless the user approves it.

## SuperAI API 卡片现状

`SuperAI` 页签和右侧 API 服务卡片当前只保留 UI 壳与设置结构，不再内置真实本地 API 代理、sidecar、language server 或账号同步链路。

- `src/App.tsx` / `src/App.css` 保留展示层与交互壳子。
- `src-tauri/src/lib.rs` 中 API 服务相关命令当前返回占位状态，用于支撑现有 UI，不提供真实转发能力。
- 如果未来要重新接回服务，请按当前产品目标重新设计实现，不要默认仓库里还保留旧代理架构。

## 命名规则

- 所有用户可见文案统一使用 `SuperAI` / `superai`。
- 历史 provider 字面量只允许出现在兼容解析代码中，且必须用非明文构造方式，避免打进前端 bundle。
- 新增设置字段、IPC 命令、事件名、CSS class、localStorage key 时，不要再引入历史命名。

## 构建模式：Public（默认） vs Full

### 中文术语映射（强制使用）

用户与代码沟通统一使用：

| 中文 | 英文 / 代码标志 | 含义 |
|---|---|---|
| **公开版** | public build / `VITE_SUPERAI_PUBLIC_BUILD=1` | 默认构建，对外分发给最终用户。脱敏 + 批量密钥导入 + 本地累计用量 |
| **完全版** | full build / 不注入环境变量 | 内部 / 开发自用，无脱敏、显示真 email、可导出原始凭证、有完整模型族 |

> 用户日常说"公开版 / 完全版"时，对应英文文档里的 public / full。新增代码、commit message、内部讨论统一沿用这两个中文词，不要写成"用户版 / 内部版 / pro 版 / lite 版"等其他叫法。

通过环境变量 `VITE_SUPERAI_PUBLIC_BUILD=1` 切换。两个 npm 脚本入口：

| 脚本 | 模式 | 注入的 env |
|---|---|---|
| `npm run dev` / `npm run build` / `npm run build:mac` / `npm run build:win:x64` / `npm run build:win:arm64` | **public**（默认） | `VITE_SUPERAI_PUBLIC_BUILD=1` |
| `npm run dev:full` / `npm run build:full` / `npm run build:mac:full` / `npm run build:win:x64:full` / `npm run build:win:arm64:full` | **full** | 不注入 |

读取入口：
- 前端：`@/src/App.tsx` 顶部 `IS_PUBLIC_BUILD = import.meta.env.VITE_SUPERAI_PUBLIC_BUILD === "1"`
- Rust：`@/src-tauri/src/lib.rs` `fn is_public_build()` 用 `option_env!("VITE_SUPERAI_PUBLIC_BUILD")`
- 类型声明：`@/src/vite-env.d.ts` 已声明 `VITE_SUPERAI_PUBLIC_BUILD?: string`

### 行为差异完整盘点

新增 build-aware 逻辑必须更新本表。

| # | 功能点 | 文件 | public（默认） | full |
|---|---|---|---|---|
| 1 | 邮箱/token 文本脱敏 | `App.tsx` `sanitizeUserFacingText` | ✅ regex 替成 `[account]`/`[secret]` | ❌ 原样 |
| 2 | SuperAI 账号卡片身份 | `App.tsx` `shouldHideAccountDetails` / `accountDisplayLabel` | ✅ 显示 `SUPERAI-XXXXXXX` | ❌ 真 email |
| 3 | API 服务模型族选项 | `App.tsx` `modelFamiliesForBuild` | ✅ 仅 `PUBLIC_MODEL_FAMILY_KEYS` | ❌ 全部 `MODEL_FAMILIES` |
| 4 | 单号导出格式 | `App.tsx` `handleExportAccount` + Rust `export_account` / `export_public_superai_account` | ✅ 走 `export_public_superai_account` 返 batch_key | ❌ 走 `export_account` 返原始 JSON |
| 5 | 批量导出格式 | `App.tsx` `handleBatchExport` 同上分支 | ✅ 同 #4 | ❌ 同 #4 |
| 6 | Rust 兜底拒绝原始凭证导出 | `lib.rs` `export_account` 中 `is_public_build()` | ✅ `Err("公开版不允许导出 SuperAI 原始凭证")` | ❌ 返回原始 JSON |
| 7 | 账号本地累计用量 | `lib.rs` `bump_public_usage` 等（详见下节） | ✅ 触发条件：账号 `auth_payload` 含 `batch_key`，只有公开版的批量密钥导入路径会写这个字段 | ❌ 不写 batch_key，helper 全部 no-op |
| 8 | SuperAI 卡片配额面板可见 metric | `App.tsx` 配额渲染处 | ✅ 仅 `superai-daily` / `superai-public` / 标签为"日限"的 metric；缺失时强制兜底 0% 进度条（公开版下 `superai-public` 的标签也固定为"日限"，UI 不暴露"额度"二字） | ❌ 全部 metric |
| 9 | SuperAI 添加账号弹窗 tab | `App.tsx` `superaiImportModeOrder` + `mode === "password"` JSX | ✅ 仅"批量密钥" | ❌ "批量密钥" + "账号密码"（邮箱密码登录单个账号，调 `add_superai_account_by_password`） |

### 与构建模式无关（不要错误地包条件）

下面这些是**所有构建**都启用，**不要**给它们加 `IS_PUBLIC_BUILD` / `is_public_build()` 判：

- **DB 加密兼容**：历史 SuperAI 账号的本地加密读取兼容不依赖构建模式。
- **codex / gemini 导入面板的可选模式**：永远是 oauth/paste/local/file 四件套，**不**由 build 决定，请勿误加 `IS_PUBLIC_BUILD` 判。
- **SuperAI 导入面板的 tab 列表**：保留当前 UI 壳的 build-aware 表现，若未来恢复真实接入需重新评估。

## SuperAI 现状

当前仓库对 `SuperAI` 的目标是：

- 保留左侧 tab、右侧页面、弹窗、卡片与设置 UI 壳。
- 保留最小占位 IPC / 设置结构，支撑现有界面。
- 不再保留真实 sidecar、language server、账号同步、批量密钥累计用量等旧运行链路。

如果未来重新接回真实服务，必须先重新设计并更新本文件，不要沿用这里已经移除的旧实现假设。

## Verification

每次有意义改动合并前跑：

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
npm run check  # = node scripts/app-build.mjs && npm run lint
```

本地浏览器预览：

```bash
npm run dev -- --host 127.0.0.1
```
