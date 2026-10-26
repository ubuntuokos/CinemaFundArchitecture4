#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the FFmpeg 9.0.2 that CFA4's supplementary FFmpeg backend links (ADR 0005), on Ubuntu
# Linux AMD64, as shared libraries in an isolated prefix that never replaces Ubuntu's FFmpeg.
#
#   scripts/dev/build-ffmpeg-9.0.2.sh              base build      → /opt/cfa4/ffmpeg-9.0.2
#   scripts/dev/build-ffmpeg-9.0.2.sh --restricted restricted build → /opt/cfa4/ffmpeg-9.0.2-restricted
#
# Licence: GPL-3 build only (--enable-gpl --enable-version3), never --enable-nonfree, so no
# libfdk-aac, libnpp or CUDA NVCC. NVIDIA encode/decode goes through the MIT nv-codec-headers and
# is loaded at run time; AMD AMF is left out (ADR 0003 §5: closed runtime; AMD encodes via VA-API).
#
# Ubuntu model (docs/licensing.md §7): the base build has no patent-restricted codecs. The
# restricted build is the same FFmpeg plus x264 and x265, for the `cfa4-restricted-extras`
# package. Both use the same library names; CFA4 binaries built with the `ffmpeg` feature search
# `…-restricted/lib` first and the base build second (apps/*/build.rs), so installing or removing
# the extras switches every FFmpeg library at once.
#
# Environment: CFA4_FFMPEG_PREFIX (base prefix), CFA4_FFMPEG_JOBS (make -j), CFA4_FFMPEG_SOURCE
# (`tarball`, the default, or `git` for the GitHub mirror when ffmpeg.org is not reachable).
set -euo pipefail

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 builds FFmpeg for Linux x86_64 (AMD64) only." >&2
  exit 2
fi

version=9.0.2
archive="ffmpeg-${version}.tar.xz"
source_url="https://ffmpeg.org/releases/${archive}"
sha256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
git_url="https://github.com/FFmpeg/FFmpeg"
git_tag="n${version}"
git_commit=946fcce07b6dcd0331c8cc609192aeff5e1924f8

restricted=0
for arg in "$@"; do
  case "$arg" in
    --restricted) restricted=1 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg (use --restricted or --help)" >&2; exit 2 ;;
  esac
done

base="${CFA4_FFMPEG_PREFIX:-/opt/cfa4/ffmpeg-${version}}"
prefix="$base"
(( restricted )) && prefix="${base}-restricted"
jobs="${CFA4_FFMPEG_JOBS:-$(nproc)}"

need=(nasm pkg-config)
for tool in "${need[@]}" curl tar make cc; do
  command -v "$tool" >/dev/null || { echo "missing build tool: $tool (sudo apt install build-essential nasm pkg-config curl)" >&2; exit 1; }
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
src="${tmp}/src"
mkdir -p "$src"
case "${CFA4_FFMPEG_SOURCE:-tarball}" in
  tarball)
    curl --fail --location --show-error --silent --retry 3 --proto '=https' --tlsv1.2 "$source_url" --output "${tmp}/${archive}"
    printf '%s  %s\n' "$sha256" "${tmp}/${archive}" | sha256sum --check --quiet
    tar -xJf "${tmp}/${archive}" -C "$src" --strip-components=1
    ;;
  git)
    git clone --quiet --depth 1 --branch "$git_tag" "$git_url" "$src"
    actual="$(git -C "$src" rev-parse HEAD)"
    if [[ "$actual" != "$git_commit" ]]; then
      echo "the ${git_tag} tag points to ${actual}, expected ${git_commit}; refusing to build" >&2
      exit 1
    fi
    ;;
  *) echo "CFA4_FFMPEG_SOURCE must be tarball or git" >&2; exit 2 ;;
esac

# Optional libraries: each is used when its development package is installed, and the build says
# which ones it found. Missing ones only remove that codec or accelerator.
has() { pkg-config --exists "$1"; }
flags=(--prefix="$prefix" --enable-gpl --enable-version3 --enable-shared --disable-static
  --disable-doc --disable-debug --disable-ffplay --disable-autodetect
  --enable-pthreads --enable-zlib --extra-ldflags="-Wl,--enable-new-dtags,-rpath,${prefix}/lib")
