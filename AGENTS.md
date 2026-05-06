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

## Verification

Run before handing off meaningful changes:

```bash
npm run build
npm run lint
```

For local preview:

```bash
npm run dev -- --host 127.0.0.1
```

## Known Environment Note

At project creation time, Node/npm were available, but `rustc` and `cargo` were not installed. Rust was later installed through Homebrew and Tauri 2 was initialized. Continue backend work inside `src-tauri/`.
