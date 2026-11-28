#!/usr/bin/env bash
# CFA4-NVDEC-CUARRAY-001 M0/M2: NVDEC decoding with pitch-linear surfaces against CUDA-array output
# (Video Codec SDK 13.1 opaque layout), on the owner's reference machine (docs/development/reference-machine.md:
# the RTX A4000 is the compute card). Ubuntu 26.04 Linux AMD64 with the NVIDIA driver; run locally,
# never on a hosted CI runner. No installs or downloads.
#
#   cargo build --release -p cfa4-cli
#   bash scripts/dev/measure-nvdec-cuarray.sh [report.md]
#
# Fixtures (H.264 8-bit and HEVC Main 10, 1080p and 2160p) are encoded with the pinned FFmpeg's NVENC into
# target/fixtures/nvdec-cuarray/ (never committed). Each file is decoded RUNS times by `cfa4-cli bench-decode`
# in both modes (CFA4_NVDEC_CUDA_ARRAY unset and =1); the report holds fps, CPU time and peak memory per run,
# the driver version and which output each run actually used.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
die() { echo "CFA4 NVDEC CUDA-ARRAY MEASUREMENT FAILED: $*" >&2; exit 1; }

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || die "Linux AMD64 required"
# shellcheck source=/dev/null
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] || die "Ubuntu 26.04 required"
[[ "$#" -le 1 ]] || die "usage: bash scripts/dev/measure-nvdec-cuarray.sh [report.md]"
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

rows=()
for file in h264_1080p.mp4 h264_2160p.mp4 hevc10_1080p.mp4 hevc10_2160p.mp4; do
    for mode in pitch array; do
        for run in $(seq 1 "$RUNS"); do
            log="$(mktemp)"
            if [[ "$mode" == array ]]; then
                CFA4_NVDEC_CUDA_ARRAY=1 /usr/bin/time -f 'TIME %U %S %M' "$CLI" bench-decode "$FIX/$file" --frames "$FRAMES" >"$log" 2>&1 || die "bench-decode failed: $(tail -n 5 "$log")"
            else
                env -u CFA4_NVDEC_CUDA_ARRAY /usr/bin/time -f 'TIME %U %S %M' "$CLI" bench-decode "$FIX/$file" --frames "$FRAMES" >"$log" 2>&1 || die "bench-decode failed: $(tail -n 5 "$log")"
            fi
            fps="$(sed -n 's/.*→ \([0-9.]*\) fps.*/\1/p' "$log" | head -n 1)"
            used="$(sed -n 's/.*hardware decoder: \(.*\)$/\1/p' "$log" | head -n 1)"
            read -r _ user sys rss < <(grep '^TIME ' "$log")
            rows+=("| $file | $mode | $run | ${fps:-?} | $user | $sys | $((rss / 1024)) | ${used:-?} |")
            rm -f "$log"
        done
    done
done

mkdir -p "$(dirname -- "$REPORT")"
{
    echo "# NVDEC: pitch-linear surfaces vs CUDA-array output (CFA4-NVDEC-CUARRAY-001)"
    echo
    echo "- Date (UTC): $(date -u +%Y-%m-%dT%H:%MZ); CFA4 $(git rev-parse --short HEAD)"
    echo "- GPU: $gpu_name (index $gpu_index, CUDA_VISIBLE_DEVICES); driver $driver"
    echo "- Kernel module: \`$kernel_module\`"
    echo "- Method: \`cfa4-cli bench-decode --frames $FRAMES\`, $RUNS runs per file and mode; CPU times and peak RSS from \`/usr/bin/time\`."
    echo
    echo "| File | Output | Run | fps | user s | sys s | peak MiB | Decoder (as the CLI reports) |"
    echo "|------|--------|-----|-----|--------|-------|----------|------------------------------|"
    printf '%s\n' "${rows[@]}"
} >"$REPORT"
echo "report: $REPORT"
