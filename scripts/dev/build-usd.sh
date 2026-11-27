#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds CFA4's minimal OpenUSD core (oneTBB + OpenUSD, no Python/imaging/MaterialX/OpenVDB/GL) from the pinned
# sources fetched by scripts/dev/fetch-usd-source.sh (docs/plans/CFA4-OPENUSD-SOURCEBUILD-001-0.1.md, S3).
# Nothing in CFA4's cargo, test or CI steps calls it: a person (or an agent working for one) runs it.
#
#   scripts/dev/build-usd.sh [--src DIR] [--prefix DIR] [--work DIR] [--jobs N]
#   scripts/dev/build-usd.sh --check-repro [same options]    build twice and compare the file hashes (SB-G3)
#
#   --src DIR      where fetch-usd-source.sh put usd-VERSION and onetbb-VERSION (default: target/third-party)
#   --prefix DIR   install prefix (default: target/usd-install); holds BUILDINFO.json and SHA256SUMS too
#   --work DIR     scratch build directories (default: target/usd-build); removed when the build succeeds
#   --jobs N       parallel jobs (default: nproc)
#   --env FILE     read the pins from FILE instead of packaging/usd-version.env (tests)
#
# Determinism: LC_ALL=C, TZ=UTC, SOURCE_DATE_EPOCH = the pinned commit time, -ffile-prefix-map for the source
# and build directories, RUNPATH $ORIGIN (relocatable), `strip --strip-unneeded -D`, a sorted hash list.
# The unpacked sources are untrusted data: this script never runs anything from inside them except their CMake.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
envfile="$root/packaging/usd-version.env"
src="$root/target/third-party" prefix="$root/target/usd-install" work="$root/target/usd-build"
jobs="$(nproc)" repro=0
fail() { echo "error: $*" >&2; exit 1; }
usage() { sed -n '3,19p' "$0"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --src) src="${2:?--src needs a directory}"; shift 2 ;;
    --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
    --work) work="${2:?--work needs a directory}"; shift 2 ;;
    --jobs) jobs="${2:?--jobs needs a number}"; shift 2 ;;
    --env) envfile="${2:?--env needs a file}"; shift 2 ;;
    --check-repro) repro=1; shift ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[[ "$jobs" =~ ^[1-9][0-9]*$ ]] || fail "--jobs must be a positive number"
[[ -r "$envfile" ]] || fail "cannot read $envfile"
for tool in cmake ninja g++ strip sha256sum python3; do command -v "$tool" >/dev/null || fail "$tool is required"; done

pin() { local line; line="$(grep -E "^$1=" "$envfile" | tail -n 1 || true)"; [[ -n "$line" ]] || fail "$envfile has no $1"; printf '%s' "${line#*=}"; }
usd_ver="$(pin CFA4_USD_VERSION)" tbb_ver="$(pin CFA4_ONETBB_VERSION)" epoch="$(pin CFA4_USD_SOURCE_DATE_EPOCH)"
[[ "$epoch" =~ ^[0-9]+$ ]] || fail "CFA4_USD_SOURCE_DATE_EPOCH is not a number"
usd_src="$src/usd-$usd_ver" tbb_src="$src/onetbb-$tbb_ver"
[[ -d "$usd_src" && -d "$tbb_src" ]] || fail "missing $usd_src or $tbb_src; run scripts/dev/fetch-usd-source.sh usd --yes and onetbb --yes first"
usd_src="$(cd "$usd_src" && pwd)" tbb_src="$(cd "$tbb_src" && pwd)"

USD_OPTIONS=(
  -DPXR_ENABLE_PYTHON_SUPPORT=OFF -DPXR_BUILD_IMAGING=OFF -DPXR_BUILD_USD_IMAGING=OFF -DPXR_BUILD_USDVIEW=OFF
  -DPXR_BUILD_TESTS=OFF -DPXR_BUILD_EXAMPLES=OFF -DPXR_BUILD_TUTORIALS=OFF -DPXR_BUILD_DOCUMENTATION=OFF
  -DPXR_BUILD_USD_TOOLS=OFF -DPXR_BUILD_EXEC=OFF -DPXR_ENABLE_MATERIALX_SUPPORT=OFF -DPXR_ENABLE_OPENVDB_SUPPORT=OFF
  -DPXR_ENABLE_PTEX_SUPPORT=OFF -DPXR_ENABLE_OSL_SUPPORT=OFF -DPXR_ENABLE_GL_SUPPORT=OFF -DPXR_ENABLE_VULKAN_SUPPORT=OFF
)

