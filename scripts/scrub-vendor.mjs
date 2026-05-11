#!/usr/bin/env node
// 把 vendor/windsurfapi 复制到 .vendor-build/windsurfapi，并替换里面所有
// 用户可见的 "Windsurf" 字面量。生成的副本喂给 bun --compile，原始 vendor
// 不动，方便上游升级时直接覆盖再重跑构建。
//
// 替换策略：
// 1) 大写开头的 `Windsurf` / `WindsurfAPI` / `Windsurf API` —— 用户可见的英
//    文展示文本，整体改成 SuperAI。
// 2) `windsurfapi`（小写）—— 项目代号，改成 superai-sidecar。
// 3) `_windsurf_id` —— /v1/models 响应里我们要隐藏的字段名，改 _internal_id。
// 4) `owned_by: info.provider` —— 同样在 /v1/models 输出，把 'windsurf' 收
//    口成 'superai'，其余 provider 透传。
// 5) `dashboard/logger.js` 落盘 JSONL 前再过一次 sanitize，防止运行时上游
//    返回的 "Windsurf" 字符串透过结构化日志被写到磁盘。
//
// 不动：
// - 协议要求的字面量：`'windsurf'`（writeStringField metadata，model
//   provider tag）、URL 里的 `windsurf.com`、`MODEL_PROVIDER_WINDSURF`、
//   `WINDSURF_*` / `WINDSURFAPI_*` 环境变量名。这些都是小写或 ALL_CAPS，
//   不会被 /Windsurf/g 正则匹配到。
// - vendor 原始目录。所有改动只落在 .vendor-build/。

import {
  cpSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..");
const SRC_DIR = join(repoRoot, "vendor", "windsurfapi");
const OUT_DIR = join(repoRoot, ".vendor-build", "windsurfapi");

if (!statSync(SRC_DIR, { throwIfNoEntry: false })?.isDirectory()) {
  console.error(`[scrub-vendor] missing vendor source directory`);
  process.exit(1);
}

rmSync(OUT_DIR, { recursive: true, force: true });
mkdirSync(dirname(OUT_DIR), { recursive: true });
cpSync(SRC_DIR, OUT_DIR, { recursive: true });

const TEXT_EXTENSIONS = new Set([
  ".js",
  ".mjs",
  ".cjs",
  ".ts",
  ".json",
  ".html",
  ".md",
  ".txt",
]);

const REGEX_REPLACEMENTS = [
  // 用户可见英文展示文本
  [/WindsurfAPI/g, "SuperAI"],
  [/Windsurf API/g, "SuperAI"],
  [/Windsurf/g, "SuperAI"],
  // 项目代号
  [/windsurfapi/g, "superai-sidecar"],
  // /v1/models 响应里要隐藏的内部字段名
  [/_windsurf_id/g, "_internal_id"],
  // server.js 给每个 OpenAI 兼容响应加的 openai-organization 响应头，调用
  // 方在 IDE 网络面板里能直接看到。
  [/org-windsurf-proxy/g, "org-superai-proxy"],
];

function walk(dir, fn) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const s = statSync(p);
    if (s.isDirectory()) walk(p, fn);
    else fn(p);
  }
}

let changed = 0;
walk(OUT_DIR, (p) => {
  const dotIdx = p.lastIndexOf(".");
  if (dotIdx < 0) return;
  const ext = p.slice(dotIdx);
  if (!TEXT_EXTENSIONS.has(ext)) return;
  const before = readFileSync(p, "utf8");
  let next = before;
  for (const [re, sub] of REGEX_REPLACEMENTS) next = next.replace(re, sub);
  if (next !== before) {
    writeFileSync(p, next);
    changed++;
  }
});

// ---- 文件级精确补丁 ----------------------------------------------------------

// /v1/models -> owned_by 收口：'windsurf' 改 'superai'，其余 provider 透传。
const modelsPath = join(OUT_DIR, "src", "models.js");
{
  const src = readFileSync(modelsPath, "utf8");
  const needle = "      owned_by: info.provider,";
  if (!src.includes(needle)) {
    console.error("[scrub-vendor] models.js owned_by anchor not found; aborting");
    process.exit(2);
  }
  const replaced = src.replace(
    needle,
    "      owned_by: info.provider === 'windsurf' ? 'superai' : info.provider,",
  );
  writeFileSync(modelsPath, replaced);
}

