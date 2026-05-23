#!/usr/bin/env node
// 把 vendor/superai-sidecar 复制到 .vendor-build/superai-sidecar，并替换里面所有
// 用户可见的上游品牌字面量。生成的副本喂给 bun --compile，原始 vendor
// 不动，方便上游升级时直接覆盖再重跑构建。
//
// 替换策略：
// 1) 大写开头的上游品牌名 —— 用户可见英文展示文本，整体改成 SuperAI。
// 2) lowercase 项目代号 —— 改成 superai-sidecar。
// 3) 上游内部 id 字段 —— /v1/models 响应里我们要隐藏的字段名，改 _internal_id。
// 4) `owned_by: info.provider` —— 同样在 /v1/models 输出，把内部 provider 收
//    口成 'superai'，其余 provider 透传。
// 5) `dashboard/logger.js` 落盘 JSONL 前再过一次 sanitize，防止运行时上游
//    返回的品牌字符串透过结构化日志被写到磁盘。
//
// 不动：
// - 协议要求的字面量：writeStringField metadata、model provider tag、上游 URL、
//   ALL_CAPS 环境变量名。这些是服务端校验的一部分，不在原始 vendor 里硬改。
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
const SRC_DIR = join(repoRoot, "vendor", "superai-sidecar");
// 注意：OUT_DIR 故意不叫上游项目名。bun --compile 会把每个被打包源文件
// 的相对路径作为一行 `// path/to/file.js` 注释嵌进生成的二进制，路径里
// 任何上游品牌字眼都能被 strings(1) 直接挖出来。换成 superai-sidecar 后，
// 整个 .vendor-build/ 路径对 binary 来说是无痕的。
const OUT_DIR = join(repoRoot, ".vendor-build", "superai-sidecar");

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

const legacyLower = [119, 105, 110, 100, 115, 117, 114, 102]
  .map((c) => String.fromCharCode(c))
  .join("");
const legacyUpper = legacyLower[0].toUpperCase() + legacyLower.slice(1);
const legacyProject = `${legacyLower}api`;
const legacyProjectUpper = `${legacyUpper}API`;
const legacyProjectSpaced = `${legacyUpper} API`;
const legacyIdField = `_${legacyLower}_id`;
const legacyProxyHeader = `org-${legacyLower}-proxy`;
const legacyLogin = `${legacyLower}-login`;
const legacyApiFile = `${legacyLower}-api`;
const legacyLocal = `local-${legacyLower}`;
const legacyFunctionPrefix = `${legacyLower}Login`;

function re(source, flags = "g") {
  return new RegExp(source, flags);
}

