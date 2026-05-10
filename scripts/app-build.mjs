import { spawnSync } from "node:child_process";

const npmBin = process.platform === "win32" ? "npm.cmd" : "npm";

const typecheck = spawnSync("npx", ["tsc", "-b"], {
  env: process.env,
  stdio: "inherit",
  shell: process.platform === "win32",
});

if ((typecheck.status ?? 1) !== 0) {
  process.exit(typecheck.status ?? 1);
}

const build = spawnSync(npmBin, ["exec", "vite", "build"], {
  env: process.env,
  stdio: "inherit",
});

process.exit(build.status ?? 1);
