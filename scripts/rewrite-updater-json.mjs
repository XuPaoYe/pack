#!/usr/bin/env node
// 把 tauri-action 自动生成的 latest.json 改写成 OSS-ready 版本：
//  1. URL 从 https://github.com/<owner>/<repo>/releases/download/v{ver}/<file>
//     改成 <base-url>/v{ver}/<file>
//  2. 去掉 tauri 重复产生的 -nsis / -app 变种 key（updater 只读 4 个标准 key）
//
// 用法：
//   node scripts/rewrite-updater-json.mjs <path> <version> <base-url>
// 例子：
//   node scripts/rewrite-updater-json.mjs latest.json 1.0.1 https://ai.talentisan.cn/SuperAI
import { readFileSync, writeFileSync } from "node:fs";

const [, , inputPath, version, baseUrl] = process.argv;
if (!inputPath || !version || !baseUrl) {
  console.error(
    "Usage: rewrite-updater-json.mjs <path> <version> <base-url>\n" +
      "  e.g. rewrite-updater-json.mjs latest.json 1.0.1 https://ai.talentisan.cn/SuperAI",
  );
  process.exit(1);
}

const ALLOWED_KEYS = new Set([
  "darwin-aarch64",
  "darwin-x86_64",
  "windows-x86_64",
  "windows-aarch64",
]);

const cleanedBase = baseUrl.replace(/\/+$/, "");
const data = JSON.parse(readFileSync(inputPath, "utf8"));

const original = data.platforms ?? {};
const platforms = {};
for (const [key, value] of Object.entries(original)) {
  if (!ALLOWED_KEYS.has(key)) continue;
  if (!value?.url) continue;
  const filename = new URL(value.url).pathname.split("/").pop();
  platforms[key] = {
    ...value,
    url: `${cleanedBase}/v${version}/${filename}`,
  };
}

const missing = [...ALLOWED_KEYS].filter((k) => !(k in platforms));
if (missing.length > 0) {
  console.warn(`⚠  缺平台: ${missing.join(", ")}（updater 不会向这些平台推送升级）`);
}

data.platforms = platforms;
writeFileSync(inputPath, `${JSON.stringify(data, null, 2)}\n`);
console.log(`✓ 已重写 ${inputPath}  v${version}  → ${cleanedBase}/v${version}/`);
