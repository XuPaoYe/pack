import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(dirname(fileURLToPath(import.meta.url)));
const bundle = process.argv[2] ?? "app";
const isFullBuild = process.argv.includes("full");
const target = process.argv.find((arg) => arg.includes("-") && arg !== "full" && arg !== bundle);
const privateKeyPath = join(rootDir, "src-tauri", "updater-private.key");
const tauriBin = join(rootDir, "node_modules", ".bin", process.platform === "win32" ? "tauri.cmd" : "tauri");
const env = { ...process.env };

if (!isFullBuild) {
  env.VITE_SUPERAI_PUBLIC_BUILD = "1";
}

if (existsSync(privateKeyPath)) {
  env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(privateKeyPath, "utf8");
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "";
}

const tauriArgs = ["build", "--bundles", bundle];
if (target) {
  tauriArgs.push("--target", target);
}

const result = spawnSync(tauriBin, tauriArgs, {
  cwd: rootDir,
  env,
  stdio: "inherit",
});

process.exit(result.status ?? 1);
