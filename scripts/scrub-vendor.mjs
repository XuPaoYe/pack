#!/usr/bin/env node
// 把 vendor/windsurfapi 复制到 .vendor-build/superai-sidecar，并替换里面所有
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
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..");
const SRC_DIR = join(repoRoot, "vendor", "windsurfapi");
// 注意：OUT_DIR 故意不叫 "windsurfapi"。bun --compile 会把每个被打包源文件
// 的相对路径作为一行 `// path/to/file.js` 注释嵌进生成的二进制，路径里
// 任何 "windsurf" 字眼都能被 strings(1) 直接挖出来。换成 superai-sidecar
// 后，整个 .vendor-build/ 路径对 binary 来说是无痕的。
const OUT_DIR = join(repoRoot, ".vendor-build", "superai-sidecar");

if (!statSync(SRC_DIR, { throwIfNoEntry: false })?.isDirectory()) {
  console.error(`[scrub-vendor] missing source: ${SRC_DIR}`);
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
  // 上游源码里几个以 windsurf 开头的内部文件名（windsurf.js、windsurf-api.js、
  // dashboard/windsurf-login.js、dashboard/local-windsurf.js）会被 bun bundler
  // 当作源路径注释嵌进二进制；同步替换文件内的 import 路径，避免下面 rename
  // 后引用失效。这里只动 ESM/CJS import 的字符串字面量，不会误伤协议字面量
  // 'windsurf'（带单引号）或 windsurf.com 之类。
  [/(["'])(\.{1,2}\/(?:dashboard\/)?)local-windsurf(\.js\1)/g, "$1$2local-superai$3"],
  [/(["'])(\.{1,2}\/(?:dashboard\/)?)windsurf-login(\.js\1)/g, "$1$2superai-login$3"],
  [/(["'])(\.{1,2}\/)windsurf-api(\.js\1)/g, "$1$2superai-api$3"],
  [/(["'])(\.{1,2}\/)windsurf(\.js\1)/g, "$1$2superai$3"],
  // 2.0.96 里几个会出现在 sidecar 日志 / 临时文件路径里的 lowercase 字面量。
  // 不影响协议（不上链路），但 strings(1) 拿 binary 时会暴露。
  //   dashboard/api.js: `local-windsurf import ...` 日志前缀
  //   dashboard/api.js: 日志导出文件名 `windsurf-api-logs-...`
  //   dashboard/windsurf-login.js: 函数名 windsurfLoginViaAuth1（+ 调用点）
  //   windsurf.js: /tmp/windsurf-sp-dump-...txt 调试 dump 文件路径
  [/local-windsurf import/g, "local-superai import"],
  [/windsurf-api-logs-/g, "superai-logs-"],
  [/windsurfLoginViaAuth1/g, "superaiLoginViaAuth1"],
  [/windsurf-sp-dump-/g, "superai-sp-dump-"],
  // 函数名（仅 dashboard / windsurf-login 用，与协议解耦）
  [/windsurfLoginViaFirebase/g, "superaiLoginViaFirebase"],
  [/\bwindsurfLogin\b/g, "superaiLogin"],
  // sidecar dashboard 内部 HTTP 路由（我们的 tiny_http 反代只转发 /v1/* 和
  // /auth/*，不会触达，但路径字符串仍嵌进 binary）
  [/\/windsurf-login\b/g, "/superai-login"],
  // 本地 Windsurf 凭证导入扫描用的状态 key / SQL LIKE 模式
  [/windsurfAuthStatus/g, "superaiAuthStatus"],
  [/windsurfAuth%/g, "superaiAuth%"],
  // 用户数据/工作目录路径（dashboard 本地导入功能用；Super AI 不暴露 dashboard，
  // 这些路径在我们的运行环境下基本是死代码，但 strings(1) 仍能看见）
  [/\/opt\/windsurf\b/g, "/opt/superai"],
  [/windsurf-workspace/g, "superai-workspace"],
  [/\.windsurf\//g, ".superai/"],
  // resolve(home, '.windsurf', 'data') - macOS 默认 LS data root，渲染成
  // 字符串列表形式，不会被 \.windsurf\/ 命中。我们用 env 覆盖了，改它无影响。
  [/(['"])\.windsurf\1/g, "$1.superai$1"],
  // langserver.js CSRF 固定 token —— sidecar 自己生成自己消费，LS 不校验内容。
  [/windsurf-api-csrf-fixed-token/g, "superai-csrf-fixed-token"],
  // local-superai.js 临时状态目录前缀（Windsurf 本地凭证扫描用）
  [/windsurf-state-/g, "superai-state-"],
  // docker-self-update.js 容器 label —— 仅 dashboard self-update 用
  [/com\.windsurf-api\./g, "com.superai-sidecar."],
];

// 同步重命名表（OUT_DIR 相对路径 -> 新名）。和上面 import 替换是一一对应的。
const FILE_RENAMES = [
  ["src/windsurf.js", "src/superai.js"],
  ["src/windsurf-api.js", "src/superai-api.js"],
  ["src/dashboard/windsurf-login.js", "src/dashboard/superai-login.js"],
  ["src/dashboard/local-windsurf.js", "src/dashboard/local-superai.js"],
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

// 重命名物理文件。import 路径已经在上面替换好了，这里仅做磁盘移动。
for (const [from, to] of FILE_RENAMES) {
  const src = join(OUT_DIR, from);
  const dst = join(OUT_DIR, to);
  if (!statSync(src, { throwIfNoEntry: false })?.isFile()) {
    console.error(`[scrub-vendor] expected vendor file missing: ${from}`);
    process.exit(2);
  }
  renameSync(src, dst);
}

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
  const persistRe =
    /(\s+\/\/ Persist to disk\s+try \{\s+const \{ app, err \} = getStreams\(\);\s+)const line = JSON\.stringify\(entry\) \+ '\\n';/m;
  if (!persistRe.test(src)) {
    console.error("[scrub-vendor] logger.js persist anchor not found; aborting");
    process.exit(2);
  }
  // 通过 fromCharCode 拼出敏感词，避免最终二进制 strings(1) 还看得到。
  const replacement = `$1const __sw = String.fromCharCode(87,105,110,100,115,117,114,102);
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
  const next = src.replace(persistRe, replacement);
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
  // 用正则吃掉两个完整函数（含函数体），整段替换成 no-op。
  // 函数体里没有嵌套独立顶层 `^}`，所以 /^}/m 就能稳定匹配右括号。
  const saveRe = /function saveAccounts\(\) \{[\s\S]*?\r?\n\}\r?\n/;
  const saveSyncRe = /export function saveAccountsSync\(\) \{[\s\S]*?\r?\n\}\r?\n/;
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

console.log(`[scrub-vendor] ${OUT_DIR} ready (rewrote ${changed} files)`);