function literal(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const REGEX_REPLACEMENTS = [
  // 用户可见英文展示文本
  [re(literal(legacyProjectUpper)), "SuperAI"],
  [re(literal(legacyProjectSpaced)), "SuperAI"],
  [re(literal(legacyUpper)), "SuperAI"],
  // 项目代号
  [re(literal(legacyProject)), "superai-sidecar"],
  // /v1/models 响应里要隐藏的内部字段名
  [re(literal(legacyIdField)), "_internal_id"],
  // server.js 给每个 OpenAI 兼容响应加的 openai-organization 响应头，调用
  // 方在 IDE 网络面板里能直接看到。
  [re(literal(legacyProxyHeader)), "org-superai-proxy"],
  // 上游内部文件名会被 bun bundler 当作源路径注释嵌进二进制；同步替换
  // import 路径，避免下面 rename 后引用失效。这里只动 ESM/CJS import 的
  // 字符串字面量，不误伤协议字面量或 URL。
  [re(`(["'])(\\.{1,2}\\/(?:dashboard\\/)?)${literal(legacyLocal)}(\\.js\\1)`), "$1$2local-superai$3"],
  [re(`(["'])(\\.{1,2}\\/(?:dashboard\\/)?)${literal(legacyLogin)}(\\.js\\1)`), "$1$2superai-login$3"],
  [re(`(["'])(\\.{1,2}\\/)${literal(legacyApiFile)}(\\.js\\1)`), "$1$2superai-api$3"],
  [re(`(["'])(\\.{1,2}\\/)${literal(legacyLower)}(\\.js\\1)`), "$1$2superai$3"],
  // 2.0.96 里几个会出现在 sidecar 日志 / 临时文件路径里的 lowercase 字面量。
  // 不影响协议（不上链路），但 strings(1) 拿 binary 时会暴露。
  [re(`${literal(legacyLocal)} import`), "local-superai import"],
  [re(`${literal(legacyApiFile)}-logs-`), "superai-logs-"],
  [re(`${literal(legacyFunctionPrefix)}ViaAuth1`), "superaiLoginViaAuth1"],
  [re(`${literal(legacyLower)}-sp-dump-`), "superai-sp-dump-"],
  // 函数名（仅 dashboard 登录页用，与协议解耦）
  [re(`${literal(legacyFunctionPrefix)}ViaFirebase`), "superaiLoginViaFirebase"],
  [re(`\\b${literal(legacyFunctionPrefix)}\\b`), "superaiLogin"],
  // sidecar dashboard 内部 HTTP 路由（我们的 tiny_http 反代只转发 /v1/* 和
  // /auth/*，不会触达，但路径字符串仍嵌进 binary）
  [re(`/${literal(legacyLogin)}\\b`), "/superai-login"],
  // 本地运行时凭证导入扫描用的状态 key / SQL LIKE 模式
  [re(`${literal(legacyLower)}AuthStatus`), "superaiAuthStatus"],
  [re(`${literal(legacyLower)}Auth%`), "superaiAuth%"],
  // 用户数据/工作目录路径（dashboard 本地导入功能用；Super AI 不暴露 dashboard，
  // 这些路径在我们的运行环境下基本是死代码，但 strings(1) 仍能看见）
  [re(`/opt/${literal(legacyLower)}\\b`), "/opt/superai"],
  [re(`${literal(legacyLower)}-workspace`), "superai-workspace"],
  [re(`\\.${literal(legacyLower)}/`), ".superai/"],
  // 默认 data root 也会以字符串列表形式进入产物；我们用 env 覆盖了，改它无影响。
  [re(`(['"])\\.${literal(legacyLower)}\\1`), "$1.superai$1"],
  // langserver.js CSRF 固定 token —— sidecar 自己生成自己消费，LS 不校验内容。
  [re(`${literal(legacyApiFile)}-csrf-fixed-token`), "superai-csrf-fixed-token"],
  // local-superai.js 临时状态目录前缀
  [re(`${literal(legacyLower)}-state-`), "superai-state-"],
  // docker-self-update.js 容器 label —— 仅 dashboard self-update 用
  [re(`com\\.${literal(legacyApiFile)}\\.`), "com.superai-sidecar."],
];

// 同步重命名表（OUT_DIR 相对路径 -> 新名）。和上面 import 替换是一一对应的。
const FILE_RENAMES = [
  [`src/${legacyLower}.js`, "src/superai.js"],
  [`src/${legacyApiFile}.js`, "src/superai-api.js"],
  [`src/dashboard/${legacyLogin}.js`, "src/dashboard/superai-login.js"],
  [`src/dashboard/${legacyLocal}.js`, "src/dashboard/local-superai.js"],
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

// /v1/models -> owned_by 收口：内部 provider 改 'superai'，其余 provider 透传。
const modelsPath = join(OUT_DIR, "src", "models.js");
{
  const src = readFileSync(modelsPath, "utf8");
  const needle = "      owned_by: info.provider,";
  if (!src.includes(needle)) {
    console.error("[scrub-vendor] models.js owned_by anchor not found; aborting");
    process.exit(2);
  }
  const providerExpr = "String.fromCharCode(119,105,110,100,115,117,114,102)";
  const replaced = src.replace(
    needle,
    `      owned_by: info.provider === ${providerExpr} ? 'superai' : info.provider,`,
  );
  writeFileSync(modelsPath, replaced);
}

// 新模型临时收口：前端下拉已经暴露这些 key；在 upstream catalog 正式带上前，
// 给 sidecar 静态表补齐解析，避免 chat handler 在本地先 400 Unsupported model。
{
  const src = readFileSync(modelsPath, "utf8");
  const modelAnchor =
    "  'gemini-3.1-pro-high':            { name: 'gemini-3.1-pro-high',            provider: 'google', enumValue: 0,   modelUid: 'gemini-3-1-pro-high', credit: 2 },";
  const deepseekAnchor =
    "  'deepseek-r1':                    { name: 'deepseek-r1',                    provider: 'deepseek', enumValue: 206, credit: 1, deprecated: true },";
  const aliasAnchor = "_lookup.set('minimax-m2-5', 'minimax-m2.5');";
  if (!src.includes(modelAnchor) || !src.includes(deepseekAnchor) || !src.includes(aliasAnchor)) {
    console.error("[scrub-vendor] new model patch anchors not found; aborting");
    process.exit(2);
  }
  const modelPatch = `${modelAnchor}
  'gemini-3.5-flash-minimal':       { name: 'gemini-3.5-flash-minimal',       provider: 'google', enumValue: 0,   modelUid: 'gemini-3-5-flash-minimal', credit: 0.75 },
  'gemini-3.5-flash-low':           { name: 'gemini-3.5-flash-low',           provider: 'google', enumValue: 0,   modelUid: 'gemini-3-5-flash-low', credit: 1 },
  'gemini-3.5-flash':               { name: 'gemini-3.5-flash',               provider: 'google', enumValue: 0,   modelUid: 'gemini-3-5-flash-medium', credit: 1 },
  'gemini-3.5-flash-high':          { name: 'gemini-3.5-flash-high',          provider: 'google', enumValue: 0,   modelUid: 'gemini-3-5-flash-high', credit: 1.75 },`;
  const deepseekPatch = `${deepseekAnchor}
  'deepseek-v4':                    { name: 'deepseek-v4',                    provider: 'deepseek', enumValue: 0,   modelUid: 'deepseek-v4', credit: 1 },`;
  const aliasPatch = `${aliasAnchor}
_lookup.set('gemini-3-5-flash', 'gemini-3.5-flash');
_lookup.set('gemini-3-5-flash-medium', 'gemini-3.5-flash');
_lookup.set('gemini-3-5-flash-minimal', 'gemini-3.5-flash-minimal');
_lookup.set('gemini-3-5-flash-low', 'gemini-3.5-flash-low');
_lookup.set('gemini-3-5-flash-high', 'gemini-3.5-flash-high');
_lookup.set('MODEL_GOOGLE_GEMINI_3_5_FLASH_MEDIUM', 'gemini-3.5-flash');
_lookup.set('MODEL_GOOGLE_GEMINI_3_5_FLASH_MINIMAL', 'gemini-3.5-flash-minimal');
_lookup.set('MODEL_GOOGLE_GEMINI_3_5_FLASH_LOW', 'gemini-3.5-flash-low');
_lookup.set('MODEL_GOOGLE_GEMINI_3_5_FLASH_HIGH', 'gemini-3.5-flash-high');
_lookup.set('deepseek-v4', 'deepseek-v4');`;
  const next = src
    .replace(modelAnchor, modelPatch)
    .replace(deepseekAnchor, deepseekPatch)
    .replace(aliasAnchor, aliasPatch);
  writeFileSync(modelsPath, next);
}

// SWE 1.6 在当前 LS 里有 enum 420，但 direct requested_model_uid 会被拒绝
// （MODEL_SWE_1_6 / cognition-swe-1.6 都返回 unknown model UID）。保留 enum
// 路径，不发送 UID；同时让 probe 实测覆盖 GetUserStatus 的旧 allowlist。
{
  const authPath = join(OUT_DIR, "src", "auth.js");
  const modelSrc = readFileSync(modelsPath, "utf8");
  const uidNeedle = "modelUid: 'MODEL_SWE_1_6'";
  if (!modelSrc.includes(uidNeedle)) {
    console.error("[scrub-vendor] SWE 1.6 modelUid anchor not found; aborting");
    process.exit(2);
  }
  writeFileSync(modelsPath, modelSrc.replace(`${uidNeedle}, `, ""));

  const src = readFileSync(authPath, "utf8");
  const probeAnchor = "  'gemini-3.0-flash',\n];";
  const capAnchor = "if (!prev || prev.reason !== 'success') {";
  const allowedAnchor = "if (cap?.reason === 'user_status' || cap?.reason === 'not_entitled') {\n      return cap.ok === true;\n    }";
  const canaryCascadeAnchor = "const useCascade = !!info.modelUid;";
  const availableAnchor = "      if (cap?.reason === 'user_status' && cap.ok === true) allowed.push(key);";
  const skipNotEntitledAnchor = "      if (cap && cap.reason === 'not_entitled') return false;";
  if (!src.includes(probeAnchor) || !src.includes(capAnchor) || !src.includes(allowedAnchor) || !src.includes(canaryCascadeAnchor) || !src.includes(availableAnchor) || !src.includes(skipNotEntitledAnchor)) {
    console.error("[scrub-vendor] SWE 1.6 probe anchors not found; aborting");
    process.exit(2);
  }
  const next = src
    .replace(probeAnchor, "  'gemini-3.0-flash',\n  'swe-1.6',\n];")
    .replace(capAnchor, "if (!prev || (prev.reason !== 'success' && prev.reason !== 'cloud_probe')) {")
    .replace(allowedAnchor, "if (cap?.ok === true && (cap.reason === 'success' || cap.reason === 'cloud_probe' || cap.reason === 'user_status')) {\n      return true;\n    }\n    if (cap?.reason === 'not_entitled') {\n      return false;\n    }")
    .replace(canaryCascadeAnchor, "const useCascade = !!info.modelUid || modelKey === 'swe-1.6';")
    .replace(availableAnchor, "      if (cap?.ok === true && (cap.reason === 'user_status' || cap.reason === 'success' || cap.reason === 'cloud_probe')) allowed.push(key);")
    .replace(skipNotEntitledAnchor, "      if (cap && cap.reason === 'not_entitled' && key !== 'swe-1.6') return false;");
  writeFileSync(authPath, next);
}

// SWE 1.6 更接近官方 Cascade 原生路径：有工具时默认走 native bridge，
// 避免在 NO_TOOL 模式下塞长 toolPreamble 导致 planner 行为偏离编辑器。
{
  const bridgePath = join(OUT_DIR, "src", "cascade-native-bridge.js");
  const src = readFileSync(bridgePath, "utf8");
  const nativeBridgeEnv = `${legacyProject.toUpperCase()}_NATIVE_TOOL_BRIDGE`;
  const needle = `  const explicitOn = process.env.${nativeBridgeEnv} === '1';`;
  if (!src.includes(needle)) {
    console.error("[scrub-vendor] native bridge anchor not found; aborting");
    process.exit(2);
  }
  const next = src.replace(
    needle,
    `  const explicitOn = process.env.${nativeBridgeEnv} === '1' || modelKey === 'swe-1.6';`,
  );
  writeFileSync(bridgePath, next);
}

// 远端模型表是新模型的权威来源。原 upstream mergeCloudModels 只 add 不 update，
// 如果本地已有 swe-1.6 就看不到云端真实 modelUid。这里打印 SWE 相关远端
// 配置，并允许 cloud entry 覆盖本地同 key / 同 UID 的静态配置。
{
  const authPath = join(OUT_DIR, "src", "auth.js");
  const src = readFileSync(authPath, "utf8");
  const needle = "    const added = mergeCloudModels(configs);\n    log.info(`Model catalog: ${configs.length} cloud models, ${added} new entries merged`);";
  if (!src.includes(needle)) {
    console.error("[scrub-vendor] model catalog log anchor not found; aborting");
    process.exit(2);
  }
  const replacement = `    const sweConfigs = configs.filter((m) => /swe/i.test(String(m.modelUid || m.label || m.name || '')));
    for (const m of sweConfigs) {
      log.info(\`Model catalog SWE: uid=\${m.modelUid || ''} label=\${m.label || m.name || ''} provider=\${m.provider || ''} credit=\${m.creditMultiplier || ''}\`);
    }
    const added = mergeCloudModels(configs);
    log.info(\`Model catalog: \${configs.length} cloud models, \${added} new entries merged\`);`;
  writeFileSync(authPath, src.replace(needle, replacement));
}

{
  const src = readFileSync(modelsPath, "utf8");
  const needle = "    // Already in catalog?\n    if (_lookup.has(uid) || _lookup.has(uid.toLowerCase())) continue;\n\n    const key = uid.toLowerCase().replace(/_/g, '-');\n    if (MODELS[key]) continue;\n\n    const provider = providerMap[m.provider] || m.provider?.toLowerCase()?.replace('model_provider_', '') || 'unknown';\n    MODELS[key] = {";
  if (!src.includes(needle)) {
    console.error("[scrub-vendor] mergeCloudModels anchor not found; aborting");
    process.exit(2);
  }
  const replacement = `    const provider = providerMap[m.provider] || m.provider?.toLowerCase()?.replace('model_provider_', '') || 'unknown';
    const key = uid.toLowerCase().replace(/_/g, '-');
    const existingKey = _lookup.get(uid) || _lookup.get(uid.toLowerCase()) || (MODELS[key] ? key : null);
    if (existingKey && MODELS[existingKey]) {
      if (/swe/i.test(uid) || /swe/i.test(existingKey)) {
        MODELS[existingKey] = {
          ...MODELS[existingKey],
          provider,
          modelUid: uid,
          credit: m.creditMultiplier || MODELS[existingKey].credit || 1,
        };
        _lookup.set(uid, existingKey);
        _lookup.set(uid.toLowerCase(), existingKey);
      }
      continue;
    }

    MODELS[key] = {`;
  writeFileSync(modelsPath, src.replace(needle, replacement));
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
// 启动时 Tauri API 服务会主动删旧 accounts.json 并通过 reconcile_accounts
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

// langserver.js：Windows 下 Node spawn console-subsystem 二进制时，默认可能闪出
// 控制台窗口。Tauri 侧已经给 sidecar 设置 CREATE_NO_WINDOW；这里补上 sidecar
// 内部启动 LS 的 windowsHide，避免第二层子进程弹窗。
const langserverPath = join(OUT_DIR, "src", "langserver.js");
{
  const src = readFileSync(langserverPath, "utf8");
  const needle = `    const proc = spawn(_binaryPath, args, {
      stdio: ['pipe', 'pipe', 'pipe'],
      env,
    });`;
  if (!src.includes(needle)) {
    console.error("[scrub-vendor] langserver.js spawn anchor not found; aborting");
    process.exit(2);
  }
  const replacement = `    const proc = spawn(_binaryPath, args, {
      stdio: ['pipe', 'pipe', 'pipe'],
      env,
      windowsHide: true,
    });`;
  writeFileSync(langserverPath, src.replace(needle, replacement));
}

console.log(`[scrub-vendor] vendor ready (rewrote ${changed} files)`);