build_once() { # prefix work
  local out="$1" wk="$2" flags
  [[ ! -e "$out" ]] || fail "$out exists; remove it first"
  rm -rf "$wk"; mkdir -p "$wk"
  export LC_ALL=C TZ=UTC SOURCE_DATE_EPOCH="$epoch"
  flags="-ffile-prefix-map=$usd_src=/usd -ffile-prefix-map=$tbb_src=/tbb -ffile-prefix-map=$wk=/build"
  local common=(-G Ninja -DCMAKE_BUILD_TYPE=Release "-DCMAKE_INSTALL_PREFIX=$out" "-DCMAKE_C_FLAGS=$flags" "-DCMAKE_CXX_FLAGS=$flags"
    '-DCMAKE_INSTALL_RPATH=$ORIGIN' -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON -DCMAKE_INSTALL_LIBDIR=lib)
  echo "== oneTBB $tbb_ver"
  cmake -S "$tbb_src" -B "$wk/tbb" "${common[@]}" -DTBB_TEST=OFF -DTBB_STRICT=OFF > "$wk/tbb-cfg.log" 2>&1 || { tail -20 "$wk/tbb-cfg.log" >&2; fail "oneTBB configure failed"; }
  cmake --build "$wk/tbb" -j "$jobs" > "$wk/tbb-build.log" 2>&1 || { tail -20 "$wk/tbb-build.log" >&2; fail "oneTBB build failed"; }
  cmake --install "$wk/tbb" > "$wk/tbb-install.log" 2>&1
  echo "== OpenUSD $usd_ver"
  cmake -S "$usd_src" -B "$wk/usd" "${common[@]}" "-DCMAKE_PREFIX_PATH=$out" "${USD_OPTIONS[@]}" > "$wk/usd-cfg.log" 2>&1 || { tail -20 "$wk/usd-cfg.log" >&2; fail "OpenUSD configure failed"; }
  cmake --build "$wk/usd" -j "$jobs" > "$wk/usd-build.log" 2>&1 || { tail -20 "$wk/usd-build.log" >&2; fail "OpenUSD build failed"; }
  cmake --install "$wk/usd" > "$wk/usd-install.log" 2>&1
  # Strip the shared libraries, then write the hash list and BUILDINFO (not part of the hashed set).
  find "$out" -type f -name '*.so*' ! -type l -exec strip --strip-unneeded -D {} +
  (cd "$out" && find . -type f ! -name SHA256SUMS ! -name BUILDINFO.json -print0 | LC_ALL=C sort -z | xargs -0 sha256sum > SHA256SUMS)
  python3 -I - "$out" "$envfile" "$epoch" "$flags" "$jobs" "${USD_OPTIONS[@]}" <<'PY'
import json, subprocess, sys
out, envfile, epoch, flags, jobs = sys.argv[1:6]
pins = dict(l.split("=", 1) for l in open(envfile).read().splitlines() if l and not l.startswith("#"))
def first(cmd):
    return subprocess.run(cmd, capture_output=True, text=True).stdout.splitlines()[0]
info = {
    "format": 1,
    "pins": {k: v for k, v in sorted(pins.items()) if k.startswith(("CFA4_USD_", "CFA4_ONETBB_"))},
    "source_date_epoch": int(epoch),
    "toolchain": {"g++": first(["g++", "--version"]), "cmake": first(["cmake", "--version"]), "ninja": first(["ninja", "--version"]), "strip": first(["strip", "--version"])},
    "compiler_flags_added": flags,
    "usd_options": sys.argv[6:],
}
open(out + "/BUILDINFO.json", "w").write(json.dumps(info, indent=2, sort_keys=True) + "\n")
PY
}

if [[ $repro -eq 0 ]]; then
  build_once "$prefix" "$work"
  rm -rf "$work"
  echo "OpenUSD $usd_ver + oneTBB $tbb_ver installed in $prefix ($(wc -l < "$prefix/SHA256SUMS") files); see $prefix/BUILDINFO.json and SHA256SUMS."
else
  # Same prefix and work directory for both builds (the install prefix is embedded in CMake config files).
  [[ ! -e "$prefix-first" ]] || fail "$prefix-first exists; remove it first"
  build_once "$prefix" "$work"
  rm -rf "$work"
  mv "$prefix" "$prefix-first"
  build_once "$prefix" "$work"
  rm -rf "$work"
  if diff -u "$prefix-first/SHA256SUMS" "$prefix/SHA256SUMS" > "$prefix-repro.diff"; then
    echo "SB-G3 holds: two builds have identical file hashes ($(wc -l < "$prefix/SHA256SUMS") files)."
    rm -f "$prefix-repro.diff"
    rm -rf "$prefix-first"
  else
    echo "SB-G3 FAILS: the builds differ, see $prefix-repro.diff (first build kept in $prefix-first)" >&2
    exit 1
  fi
fi
