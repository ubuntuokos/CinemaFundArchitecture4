#!/usr/bin/env bash
# Recheck ONLY FFmpeg/libplacebo on the previously generated CFA4 test video.
# No installs, Rust builds, GPU encoder runs or regeneration of reference files.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
die() { echo "CFA4 LIBPLACEBO RECHECK FAILED: $*" >&2; exit 1; }

[[ "$#" -eq 1 ]] || die "usage: bash scripts/dev/recheck-libplacebo.sh target/audit/ffmpeg/<old-run>"
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || die "Linux x86_64 required"
# shellcheck source=/dev/null
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] || die "Ubuntu 26.04 required"

OLD="$(realpath -e -- "$1")" || die "previous evidence directory not found"
[[ -r "$OLD/source.mkv" && -r "$OLD/results.tsv" ]] ||
    die "missing source.mkv or results.tsv in $OLD"
for program in ffmpeg ffprobe nvidia-smi sha256sum awk grep; do
    command -v "$program" >/dev/null 2>&1 || die "$program is missing"
done
FFMPEG="${CFA4_FFMPEG_BIN:-$(command -v ffmpeg)}"
FFPROBE="${CFA4_FFPROBE_BIN:-$(dirname -- "$FFMPEG")/ffprobe}"
[[ -x "$FFMPEG" && -x "$FFPROBE" ]] || die "FFmpeg and matching FFprobe executable required"
[[ "$("$FFMPEG" -version | awk 'NR == 1 { print $3 }')" == 9.0.2 ]] ||
    die "FFmpeg 9.0.2 required"
[[ "$("$FFPROBE" -version | awk 'NR == 1 { print $3 }')" == 9.0.2 ]] ||
    die "FFprobe 9.0.2 required"

# Pin the exact original binary, not merely its version number, so the
# isolated retest can be compared directly with the successful media oracles.
[[ -r "$OLD/metadata.txt" ]] || die "original FFmpeg metadata.txt missing"
old_hash="$(bash scripts/ci/read-ffmpeg-audit-sha256.sh "$OLD/metadata.txt")" ||
    die "cannot parse the original FFmpeg SHA256 metadata"
new_hash="$(sha256sum -- "$FFMPEG" | awk '{ print $1 }')"
[[ "$new_hash" == "$old_hash" ]] ||
    die "FFmpeg executable SHA256 differs from the original audit ($old_hash vs $new_hash)"

# Reuse exactly the already verified, eight-frame 640x360 MPEG4 source.
video_info="$("$FFPROBE" -v error -select_streams v:0 \
    -show_entries stream=codec_name,width,height -of csv=p=0 "$OLD/source.mkv")" ||
    die "cannot probe existing synthetic source"
[[ "$video_info" == "mpeg4,640,360" ]] || die "unexpected reference: $video_info"
frames="$("$FFPROBE" -v error -count_frames -select_streams v:0 \
    -show_entries stream=nb_read_frames -of csv=p=0 "$OLD/source.mkv")" ||
    die "cannot count reference frames"
[[ "$frames" == 8 ]] || die "expected 8 reference frames; found $frames"

GPU_NAME="$(nvidia-smi --query-gpu=name --format=csv,noheader | head -n 1)"
[[ "$GPU_NAME" == NVIDIA* ]] || die "physical NVIDIA GPU is not detected"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$(mktemp -d "$(dirname -- "$OLD")/libplacebo-recheck-$STAMP-XXXXXXXX")"
LOG="$OUT/libplacebo.log"
HASH="$OUT/libplacebo.framehash"
{
    printf 'Date UTC: %s\n' "$(date -u --iso-8601=seconds)"
    printf 'Git SHA for recheck: %s\n' "$(git rev-parse HEAD)"
    printf 'Original run folder: %s\n' "$OLD"
    printf 'FFmpeg: %s\n' "$(readlink -f -- "$FFMPEG")"
    sha256sum -- "$FFMPEG"
    printf 'GPU: %s\n' "$GPU_NAME"
    nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
} > "$OUT/metadata.txt" 2>&1

# Do NOT pass -filter_hw_device: FFmpeg 9.x can reject imported Vulkan
# queue flags on libplacebo API v360; let libplacebo create its own Vulkan
# device. A physical NVIDIA "Device Name:" log is mandatory to accept it.
printf 'COMMAND:' > "$LOG"
printf ' %q' "$FFMPEG" -nostdin -hide_banner -loglevel debug -y \
    -i "$OLD/source.mkv" -an -frames:v 8 \
    -vf "libplacebo=w=320:h=180:format=yuv420p" -f framehash "$HASH" >> "$LOG"
printf '\n' >> "$LOG"
if ! "$FFMPEG" -nostdin -hide_banner -loglevel debug -y \
    -i "$OLD/source.mkv" -an -frames:v 8 \
    -vf "libplacebo=w=320:h=180:format=yuv420p" \
    -f framehash "$HASH" >> "$LOG" 2>&1; then
    echo "FAIL: libplacebo could not process the reference clip; $LOG" >&2
    tail -n 50 "$LOG" >&2
    exit 1
fi

actual="$(awk '!/^#/ && NF { n++ } END { print n+0 }' "$HASH")"
[[ "$actual" == 8 ]] || die "libplacebo produced $actual frames, expected 8; $LOG"
if ! grep -Fq "Device Name: $GPU_NAME" "$LOG"; then
    echo "Vulkan/libplacebo hardware identification excerpt:" >&2
    grep -E 'Probing for vulkan|GPU [0-9]+:|Device Name:|Device ID:' "$LOG" | tail -n 25 >&2 || true
    die "libplacebo did not prove it selected the physical $GPU_NAME; $LOG"
fi
printf 'PASS: libplacebo processed 8 frames using the NVIDIA GPU (%s)\n' "$GPU_NAME"
printf 'Evidence: %s\n' "$OUT"
