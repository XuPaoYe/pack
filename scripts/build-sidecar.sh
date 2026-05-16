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
#   SUPERAI_RUNTIME_PATH=/path scripts/build-sidecar.sh
#   TARGET=darwin-arm64 scripts/build-sidecar.sh
#   TARGET=universal-apple-darwin scripts/build-sidecar.sh
#   TARGET=windows-x64 scripts/build-sidecar.sh
#   TARGET=windows-arm64 scripts/build-sidecar.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_SRC_DIR="$REPO_ROOT/vendor/superai-sidecar"
# scrub-vendor.mjs 会把原始 vendor 复制到这里，并替换掉用户可见的上游品牌字面量；
# bun --compile 实际读取的是这份副本。
SCRUBBED_DIR="$REPO_ROOT/.vendor-build/superai-sidecar"
VENDOR_DIR="$SCRUBBED_DIR"
OUTPUT_DIR="$REPO_ROOT/src-tauri/binaries"

if [[ ! -d "$VENDOR_SRC_DIR/src" ]]; then
  echo "❌ 未找到 vendor/superai-sidecar/src，先把上游代码 vendor 进来" >&2
  exit 1
fi

runtime_brand() {
  printf '\127\151\156\144\163\165\162\146'
}

runtime_slug() {
  printf '\167\151\156\144\163\165\162\146'
}

runtime_project_name() {
  printf '\127\151\156\144\163\165\162\146\101\120\111'
}

runtime_brand_name="$(runtime_brand)"
runtime_slug_name="$(runtime_slug)"
runtime_project="$(runtime_project_name)"
runtime_ext_path="resources/app/extensions/${runtime_slug_name}/bin"
runtime_ext_path_mac="Contents/Resources/app/extensions/${runtime_slug_name}/bin"

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
  if ! bun build --compile --target="$bun_target" \
    "$VENDOR_DIR/src/index.js" \
    --outfile "$out"; then
    if [[ "$bun_target" == "bun-windows-arm64" ]]; then
      echo "⚠️  bun-windows-arm64 编译失败，改用 x64 sidecar 兼容 Windows ARM64" >&2
      bun build --compile --target="bun-windows-x64" \
        "$VENDOR_DIR/src/index.js" \
        --outfile "$out"
    else
      return 1
    fi
  fi
  repair_macos_binary "$out"
  echo "✓ $out"
}

find_ls() {
  local rust_triple="$1"
  local env_name=""
  local candidates=()

  # 多 app bundle 路径：用户经常把第二个架构的运行时应用装到 -arm64 / -x64
  # 后缀，或者 ~/Applications；都纳入候选省掉手动设 env 的麻烦。
  local mac_app_bundles=(
    "/Applications/${runtime_brand_name}-arm64.app"
    "/Applications/${runtime_brand_name}-arm.app"
    "/Applications/${runtime_brand_name}-x64.app"
    "/Applications/${runtime_brand_name}-intel.app"
    "/Applications/${runtime_brand_name}.app"
    "$HOME/Applications/${runtime_brand_name}-arm64.app"
    "$HOME/Applications/${runtime_brand_name}-arm.app"
    "$HOME/Applications/${runtime_brand_name}-x64.app"
    "$HOME/Applications/${runtime_brand_name}-intel.app"
    "$HOME/Applications/${runtime_brand_name}.app"
  )

  local win_install_dirs=(
    "C:/Program Files/${runtime_brand_name}"
    "C:/Program Files (x86)/${runtime_brand_name}"
    "$HOME/AppData/Local/Programs/${runtime_brand_name}"
  )

  case "$rust_triple" in
    aarch64-apple-darwin)
      env_name="SUPERAI_RUNTIME_ARM64_PATH"
      local app
      for app in "${mac_app_bundles[@]}"; do
        candidates+=(
          "$app/${runtime_ext_path_mac}/language_server_macos_arm"
          "$app/${runtime_ext_path_mac}/language_server_macos_arm64"
        )
      done
      ;;
    x86_64-apple-darwin)
      env_name="SUPERAI_RUNTIME_X64_PATH"
      local app
      for app in "${mac_app_bundles[@]}"; do
        candidates+=(
          "$app/${runtime_ext_path_mac}/language_server_macos_x64"
        )
      done
      ;;
    x86_64-unknown-linux-gnu)
      candidates+=("/opt/${runtime_slug_name}/language_server_linux_x64")
      ;;
    aarch64-unknown-linux-gnu)
      candidates+=("/opt/${runtime_slug_name}/language_server_linux_arm")
      ;;
    x86_64-pc-windows-msvc)
      env_name="SUPERAI_RUNTIME_X64_PATH"
      local d
      for d in "${win_install_dirs[@]}"; do
        candidates+=("$d/${runtime_ext_path}/language_server_windows_x64.exe")
      done
      ;;
    aarch64-pc-windows-msvc)
      env_name="SUPERAI_RUNTIME_ARM64_PATH"
      local d
      for d in "${win_install_dirs[@]}"; do
        candidates+=(
          "$d/${runtime_ext_path}/language_server_windows_arm64.exe"
          "$d/${runtime_ext_path}/language_server_windows_arm.exe"
        )
      done
      ;;
  esac

  if [[ -n "$env_name" ]]; then
    local env_path="${!env_name:-}"
    if [[ -n "$env_path" && -f "$env_path" ]]; then
      echo "$env_path"
      return 0
    fi
  fi

  if [[ -n "${SUPERAI_RUNTIME_PATH:-}" && -f "$SUPERAI_RUNTIME_PATH" ]]; then
    echo "$SUPERAI_RUNTIME_PATH"
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
   - SUPERAI_RUNTIME_PATH：当前单架构目标
   - SUPERAI_RUNTIME_ARM64_PATH / SUPERAI_RUNTIME_X64_PATH：mac universal 或指定架构目标
   候选位置：
