#!/usr/bin/env node
// CI-only：把 tag 里的版本号同步写回 tauri.conf.json 和 src-tauri/Cargo.toml。
// tauri-action 默认拿 tauri.conf.json 的 version 当产物文件名 + updater 的 version
// 字段，如果它和 git tag 不一致，updater 的 latest.json 路径里会塞 tag 版本，
// 但文件名里仍是 conf 里的旧版本，OSS 上对不上就 404。
//
// 用法：node scripts/sync-version.mjs <version>
//   e.g. node scripts/sync-version.mjs 1.1.2
import { readFileSync, writeFileSync } from "node:fs";

const [, , rawVersion] = process.argv;
if (!rawVersion) {
  console.error("Usage: sync-version.mjs <version>");
  process.exit(1);
}
const version = rawVersion.replace(/^v/, "");
if (!/^\d+\.\d+\.\d+(?:[-+].+)?$/.test(version)) {
  console.error(`✗ 非法版本号: ${rawVersion}`);
  process.exit(1);
}

const tauriConfPath = "src-tauri/tauri.conf.json";
const cargoPath = "src-tauri/Cargo.toml";

// tauri.conf.json: 顶层 "version": "x.y.z"
const conf = JSON.parse(readFileSync(tauriConfPath, "utf8"));
const oldConfVersion = conf.version;
conf.version = version;
writeFileSync(tauriConfPath, `${JSON.stringify(conf, null, 2)}\n`);
console.log(`✓ ${tauriConfPath}: ${oldConfVersion} → ${version}`);

// Cargo.toml: 只改 [package] 段第一个 version =
const cargo = readFileSync(cargoPath, "utf8");
let replaced = false;
const next = cargo.replace(
  /(\[package\][\s\S]*?\nversion\s*=\s*")([^"]+)(")/,
  (_m, head, oldV, tail) => {
    replaced = true;
    console.log(`✓ ${cargoPath}: ${oldV} → ${version}`);
    return `${head}${version}${tail}`;
  },
);
if (!replaced) {
  console.error(`✗ 未在 ${cargoPath} 找到 [package].version`);
  process.exit(1);
}
writeFileSync(cargoPath, next);
