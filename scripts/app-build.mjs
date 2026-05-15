import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(dirname(fileURLToPath(import.meta.url)));
const nodeBin = process.execPath;
const tscEntry = join(rootDir, "node_modules", "typescript", "bin", "tsc");
const viteEntry = join(rootDir, "node_modules", "vite", "bin", "vite.js");

const typecheck = spawnSync(nodeBin, [tscEntry, "-b"], {
  env: process.env,
  stdio: "inherit",
});

if ((typecheck.status ?? 1) !== 0) {
  process.exit(typecheck.status ?? 1);
}

const build = spawnSync(nodeBin, [viteEntry, "build"], {
  env: process.env,
  stdio: "inherit",
});

process.exit(build.status ?? 1);
