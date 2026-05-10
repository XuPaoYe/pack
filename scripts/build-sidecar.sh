#!/usr/bin/env bash
# 构建 SuperAI 本地 API 服务所需的两个 sidecar 二进制：
#   1) superai-api-<target>     —— 用 bun --compile 把 vendor 里的上游服务打成单文件
#   2) language_server_<target> —— 从已安装的运行时应用 / 用户指定路径里抽
#
# 输出统一放到 src-tauri/binaries/，遵循 Tauri externalBin 的 <name>-<rust-target-triple>
# 命名规范，调用方按 target triple 选择对应文件。
#
# 用法:
#   scripts/build-sidecar.sh                  # 自动识别当前平台
#   WINDSURF_LS_PATH=/path scripts/build-sidecar.sh
#   TARGET=darwin-arm64 scripts/build-sidecar.sh
#   TARGET=universal-apple-darwin scripts/build-sidecar.sh
#   TARGET=windows-x64 scripts/build-sidecar.sh
#   TARGET=windows-arm64 scripts/build-sidecar.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_SRC_DIR="$REPO_ROOT/vendor/windsurfapi"
# scrub-vendor.mjs 会把原始 vendor 复制到这里，并替换掉用户可见的
# Windsurf 字面量；bun --compile 实际读取的是这份副本。
SCRUBBED_DIR="$REPO_ROOT/.vendor-build/windsurfapi"
VENDOR_DIR="$SCRUBBED_DIR"
OUTPUT_DIR="$REPO_ROOT/src-tauri/binaries"

if [[ ! -d "$VENDOR_SRC_DIR/src" ]]; then
  echo "❌ 未找到 vendor/windsurfapi/src，先把上游代码 vendor 进来" >&2
  exit 1
fi

if ! command -v node >/dev/null 2>&1; then
  echo "❌ 需要 node 来跑 scripts/scrub-vendor.mjs" >&2
  exit 1
fi

if ! command -v bun >/dev/null 2>&1; then
  echo "❌ 需要 bun (>=1.3) 来编译 sidecar，请先安装：https://bun.sh" >&2
  exit 1
fi

echo "▶ scripts/scrub-vendor.mjs"
node "$REPO_ROOT/scripts/scrub-vendor.mjs"

target_pair_from_alias() {
  case "${1:-}" in
    darwin-arm64|mac-arm64|aarch64-apple-darwin)
      echo "bun-darwin-arm64|aarch64-apple-darwin" ;;
    darwin-x64|mac-x64|x86_64-apple-darwin)
      echo "bun-darwin-x64|x86_64-apple-darwin" ;;
    universal-apple-darwin|darwin-universal|mac-universal)
      echo "universal-apple-darwin|universal-apple-darwin" ;;
    windows-x64|win-x64|x86_64-pc-windows-msvc)
      echo "bun-windows-x64|x86_64-pc-windows-msvc" ;;
    windows-arm64|win-arm64|aarch64-pc-windows-msvc)
      echo "bun-windows-arm64|aarch64-pc-windows-msvc" ;;
    linux-x64|x86_64-unknown-linux-gnu)
      echo "bun-linux-x64|x86_64-unknown-linux-gnu" ;;
    linux-arm64|aarch64-unknown-linux-gnu)
      echo "bun-linux-arm64|aarch64-unknown-linux-gnu" ;;
    "") echo "" ;;
    *) echo "" ;;
  esac
}

# 推断当前平台的 bun target + Rust target triple
detect_current_target() {
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
      case "$arch" in
        arm64|aarch64) echo "bun-windows-arm64|aarch64-pc-windows-msvc" ;;
        *)             echo "bun-windows-x64|x86_64-pc-windows-msvc" ;;
      esac
      ;;
    *) echo "" ;;
  esac
}

TARGET_ALIAS="${TARGET:-${1:-}}"
TARGET_PAIR="$(target_pair_from_alias "$TARGET_ALIAS")"
if [[ -z "$TARGET_PAIR" ]]; then
  TARGET_PAIR="$(detect_current_target)"
fi

read -r BUN_TARGET RUST_TRIPLE <<<"$(echo "$TARGET_PAIR" | tr '|' ' ')"
if [[ -z "${BUN_TARGET:-}" || -z "${RUST_TRIPLE:-}" ]]; then
  echo "❌ 无法识别目标平台：${TARGET_ALIAS:-当前平台}" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

repair_macos_binary() {
  local bin="$1"
  chmod +x "$bin"
  if command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.quarantine "$bin" 2>/dev/null || true
  fi
  if [[ "$(uname -s)" == "Darwin" ]] && command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "$bin" >/dev/null 2>&1 || true
  fi
}

windows_suffix_for_triple() {
  case "$1" in
    *windows*) echo ".exe" ;;
    *) echo "" ;;
  esac
}

