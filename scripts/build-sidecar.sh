#!/usr/bin/env bash
# 构建 Windsurf 本地 API 服务所需的两个 sidecar 二进制：
#   1) windsurfapi-<target>     —— 用 bun --compile 把 vendor/windsurfapi 打成单文件
#   2) language_server_<target> —— 从已安装的 Windsurf.app / 用户指定路径里抽
#
# 输出统一放到 src-tauri/binaries/，遵循 Tauri externalBin 的 <name>-<rust-target-triple>
# 命名规范，调用方按 target triple 选择对应文件。
#
# 用法:
#   scripts/build-sidecar.sh                  # 自动识别当前平台
#   WINDSURF_LS_PATH=/path scripts/build-sidecar.sh
#   TARGET=darwin-arm64 scripts/build-sidecar.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_DIR="$REPO_ROOT/vendor/windsurfapi"
OUTPUT_DIR="$REPO_ROOT/src-tauri/binaries"

if [[ ! -d "$VENDOR_DIR/src" ]]; then
  echo "❌ 未找到 vendor/windsurfapi/src，先把上游代码 vendor 进来" >&2
  exit 1
fi

if ! command -v bun >/dev/null 2>&1; then
  echo "❌ 需要 bun (>=1.3) 来编译 sidecar，请先安装：https://bun.sh" >&2
  exit 1
fi

# 推断当前平台的 bun target + Rust target triple
detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64)  echo "bun-darwin-arm64|aarch64-apple-darwin" ;;
        x86_64) echo "bun-darwin-x64|x86_64-apple-darwin" ;;
        *)      echo "" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        aarch64) echo "bun-linux-arm64|aarch64-unknown-linux-gnu" ;;
        x86_64)  echo "bun-linux-x64|x86_64-unknown-linux-gnu" ;;
        *)       echo "" ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*)
      echo "bun-windows-x64|x86_64-pc-windows-msvc"
      ;;
    *) echo "" ;;
  esac
}

read -r BUN_TARGET RUST_TRIPLE <<<"$(detect_target | tr '|' ' ')"
if [[ -z "${BUN_TARGET:-}" || -z "${RUST_TRIPLE:-}" ]]; then
  echo "❌ 无法识别当前平台" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

# 1) 编译 WindsurfPoolAPI sidecar
echo "▶ bun build --compile --target=$BUN_TARGET"
SIDECAR_OUT="$OUTPUT_DIR/windsurfapi-$RUST_TRIPLE"
WINDOWS_SUFFIX=""
if [[ "$RUST_TRIPLE" == *windows* ]]; then
  WINDOWS_SUFFIX=".exe"
  SIDECAR_OUT="$SIDECAR_OUT.exe"
fi
bun build --compile --target="$BUN_TARGET" \
  "$VENDOR_DIR/src/index.js" \
  --outfile "$SIDECAR_OUT"
chmod +x "$SIDECAR_OUT"
echo "✓ $SIDECAR_OUT"

# 2) 抽 Windsurf Language Server 二进制
LS_OUT="$OUTPUT_DIR/language_server-$RUST_TRIPLE$WINDOWS_SUFFIX"

# 用户显式指定优先
if [[ -n "${WINDSURF_LS_PATH:-}" && -f "$WINDSURF_LS_PATH" ]]; then
  cp "$WINDSURF_LS_PATH" "$LS_OUT"
  chmod +x "$LS_OUT"
  echo "✓ $LS_OUT (来自 \$WINDSURF_LS_PATH)"
  exit 0
fi

# 否则按平台自动探测
candidates=()
case "$RUST_TRIPLE" in
  aarch64-apple-darwin)
    candidates+=(
      "/Applications/Windsurf.app/Contents/Resources/app/extensions/windsurf/bin/language_server_macos_arm"
      "/Applications/Windsurf.app/Contents/Resources/app/extensions/windsurf/bin/language_server_macos_arm64"
      "/Applications/Windsurf.app/Contents/Resources/app/extensions/windsurf/bin/language_server_macos_x64"
    )
    ;;
  x86_64-apple-darwin)
    candidates+=(
      "/Applications/Windsurf.app/Contents/Resources/app/extensions/windsurf/bin/language_server_macos_x64"
    )
    ;;
  x86_64-unknown-linux-gnu)
    candidates+=("/opt/windsurf/language_server_linux_x64")
    ;;
  aarch64-unknown-linux-gnu)
    candidates+=("/opt/windsurf/language_server_linux_arm")
    ;;
  x86_64-pc-windows-msvc)
    candidates+=("C:/Program Files/Windsurf/resources/app/extensions/windsurf/bin/language_server_windows_x64.exe")
    ;;
esac

found=""
for c in "${candidates[@]}"; do
  if [[ -f "$c" ]]; then found="$c"; break; fi
done

if [[ -z "$found" ]]; then
  cat >&2 <<EOF
❌ 没找到 Windsurf Language Server 二进制
   请安装 Windsurf 应用后重试，或手动设置 WINDSURF_LS_PATH 指向已有的 LS 文件。
   候选位置：
$(printf '   - %s\n' "${candidates[@]}")
EOF
  exit 1
fi

cp "$found" "$LS_OUT"
chmod +x "$LS_OUT"
echo "✓ $LS_OUT (来自 $found)"
echo
echo "🟢 sidecar 构建完成。可以 npm run tauri dev 了。"