$(printf '   - %s\n' "${candidates[@]}")
EOF
  return 1
}

# LS 二进制是平台特定原生码（x64/arm64 + macos/linux/windows 互不通用）。
# 自动获取来源按优先级：
#   1) 本地运行时安装（find_ls 已搜过 mac/win/linux 的多个候选目录）
#   2) 上游 GitHub Release
#      —— 仅 mac/linux 资产
#   3) 上游官方 archive
#      —— Windows zip / Linux tar.gz / mac dmg / mac zip 都拿得到，需要解压
# 下载产物缓存到 .vendor-build/ls-cache/，避免重复拉 ~150MB。
LS_CACHE_DIR="$REPO_ROOT/.vendor-build/ls-cache"
UPSTREAM_GH_PRIMARY="https://github.com/dwgx/${runtime_project}/releases/latest/download"
UPSTREAM_GH_FALLBACK="https://github.com/CaiJingLong/${runtime_slug_name}-linux-server-release/releases/latest/download"
RUNTIME_RELEASES_PAGE="https://${runtime_slug_name}.com/editor/releases"

# 上游 Github release 直接发的扁平资产（仅 macOS/Linux）。
upstream_asset_for_triple() {
  case "$1" in
    aarch64-apple-darwin)        echo "language_server_macos_arm" ;;
    x86_64-apple-darwin)         echo "language_server_macos_x64" ;;
    x86_64-unknown-linux-gnu)    echo "language_server_linux_x64" ;;
    aarch64-unknown-linux-gnu)   echo "language_server_linux_arm" ;;
    *) echo "" ;;
  esac
}

download_ls_from_github() {
  local rust_triple="$1"
  local asset
  asset="$(upstream_asset_for_triple "$rust_triple")"
  [[ -z "$asset" ]] && return 1

  local cached="$LS_CACHE_DIR/$asset"
  if [[ -f "$cached" ]]; then
    echo "$cached"
    return 0
  fi

  local tmp="${cached}.partial.$$"
  for base in "$UPSTREAM_GH_PRIMARY" "$UPSTREAM_GH_FALLBACK"; do
    local url="$base/$asset"
    echo "▶ curl $url" >&2
    if curl -fL --progress-bar -o "$tmp" "$url" 2>/dev/null; then
      mv -f "$tmp" "$cached"
      chmod +x "$cached"
      echo "$cached"
      return 0
    fi
    rm -f "$tmp"
  done
  return 1
}

# 上游官方 release 页里的 archive 直链（zip / tar.gz / dmg），
# 解压后 LS 在运行时扩展目录下。
# 我们对每个 rust_triple 关心的：archive 类型路径 + 内层 LS 文件名 + 缓存名。
release_archive_descriptor() {
  case "$1" in
    x86_64-pc-windows-msvc)
      echo "win32-x64-archive|.zip|${runtime_ext_path}/language_server_windows_x64.exe|language_server_windows_x64.exe"
      ;;
    aarch64-pc-windows-msvc)
      # 上游 archive 内层文件名是 _arm.exe（与 macOS arm 同样的简写习惯），
      # 不是 _arm64.exe；曾因这里写错导致 unzip 静默失败。
      echo "win32-arm64-archive|.zip|${runtime_ext_path}/language_server_windows_arm.exe|language_server_windows_arm.exe"
      ;;
    x86_64-apple-darwin)
      # mac dmg 复杂，优先用 GitHub release；这里给个补救路径，用 zip archive。
      echo "darwin-x64|.zip|${runtime_brand_name}.app/${runtime_ext_path_mac}/language_server_macos_x64|language_server_macos_x64"
      ;;
    aarch64-apple-darwin)
      echo "darwin-arm64|.zip|${runtime_brand_name}.app/${runtime_ext_path_mac}/language_server_macos_arm|language_server_macos_arm"
      ;;
    x86_64-unknown-linux-gnu)
      echo "linux-x64|.tar.gz|${runtime_brand_name}/${runtime_ext_path}/language_server_linux_x64|language_server_linux_x64"
      ;;
    *) echo "" ;;
  esac
}

