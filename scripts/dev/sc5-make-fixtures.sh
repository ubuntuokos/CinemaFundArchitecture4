#!/usr/bin/env bash
# SC-5.0 (CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001-SC5-MLT-IMPORT.md): generate the media the Shotcut-style
# .mlt fixtures in crates/interchange/tests/data/shotcut/ refer to (a.mp4, b.mp4: 4 s, 320x180, 25 fps,
# test pattern + sine) and, if `melt` is installed, render each fixture as a test oracle
# (frame count only; melt is a test oracle, never a product dependency).
# Usage: scripts/dev/sc5-make-fixtures.sh [output-dir]   (default: target/fixtures/shotcut; never committed)
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="${1:-$repo/target/fixtures/shotcut}"
data="$repo/crates/interchange/tests/data/shotcut"
mkdir -p "$out"
ffmpeg -loglevel error -y -f lavfi -i testsrc=size=320x180:rate=25:duration=4 -f lavfi -i sine=frequency=440:duration=4 \
  -shortest -c:v libx264 -pix_fmt yuv420p -c:a aac "$out/a.mp4"
ffmpeg -loglevel error -y -f lavfi -i testsrc2=size=320x180:rate=25:duration=4 -f lavfi -i sine=frequency=880:duration=4 \
  -shortest -c:v libx264 -pix_fmt yuv420p -c:a aac "$out/b.mp4"
# shotcut-26.2.16-single-clip.mlt (real Shotcut 26.2.16 file, path sanitised): 1280x720, 30 fps, 4 s
ffmpeg -loglevel error -y -f lavfi -i testsrc=size=1280x720:rate=30:duration=4 -c:v libx264 -pix_fmt yuv420p "$out/sample.mp4"
cp "$data"/*.mlt "$out/"
sed -i "s#/home/user/Videos/sample.mp4#$out/sample.mp4#" "$out/shotcut-26.2.16-single-clip.mlt"
if command -v melt >/dev/null 2>&1; then
  for f in "$out"/*.mlt; do
    (cd "$out" && timeout 120 melt "$f" -consumer "avformat:${f%.mlt}.oracle.mp4" vcodec=libx264 acodec=aac >/dev/null 2>&1 || true)
    n="$(ffprobe -v error -select_streams v -count_frames -show_entries stream=nb_read_frames -of csv=p=0 "${f%.mlt}.oracle.mp4" 2>/dev/null || echo '?')"
    echo "$(basename "$f"): melt frames=$n"
  done
else
  echo "melt not installed: oracle render skipped"
fi