// dashboard/logger.js：落盘 JSONL 前再 sanitize 一次。运行时上游 5xx body
// 之类的字符串会经 log.error 进入 entry.msg / entry.ctx，正则替换搞不定。
const loggerPath = join(OUT_DIR, "src", "dashboard", "logger.js");
{
  const src = readFileSync(loggerPath, "utf8");
  const anchor = "    // Persist to disk\n    try {\n      const { app, err } = getStreams();\n      const line = JSON.stringify(entry) + '\\n';";
  if (!src.includes(anchor)) {
    console.error("[scrub-vendor] logger.js persist anchor not found; aborting");
    process.exit(2);
  }
  // 通过 fromCharCode 拼出敏感词，避免最终二进制 strings(1) 还看得到。
  const replacement = `    // Persist to disk
    try {
      const { app, err } = getStreams();
      const __sw = String.fromCharCode(87,105,110,100,115,117,114,102);
      const __swLower = String.fromCharCode(119,105,110,100,115,117,114,102) + 'api';
      const __reA = new RegExp(__sw + 'API', 'g');
      const __reB = new RegExp(__sw + ' API', 'g');
      const __reC = new RegExp(__sw, 'g');
      const __reD = new RegExp(__swLower, 'g');
      const sanitized = JSON.parse(JSON.stringify(entry, (_, v) =>
        typeof v === 'string'
          ? v.replace(__reA, 'SuperAI').replace(__reB, 'SuperAI')
              .replace(__reC, 'SuperAI').replace(__reD, 'superai-sidecar')
          : v));
      const line = JSON.stringify(sanitized) + '\\n';`;
  const next = src.replace(anchor, replacement);
  writeFileSync(loggerPath, next);
}

// auth.js：禁掉 accounts.json 落盘。我们的 sidecar 由 Tauri 子进程托管，
// 启动时 windsurf_api::start 会主动删旧 accounts.json 并通过 reconcile_accounts
// 把 SuperAI DB 里的账号 POST 到 sidecar /auth/login 重建池子。落盘的 JSON
// 含明文 email/apiKey/refreshToken，是被人拿走 app data 目录后最大的泄漏面。
// 改成 no-op 后所有运行期状态只活在内存里，应用关闭即销毁。
//
// 不影响：
//   - 请求热路径（getApiKey 是纯内存读，从不调 saveAccounts）
//   - 启动重建（reconcile_accounts → /auth/login，与磁盘无关）
//   - 配额刷新（GetUserStatus 走网络，更新内存即可）
//
// 唯一代价：sidecar 进程内 banned 状态 / blockedModels / tierManual 在进程
// 重启后丢失，会被下次 reconcile 后的 probe-all + refresh-credits 重建。
// SuperAI 不暴露 sidecar dashboard 给终端用户，可接受。
const authPath = join(OUT_DIR, "src", "auth.js");
{
  const src = readFileSync(authPath, "utf8");

  const saveAnchor = "function saveAccounts() {\n  if (_saveInFlight) { _savePending = true; return; }";
  if (!src.includes(saveAnchor)) {
    console.error("[scrub-vendor] auth.js saveAccounts anchor not found; aborting");
    process.exit(2);
  }
  const saveSyncAnchor = "export function saveAccountsSync() {\n  const tempFile = ACCOUNTS_FILE + '.shutdown.tmp';";
  if (!src.includes(saveSyncAnchor)) {
    console.error("[scrub-vendor] auth.js saveAccountsSync anchor not found; aborting");
    process.exit(2);
  }

  // 用正则吃掉两个完整函数（含函数体），整段替换成 no-op。
  // 函数体里没有嵌套独立顶层 `^}`，所以 /^}/m 就能稳定匹配右括号。
  const saveRe = /function saveAccounts\(\) \{[\s\S]*?\n\}\n/;
  const saveSyncRe = /export function saveAccountsSync\(\) \{[\s\S]*?\n\}\n/;
  if (!saveRe.test(src) || !saveSyncRe.test(src)) {
    console.error("[scrub-vendor] auth.js saveAccounts/saveAccountsSync body regex failed; aborting");
    process.exit(2);
  }

  const noopSave = `function saveAccounts() {
  // SuperAI patch: 禁止 accounts.json 落盘，参见 scripts/scrub-vendor.mjs。
  // 保留 _saveInFlight / _savePending 引用避免未使用变量在严格模式下警告。
  void _saveInFlight; void _savePending;
}
`;
  const noopSaveSync = `export function saveAccountsSync() {
  // SuperAI patch: 关闭流程也不落盘，内存数据随进程销毁。
}
`;

  const next = src.replace(saveRe, noopSave).replace(saveSyncRe, noopSaveSync);
  if (next === src) {
    console.error("[scrub-vendor] auth.js no-op patch produced no change; aborting");
    process.exit(2);
  }
  writeFileSync(authPath, next);
}

console.log(`[scrub-vendor] vendor ready (rewrote ${changed} files)`);