# 抓 release 页第一条匹配前缀 + 后缀的 URL（按 stable 通道排序，第一条即最新）。
fetch_release_url() {
  local archive_path="$1"  # e.g. win32-x64-archive
  local extension="$2"     # e.g. .zip
  local cache="$LS_CACHE_DIR/.releases-page.html"
  mkdir -p "$LS_CACHE_DIR"
  if [[ ! -f "$cache" ]] || [[ $(($(date +%s) - $(stat -f %m "$cache" 2>/dev/null || echo 0))) -gt 3600 ]]; then
    if ! curl -fsSL "$RUNTIME_RELEASES_PAGE" -o "$cache.tmp"; then
      rm -f "$cache.tmp"
      return 1
    fi
    mv -f "$cache.tmp" "$cache"
  fi
  # release 页面里 stable 链接和 next 链接都有，优先 stable。
  local pattern="https://${runtime_slug_name}-stable\\.codeiumdata\\.com/${archive_path}/stable/[^\" ]+${extension//./\\.}"
  grep -oE "$pattern" "$cache" | head -1
}

extract_ls_from_archive() {
  local archive="$1"        # 下载下来的 zip / tar.gz
  local member="$2"         # archive 内 LS 路径
  local out_name="$3"       # 抽出后存到 LS_CACHE_DIR 的文件名
  local out="$LS_CACHE_DIR/$out_name"

  case "$archive" in
    *.zip)
      # unzip -p 直接管到 stdout
      if ! unzip -p "$archive" "$member" >"$out.partial" 2>/dev/null; then
        rm -f "$out.partial"
        return 1
      fi
      ;;
    *.tar.gz|*.tgz)
      if ! tar -xzf "$archive" -O "$member" >"$out.partial" 2>/dev/null; then
        rm -f "$out.partial"
        return 1
      fi
      ;;
    *) return 1 ;;
  esac

  if [[ ! -s "$out.partial" ]]; then
    rm -f "$out.partial"
    return 1
  fi
  mv -f "$out.partial" "$out"
  chmod +x "$out"
  echo "$out"
}

download_ls_from_release_archive() {
  local rust_triple="$1"
  local desc
  desc="$(release_archive_descriptor "$rust_triple")"
  [[ -z "$desc" ]] && return 1

  local archive_path extension member cache_name
  IFS='|' read -r archive_path extension member cache_name <<<"$desc"

  local cached="$LS_CACHE_DIR/$cache_name"
  if [[ -f "$cached" ]]; then
    echo "$cached"
    return 0
  fi

  local url
  url="$(fetch_release_url "$archive_path" "$extension")"
  if [[ -z "$url" ]]; then
    return 1
  fi

  local archive="$LS_CACHE_DIR/$(basename "$url")"
  if [[ ! -f "$archive" ]]; then
    echo "▶ curl $url" >&2
    if ! curl -fL --progress-bar -o "$archive.partial" "$url"; then
      rm -f "$archive.partial"
      return 1
    fi
    mv -f "$archive.partial" "$archive"
  fi

  extract_ls_from_archive "$archive" "$member" "$cache_name" || return 1
}

download_ls() {
  local rust_triple="$1"
  if ! command -v curl >/dev/null 2>&1; then
    return 1
  fi
  mkdir -p "$LS_CACHE_DIR"

  # 1) GitHub release 直发资产：mac / linux 极快（小，~150MB 单文件）
  if found="$(download_ls_from_github "$rust_triple")"; then
    echo "$found"
    return 0
  fi
  # 2) 上游官方 archive：Windows / 兜底 mac+linux
  if found="$(download_ls_from_release_archive "$rust_triple")"; then
    echo "$found"
    return 0
  fi
  return 1
}

copy_ls() {
  local rust_triple="$1"
  local out="$2"
  local found=""
  if found="$(find_ls "$rust_triple" 2>/dev/null)"; then
    :
  else
    echo "▶ 本地未找到 $rust_triple 的 LS，尝试从上游 release 下载..." >&2
    if ! found="$(download_ls "$rust_triple")"; then
      # 触发 find_ls 的清晰报错（候选列表）
      find_ls "$rust_triple" >/dev/null
      return 1
    fi
  fi
  cp "$found" "$out"
  repair_macos_binary "$out"
  echo "✓ $out (来自本地运行时或缓存)"
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
