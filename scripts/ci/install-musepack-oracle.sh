#!/usr/bin/env bash
# Builds the external test oracle of the Musepack decoder (crates/musepack) from the pinned
# Musepack reference sources (musepack_src_r475, BSD-3-Clause): the SV8 encoder `mpcenc`, and
# `cfa4-mpc-oracle` (scripts/ci/musepack-oracle/cfa4-mpc-oracle.c), which decodes with libmpcdec
# and writes its floating-point samples unchanged. Nothing here is linked or shipped with CFA4.
# The Musepack oracle tests skip without them (unless CFA4_REQUIRE_ORACLES=1).
# Owner approval for building them locally and in CI: 2026-11-14.
#
# The decoder is built with -ffp-contract=off so the compiler does not fuse multiply-adds:
# CFA4's decoder keeps libmpcdec's float operation order, and the comparison is exact.
# -fcommon: the 2009 headers declare tables without `extern` (GCC 10 stopped merging those).
# -std=gnu17 pins the dialect (GCC 15 defaults to C23). mpcenc passes mismatched pointer types,
# which GCC 14 and later reject: -Wno-error=incompatible-pointer-types keeps them a warning.
set -euo pipefail
url=https://files.musepack.net/source/musepack_src_r475.tar.gz
sha256=a4b1742f997f83e1056142d556a8c20845ba764b70365ff9ccf2e3f81c427b2b
prefix="${1:-/usr/local}"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -sSfL "$url" -o "$work/src.tar.gz"
echo "$sha256  $work/src.tar.gz" | sha256sum -c --quiet -
tar xzf "$work/src.tar.gz" -C "$work"
s="$work/musepack_src_r475"
gcc -std=gnu17 -O2 -ffp-contract=off -fcommon -o "$work/cfa4-mpc-oracle" "$here/musepack-oracle/cfa4-mpc-oracle.c" \
  "$s"/libmpcdec/{huffman,mpc_decoder,mpc_reader,streaminfo,mpc_bits_reader,mpc_demux,requant,synth_filter}.c \
  "$s/common/crc32.c" -I"$s/include" -lm
gcc -std=gnu17 -O2 -DFAST_MATH -DCVD_FASTLOG -w -fcommon -Wno-error=incompatible-pointer-types -o "$work/mpcenc" \
  "$s"/mpcenc/{keyboard,mpcenc,pipeopen,stderr,wave_in,winmsg}.c "$s/common/tags.c" \
  "$s"/libmpcpsy/{ans,cvd,fft4g,fft_routines,profile,psy,psy_tab}.c "$s/common/fastmath.c" \
  "$s"/libmpcenc/{analy_filter,bitstream,encode_sv7,huffsv7,quant}.c "$s/common/crc32.c" \
  -I"$s/include" -I"$s/libmpcpsy" -I"$s/libmpcenc" -lm
for tool in cfa4-mpc-oracle mpcenc; do
  if [[ -w "$prefix/bin" ]]; then
    install -m 0755 "$work/$tool" "$prefix/bin/$tool"
  else
    sudo install -m 0755 "$work/$tool" "$prefix/bin/$tool"
  fi
done
echo "cfa4-mpc-oracle, mpcenc: musepack_src_r475"
