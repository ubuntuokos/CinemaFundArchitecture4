#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds the libobs 33.0.1 (OBS Studio core) that CFA4's live studio runs in the separate
# cfa4-studio-host process (CFA4-LIVE-STUDIO-001 D1, D2), on Ubuntu Linux AMD64, as shared
# libraries in an isolated prefix that never replaces Ubuntu's libobs.
#
#   scripts/dev/build-libobs-33.0.1.sh                 → /opt/cfa4/libobs-33.0.1
#
# Pinned source: the OBS Studio 33.0.1 source release (with its submodules), checked against the
# SHA-256 below, or (CFA4_LIBOBS_SOURCE=git) the 33.0.1 tag, checked against its commit. Ubuntu's own
# package is older (26.04: 32.1.0, without the browser source) and is not used.
#
# What is built (S0): libobs and libobs-opengl only. No OBS frontend (Qt), no scripting, no core
# plugins yet: later phases add the plugins they need (S2 capture, S5 outputs, S7 encoders), and
# the browser source comes from WPE WebKit, not CEF (D3). Licence: libobs is GPL-2.0-or-later,
# combined with CFA4 under GPL-3.0-or-later; the corresponding source is the unmodified upstream
# release named in share/doc/libobs/BUILDINFO.
#
# FFmpeg: libobs 33 needs FFmpeg >= 8.0; it is built against CFA4's own FFmpeg 9.0.2
# (scripts/dev/build-ffmpeg-9.0.2.sh, ADR 0005), so there is one FFmpeg in a CFA4 installation.
#
# Environment: CFA4_LIBOBS_PREFIX (install prefix), CFA4_FFMPEG_PREFIX (default
# /opt/cfa4/ffmpeg-9.0.2), CFA4_LIBOBS_JOBS (parallel jobs), CFA4_LIBOBS_SOURCE (`tarball`, the
# default, or `git`).
#
# Build dependencies (Ubuntu): build-essential cmake ninja-build pkg-config extra-cmake-modules
#   libsimde-dev uthash-dev libjansson-dev zlib1g-dev uuid-dev libx11-dev libx11-xcb-dev
#   libxcb1-dev libxcb-xinput-dev libxcb-randr0-dev libxcb-xfixes0-dev libxcb-composite0-dev
#   libxcb-shm0-dev libglib2.0-dev libpulse-dev libwayland-dev libxkbcommon-dev libdrm-dev
#   libegl-dev libgl-dev libopengl-dev
set -euo pipefail

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 builds libobs for Linux x86_64 (AMD64) only." >&2
  exit 2
fi

version=33.0.1
archive="OBS-Studio-${version}-Sources.tar.gz"
source_url="https://github.com/obsproject/obs-studio/releases/download/${version}/${archive}"
sha256=c8d3a8f69a0f17970ac0a8f2a2462f1bc4dacd324f6da0c4b408d442d4798913
git_url="https://github.com/obsproject/obs-studio"
git_tag="${version}"
git_commit=65c320b46e1af5c7a555161aea6182cdf7a8c524

for arg in "$@"; do
  case "$arg" in
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg (use --help)" >&2; exit 2 ;;
  esac
done

prefix="${CFA4_LIBOBS_PREFIX:-/opt/cfa4/libobs-${version}}"
ffmpeg_prefix="${CFA4_FFMPEG_PREFIX:-/opt/cfa4/ffmpeg-9.0.2}"
jobs="${CFA4_LIBOBS_JOBS:-$(nproc)}"

for tool in cmake ninja pkg-config curl tar cc c++ sha256sum; do
  command -v "$tool" >/dev/null || { echo "missing build tool: $tool (see the dependency list in --help)" >&2; exit 1; }
done
if [[ ! -f "${ffmpeg_prefix}/lib/pkgconfig/libavcodec.pc" ]]; then
  echo "CFA4 FFmpeg 9.0.2 not found in ${ffmpeg_prefix}: run scripts/dev/build-ffmpeg-9.0.2.sh first (libobs 33 needs FFmpeg >= 8.0)" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
src="${tmp}/src"
mkdir -p "$src"
case "${CFA4_LIBOBS_SOURCE:-tarball}" in
  tarball)
    curl --fail --location --show-error --silent --retry 3 --proto '=https' --tlsv1.2 "$source_url" --output "${tmp}/${archive}"
    printf '%s  %s\n' "$sha256" "${tmp}/${archive}" | sha256sum --check --quiet
    tar -xzf "${tmp}/${archive}" -C "$src" --strip-components=1
    ;;
  git)
    git clone --quiet --depth 1 --branch "$git_tag" --recurse-submodules --shallow-submodules "$git_url" "$src"
    actual="$(git -C "$src" rev-parse HEAD)"
    if [[ "$actual" != "$git_commit" ]]; then
      echo "the ${git_tag} tag points to ${actual}, expected ${git_commit}; refusing to build" >&2
      exit 1
    fi
    ;;
  *) echo "CFA4_LIBOBS_SOURCE must be tarball or git" >&2; exit 2 ;;
esac

