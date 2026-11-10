#!/usr/bin/env bash
# R3D-REDLINE: render reference frames and metadata from an R3D clip with RED REDline,
# run as an external process, with fixed, documented settings so the output can serve
# as an oracle for CFA4's R3D import path and its regression tests.
#
# Settings (docs/development/redline.md §4): CPU only (--gpuPlatform 0, vendor-neutral
# and reproducible), full resolution, IPP2, uncompressed OpenEXR, no audio. The default
# output is primary development only (ungraded REDWideGamutRGB / Log3G10, the clip's look
# metadata is NOT applied); --output hlg / pq gives the Rec.2020 HDR broadcast outputs.
#
# Usage: scripts/dev/r3d-reference.sh [options] <clip.R3D> <outdir>
#   --start N               first frame (default 0)
#   --frames N              number of frames (default 3)
#   --output log3g10|hlg|pq RWG/Log3G10 primary development (default), Rec.2020 + HLG,
#                           or Rec.2020 + ST 2084 (PQ)
#   --check-determinism     render twice and require identical files
# REDline is taken from $CFA4_REDLINE (see scripts/dev/redline-unpack.sh), else from PATH.
# Output: <outdir>/frames/*.exr, metadata.txt, manifest.txt, SHA256SUMS, redline.log.
# Never commit R3D clips or rendered frames (AGENTS.md: no large binary fixtures).
set -euo pipefail

die() { echo "r3d-reference: $*" >&2; exit 2; }

START=0
FRAMES=3
TWICE=0
OUTPUT=log3g10
while [[ $# -gt 0 ]]; do
    case "$1" in
        --start) START="${2:-}"; shift 2 ;;
        --frames) FRAMES="${2:-}"; shift 2 ;;
        --check-determinism) TWICE=1; shift ;;
        --output) OUTPUT="${2:-}"; shift 2 ;;
        --) shift; break ;;
        -*) die "unknown option $1" ;;
        *) break ;;
    esac
done
[[ $# -eq 2 ]] || die "usage: $0 [--start N] [--frames N] [--output log3g10|hlg|pq] [--check-determinism] <clip.R3D> <outdir>"
CLIP="$(readlink -f -- "$1")"
OUT="$2"
[[ "$START" =~ ^[0-9]+$ ]] || die "--start must be a non-negative integer"
[[ "$FRAMES" =~ ^[0-9]+$ && "$FRAMES" -ge 1 && "$FRAMES" -le 10000 ]] \
    || die "--frames must be between 1 and 10000"
[[ "$OUTPUT" =~ ^(log3g10|hlg|pq)$ ]] || die "--output must be log3g10, hlg or pq"
[[ -f "$CLIP" ]] || die "clip not found: $1"
if [[ -e "$OUT" ]] && [[ -n "$(ls -A "$OUT" 2>/dev/null)" ]]; then
    die "output directory is not empty: $OUT"
fi

REDLINE="${CFA4_REDLINE:-}"
if [[ -z "$REDLINE" ]]; then
    REDLINE="$(command -v REDline || true)"
fi
[[ -n "$REDLINE" && -x "$REDLINE" ]] \
    || die "REDline not found: set CFA4_REDLINE (scripts/dev/redline-unpack.sh) or put REDline on PATH"

mkdir -p "$OUT"
OUT="$(readlink -f -- "$OUT")"

VERSION_TEXT="$("$REDLINE" --help 2>&1 || true)"
BUILD="$(grep -o 'REDline Build [0-9.]*' <<<"$VERSION_TEXT" | head -n 1 || true)"
SDK="$(grep -o 'R3DSDK [0-9][0-9.]*-[0-9a-f]*' <<<"$VERSION_TEXT" | head -n 1 || true)"
[[ -n "$BUILD" ]] || die "$REDLINE does not look like REDline (no build string in --help)"

case "$OUTPUT" in
    log3g10) COLOR_ARGS=(--primaryDev) ;;
    hlg) COLOR_ARGS=(--colorSpace 24 --gammaCurve 35) ;;
    pq) COLOR_ARGS=(--colorSpace 24 --gammaCurve 31) ;;
    *) die "--output must be log3g10, hlg or pq" ;;
esac
RENDER_ARGS=(
    --format 2 --exrCompression 0 --res 1
    --gpuPlatform 0
    --colorSciVersion 3 "${COLOR_ARGS[@]}"
    --start "$START" --frameCount "$FRAMES"
    --pad 6 --noAudio
)

render() { # <frames dir>
    mkdir -p "$1"
    "$REDLINE" --i "$CLIP" --outDir "$1" --o ref "${RENDER_ARGS[@]}" >>"$OUT/redline.log" 2>&1
}

echo "Reading metadata …"
if ! "$REDLINE" --i "$CLIP" --printMeta 3 >"$OUT/metadata.txt" 2>>"$OUT/redline.log"; then
    tail -n 5 "$OUT/redline.log" >&2 || true
    die "REDline could not read the clip (log: $OUT/redline.log)"
fi

echo "Rendering $FRAMES frame(s) from frame $START on the CPU …"
if ! render "$OUT/frames"; then
    tail -n 5 "$OUT/redline.log" >&2 || true
    die "REDline render failed (log: $OUT/redline.log)"
fi
mapfile -t EXRS < <(cd "$OUT/frames" && find . -type f -name '*.exr' | sort)
[[ ${#EXRS[@]} -eq "$FRAMES" ]] \
    || die "expected $FRAMES EXR file(s), REDline wrote ${#EXRS[@]} (log: $OUT/redline.log)"
(cd "$OUT/frames" && sha256sum "${EXRS[@]}") >"$OUT/SHA256SUMS"

DETERMINISM="not checked"
if [[ "$TWICE" -eq 1 ]]; then
    echo "Rendering again to check determinism …"
    render "$OUT/frames-2" || die "second REDline render failed (log: $OUT/redline.log)"
    if (cd "$OUT/frames-2" && sha256sum --quiet -c "$OUT/SHA256SUMS"); then
        DETERMINISM="identical on two CPU renders"
        rm -rf "$OUT/frames-2"
    else
        DETERMINISM="DIFFERENT on two CPU renders (kept frames-2/)"
    fi
fi

{
    echo "clip: $(basename -- "$CLIP")"
    echo "clip-sha256: $(sha256sum "$CLIP" | cut -c1-64)"
    echo "clip-bytes: $(stat -c %s "$CLIP")"
    echo "redline: $BUILD"
    echo "r3dsdk: ${SDK:-unknown}"
    echo "args: ${RENDER_ARGS[*]}"
    echo "frames: ${#EXRS[@]}"
    echo "determinism: $DETERMINISM"
    echo "host: $(uname -srm)"
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$OUT/manifest.txt"
cat "$OUT/manifest.txt"
[[ "$DETERMINISM" != DIFFERENT* ]] || exit 1
