import { spawnSync } from "node:child_process";

const isFullBuild = process.argv.includes("full");
const env = { ...process.env };

if (!isFullBuild) {
  env.VITE_SUPERAI_PUBLIC_BUILD = "1";
}

const npmBin = process.platform === "win32" ? "npm.cmd" : "npm";
const result = spawnSync(npmBin, ["exec", "tauri", "dev"], {
  env,
  stdio: "inherit",
});

process.exit(result.status ?? 1);
