#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the FFmpeg that CFA4's supplementary FFmpeg backend links (ADR 0005; the version, source
# SHA-256 and tag commit are pinned in packaging/ffmpeg-version.env), on Ubuntu Linux AMD64, as
# shared libraries in an isolated prefix that never replaces Ubuntu's FFmpeg.
#
#   scripts/dev/build-ffmpeg.sh                    base build      → /opt/cfa4/ffmpeg-VERSION
#   scripts/dev/build-ffmpeg.sh --restricted       restricted build → /opt/cfa4/ffmpeg-VERSION-restricted
#
# Licence: GPL-3 build only (--enable-gpl --enable-version3), never --enable-nonfree, so no
# libfdk-aac, libnpp or CUDA NVCC. NVIDIA encode/decode goes through the MIT nv-codec-headers and
# is loaded at run time; AMD AMF likewise: MIT SDK headers at build time, AMD's driver runtime loaded
# at run time (CFA4-FFMPEG-9.1-001 D2; ADR 0003 §5), next to the VA-API path.
#
# Ubuntu model (docs/licensing.md §7): the base build has no patent-restricted codecs. The
# restricted build is the same FFmpeg plus x264 and x265, for the `cfa4-restricted-extras`
# package. Both use the same library names; CFA4 binaries built with the `ffmpeg` feature search
# `…-restricted/lib` first and the base build second (apps/*/build.rs), so installing or removing
# the extras switches every FFmpeg library at once.
#
# Environment: CFA4_FFMPEG_PREFIX (base prefix), CFA4_FFMPEG_JOBS (make -j), CFA4_FFMPEG_SOURCE
# (`tarball`, the default, or `git` for the GitHub mirror when ffmpeg.org is not reachable),
# CFA4_FFMPEG_FROM_SOURCE=1 (compile even where scripts/dev/fetch-ffmpeg.sh has a prebuilt one).
set -euo pipefail

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 builds FFmpeg for Linux x86_64 (AMD64) only." >&2
  exit 2
fi

# shellcheck source=packaging/ffmpeg-version.env
. "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/packaging/ffmpeg-version.env"
version="$CFA4_FFMPEG_VERSION"
archive="ffmpeg-${version}.tar.xz"
source_url="https://ffmpeg.org/releases/${archive}"
sha256="$CFA4_FFMPEG_SOURCE_SHA256"
git_url="https://github.com/FFmpeg/FFmpeg"
git_tag="n${version}"
git_commit="$CFA4_FFMPEG_GIT_COMMIT"

restricted=0
for arg in "$@"; do
  case "$arg" in
    --restricted) restricted=1 ;;
    -h|--help) sed -n '2,23p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg (use --restricted or --help)" >&2; exit 2 ;;
  esac
done

base="${CFA4_FFMPEG_PREFIX:-/opt/cfa4/ffmpeg-${version}}"
prefix="$base"
(( restricted )) && prefix="${base}-restricted"
jobs="${CFA4_FFMPEG_JOBS:-$(nproc)}"

# The base build in its default prefix is published prebuilt (scripts/dev/fetch-ffmpeg.sh,
# GitHub prerelease, SHA-256 pinned): install that instead of compiling, unless
# CFA4_FFMPEG_FROM_SOURCE=1 (the prebuilt workflow itself) or the host has no prebuilt archive.
if (( ! restricted )) && [[ "$prefix" == "/opt/cfa4/ffmpeg-${version}" && "${CFA4_FFMPEG_FROM_SOURCE:-0}" != 1 ]]; then
  status=0
  bash "$(dirname "${BASH_SOURCE[0]}")/fetch-ffmpeg.sh" || status=$?
  if (( status == 0 )); then
    exit 0
  elif (( status != 3 )); then
    echo "the prebuilt FFmpeg could not be installed; set CFA4_FFMPEG_FROM_SOURCE=1 to compile it" >&2
    exit "$status"
  fi
  echo "no prebuilt FFmpeg ${version} for this host; building from source" >&2
fi

# glslc (shaderc) compiles FFmpeg's Vulkan shaders (APV/FFv1/ProRes Vulkan encoders and hwaccels): a
# required build tool (CFA4-FFMPEG-9.1-001 D9, R12), never left out silently.
need=(nasm pkg-config glslc git)
for tool in "${need[@]}" curl tar make cc; do
  command -v "$tool" >/dev/null || { echo "missing build tool: $tool (sudo apt install build-essential nasm pkg-config curl git glslc)" >&2; exit 1; }
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

