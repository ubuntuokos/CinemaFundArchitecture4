#!/usr/bin/env bash
# Read the FFmpeg executable SHA-256 from CFA4 hardware-audit metadata.
# Original metadata format: "FFmpeg SHA256: <64hex>  /path/to/ffmpeg".
# No FFmpeg invocation, codec execution, files changed or downloads.
set -euo pipefail

[[ "$#" -eq 1 && -r "$1" ]] || {
    echo "usage: bash scripts/ci/read-ffmpeg-audit-sha256.sh readable-metadata.txt" >&2
    exit 2
}

# Use fields, not fragile sed BRE capture/interval escaping. Require exactly
# one record: a missing or duplicate label must not pass as a matching hash.
hash="$(awk '$1 == "FFmpeg" && $2 == "SHA256:" { print $3; count++ } END { if (count != 1) exit 1 }' "$1")" || {
    echo "missing or duplicate FFmpeg SHA256 field in $1" >&2
    exit 1
}
[[ "$hash" =~ ^[0-9a-f]{64}$ ]] || {
    echo "invalid FFmpeg SHA256 value in $1" >&2
    exit 1
}
printf '%s\n' "$hash"
