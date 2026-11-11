#!/usr/bin/env bash
# Builds the reference Monkey's Audio console encoder/decoder (`mac`) from the pinned
# BSD-3-Clause SDK, as the external test oracle of the APE decoder (crates/ape). It is never
# linked or shipped. The APE oracle tests skip when `mac` is missing.
set -euo pipefail
version=1346
sha256=17359c36099b7e1000da40306905f027088ac798fed1cb5e2405c33aaa030f83
prefix="${1:-/usr/local/bin}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 -o "$work/sdk.zip" "https://monkeysaudio.com/files/MAC_${version}_SDK.zip"
echo "$sha256  $work/sdk.zip" | sha256sum -c -
mkdir "$work/src"
(cd "$work/src" && unzip -q ../sdk.zip)
cmake -S "$work/src" -B "$work/build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED=OFF >/dev/null
cmake --build "$work/build" --target macutil -j "$(nproc)" >/dev/null
sudo install -m 0755 "$work/build/mac" "$prefix/mac"
# a static build: the binary must run on its own once the build directory is gone
banner="$("$prefix/mac" 2>&1 || true)"
grep -q "Monkey's Audio" <<<"$banner" || { echo "mac does not run: $banner" >&2; exit 1; }
head -1 <<<"$banner"
