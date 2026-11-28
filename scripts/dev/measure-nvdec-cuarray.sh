#!/usr/bin/env bash
# CFA4-NVDEC-CUARRAY-001 M0/M2: NVDEC decoding with pitch-linear surfaces against CUDA-array output
# (Video Codec SDK 13.1 opaque layout), on the owner's reference machine (docs/development/reference-machine.md:
# the RTX A4000 is the compute card). Ubuntu 26.04 Linux AMD64 with the NVIDIA driver; run locally,
# never on a hosted CI runner. No installs or downloads.
#
#   cargo build --release -p cfa4-cli
#   bash scripts/dev/measure-nvdec-cuarray.sh [--identity-only] [report.md]
#
# Fixtures (H.264 8-bit and HEVC Main 10, 1080p and 2160p) are encoded with the pinned FFmpeg's NVENC into
# target/fixtures/nvdec-cuarray/ (never committed).
# N3 (bit identity): each file is decoded once per mode with `bench-decode --hash`; the SHA-256 of the decoded
#   samples must be the same, and each run must have decoded every picture in hardware (no fallback).
# N4 (speed, skipped with --identity-only): each file is decoded RUNS times per mode, the two modes alternating
#   their order run by run (the order is recorded); fps, CPU time and peak memory per run, the driver version
#   and the decoder counters of each run go into the report.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
die() { echo "CFA4 NVDEC CUDA-ARRAY MEASUREMENT FAILED: $*" >&2; exit 1; }

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || die "Linux AMD64 required"
# shellcheck source=/dev/null
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] || die "Ubuntu 26.04 required"
IDENTITY_ONLY=0
if [[ "${1:-}" == --identity-only ]]; then
    IDENTITY_ONLY=1
    shift
fi
[[ "$#" -le 1 ]] || die "usage: bash scripts/dev/measure-nvdec-cuarray.sh [--identity-only] [report.md]"
for program in nvidia-smi python3 /usr/bin/time; do
    command -v "$program" >/dev/null 2>&1 || die "missing $program"
done
[[ -r /proc/driver/nvidia/version ]] || die "no NVIDIA kernel driver loaded"

CLI="${CFA4_CLI_BIN:-target/release/cfa4-cli}"
[[ -x "$CLI" ]] || die "$CLI missing: cargo build --release -p cfa4-cli (or set CFA4_CLI_BIN)"
FFMPEG="${CFA4_FFMPEG_BIN:-$(command -v ffmpeg || true)}"
[[ -x "$FFMPEG" ]] || die "ffmpeg not executable; CFA4_FFMPEG_BIN may override the path"
# shellcheck source=packaging/ffmpeg-version.env
. "$ROOT/packaging/ffmpeg-version.env"
version="$("$FFMPEG" -version | awk 'NR == 1 { print $3 }')"
[[ "$version" == "$CFA4_FFMPEG_VERSION" ]] || die "requires ffmpeg $CFA4_FFMPEG_VERSION; found $version"

RUNS="${CFA4_MEASURE_RUNS:-3}"
FRAMES="${CFA4_MEASURE_FRAMES:-240}"
[[ "$RUNS" =~ ^[1-9][0-9]{0,2}$ ]] || die "CFA4_MEASURE_RUNS must be 1..999, not '$RUNS'"
[[ "$FRAMES" =~ ^[1-9][0-9]{0,4}$ ]] || die "CFA4_MEASURE_FRAMES must be 1..99999, not '$FRAMES'"
REPORT="${1:-docs/audit/nvdec-cuarray-$(date -u +%Y%m%d).md}"

# The compute card: the first GPU whose name has A4000, else GPU 0 (named in the report).
gpu_line="$(nvidia-smi --query-gpu=index,name,driver_version --format=csv,noheader | awk -F', ' '$2 ~ /A4000/ { print; exit }')"
[[ -n "$gpu_line" ]] || gpu_line="$(nvidia-smi --query-gpu=index,name,driver_version --format=csv,noheader | head -n 1)"
IFS=',' read -r gpu_index gpu_name driver <<<"$gpu_line"
gpu_name="${gpu_name# }"
driver="${driver# }"
export CUDA_VISIBLE_DEVICES="$gpu_index"
kernel_module="$(head -n 1 /proc/driver/nvidia/version)"

FIX="target/fixtures/nvdec-cuarray"
mkdir -p "$FIX"
make_fixture() { # name size encoder extra...
    local name="$1" size="$2" encoder="$3"
    shift 3
    [[ -s "$FIX/$name" ]] && return 0
    "$FFMPEG" -hide_banner -loglevel error -y -f lavfi -i "testsrc2=size=$size:rate=25" -t 12 \
        -c:v "$encoder" "$@" -an "$FIX/$name" || die "cannot encode $name with $encoder"
}
make_fixture h264_1080p.mp4 1920x1080 h264_nvenc -profile:v high -pix_fmt yuv420p -b:v 20M
make_fixture h264_2160p.mp4 3840x2160 h264_nvenc -profile:v high -pix_fmt yuv420p -b:v 60M
make_fixture hevc10_1080p.mp4 1920x1080 hevc_nvenc -profile:v main10 -pix_fmt p010le -b:v 20M
make_fixture hevc10_2160p.mp4 3840x2160 hevc_nvenc -profile:v main10 -pix_fmt p010le -b:v 60M

