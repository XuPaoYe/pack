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
- Tauri 2 + Rust desktop shell（业务命令在 `src-tauri/src/lib.rs`，sidecar 反代在 `src-tauri/src/windsurf_api.rs`）
- Bun-compiled sidecar from vendored `WindsurfPoolAPI`，详见后面 "Windsurf 本地 API 服务" 一节

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

Do not copy huge unrelated parts from either project. Extract ideas and narrow implementation to Codex + Gemini + Windsurf。

## Product Scope

支持的 provider：

- **codex**：本地 `~/.codex/auth.json`、JSON 粘贴、OAuth
- **gemini**：本地 `~/.gemini/oauth_creds.json` / `google_accounts.json` / `settings.json`、JSON 粘贴、OAuth
- **windsurf**（公开版核心）：批量密钥导入；完全版还支持邮箱密码 / token / OAuth 等导入路径

核心账号操作（已实现）：

- 列表 / 搜索 / 过滤
- 导入 / 更新 / 导出（公开版只导出 batch_key，完全版导出原始凭证）
- 切换：写回 codex/gemini 本地配置；windsurf 通过 sidecar 池调度
- 刷新 token / 配额，含 windsurf 公开版本地累计用量（详见后节）

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

- 不要把本地 auth 文件 / token 发送到任何第三方服务（vendor sidecar 算"我们自己的本机进程"，不算第三方）
- 不要把原始 token 写日志；sidecar 转发日志走 `sanitize_sidecar_log_line` 兜底
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

## Windsurf 本地 API 服务

Super AI 暴露 OpenAI / Anthropic 兼容入口，外部 IDE 可通过 `Authorization: Bearer agt_wsf_*` 调用本机推理。架构：

```
[外部 IDE]
    | Bearer agt_wsf_*
    v
[tiny_http 反向代理 @ src-tauri/src/windsurf_api.rs]
    | Bearer <inner_key>
    v
[superai-api sidecar (bun --compile vendor/windsurfapi)]
    | spawns
    v
[language_server 二进制 (Windsurf 闭源 LS)]
```