found=()
opt() { if has "$1"; then flags+=("$2"); found+=("$3"); fi; }
opt libva --enable-vaapi "VA-API (Intel, AMD, NVIDIA via nvidia-vaapi-driver)"
opt libdrm --enable-libdrm "libdrm"
opt ffnvcodec "--enable-ffnvcodec --enable-nvenc --enable-nvdec --enable-cuvid" "NVENC/NVDEC (nv-codec-headers, loaded at run time)"
opt vpl --enable-libvpl "Intel Quick Sync (oneVPL)"
opt dav1d --enable-libdav1d "dav1d (AV1)"
opt vpx --enable-libvpx "libvpx (VP8/VP9)"
opt opus --enable-libopus "Opus"
opt libwebp --enable-libwebp "WebP"
if [[ -e /usr/include/lame/lame.h ]]; then flags+=(--enable-libmp3lame); found+=("LAME (MP3)"); fi
# Vulkan Video needs headers at least as new as FFmpeg 9 expects; older ones are reported, not fatal.
if has vulkan && pkg-config --atleast-version=1.4.300 vulkan; then flags+=(--enable-vulkan); found+=("Vulkan Video"); else found+=("(no Vulkan Video: vulkan headers older than 1.4.300 or missing)"); fi
if (( restricted )); then
  has x264 || { echo "the restricted build needs libx264-dev" >&2; exit 1; }
  has x265 || { echo "the restricted build needs libx265-dev" >&2; exit 1; }
  flags+=(--enable-libx264 --enable-libx265)
  found+=("x264, x265 (patent-restricted: cfa4-restricted-extras only)")
fi
# flags with spaces were added as one word on purpose; split them now
read -r -a flags <<<"${flags[*]}"

( cd "$src" && ./configure "${flags[@]}" >"${tmp}/configure.log" ) || { tail -20 "${src}/ffbuild/config.log" >&2; exit 1; }
grep -q '^License: GPL version 3 or later' "${tmp}/configure.log" || { echo "configure did not produce a GPL-3 build" >&2; exit 1; }
make -C "$src" -j"$jobs" >"${tmp}/make.log" 2>&1 || { tail -30 "${tmp}/make.log" >&2; exit 1; }

install_cmd=(make -C "$src" install)
if [[ ! -w "$(dirname "$prefix")" && "$(id -u)" != 0 ]]; then
  install_cmd=(sudo "${install_cmd[@]}")
  sudo mkdir -p "$(dirname "$prefix")"
fi
"${install_cmd[@]}" >"${tmp}/install.log"

ffmpeg="${prefix}/bin/ffmpeg"
actual="$("$ffmpeg" -version | awk 'NR==1 {print $3}')"
[[ "$actual" == "$version" || "$actual" == "n$version" ]] || { echo "built ${actual}, expected ${version}" >&2; exit 1; }
if "$ffmpeg" -hide_banner -buildconf | grep -q -- '--enable-nonfree'; then
  echo "refusing: a non-free build cannot be distributed" >&2
  exit 1
fi
restricted_found="$("$ffmpeg" -hide_banner -encoders | awk '$2 == "libx264" || $2 == "libx265" {print $2}' | tr '\n' ' ')"
if (( ! restricted )) && [[ -n "$restricted_found" ]]; then
  echo "refusing: the base build contains patent-restricted encoders: ${restricted_found}" >&2
  exit 1
fi

echo "FFmpeg ${version} installed in ${prefix} (GPL-3 or later, shared libraries)."
printf '  %s\n' "${found[@]}"
echo "Hardware acceleration methods: $("$ffmpeg" -hide_banner -hwaccels | tail -n +2 | tr '\n' ' ')"
echo
echo "Build CFA4 against it:"
echo "  export PKG_CONFIG_PATH=${base}/lib/pkgconfig"
echo "  cargo build --release -p cfa4 -p cfa4-cli --features cfa4/ffmpeg,cfa4-cli/ffmpeg"
