#!/usr/bin/env bash
# Builds the external test oracles of the AVS3 decoder (crates/avs3) from pinned upstream commits:
#   uavs3d (0e20d2c2, BSD-3-Clause, the decoder crates/avs3 is ported from)  -> uavs3dec, uavs3dec-10bit
#   uavs3e (e1ff0f37, BSD-4-Clause, the encoder that makes the test streams) -> uavs3enc, uavs3enc-10bit
# uavs3d is built C-only: its x86 SIMD paths differ from its own C code on extreme content (noise at
# high bit rates), and the C code is what crates/avs3 is ported from. uavs3e is used only as an
# external test encoder; it is never linked into or shipped with CFA4 (its licence has an
# advertising clause). Nothing here is linked or shipped. The AVS3 oracle tests skip without them.
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

build() { # dir bit-depth extra-c-flags
  cmake -S "$1" -B "$1/b$2" -DCMAKE_BUILD_TYPE=Release -DCOMPILE_10BIT="$([[ $2 == 10 ]] && echo 1 || echo 0)" \
    -DCMAKE_C_FLAGS="$3" >/dev/null
  cmake --build "$1/b$2" -j "$(nproc)" >/dev/null
}

fetch uavs3d "$uavs3d_commit" "$work/uavs3d"
fetch uavs3e "$uavs3e_commit" "$work/uavs3e"
# C paths only
sed -i '/^#define ENABLE_FUNCTION_X86 1$/d' "$work/uavs3d/source/decore/com_util.h"

build "$work/uavs3d" 8 ""
build "$work/uavs3d" 10 ""
weak="-Wno-error=incompatible-pointer-types -Wno-error=int-conversion -Wno-error=implicit-function-declaration -Wno-incompatible-pointer-types"
build "$work/uavs3e" 8 "$weak"
build "$work/uavs3e" 10 "$weak"

$sudo mkdir -p "$prefix/bin"
$sudo install -m 0755 "$work/uavs3d/b8/uavs3dec" "$prefix/bin/uavs3dec"
$sudo install -m 0755 "$work/uavs3d/b10/uavs3dec" "$prefix/bin/uavs3dec-10bit"
$sudo install -m 0755 "$work/uavs3e/b8/uavs3enc" "$prefix/bin/uavs3enc"
$sudo install -m 0755 "$work/uavs3e/b10/uavs3enc" "$prefix/bin/uavs3enc-10bit"
echo "AVS3 oracle: uavs3dec, uavs3enc, uavs3dec-10bit, uavs3enc-10bit in $prefix/bin"
