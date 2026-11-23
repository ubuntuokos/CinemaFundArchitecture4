#!/usr/bin/env bash
# Builds the external test oracles of the AVS2 decoder (crates/avs2), all GPL-2.0-or-later, from
# pinned upstream commits:
#   davs2 1.7 (b41cf117, the decoder crates/avs2 is ported from)      -> davs2
#   xavs2 1.4 (eae1e8b9, the encoder that makes the test streams)     -> xavs2
#   the same two built with 10-bit samples                            -> davs2-10bit, xavs2-10bit
# Upstream refuses 10-bit builds in configure and the encoder refuses the Main10 profile; the
# 10-bit builds lift those checks and fix the two pointer casts that then fail to compile.
# Nothing here is linked or shipped with CFA4. The AVS2 oracle test skips when a tool is missing.
set -euo pipefail
davs2_commit=b41cf117452e2d73d827f02d3e30aa20f1c721ac
xavs2_commit=eae1e8b9d12468059bdd7dee893508e470fa83d8
prefix="${1:-/usr/local}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sudo=""
if [[ ! -w "$prefix" ]]; then
  sudo="sudo"
fi

fetch() { # name commit dir
  git clone -q "https://github.com/pkuvcl/$1.git" "$3"
  git -C "$3" -c advice.detachedHead=false checkout -q "$2"
}

# GCC 14 and later make these C diagnostics errors by default; the pinned sources predate that.
relaxed="-Wno-error=incompatible-pointer-types -Wno-error=int-conversion -Wno-error=implicit-function-declaration"

build() { # dir bit-depth
  (cd "$1/build/linux" && ./configure --disable-asm --enable-pic --bit-depth="$2" --extra-cflags="$relaxed" >/dev/null \
    && make -j "$(nproc)" >/dev/null)
}

allow_10bit() { # dir
  sed -i '/BitDepth \$bit_depth not supported currently/{n;s/exit 1/:/}' "$1/build/linux/configure"
}

fetch davs2 "$davs2_commit" "$work/davs2"
fetch xavs2 "$xavs2_commit" "$work/xavs2"
cp -a "$work/davs2" "$work/davs2-10bit"
cp -a "$work/xavs2" "$work/xavs2-10bit"

allow_10bit "$work/davs2-10bit"
sed -i 's/(p) = (uint8_t \*)((intptr_t)/(p) = (__typeof__(p))((intptr_t)/' "$work/davs2-10bit/source/common/osdep.h"
sed -i 's/pic->planes\[\([012]\)\]  = frame->planes/pic->planes[\1]  = (uint8_t *)frame->planes/' \
  "$work/davs2-10bit/source/common/decoder.cc"
allow_10bit "$work/xavs2-10bit"
sed -i 's/if (param->profile_id != MAIN_PROFILE) {/if (param->profile_id != MAIN_PROFILE \&\& param->profile_id != MAIN10_PROFILE) {/' \
  "$work/xavs2-10bit/source/encoder/encoder.c"

build "$work/davs2" 8
build "$work/xavs2" 8
build "$work/davs2-10bit" 10
build "$work/xavs2-10bit" 10

$sudo mkdir -p "$prefix/bin"
for tool in davs2 xavs2; do
  $sudo install -m 0755 "$work/$tool/build/linux/$tool" "$prefix/bin/$tool"
  $sudo install -m 0755 "$work/$tool-10bit/build/linux/$tool" "$prefix/bin/$tool-10bit"
done
echo "AVS2 oracle: davs2, xavs2, davs2-10bit, xavs2-10bit in $prefix/bin"
