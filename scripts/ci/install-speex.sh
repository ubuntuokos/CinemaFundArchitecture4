#!/usr/bin/env bash
# Builds the reference Speex tools (`speexenc`, `speexdec`, libspeex 1.2.1) from the pinned
# upstream tag, as the external test oracle of the Speex decoder (crates/speex). They are never
# linked or shipped. The Speex oracle tests skip when the tools are missing.
#
# Why not the distribution package: Ubuntu builds libspeex for x86-64-v3, where the compiler
# fuses multiplies and adds (FMA) and the float decoder's output moves by a few LSB. The
# reference is the portable float build: SSE (as upstream's configure enables on x86-64), no
# FMA contraction.
set -euo pipefail
tag=Speex-1.2.1
commit=5dceaaf3e23ee7fd17c80cb5f02a838fd6c18e01
prefix="${1:-/usr/local}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sudo apt-get install -y --no-install-recommends autoconf automake libtool libogg-dev >/dev/null
git clone -q --depth 1 --branch "$tag" https://github.com/xiph/speex.git "$work/src"
got="$(git -C "$work/src" rev-parse HEAD)"
[[ "$got" == "$commit" ]] || { echo "speex $tag is $got, expected $commit" >&2; exit 1; }
(
  cd "$work/src"
  ./autogen.sh >/dev/null 2>&1
  ./configure -q --prefix="$prefix" --disable-shared --enable-static CFLAGS="-O2 -ffp-contract=off"
  grep -q '#define _USE_SSE' config.h || { echo "speex: SSE not enabled" >&2; exit 1; }
  make -s -j "$(nproc)" >/dev/null
  sudo make -s install >/dev/null
)
"$prefix/bin/speexdec" --version 2>&1 | head -1