FILES=(h264_1080p.mp4 h264_2160p.mp4 hevc10_1080p.mp4 hevc10_2160p.mp4)
# One bench-decode run; sets OUT to its output (with the /usr/bin/time line).
bench() { # mode file extra...
    local mode="$1" file="$2"
    shift 2
    local log
    log="$(mktemp)"
    if [[ "$mode" == array ]]; then
        CFA4_NVDEC_CUDA_ARRAY=1 /usr/bin/time -f 'TIME %U %S %M' "$CLI" bench-decode "$FIX/$file" --frames "$FRAMES" "$@" >"$log" 2>&1 || die "bench-decode failed: $(tail -n 5 "$log")"
    else
        env -u CFA4_NVDEC_CUDA_ARRAY /usr/bin/time -f 'TIME %U %S %M' "$CLI" bench-decode "$FIX/$file" --frames "$FRAMES" "$@" >"$log" 2>&1 || die "bench-decode failed: $(tail -n 5 "$log")"
    fi
    OUT="$(cat "$log")"
    rm -f "$log"
}
field() { sed -n "$1" <<<"$OUT" | head -n 1; }
# At least every requested picture decoded in hardware (the decoder may run ahead), no fallback to
# software, and the hardware pictures all from CUDA arrays (array mode) or none of them (pitch mode).
check_counters() { # mode
    local hw arrays fallbacks
    hw="$(field 's/.*(\([0-9]*\) hardware pictures.*/\1/p')"
    fallbacks="$(field 's/.* \([0-9]*\) fallbacks to software.*/\1/p')"
    arrays="$(field 's/.*NVDEC CUDA-array output: [a-z]* (\([0-9]*\) pictures).*/\1/p')"
    [[ -n "$hw" && -n "$arrays" && "$hw" -ge "$FRAMES" && "$fallbacks" == 0 ]] || return 1
    if [[ "$1" == array ]]; then [[ "$arrays" == "$hw" ]]; else [[ "$arrays" == 0 ]]; fi
}

identity=()
identity_ok=1
for file in "${FILES[@]}"; do
    bench pitch "$file" --hash
    pitch_hash="$(field 's/^decoded samples sha256: //p')"
    check_counters pitch && pitch_path=ok || pitch_path=NO
    bench array "$file" --hash
    array_hash="$(field 's/^decoded samples sha256: //p')"
    check_counters array && array_path=ok || array_path=NO
    verdict=identical
    if [[ -z "$pitch_hash" || "$pitch_hash" != "$array_hash" ]]; then verdict=DIFFERENT; identity_ok=0; fi
    if [[ "$pitch_path" != ok || "$array_path" != ok ]]; then verdict="$verdict, NOT ALL IN THE EXPECTED PATH"; identity_ok=0; fi
    identity+=("| $file | \`${pitch_hash:0:16}\` | \`${array_hash:0:16}\` | $verdict |")
done

rows=()
if [[ "$IDENTITY_ONLY" == 0 ]]; then
    for file in "${FILES[@]}"; do
        for run in $(seq 1 "$RUNS"); do
            # alternate which mode goes first, so drift (clocks, heat, caches) does not favour one
            if (( run % 2 )); then order=(pitch array); else order=(array pitch); fi
            for position in 0 1; do
                mode="${order[$position]}"
                bench "$mode" "$file"
                fps="$(field 's/.*→ \([0-9.]*\) fps.*/\1/p')"
                counters="$(field 's/^hardware decoder: //p')"
                read -r _ user sys rss < <(grep '^TIME ' <<<"$OUT")
                rows+=("| $file | $mode | $run | $((position + 1)) of 2 | ${fps:-?} | $user | $sys | $((rss / 1024)) | ${counters:-?} |")
            done
        done
    done
fi

mkdir -p "$(dirname -- "$REPORT")"
{
    echo "# NVDEC: pitch-linear surfaces vs CUDA-array output (CFA4-NVDEC-CUARRAY-001)"
    echo
    echo "- Date (UTC): $(date -u +%Y-%m-%dT%H:%MZ); CFA4 $(git rev-parse --short HEAD)"
    echo "- GPU: $gpu_name (index $gpu_index, CUDA_VISIBLE_DEVICES); driver $driver"
    echo "- Kernel module: \`$kernel_module\`"
    echo "- Method: \`cfa4-cli bench-decode --frames $FRAMES\`; CPU times and peak RSS from \`/usr/bin/time\`."
    echo
    echo "## N3 – bit identity (\`bench-decode --hash\`, $FRAMES pictures, SHA-256 of the decoded samples)"
    echo
    echo "| File | pitch-linear | CUDA array | Result |"
    echo "|------|--------------|------------|--------|"
    printf '%s\n' "${identity[@]}"
    echo
    if [[ "$IDENTITY_ONLY" == 0 ]]; then
        echo "## N4 – speed ($RUNS runs per file and mode, the order alternating run by run)"
        echo
        echo "| File | Output | Run | Order | fps | user s | sys s | peak MiB | Decoder counters of the run |"
        echo "|------|--------|-----|-------|-----|--------|-------|----------|-----------------------------|"
        printf '%s\n' "${rows[@]}"
    fi
} >"$REPORT"
echo "report: $REPORT"
[[ "$identity_ok" == 1 ]] || die "N3: the outputs differ or a run left the expected path (see $REPORT)"