flags=(
  -G Ninja
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_INSTALL_PREFIX="$prefix"
  -DCMAKE_PREFIX_PATH="$ffmpeg_prefix"
  # OBS sets its own (empty) install RPATH on Linux; the linker flag keeps ours, so libobs finds
  # libobs-opengl next to it and CFA4's FFmpeg without LD_LIBRARY_PATH.
  -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--enable-new-dtags,-rpath,${prefix}/lib:${ffmpeg_prefix}/lib"
  -DCMAKE_COMPILE_WARNING_AS_ERROR=OFF
  -DENABLE_FRONTEND=OFF
  -DENABLE_SCRIPTING=OFF
  -DENABLE_CORE_MODULES=OFF
  -DENABLE_PULSEAUDIO=ON
  -DENABLE_WAYLAND=ON
  -DENABLE_RELOCATABLE=OFF
  -DOBS_VERSION_OVERRIDE="${version}"
)
PKG_CONFIG_PATH="${ffmpeg_prefix}/lib/pkgconfig${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}" \
  cmake -S "$src" -B "${tmp}/build" "${flags[@]}" >"${tmp}/configure.log" 2>&1 || { tail -40 "${tmp}/configure.log" >&2; exit 1; }
cmake --build "${tmp}/build" --parallel "$jobs" >"${tmp}/build.log" 2>&1 || { tail -40 "${tmp}/build.log" >&2; exit 1; }

install_cmd=(cmake --install "${tmp}/build")
if [[ ! -w "$(dirname "$prefix")" && "$(id -u)" != 0 ]]; then
  install_cmd=(sudo "${install_cmd[@]}")
  sudo mkdir -p "$(dirname "$prefix")"
fi
"${install_cmd[@]}" >"${tmp}/install.log"

# libobs keeps the ABI soname of OBS 30 (OBS_SOVERSION in cmake/linux/helpers.cmake).
lib="${prefix}/lib/libobs.so.30"
[[ -e "$lib" ]] || { echo "libobs was not installed as ${lib}" >&2; exit 1; }
if LC_ALL=C ldd "$lib" | grep -q 'not found'; then
  LC_ALL=C ldd "$lib" | grep 'not found' >&2
  echo "libobs does not find all its libraries (RUNPATH)" >&2
  exit 1
fi

# Licence, source and checksum information next to the libraries: packages that ship this build
# ship this directory with it (GPL: the corresponding source is the unmodified release named here).
doc="${tmp}/doc"
mkdir -p "$doc"
cp "$src"/COPYING "$doc/COPYING"
{
  echo "libobs ${version} (OBS Studio core), built for CFA4 by scripts/dev/build-libobs-33.0.1.sh"
  echo "(CFA4-LIVE-STUDIO-001 D1). Runs only in the cfa4-studio-host process (D2)."
  echo "Licence: GPL-2.0-or-later (libobs; some files ISC, MIT, BSD, LGPL-2.1-or-later), combined with"
  echo "CFA4 under GPL-3.0-or-later. Copyright (C) Lain Bailey and the OBS Studio contributors."
  case "${CFA4_LIBOBS_SOURCE:-tarball}" in
    tarball) echo "Source: ${source_url} (SHA-256 ${sha256}), unmodified." ;;
    git) echo "Source: ${git_url} tag ${git_tag}, commit ${git_commit}, unmodified." ;;
  esac
  echo "FFmpeg: ${ffmpeg_prefix} ($(PKG_CONFIG_PATH="${ffmpeg_prefix}/lib/pkgconfig" pkg-config --modversion libavcodec 2>/dev/null || echo unknown) libavcodec)"
  echo "Components: libobs, libobs-opengl (no frontend, no scripting, no core plugins, no browser source)"
  echo "CMake flags:"
  printf '  %s\n' "${flags[@]}"
} >"${doc}/BUILDINFO"
# Checksums of the installed libraries: the package build compares them with what it ships.
( cd "$prefix" && find lib -maxdepth 1 -type f -name '*.so*' -print0 | sort -z | xargs -0 sha256sum ) >"${doc}/SHA256SUMS"
docdir="${prefix}/share/doc/libobs"
if [[ "${install_cmd[0]}" == sudo ]]; then
  sudo mkdir -p "$docdir" && sudo cp "$doc"/* "$docdir/"
else
  mkdir -p "$docdir" && cp "$doc"/* "$docdir/"
fi

# The built version must be the pinned one.
probe="${tmp}/probe"
cat >"${probe}.c" <<'C'
#include <stdio.h>
#include <obs.h>
int main(void) { printf("%s\n", obs_get_version_string()); return 0; }
C
cc "${probe}.c" -I"${prefix}/include/obs" -L"${prefix}/lib" -lobs -Wl,-rpath,"${prefix}/lib" -Wl,-rpath-link,"${ffmpeg_prefix}/lib" -o "$probe"
actual="$("$probe")"
[[ "$actual" == "$version" ]] || { echo "built libobs ${actual}, expected ${version}" >&2; exit 1; }

echo "libobs ${version} installed in ${prefix}"
echo "  library: ${lib}"
echo "  graphics: $(ls "${prefix}"/lib/libobs-opengl.so* 2>/dev/null | head -1)"
echo "  licence and checksums: ${docdir}"
echo "Run the studio host with it: CFA4_LIBOBS_DIR=${prefix} cfa4-studio-host --backend libobs …"