# AMD AMF SDK headers (MIT; plan D2): not packaged in Ubuntu 26.04, so the pinned tag of the SDK
# repository, checked against its commit. Headers only; the AMF runtime ships with AMD's driver and
# FFmpeg loads it at run time (without it the VA-API, Vulkan or CPU path runs).
amf_url="https://github.com/GPUOpen-LibrariesAndSDKs/AMF"
amf="${tmp}/amf"
git clone --quiet --depth 1 --filter=blob:none --sparse --branch "$CFA4_AMF_HEADERS_TAG" "$amf_url" "$amf"
git -C "$amf" sparse-checkout set amf/public/include >/dev/null
actual="$(git -C "$amf" rev-parse HEAD)"
if [[ "$actual" != "$CFA4_AMF_HEADERS_COMMIT" ]]; then
  echo "the AMF ${CFA4_AMF_HEADERS_TAG} tag points to ${actual}, expected ${CFA4_AMF_HEADERS_COMMIT}; refusing to build" >&2
  exit 1
fi
mkdir -p "${tmp}/amf-include"
cp -R "${amf}/amf/public/include" "${tmp}/amf-include/AMF"

# Optional libraries: each is used when its development package is installed, and the build says
# which ones it found. Missing ones only remove that codec or accelerator.
has() { pkg-config --exists "$1"; }
flags=(--prefix="$prefix" --enable-gpl --enable-version3 --enable-shared --disable-static
  --disable-doc --disable-debug --disable-ffplay --disable-autodetect
  --enable-pthreads --enable-zlib --extra-ldflags="-Wl,--enable-new-dtags,-rpath,${prefix}/lib"
  --enable-amf --extra-cflags=-I${tmp}/amf-include)
found+=("AMD AMF (headers ${CFA4_AMF_HEADERS_TAG}; runtime from AMD's driver)")
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
# Vulkan needs headers of at least 1.4.317 (FFmpeg's vulkan_1_4 check; Ubuntu 26.04 has them); older
# ones are reported, not fatal (development containers). Its shaders are compiled with glslc.
vulkan=0
if has vulkan && pkg-config --atleast-version=1.4.317 vulkan; then
  vulkan=1
  flags+=(--enable-vulkan --glslc=glslc); found+=("Vulkan Video and Vulkan encoders (APV, FFv1, ProRes; shaders by glslc)")
else
  found+=("(no Vulkan: vulkan headers older than 1.4.317 or missing)")
fi
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

# Licence and source information next to the libraries: packaging/linux/stage-ffmpeg.sh ships this
# directory with every CFA4 package that contains this build (GPL-3: the corresponding source is
# the unmodified upstream release named here, configured with the flags below).
doc="${tmp}/doc"
mkdir -p "$doc"
cp "$src"/LICENSE.md "$src"/COPYING.* "$doc/"
git -C "$amf" show HEAD:LICENSE.txt >"${doc}/AMF-LICENSE.txt"
{
  echo "FFmpeg ${version}, built for CFA4 by scripts/dev/build-ffmpeg.sh (ADR 0005)."
  echo "Licence: GPL version 3 or later (--enable-gpl --enable-version3, never --enable-nonfree)."
  case "${CFA4_FFMPEG_SOURCE:-tarball}" in
    tarball) echo "Source: ${source_url} (SHA-256 ${sha256}), unmodified." ;;
    git) echo "Source: ${git_url} tag ${git_tag}, commit ${git_commit}, unmodified." ;;
  esac
  echo "AMF headers: ${amf_url} tag ${CFA4_AMF_HEADERS_TAG}, commit ${CFA4_AMF_HEADERS_COMMIT} (MIT, AMF-LICENSE.txt)."
  echo "SPIR-V compiler: $( (( vulkan )) && glslc --version 2>/dev/null | head -1 || echo 'not used (no Vulkan)')"
  echo "Build: $( (( restricted )) && echo 'restricted (x264, x265; cfa4-restricted-extras)' || echo 'base (no patent-restricted codecs)')"
  echo "configure flags:"
  printf '  %s\n' "${flags[@]}"
} >"${doc}/BUILDINFO"
docdir="${prefix}/share/doc/ffmpeg"
if [[ "${install_cmd[0]}" == sudo ]]; then
  sudo mkdir -p "$docdir" && sudo cp "$doc"/* "$docdir/"
else
  mkdir -p "$docdir" && cp "$doc"/* "$docdir/"
fi

ffmpeg="${prefix}/bin/ffmpeg"
actual="$("$ffmpeg" -version | awk 'NR==1 {print $3}')"
[[ "$actual" == "$version" || "$actual" == "n$version" ]] || { echo "built ${actual}, expected ${version}" >&2; exit 1; }
if "$ffmpeg" -hide_banner -buildconf | grep -q -- '--enable-nonfree'; then
  echo "refusing: a non-free build cannot be distributed" >&2
  exit 1
fi
# R12: with Vulkan the SPIR-V shaders must have been built (glslc), so the APV Vulkan encoder exists.
if (( vulkan )) && ! "$ffmpeg" -hide_banner -encoders | awk '{print $2}' | grep -qx apv_vulkan; then
  echo "refusing: Vulkan is enabled but the apv_vulkan encoder is missing (glslc shader build failed?)" >&2
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
