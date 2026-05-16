import { spawnSync } from "node:child_process";
<<<<<<< HEAD
import { existsSync } from "node:fs";
import { join } from "node:path";

// 直接走 node_modules/.bin/<bin>(.cmd) 而不是 npx —— 避免 Windows 上
// `spawnSync("npx", ..., { shell: true })` 偶发解析失败导致静默 exit 1
// （tauri-action 的 beforeBuildCommand 容易把这种 stdio 吞掉）。
const isWin = process.platform === "win32";
const binDir = join(process.cwd(), "node_modules", ".bin");
const tscBin = join(binDir, isWin ? "tsc.cmd" : "tsc");
const viteBin = join(binDir, isWin ? "vite.cmd" : "vite");

function run(label, bin, args) {
  console.log(`▶ [app-build] ${label}: ${bin} ${args.join(" ")}`);
  if (!existsSync(bin)) {
    console.error(`✗ [app-build] ${label} binary missing: ${bin}`);
    process.exit(1);
  }
  const r = spawnSync(bin, args, {
    env: process.env,
    stdio: "inherit",
    // Windows 下 .cmd 必须经 shell 解释。
    shell: isWin,
  });
  if (r.error) {
    console.error(`✗ [app-build] ${label} spawn error: ${r.error.message}`);
    process.exit(1);
  }
  const code = r.status ?? 1;
  if (code !== 0) {
    console.error(`✗ [app-build] ${label} exited with ${code}`);
    process.exit(code);
  }
  console.log(`✓ [app-build] ${label} done`);
}

run("tsc -b", tscBin, ["-b"]);
run("vite build", viteBin, ["build"]);
=======
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
>>>>>>> dev
