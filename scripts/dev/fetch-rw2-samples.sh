#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Downloads the public Panasonic RW2 test samples of the FFmpeg RW2 tests (F2d,
# docs/plans/CFA4-FFMPEG-9.1-001.md; crates/ffmpeg/src/rw2.rs) into target/fixtures/rw2 (or DIR).
# Every file comes from the raw.pixls.us archive under the CC0 1.0 public domain dedication (the
# licence was checked per file), and is checked against the SHA-256 pinned below. The samples are
# never committed (docs/testing.md: no media in the repository).
#
#   scripts/dev/fetch-rw2-samples.sh [DIR]
#
# The tests read CFA4_RW2_SAMPLES (default: target/fixtures/rw2) and skip when a sample is missing
# (CFA4_REQUIRE_ORACLES=1 makes that a failure).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dir="${1:-${CFA4_RW2_SAMPLES:-$root/target/fixtures/rw2}}"
mkdir -p "$dir"
command -v curl >/dev/null || { echo "missing tool: curl" >&2; exit 1; }

# file name | raw.pixls.us URL | SHA-256
samples=(
  # Panasonic DC-GH6, 16-bit raw (RawFormat 8): decoded by FFmpeg
  "DC-GH6.rw2|https://raw.pixls.us/getfile.php/5879/nice/Panasonic%20-%20DC-GH6%20-%201:1.RW2|69da3d4ad6ea3baee84801c59402b12e92378192f272b03eb6cc3c3bcbf38413"
  # Panasonic DC-S5M2, 12-bit mode, 3:2: decoded by FFmpeg
  "DC-S5M2.rw2|https://raw.pixls.us/getfile.php/7792/nice/Panasonic%20-%20DC-S5M2%20-%2012bit%20%283:2%29.RW2|7480e44ed8557b71b1618e2d91c0f3eea58b60cb5a9196d2a1a552cfa509a1d2"
  # Panasonic DMC-LX7 (RawFormat 4): FFmpeg does not decode it; the tests expect a clear error
  "DMC-LX7.rw2|https://raw.pixls.us/getfile.php/7008/nice/Panasonic%20-%20DMC-LX7%20-%201:1.RW2|d142a23aca836053ed53e9ce3cb3ed2d434541d734d71a94a6eefadcd08bd31b"
)

for entry in "${samples[@]}"; do
  IFS='|' read -r name url sha256 <<<"$entry"
  out="$dir/$name"
  if [[ -f "$out" ]] && printf '%s  %s\n' "$sha256" "$out" | sha256sum --check --quiet --status; then
    echo "$name: present"
    continue
  fi
  part="$out.part.$$"
  trap 'rm -f "$part"' EXIT
  curl --fail --location --show-error --silent --retry 5 --retry-all-errors --retry-delay 5 \
    --proto '=https' --tlsv1.2 "$url" --output "$part"
  if ! printf '%s  %s\n' "$sha256" "$part" | sha256sum --check --quiet; then
    echo "$name: SHA-256 mismatch; refusing it" >&2
    rm -f "$part"
    exit 1
  fi
  mv -f "$part" "$out"
  trap - EXIT
  echo "$name: downloaded"
done
