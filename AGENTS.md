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
- Tauri 2 + Rust desktop shell is initialized
- Rust business commands are not implemented yet

Current frontend entry points:

- `src/App.tsx`
- `src/App.css`
- `src/index.css`
- `src/lib/authParser.ts`

## Important Context

This repo was started after reviewing two reference projects in the parent directory:

- `../cockpit-tools-main`
  - Tauri 2 + React + Rust desktop app
  - Strong reference for UI patterns, OAuth flow, local account management, Codex/Gemini modules
- `../codex-auth-0.3.0-alpha.5`
  - Zig CLI for Codex auth switching
  - Strong reference for Codex `auth.json` parsing, account identity, registry logic

Do not copy huge unrelated parts from either project. Extract ideas and narrow implementation to Codex + Gemini.

## Product Scope

Required import methods:

- Read local account
  - Codex: `~/.codex/auth.json`
  - Gemini: `~/.gemini/oauth_creds.json`, `google_accounts.json`, `settings.json`
- Import JSON from local file
- Paste `auth.json` / token JSON
- OAuth authorization

Core account operations planned:

- List accounts
- Search/filter accounts
- Import/update accounts
- Export accounts
- Switch/inject selected account into local Codex or Gemini config
- Refresh token/quota later, if kept small and explicit

Avoid expanding into many providers. This app is intentionally not a full clone of Cockpit Tools.

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

`src/lib/authParser.ts` owns browser-side prototype parsing only.

Current parser recognizes:

- Codex standard `auth.json`
  - `tokens.id_token`
  - `tokens.access_token`
  - `tokens.refresh_token`
  - `tokens.account_id`
  - optional `auth_mode`
  - optional `OPENAI_API_KEY`
- Codex exported account-like objects
- Gemini token objects
  - `access_token`
  - `refresh_token`
  - `id_token`
  - `expiry_date`
- Arrays and `{ accounts: [...] }`

Sensitive parsing and local file reads should move to Rust backend. Frontend may keep lightweight preview parsing, but should not become the security boundary.

## Planned Tauri Backend

Tauri 2 is initialized. Add commands around these modules:

- `src-tauri/src/commands/codex.rs`
- `src-tauri/src/commands/gemini.rs`
- `src-tauri/src/services/storage.rs`
- `src-tauri/src/services/oauth.rs`
- `src-tauri/src/services/codex_auth.rs`
- `src-tauri/src/services/gemini_auth.rs`

Planned commands:

- `list_accounts`
- `import_codex_from_local`
- `import_gemini_from_local`
- `import_accounts_from_json`
- `start_codex_oauth`
- `complete_codex_oauth`
- `start_gemini_oauth`
- `complete_gemini_oauth`
- `switch_codex_account`
- `switch_gemini_account`

Security expectations:

- Do not send local auth files to any third-party service.
- Do not log raw tokens.
- Mask tokens in UI by default.
- Use atomic writes for credential files.
- Use restrictive file permissions where supported.
- On macOS, account for Keychain if Gemini/Codex requires it.
- On Windows, account for Credential Manager where needed.

## Engineering Rules

- Keep changes focused.
- Do not introduce large UI frameworks unless there is a clear payoff.
- Do not add Redux; local state or a small store is enough for now.
- Prefer boring TypeScript types over clever abstractions.
- Keep provider-specific logic separated from shared UI.
- Do not hardcode user-specific absolute paths in source code.
- Do not commit generated `dist/` output unless explicitly requested.
- Keep README user-facing and AGENTS.md agent-facing.

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

**回归检查命令**（每次有可能影响外观的改动后跑）：

```bash
npm run check                       # vite build
grep -c Windsurf dist/assets/*.js   # 必须为 0
grep -c windsurf dist/assets/*.js   # 必须为 0
grep -ic windsurf dist/assets/*.css # 必须为 0
strings src-tauri/binaries/superai-api-* | grep -c Windsurf  # 必须为 0
```

任何一项 `> 0`，新增的字面量必须按上面规则消化掉再合并。

开发前必跑（否则 Tauri 找不到 sidecar 会报错）：

```bash
npm run build:sidecar
```

需要 `bun >= 1.3` + 已安装 Windsurf 应用（或设置 `WINDSURF_LS_PATH`）。

不要把 sidecar / LS 二进制提交到 Git，`.gitignore` 已经覆盖。

## Verification

Run before handing off meaningful changes:

```bash
npm run build:sidecar  # 第一次或 vendor 升级后
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --lib windsurf_api -- --test-threads=1
npm run check  # = node scripts/app-build.mjs && npm run lint

# 品牌脱敏回归（详见上一节"品牌脱敏规则"）
grep -c Windsurf dist/assets/*.js                        # 必须为 0
grep -c windsurf dist/assets/*.js                        # 必须为 0
grep -ic windsurf dist/assets/*.css                      # 必须为 0
strings src-tauri/binaries/superai-api-* | grep -c Windsurf  # 必须为 0
```

E2E 反向代理验证（需要 sidecar 已构建）：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib e2e_proxy_models -- --ignored --test-threads=1 --nocapture
```

For local preview:

```bash
npm run dev -- --host 127.0.0.1
```

## Known Environment Note

At project creation time, Node/npm were available, but `rustc` and `cargo` were not installed. Rust was later installed through Homebrew and Tauri 2 was initialized. Continue backend work inside `src-tauri/`.
