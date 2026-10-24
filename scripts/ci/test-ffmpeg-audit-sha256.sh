#!/usr/bin/env bash
# Regression for CFA4 FFmpeg metadata SHA256 parsing; no GPU/FFmpeg needed.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECKER="$ROOT/scripts/ci/read-ffmpeg-audit-sha256.sh"
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

# Exact field shape produced by scripts/dev/audit-ffmpeg-hwaccel.sh.
KNOWN="502cff20212a8b2d03f12980637a9070998572efc05bb8999280d47d55f10a8e"
printf 'Date UTC: 2026-10-22T15:30:37+00:00\nFFmpeg executable: /usr/bin/ffmpeg\nFFmpeg SHA256: %s  /usr/bin/ffmpeg\nFFprobe executable: /usr/bin/ffprobe\n' "$KNOWN" > "$TMP/valid.txt"
actual="$(bash "$CHECKER" "$TMP/valid.txt")"
[[ "$actual" == "$KNOWN" ]] || {
    echo "FFmpeg metadata parser did not return the original 64-hex SHA256" >&2
    exit 1
}

must_reject() {
    if bash "$CHECKER" "$1" > "$TMP/stdout" 2> "$TMP/stderr"; then
        echo "FFmpeg SHA256 parser accepted invalid metadata: $1" >&2
        exit 1
    fi
}
printf 'FFmpeg executable: /usr/bin/ffmpeg\n' > "$TMP/missing.txt"
must_reject "$TMP/missing.txt"
printf 'FFmpeg SHA256: abc123  /usr/bin/ffmpeg\n' > "$TMP/short.txt"
must_reject "$TMP/short.txt"
printf 'FFmpeg SHA256: %s  /usr/bin/ffmpeg\nFFmpeg SHA256: %s  /usr/bin/ffmpeg\n' "$KNOWN" "$KNOWN" > "$TMP/duplicate.txt"
must_reject "$TMP/duplicate.txt"
must_reject "$TMP/does-not-exist.txt"

echo "PASS: valid FFmpeg SHA256 metadata parsed; malformed, missing and duplicate hashes rejected."
