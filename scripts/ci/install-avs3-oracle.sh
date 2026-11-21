#!/usr/bin/env bash
# Builds the external test oracles of the AVS3 decoder (crates/avs3) from pinned upstream commits:
#   uavs3d (0e20d2c2, BSD-3-Clause, the decoder crates/avs3 is ported from)  -> uavs3d
#   uavs3e (e1ff0f37, BSD-4-Clause, the encoder that makes the test streams)  -> uavs3e
#   the same two built for 10-bit samples                                     -> uavs3d-10bit, uavs3e-10bit
# Plain cmake builds; uavs3d is built C-only (its x86 SIMD paths differ from its own C code on extreme
# content such as noise at high bit rates, and the C code is what crates/avs3 is ported from). Nothing here is linked or shipped with CFA4. The AVS3 oracle test
# skips when a tool is missing.
set -euo pipefail
uavs3d_commit=0e20d2c291853f196c68922a264bcd8471d75b68
uavs3e_commit=e1ff0f37a8d67814e1d650f6b49d495c49dde946
prefix="${1:-/usr/local}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sudo=""
if [[ ! -w "$prefix" ]]; then
  sudo="sudo"
fi

fetch() { # name commit dir
  git clone -q "https://github.com/uavs3/$1.git" "$3"
  git -C "$3" -c advice.detachedHead=false checkout -q "$2"
}

build() { # dir bit-depth(0|1 for COMPILE_10BIT)
  cmake -S "$1" -B "$1/b" -DCMAKE_BUILD_TYPE=Release -DCOMPILE_10BIT="$2" >/dev/null
  cmake --build "$1/b" -j "$(nproc)" >/dev/null
}

fetch uavs3d "$uavs3d_commit" "$work/uavs3d"
fetch uavs3e "$uavs3e_commit" "$work/uavs3e"
sed -i '/^#define ENABLE_FUNCTION_X86 1$/d' "$work/uavs3d/source/decore/com_util.h"
cp -a "$work/uavs3d" "$work/uavs3d-10bit"
cp -a "$work/uavs3e" "$work/uavs3e-10bit"
build "$work/uavs3d" 0
build "$work/uavs3e" 0
build "$work/uavs3d-10bit" 1
build "$work/uavs3e-10bit" 1

$sudo mkdir -p "$prefix/bin"
$sudo install -m 0755 "$work/uavs3d/b/uavs3dec" "$prefix/bin/uavs3d"
$sudo install -m 0755 "$work/uavs3d-10bit/b/uavs3dec" "$prefix/bin/uavs3d-10bit"
$sudo install -m 0755 "$work/uavs3e/b/uavs3enc" "$prefix/bin/uavs3e"
$sudo install -m 0755 "$work/uavs3e-10bit/b/uavs3enc" "$prefix/bin/uavs3e-10bit"
