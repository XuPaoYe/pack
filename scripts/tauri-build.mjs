import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(dirname(fileURLToPath(import.meta.url)));

// argv 只看脚本以后的部分，否则 process.argv[1] 是带连字符的脚本路径，
// 会被误认成 target triple，让 tauri-cli 报 `Invalid target triple: ...mjs`。
const argv = process.argv.slice(2);

const KNOWN_BUNDLES = new Set(["app", "deb", "rpm", "appimage", "msi", "nsis", "dmg", "updater"]);
const KNOWN_TARGETS = new Set([
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "universal-apple-darwin",
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
]);

const isFullBuild = argv.includes("full");
const bundle = argv.find((a) => KNOWN_BUNDLES.has(a)) ?? "app";
const explicitTargets = argv.filter((a) => KNOWN_TARGETS.has(a));

// 默认覆盖 4 个最终用户平台：Mac M 系列、Mac Intel、Win x64、Win ARM64。
// 没显式传 target 时全部跑一遍；任一目标失败不阻塞下一个，最后汇总。
const DEFAULT_TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
];

const targets = explicitTargets.length > 0 ? explicitTargets : DEFAULT_TARGETS;

const privateKeyPath = join(rootDir, "src-tauri", "updater-private.key");
const tauriBin = join(rootDir, "node_modules", ".bin", process.platform === "win32" ? "tauri.cmd" : "tauri");

const baseEnv = { ...process.env };
if (!isFullBuild) {
  baseEnv.VITE_SUPERAI_PUBLIC_BUILD = "1";
}
if (existsSync(privateKeyPath)) {
  baseEnv.TAURI_SIGNING_PRIVATE_KEY = readFileSync(privateKeyPath, "utf8");
  baseEnv.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = baseEnv.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "";
}

// macOS / Linux 主机交叉编译 Windows 时，需要：
//   - lld-link (brew install lld)
//   - cargo-xwin (~/.cargo/bin/cargo-xwin)
//   - **rustup 管理的 cargo/rustc**（不能用 Homebrew 装的 rust，brew 那个
//     是单 toolchain，看不到 `rustup target add ...-windows-msvc` 装下来
//     的 std；调它会报 `can't find crate for core`）。
//   实测路径：~/.rustup/toolchains/<channel>/bin
function augmentPathForWindowsCross(env) {
  const home = process.env.HOME ?? "";
  const extras = [
    "/usr/local/opt/lld/bin",     // Intel Mac brew
    "/opt/homebrew/opt/lld/bin",  // Apple Silicon brew
    `${home}/.cargo/bin`,
  ];

  // 用 rustup 问出当前默认 toolchain 的 cargo/rustc 真实路径，把那个目录
  // 怼到 PATH 最前。这样 cargo-xwin 派生子进程 cargo 时拿到的是 rustup 版。
  try {
    const which = spawnSync("rustup", ["which", "cargo"], { encoding: "utf8" });
    if (which.status === 0) {
      const cargoPath = which.stdout.trim();
      if (cargoPath) {
        extras.unshift(dirname(cargoPath));
      }
    }
  } catch {
    // rustup 不在 PATH —— 用户没装 rustup 的话这条路本来就走不通，让后续
    // cargo-xwin 自己报错给用户看。
  }

  const current = env.PATH ?? "";
  const merged = [...extras, ...current.split(":")].filter(Boolean);
  const seen = new Set();
  env.PATH = merged.filter((p) => (seen.has(p) ? false : (seen.add(p), true))).join(":");
}

function rustupHasTarget(target) {
  const result = spawnSync("rustup", ["target", "list", "--installed"], { encoding: "utf8" });
  if (result.status !== 0) return null; // rustup 不在或调用失败时不阻塞
  return result.stdout.split(/\r?\n/).map((l) => l.trim()).includes(target);
}

const summary = [];
for (const target of targets) {
  console.log(`\n── tauri build ──────────────────────────────────────`);
  console.log(`target  : ${target}`);
  console.log(`bundle  : ${bundle}`);
  console.log(`profile : ${isFullBuild ? "full" : "public"}`);

  const hasTarget = rustupHasTarget(target);
  if (hasTarget === false) {
    console.warn(
      `⚠️  跳过 ${target}: rustup 未安装该 target，请先 \`rustup target add ${target}\``,
    );
    summary.push({ target, status: "skipped", reason: "missing-rust-target" });
    continue;
  }

  // Windows 目标在 macOS/Linux 主机上走 cargo-xwin：
  //   - --runner cargo-xwin 让 tauri-cli 用 cargo-xwin 替代 cargo，自动拉
  //     Windows SDK + 注入交叉编译环境
  //   - 把 bundle 强制压成 nsis：
  //       * msi 走 WiX，需要 Wine + Mono，太重
  //       * NSIS 只要 makensis (brew install makensis)，已就绪
  //     用户显式传 --bundles 时（msi/nsis）尊重原值
  const isCrossWindows = target.endsWith("-pc-windows-msvc") &&
    process.platform !== "win32";
  const effectiveBundle = isCrossWindows && bundle === "app" ? "nsis" : bundle;
  const tauriArgs = ["build", "--bundles", effectiveBundle, "--target", target];
  const env = { ...baseEnv };
  if (isCrossWindows) {
    augmentPathForWindowsCross(env);
    tauriArgs.push("--runner", "cargo-xwin");
  }
  const result = spawnSync(tauriBin, tauriArgs, {
    cwd: rootDir,
    env,
    stdio: "inherit",
  });

  if ((result.status ?? 1) !== 0) {
    summary.push({ target, status: "failed", code: result.status ?? 1 });
    if (explicitTargets.length > 0) {
      // 显式指定单一 target 时按用户预期立刻退出。
      process.exit(result.status ?? 1);
    }
    continue;
  }

  summary.push({ target, status: "ok" });
}

console.log("\n== build summary ==");
for (const row of summary) {
  const tag = row.status === "ok" ? "✓"
    : row.status === "skipped" ? "·"
    : "✗";
  const detail = row.reason ?? (row.code ? `exit=${row.code}` : "");
  console.log(`  ${tag} ${row.target}${detail ? `   (${detail})` : ""}`);
}

const okCount = summary.filter((r) => r.status === "ok").length;
const failedCount = summary.filter((r) => r.status === "failed").length;
const skippedCount = summary.filter((r) => r.status === "skipped").length;
console.log(`  ${okCount} ok / ${failedCount} failed / ${skippedCount} skipped`);

// 没有任何成功 → 视为失败；有失败也回非 0 给 CI。
if (okCount === 0 || failedCount > 0) {
  process.exit(1);
}
process.exit(0);