compile_sidecar() {
  local bun_target="$1"
  local rust_triple="$2"
  local out="$3"
  echo "▶ bun build --compile --target=$bun_target"
  bun build --compile --target="$bun_target" \
    "$VENDOR_DIR/src/index.js" \
    --outfile "$out"
  repair_macos_binary "$out"
  echo "✓ $out"
}

find_ls() {
  local rust_triple="$1"
  local env_name=""
  local candidates=()

  case "$rust_triple" in
    aarch64-apple-darwin)
      env_name="WINDSURF_LS_ARM64_PATH"
      candidates+=(
        "/Applications/Windsurf.app/Contents/Resources/app/extensions/windsurf/bin/language_server_macos_arm"
        "/Applications/Windsurf.app/Contents/Resources/app/extensions/windsurf/bin/language_server_macos_arm64"
      )
      ;;
    x86_64-apple-darwin)
      env_name="WINDSURF_LS_X64_PATH"
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
      env_name="WINDSURF_LS_X64_PATH"
      candidates+=("C:/Program Files/Windsurf/resources/app/extensions/windsurf/bin/language_server_windows_x64.exe")
      ;;
    aarch64-pc-windows-msvc)
      env_name="WINDSURF_LS_ARM64_PATH"
      candidates+=(
        "C:/Program Files/Windsurf/resources/app/extensions/windsurf/bin/language_server_windows_arm64.exe"
        "C:/Program Files/Windsurf/resources/app/extensions/windsurf/bin/language_server_windows_arm.exe"
      )
      ;;
  esac

  if [[ -n "$env_name" ]]; then
    local env_path="${!env_name:-}"
    if [[ -n "$env_path" && -f "$env_path" ]]; then
      echo "$env_path"
      return 0
    fi
  fi

  if [[ -n "${WINDSURF_LS_PATH:-}" && -f "$WINDSURF_LS_PATH" ]]; then
    echo "$WINDSURF_LS_PATH"
    return 0
  fi

  local c
  for c in "${candidates[@]}"; do
    if [[ -f "$c" ]]; then
      echo "$c"
      return 0
    fi
  done

  cat >&2 <<EOF
❌ 没找到 $rust_triple 的 SuperAI runtime 二进制
   请安装对应架构的运行时应用后重试，或手动设置：
   - WINDSURF_LS_PATH：当前单架构目标
   - WINDSURF_LS_ARM64_PATH / WINDSURF_LS_X64_PATH：mac universal 或指定架构目标
   候选位置：
$(printf '   - %s\n' "${candidates[@]}")
EOF
  return 1
}

copy_ls() {
  local rust_triple="$1"
  local out="$2"
  local found
  found="$(find_ls "$rust_triple")"
  cp "$found" "$out"
  repair_macos_binary "$out"
  echo "✓ $out (来自 $found)"
}

if [[ "$RUST_TRIPLE" == "universal-apple-darwin" ]]; then
  if ! command -v lipo >/dev/null 2>&1; then
    echo "❌ 构建 mac universal sidecar 需要 lipo" >&2
    exit 1
  fi
  ARM_API="$OUTPUT_DIR/superai-api-aarch64-apple-darwin"
  X64_API="$OUTPUT_DIR/superai-api-x86_64-apple-darwin"
  UNI_API="$OUTPUT_DIR/superai-api-universal-apple-darwin"
  ARM_LS="$OUTPUT_DIR/language_server-aarch64-apple-darwin"
  X64_LS="$OUTPUT_DIR/language_server-x86_64-apple-darwin"
  UNI_LS="$OUTPUT_DIR/language_server-universal-apple-darwin"

  compile_sidecar "bun-darwin-arm64" "aarch64-apple-darwin" "$ARM_API"
  compile_sidecar "bun-darwin-x64" "x86_64-apple-darwin" "$X64_API"
  echo "▶ lipo -create superai-api"
  lipo -create "$ARM_API" "$X64_API" -output "$UNI_API"
  repair_macos_binary "$UNI_API"
  echo "✓ $UNI_API"

  copy_ls "aarch64-apple-darwin" "$ARM_LS"
  copy_ls "x86_64-apple-darwin" "$X64_LS"
  echo "▶ lipo -create language_server"
  lipo -create "$ARM_LS" "$X64_LS" -output "$UNI_LS"
  repair_macos_binary "$UNI_LS"
  echo "✓ $UNI_LS"
else
  WINDOWS_SUFFIX="$(windows_suffix_for_triple "$RUST_TRIPLE")"
  SIDECAR_OUT="$OUTPUT_DIR/superai-api-$RUST_TRIPLE$WINDOWS_SUFFIX"
  LS_OUT="$OUTPUT_DIR/language_server-$RUST_TRIPLE$WINDOWS_SUFFIX"

  compile_sidecar "$BUN_TARGET" "$RUST_TRIPLE" "$SIDECAR_OUT"
  copy_ls "$RUST_TRIPLE" "$LS_OUT"
fi
echo
echo "🟢 sidecar 构建完成。可以 npm run dev 了。"
