#!/usr/bin/env bash
# Builds the external test oracle of the LCEVC decoder (crates/lcevc): V-Nova's LCEVCdec 4.3.1
# (BSD-3-Clause-Clear, no patent licence granted; static CPU-pipeline libraries, no executables)
# from the pinned upstream tag, and links the driver crates/lcevc/tests/oracle/lcevc_oracle.c
# against it as `lcevc_oracle`. Nothing here is linked or shipped with CFA4. The LCEVC oracle test
# skips when the tool is missing.
set -euo pipefail
tag=4.3.1
commit=e295d8438ba554bc541fb269baa9cbd567b626a8
prefix="${1:-/usr/local}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sudo=""
if [[ ! -w "$prefix" ]]; then
  sudo="sudo"
fi
git -c advice.detachedHead=false clone -q --depth 1 --branch "$tag" https://github.com/v-novaltd/LCEVCdec "$work/src"
got="$(git -C "$work/src" rev-parse HEAD)"
[[ "$got" == "$commit" ]] || { echo "LCEVCdec $tag is $got, expected $commit" >&2; exit 1; }
cmake -S "$work/src" -B "$work/build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
  -DVN_SDK_EXECUTABLES=OFF -DVN_SDK_UNIT_TESTS=OFF -DVN_SDK_SAMPLE_SOURCE=OFF -DVN_SDK_PIPELINE_VULKAN=OFF \
  -DVN_SDK_DOCS=OFF -DVN_SDK_BENCHMARK=OFF -DCMAKE_INSTALL_PREFIX="$work/install" >/dev/null
cmake --build "$work/build" -j "$(nproc)" >/dev/null
cmake --install "$work/build" >/dev/null
lib="$work/install/lib"
[[ -d "$lib" ]] || lib="$work/install/lib64"
cc -O2 -I"$work/install/include" "$root/crates/lcevc/tests/oracle/lcevc_oracle.c" -o "$work/lcevc_oracle" \
  -Wl,--start-group "$lib"/liblcevc_dec_*.a -Wl,--end-group -lstdc++ -lpthread -lm -ldl
$sudo mkdir -p "$prefix/bin"
$sudo install -m 0755 "$work/lcevc_oracle" "$prefix/bin/lcevc_oracle"
echo "LCEVC oracle: LCEVCdec $tag in $prefix/bin/lcevc_oracle"
