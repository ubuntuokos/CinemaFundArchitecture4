#!/usr/bin/env bash
# Builds `cfa4-amr-oracle` (scripts/ci/amr-oracle/cfa4-amr-oracle.c), the external test oracle
# of the AMR decoders (crates/amr): opencore-amr 0.1.6 (AMR-NB encoder and decoder, AMR-WB
# decoder) and vo-amrwbenc 0.1.3 (AMR-WB encoder), Apache-2.0, built from the pinned upstream
# tags and linked statically into the helper. Nothing here is linked or shipped with CFA4. The
# AMR oracle tests skip when the helper is missing.
#
# Why not the distribution packages: Ubuntu builds opencore-amr with link-time optimisation.
# The reference code relies on signed overflow (`L_add` tests for it after the fact); once
# `pv_round` is inlined into `A_Refl`, GCC drops that saturation and the comfort noise moves by
# 1 LSB. The oracle is the plain build (-O2, no LTO, -fwrapv), which keeps the source's
# saturating arithmetic.
set -euo pipefail
nb_tag=v0.1.6
nb_commit=7dba8c32238418ce0b316a852b2224df586ca896
wb_tag=v0.1.3
wb_commit=080af7540562e4adfbd3312837fe5fc6771c4cd0
prefix="${1:-/usr/local}"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sudo apt-get install -y --no-install-recommends autoconf automake libtool >/dev/null

fetch() { # url tag commit dir
  git -c advice.detachedHead=false clone -q --depth 1 --branch "$2" "$1" "$4"
  local got
  got="$(git -C "$4" rev-parse HEAD)"
  [[ "$got" == "$3" ]] || { echo "$1 $2 is $got, expected $3" >&2; exit 1; }
}
build() { # dir
  (
    cd "$1"
    autoreconf -i >/dev/null 2>&1
    ./configure -q --prefix="$work/prefix" --disable-shared --enable-static \
      CFLAGS="-O2 -fwrapv" CXXFLAGS="-O2 -fwrapv"
    make -s -j "$(nproc)" >/dev/null
    make -s install >/dev/null
  )
}
fetch https://git.code.sf.net/p/opencore-amr/code "$nb_tag" "$nb_commit" "$work/opencore-amr"
fetch https://git.code.sf.net/p/opencore-amr/vo-amrwbenc "$wb_tag" "$wb_commit" "$work/vo-amrwbenc"
build "$work/opencore-amr"
build "$work/vo-amrwbenc"
gcc -O2 -o "$work/cfa4-amr-oracle" "$here/amr-oracle/cfa4-amr-oracle.c" -I"$work/prefix/include" \
  "$work/prefix/lib/libopencore-amrnb.a" "$work/prefix/lib/libopencore-amrwb.a" \
  "$work/prefix/lib/libvo-amrwbenc.a" -lstdc++ -lm
sudo install -m 0755 "$work/cfa4-amr-oracle" "$prefix/bin/cfa4-amr-oracle"
echo "cfa4-amr-oracle: opencore-amr $nb_tag, vo-amrwbenc $wb_tag"
