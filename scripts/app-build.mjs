import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(dirname(fileURLToPath(import.meta.url)));
const binExt = process.platform === "win32" ? ".cmd" : "";
const tscBin = join(rootDir, "node_modules", ".bin", `tsc${binExt}`);
const viteBin = join(rootDir, "node_modules", ".bin", `vite${binExt}`);

const typecheck = spawnSync(tscBin, ["-b"], {
  env: process.env,
  stdio: "inherit",
});

if ((typecheck.status ?? 1) !== 0) {
  process.exit(typecheck.status ?? 1);
}

const build = spawnSync(viteBin, ["build"], {
  env: process.env,
  stdio: "inherit",
});

process.exit(build.status ?? 1);
