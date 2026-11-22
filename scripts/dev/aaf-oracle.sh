#!/usr/bin/env bash
# Opens CFA4's AAF exports with the AAF SDK as a test oracle (issue #1509, docs/testing.md).
# The SDK (github.com/dneg/aaf) is used only to check our output: it is compiled in a copy outside
# the repository and none of its code or headers is used by CFA4.
#
#   AAF_SDK_SRC=~/dneg/aaf AAF_ORACLE_DIR=~/aaf-oracle scripts/dev/aaf-oracle.sh
#
# Builds the SDK once (marker file), writes our export in both sector sizes with the ignored
# `export_for_the_sdk_oracle` test, and runs the SDK's ComAAFInfo on each. Fails when the SDK cannot
# open a file, crashes, or reports no mobs. Needs a C++ toolchain and uuid-dev.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
SRC=${AAF_SDK_SRC:-$HOME/dneg/aaf}
W=${AAF_ORACLE_DIR:-$HOME/aaf-oracle}
SDK=$W/sdk
BIN=$SDK/AAFx86_64LinuxSDK/g++/bin
UUIDLIB=${UUIDLIB_PATH:-/usr/lib/x86_64-linux-gnu}
mkdir -p "$W/files"

if [[ ! -f "$W/.built" ]]; then
  [[ -d "$SRC/ref-impl" ]] || { echo "AAF_SDK_SRC=$SRC is not a checkout of github.com/dneg/aaf" >&2; exit 2; }
  rm -rf "$SDK"
  cp -r "$SRC" "$SDK"
  # Current compilers: no -Werror / -march=k8, and the dialect and flags the 2010 sources need.
  sed -i -e 's/-Werror//g' -e 's/-march=k8//g' "$SDK"/build/*.mk
  sed -i -e 's/^REL_FLAGS = .*/REL_FLAGS = -g -O0 -fno-lifetime-dse -fno-strict-aliasing -fno-delete-null-pointer-checks -fpermissive -std=gnu++98 -Wno-non-virtual-dtor -Wno-reorder -Wno-unused/' "$SDK/build/cdefs-g++.mk"
  (cd "$SDK" && make AAFTARGET=Release UUIDLIB_PATH="$UUIDLIB" -k -j"$(nproc)" >"$W/build.log" 2>&1 || true
   make AAFTARGET=Release UUIDLIB_PATH="$UUIDLIB" install >>"$W/build.log" 2>&1 || true)
  (cd "$SDK/examples/com-api/comClientTest" && make AAFTARGET=Release UUIDLIB_PATH="$UUIDLIB" >>"$W/build.log" 2>&1)
  [[ -x "$BIN/ComAAFInfo" ]] || { echo "ComAAFInfo was not built, see $W/build.log" >&2; exit 2; }
  touch "$W/.built"
fi

rm -f "$W"/files/*.aaf
(cd "$REPO" && CFA4_AAF_ORACLE_DIR="$W/files" cargo test -p cfa4-interchange --lib export_for_the_sdk_oracle -- --ignored)

status=0
for f in "$W"/files/*.aaf; do
  out=$(cd "$BIN" && LD_LIBRARY_PATH=. ./ComAAFInfo "$f" 2>&1) && rc=0 || rc=$?
  if [[ $rc -ne 0 ]] || ! grep -q "Number of Mobs" <<<"$out"; then
    echo "FAIL $(basename "$f") (exit $rc)"; echo "$out" | head -5; status=1
  else
    echo "ok   $(basename "$f"): $(grep 'Number of Mobs' <<<"$out")"
  fi
done
exit $status