- `vendor/windsurfapi/` 镜像了上游 [WindsurfPoolAPI](https://github.com/guanxiaol/WindsurfPoolAPI)（MIT），版本写在 `VERSION.txt`。不要直接改 vendor 里的 JS，要升级请 bump version 后重新拷贝。
- `scripts/scrub-vendor.mjs` 把 vendor 复制到 `.vendor-build/windsurfapi/` 并替换掉用户可见的 `Windsurf` / `WindsurfAPI` / `windsurfapi` / `_windsurf_id` / `org-windsurf-proxy` 字面量，同时给 `models.js` 的 `owned_by`、`dashboard/logger.js` 的落盘 JSONL 打补丁。协议字面量（`'windsurf'` 在 protobuf metadata、URL 里的 `windsurf.com`、`MODEL_PROVIDER_WINDSURF`、`WINDSURF_*`/`WINDSURFAPI_*` 环境变量、`User-Agent`）保留不动。
- `scripts/build-sidecar.sh` 先跑 scrub-vendor，再用 `bun build --compile` 编 `.vendor-build/windsurfapi/src/index.js` 为当前平台的 sidecar，并从 `/Applications/Windsurf.app/...`（或 `WINDSURF_LS_PATH`）抽 LS 二进制，统一放到 `src-tauri/binaries/<name>-<rust-target-triple>(.exe)`。
- `tauri.conf.json` 通过 `bundle.externalBin` 注册 `binaries/superai-api` 与 `binaries/language_server`；dev 与打包时 Tauri-CLI 自动复制到 app 可执行同目录。
- 启动流程：`start_api_service` 命令 → 预挑两个空闲端口 → spawn sidecar 子进程 → 解析 stdout `Server on http://0.0.0.0:N` 拿 inner port → 起 tiny_http 反向代理。
- 双层鉴权：外层 `agt_wsf_*` 由我们校验，内层 sidecar 用我们生成的随机 inner key（不持久化）。
- 账号同步：`sync_superai_accounts_to_api` 命令把 DB 里的 Windsurf 账号映射成 `{refresh_token | api_key | token, label}` POST 到 sidecar `/auth/login`。`upsert_accounts_into_db` 写库后会在 API 服务运行时自动触发同步。

## 品牌脱敏规则（永久执行）

> **必须遵守**：所有用户可见 / 可观察的位置，凡是出现 `Windsurf` 都改成 `SuperAI`。任何新增代码、UI 文本、CSS 类名、IPC 命令名、emit 事件名、日志字符串都直接用 SuperAI / superai。Vendor 升级后跑 `npm run build:sidecar` 会自动经 `scripts/scrub-vendor.mjs` 完成同样替换。

**强制替换映射**（任何 PR / commit 不得偏离）：

| 原始 | 替换 |
|---|---|
| `Windsurf` / `WindsurfAPI` / `Windsurf API`（用户可见英文）| `SuperAI` |
| `windsurfapi`（项目 / 路径代号） | `superai-sidecar` |
| `_windsurf_id`（/v1/models 输出字段） | `_internal_id` |
| `owned_by: 'windsurf'`（/v1/models 输出） | `owned_by: 'superai'` |
| `org-windsurf-proxy`（响应头） | `org-superai-proxy` |
| Tauri command `*_windsurf_*` / `*_windsurf_api*` | `*_api_service*` 或 `*_superai_*` |
| `AppSettings` 字段 `windsurf_api_*` | `api_service_*`（必须挂 `serde(alias = "windsurfApi*")` 兼容老 settings） |
| CSS class `.windsurf-*` | `.superai-*` |
| localStorage key `super-ai:windsurf-*` | `super-ai:api-service-*` 或 `super-ai:superai-*` |
| emit event `windsurf-*` | `superai-*` 或 `api-service-*` |

**TS / JS 字面量要绕过 esbuild / vite 常量折叠**。直接 `String.fromCharCode(87,...)` 会被构建器在编译期折叠成 `"Windsurf"` 又塞回 bundle。统一用：

```ts
const PROVIDER_WSF = [119, 105, 110, 100, 115, 117, 114, 102]
  .map((c) => String.fromCharCode(c))
  .join("") as "windsurf";
```

`.map(...).join("")` 形式 esbuild 不会折叠，dist 里就拿不到字面量。`src/App.tsx` 和 `src/lib/authParser.ts` 顶部已有此模式，新增组件请复用同样的 `PROVIDER_WSF` 常量，不要手写新的 `"windsurf"` 字符串字面量。

**Rust 同样规则**：`src-tauri/src/windsurf_api.rs::sanitize_sidecar_log_line` 给 sidecar 转发到主进程 stderr 的日志做兜底替换；新增 sidecar 输出处理路径必须经过它。

**协议字面量保留**（动了会立即坏功能，不要替换）：
- protobuf metadata 里的 `'windsurf'`（`writeStringField(1,'windsurf')` / `(12,'windsurf')`）—— 上游服务端校验 ide_name / extension_name
- URL 里的 `windsurf.com`、`server.self-serve.windsurf.com` 等 —— 上游 API 域名
- `User-Agent: windsurf/...` —— 上游校验
- `MODEL_PROVIDER_WINDSURF` 等 ALL_CAPS 常量、`WINDSURF_*` / `WINDSURFAPI_*` 环境变量名 —— 内部路由 / 配置开关
- 内部 model provider tag `provider: 'windsurf'` —— 走 `models.js` 的 `owned_by` 收口替换，不要改值本身
- DB 里 `provider = 'windsurf'` 行 —— 用户开 sqlite cli 才看得到，改它要 schema 迁移，风险/收益不划算

**回归命令统一在最末 "Verification" 节**。任何一项 `> 0`，新增的字面量必须按上面规则消化掉再合并。

不要把 sidecar / LS 二进制提交到 Git，`.gitignore` 已经覆盖。

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
| 2 | windsurf 账号卡片身份 | `App.tsx` `shouldHideAccountDetails` / `accountDisplayLabel` | ✅ 显示 `SUPERAI-XXXXXXX` | ❌ 真 email |
| 3 | API 服务模型族选项 | `App.tsx` `modelFamiliesForBuild` | ✅ 仅 `PUBLIC_MODEL_FAMILY_KEYS` | ❌ 全部 `MODEL_FAMILIES` |
| 4 | 单号导出格式 | `App.tsx` `handleExportAccount` + Rust `export_account` / `export_public_superai_account` | ✅ 走 `export_public_superai_account` 返 batch_key | ❌ 走 `export_account` 返原始 JSON |
| 5 | 批量导出格式 | `App.tsx` `handleBatchExport` 同上分支 | ✅ 同 #4 | ❌ 同 #4 |
| 6 | Rust 兜底拒绝原始凭证导出 | `lib.rs` `export_account` 中 `is_public_build()` | ✅ `Err("公开版不允许导出 SuperAI 原始凭证")` | ❌ 走 `build_windsurf_payload` |
| 7 | 账号本地累计用量 | `lib.rs` `bump_public_usage` 等（详见下节） | ✅ 触发条件：账号 `auth_payload` 含 `batch_key`，只有公开版的批量密钥导入路径会写这个字段 | ❌ 不写 batch_key，helper 全部 no-op |
| 8 | windsurf 卡片配额面板可见 metric | `App.tsx` 配额渲染处 | ✅ 仅 `superai-daily` / `superai-public` / 标签为"日限"的 metric；缺失时强制兜底 0% 进度条（公开版下 `superai-public` 的标签也固定为"日限"，UI 不暴露"额度"二字） | ❌ 全部 metric |
| 9 | windsurf 添加账号弹窗 tab | `App.tsx` `superaiImportModeOrder` + `mode === "password"` JSX | ✅ 仅"批量密钥" | ❌ "批量密钥" + "账号密码"（邮箱密码登录单个账号，调 `add_superai_account_by_password`） |

### 与构建模式无关（不要错误地包条件）

下面这些是**所有构建**都启用，**不要**给它们加 `IS_PUBLIC_BUILD` / `is_public_build()` 判：

- **DB 加密**（`@/src-tauri/src/lib.rs` `serialize_account_for_storage` / `superai_encrypt_text`）：所有 windsurf 行 AES 加密 + email 列填合成 ID + display_name 置 NULL。判定按 `provider == "windsurf"`，不按 build。
- **vendor scrub**（`@/scripts/scrub-vendor.mjs`）：脱敏 `Windsurf`/`windsurfapi` 字面量、改写 `models.js` `owned_by`、给 `dashboard/logger.js` JSONL 落盘加 sanitize、把 `auth.js` 的 `saveAccounts` / `saveAccountsSync` no-op 化。所有构建都跑。
- **sidecar accounts.json 不落盘**：scrub-vendor 的 no-op patch 让 `saveAccounts` 一直空转，应用关闭 = 内存账号池蒸发。所有构建都生效。
- **3 秒 IPC `sync_api_service_active_account` 缓存短路**（`@/src-tauri/src/windsurf_api.rs` `LAST_SYNCED_ACTIVE_EMAIL`）：所有构建都启用。
- **DB / 3DES / refresh / sidecar 反代行为**：所有构建一致。
- **codex / gemini 导入面板的可选模式**：永远是 oauth/paste/local/file 四件套，**不**由 build 决定，请勿误加 `IS_PUBLIC_BUILD` 判。（windsurf 的 tab 列表是 build-aware 的，详见上面差异表第 9 行）

## 公开版账号"本地累计用量"机制

**只对 windsurf provider 且 auth_payload 含 `batch_key` 的账号生效**。full 版用其他导入路径（密码/token/OAuth）入库的号没有 `batch_key`，所有 helper 自动 no-op，对 full 流程零影响。

### 数据字段（`auth_payload` 内，随 windsurf 行 AES 加密落盘）

| 字段 | 含义 |
|---|---|
| `batch_key` | 公开版批量密钥原文，导出时复用 |
| `license_expires_at` | 批量密钥到期 unix 时间，到期由 `delete_expired_windsurf_accounts` 删账号 |
| `usage_baseline_remaining` | 入库时上游 daily% 快照（公开版 UI 的"日限"基线） |
| `usage_last_remote_remaining` | 上次刷新拿到的上游 daily%；下次比对差值 |
| `usage_consumed_local` | 本地累计已用 0..100，**单调递增不可回退** |
| `usage_exhausted_at` | 用满 unix 时间戳；存在即视为已耗尽 |
| `usage_basis` | 当前累计依据；应为 `daily`，旧账号缺失时会按当前 daily 重新初始化 |

### 累计算法（`bump_public_usage`）

```
首次（last_remote 缺失）:
  baseline = last_remote = 当前 daily%
  consumed 保持 0
后续:
  diff = last_remote - this_daily
  diff > 0  → consumed = clamp(consumed + diff, 0, 100)
  diff <= 0 → 上游重置或抖动，不动 consumed
  无论正负都更新 last_remote = this_daily
触达 100:
  写 usage_exhausted_at
  status = unavailable / "已耗尽" / reason="本地累计额度已用满"
```

### 集成点（不要漏写）

新增 windsurf refresh 路径必须在收尾调 `apply_public_usage_after_refresh`：

| 调用位置 | 何时 |
|---|---|
| `attach_windsurf_batch_key` 末尾 | 批量密钥入库即刻初始化 baseline |
| `refresh_account` windsurf 分支后 | 单号刷新（含 15s active-account 静默刷新） |
| `refresh_provider_accounts` 循环内 | 整个 provider 手动刷新 |
| `refresh_all_accounts` 当前**未**接 windsurf 路径，新增 windsurf 分支时务必同步加 hook |

耗尽事件链：

1. `bump_public_usage` 返回 `(just_exhausted=true, _)`
2. caller emit `account-exhausted` 给前端（`@/src-tauri/src/lib.rs` `emit_account_exhausted`）
3. caller `schedule_windsurf_sync(app)` 触发 reconcile
4. `windsurf_account_to_sidecar_payload` 看到 `public_usage_is_exhausted` → 返 None
5. `reconcile_accounts` desired_emails 不含此号 → DELETE 到 sidecar `/auth/accounts/:id`
6. 前端 `account-exhausted` listener toast + 重读 `list_accounts`

### UI 显示约束

`rewrite_quota_for_public_usage` 在公开版账号上把 `quota.metrics` 整段改写成单条 `superai-public` metric，`remainingPercent = usage_baseline_remaining - usage_consumed_local`。这意味着：

- UI 进度条对公开版账号显示**本地日限剩余**，不是上游 weekly%
- 上游日重置不会让 UI 进度条假性回血
- full 版 / 非公开版 windsurf 号（无 batch_key）不被改写，照旧显示上游 daily/weekly

### 不要做的事

- 不要让 `bump_public_usage` 在 diff < 0 时回退 consumed（会造成"上游重置 → 我们送配额"）
- 不要在 full 版导入路径写 `batch_key` 字段（会让 full 用户也被本地配额限制）
- 不要在 `apply_windsurf_plan_status` 之前调 bump（顺序：先 apply_*_remote → 再 apply_public_usage_after_refresh，否则 status 会被覆盖回"可用"）
- 不要直接在前端读写 usage 字段（`auth_payload` 在 `account_for_frontend` 里被 redact，前端拿不到原始 payload）
- 新增可能改 `account.status` 或 `quota` 的代码路径，必须考虑"已耗尽"账号不能被重置回"可用"

## Verification

首次或 vendor 升级后必跑（否则 Tauri 找不到 sidecar 会报错；需要 `bun >= 1.3` + 已安装 Windsurf 应用或 `WINDSURF_LS_PATH`）：

```bash
npm run build:sidecar
```

每次有意义改动合并前跑：

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --lib windsurf_api -- --test-threads=1
npm run check  # = node scripts/app-build.mjs && npm run lint

# 品牌脱敏回归（规则在 "品牌脱敏规则" 节）
grep -c Windsurf dist/assets/*.js                            # 必须为 0
grep -c windsurf dist/assets/*.js                            # 必须为 0
grep -ic windsurf dist/assets/*.css                          # 必须为 0
strings src-tauri/binaries/superai-api-* | grep -c Windsurf  # 必须为 0
```

E2E 反向代理验证（慢，需要 sidecar 已构建；公开版分发前必跑）：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib e2e_proxy_models -- --ignored --test-threads=1 --nocapture
```

本地浏览器预览：

```bash
npm run dev -- --host 127.0.0.1
```
