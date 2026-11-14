#!/usr/bin/env bash
# Builds `cfa4-g729-oracle` (scripts/ci/g729-oracle/cfa4-g729-oracle.c), the external test oracle
# of the G.729 decoder (crates/g729): bcg729 1.1.2 (GPL-3.0-or-later), built from the pinned
# upstream tag and linked statically into the helper. Nothing here is linked or shipped with
# CFA4. The G.729 oracle tests skip when the helper is missing (unless CFA4_REQUIRE_ORACLES=1).
# Owner approval for building it locally and in CI: 2026-11-14.
set -euo pipefail
tag=1.1.2
commit=fb195811fe630415c9bf433c0c0bebde14314ff2
prefix="${1:-/usr/local}"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git -c advice.detachedHead=false clone -q --depth 1 --branch "$tag" \
  https://github.com/BelledonneCommunications/bcg729.git "$work/bcg729"
got="$(git -C "$work/bcg729" rev-parse HEAD)"
[[ "$got" == "$commit" ]] || { echo "bcg729 $tag is $got, expected $commit" >&2; exit 1; }
# The library sources compiled straight into the helper (no CMake install needed).
gcc -O2 -fwrapv -o "$work/cfa4-g729-oracle" "$here/g729-oracle/cfa4-g729-oracle.c" \
  "$work"/bcg729/src/*.c -I"$work/bcg729/include" -I"$work/bcg729/src" -lm
if [[ -w "$prefix/bin" ]]; then
  install -m 0755 "$work/cfa4-g729-oracle" "$prefix/bin/cfa4-g729-oracle"
else
  sudo install -m 0755 "$work/cfa4-g729-oracle" "$prefix/bin/cfa4-g729-oracle"
fi
echo "cfa4-g729-oracle: bcg729 $tag"
