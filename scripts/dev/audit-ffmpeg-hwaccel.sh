#!/usr/bin/env bash
# CFA4 R0 FFmpeg 9.0.2 hardware-media audit; Ubuntu 26.04 Linux AMD64.
# Run locally on a physical NVIDIA GPU, not on a hosted CI software runner.
# No installs, downloads or Rust builds. Uses eight standard-size frames only.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
die() { echo "CFA4 FFMPEG HW AUDIT FAILED: $*" >&2; exit 1; }

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || die "Linux AMD64 required"
# shellcheck source=/dev/null
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] || die "Ubuntu 26.04 required"
[[ "$#" -eq 0 ]] || die "usage: bash scripts/dev/audit-ffmpeg-hwaccel.sh"
for program in git nvidia-smi python3 sha256sum; do
    command -v "$program" >/dev/null 2>&1 || die "missing $program"
done

FFMPEG="${CFA4_FFMPEG_BIN:-$(command -v ffmpeg || true)}"
[[ -x "$FFMPEG" ]] || die "ffmpeg not executable; CFA4_FFMPEG_BIN may override the path"
FFPROBE="${CFA4_FFPROBE_BIN:-$(dirname -- "$FFMPEG")/ffprobe}"
[[ -x "$FFPROBE" ]] || die "matching ffprobe missing: $FFPROBE"
version="$("$FFMPEG" -version | awk 'NR == 1 { print $3 }')"
probe_version="$("$FFPROBE" -version | awk 'NR == 1 { print $3 }')"
[[ "$version" == 9.0.2 && "$probe_version" == 9.0.2 ]] ||
    die "requires ffmpeg and ffprobe 9.0.2; found $version and $probe_version"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${CFA4_FFMPEG_AUDIT_DIR:-$ROOT/target/audit/ffmpeg}/$STAMP"
mkdir -p "$OUT/logs"
printf 'CFA4 FFmpeg hardware-audit evidence: %s\n' "$OUT"
printf 'case\trequirement\tstatus\tlog\n' > "$OUT/results.tsv"

GPU_NAME="$(nvidia-smi --query-gpu=name --format=csv,noheader | head -n 1)"
[[ -n "$GPU_NAME" ]] || die "NVIDIA GPU not available"
{
    printf 'Date UTC: %s\n' "$(date -u --iso-8601=seconds)"
    printf 'Git commit: %s\n' "$(git rev-parse HEAD)"
    printf 'Git branch: %s\n' "$(git branch --show-current)"
    printf 'OS: %s\n' "${PRETTY_NAME:-unknown}"
    printf 'Kernel: %s\n' "$(uname -r)"
    printf 'Session: %s\n' "${XDG_SESSION_TYPE:-unknown}"
    printf 'FFmpeg executable: %s\n' "$(readlink -f -- "$FFMPEG")"
    printf 'FFmpeg SHA256: %s\n' "$(sha256sum -- "$FFMPEG")"
    printf 'FFprobe executable: %s\n' "$(readlink -f -- "$FFPROBE")"
    printf 'GPU name (first NVIDIA adapter): %s\n' "$GPU_NAME"
    nvidia-smi --query-gpu=name,driver_version,memory.total,compute_cap --format=csv,noheader
    printf 'Vulkan GPU filter selector: %s\n' "${CFA4_VULKAN_DEVICE:-NVIDIA}"
} > "$OUT/metadata.txt"
"$FFMPEG" -version > "$OUT/ffmpeg-version.txt" 2>&1
"$FFMPEG" -buildconf > "$OUT/ffmpeg-buildconf.txt" 2>&1
"$FFMPEG" -hide_banner -hwaccels > "$OUT/hwaccels.txt" 2>&1
"$FFMPEG" -hide_banner -encoders > "$OUT/encoders.txt" 2>&1
"$FFMPEG" -hide_banner -decoders > "$OUT/decoders.txt" 2>&1
"$FFMPEG" -hide_banner -filters > "$OUT/filters.txt" 2>&1
"$FFPROBE" -version > "$OUT/ffprobe-version.txt" 2>&1

failures=0
not_built=0
blocked=0
record() {
    local case_name="$1" requirement="$2" status="$3" note="$4"
    printf '%s\t%s\t%s\t%s\n' "$case_name" "$requirement" "$status" "$note" >> "$OUT/results.tsv"
    printf '%-26s %-22s %s\n' "$case_name" "$status" "$note"
    if [[ "$status" == FAIL ]]; then failures=$((failures + 1)); fi
    if [[ "$status" == NOT_BUILT && "$requirement" == REQUIRED ]]; then not_built=$((not_built + 1)); fi
    if [[ "$status" == BLOCKED && "$requirement" == REQUIRED ]]; then blocked=$((blocked + 1)); fi
}
run_step() {
    local case_name="$1" requirement="$2"
    shift 2
    local log="$OUT/logs/$case_name.log"
    if {
        printf 'COMMAND:'
        printf ' %q' "$@"
        printf '\n'
        "$@"
    } > "$log" 2>&1; then
        record "$case_name" "$requirement" PASS "logs/$case_name.log"
        return 0
    else
        record "$case_name" "$requirement" FAIL "logs/$case_name.log"
        return 1
    fi
}
has_encoder() {
    awk -v name="$1" '$1 ~ /^V/ && $2 == name { found=1 } END { exit !found }' "$OUT/encoders.txt"
}
has_filter() {
    awk -v name="$1" '$2 == name { found=1 } END { exit !found }' "$OUT/filters.txt"
}
has_hwaccel() { grep -Fxq "$1" "$OUT/hwaccels.txt"; }
check_video() {
    local file="$1" codec="$2" width="$3" height="$4" result frames
    result="$("$FFPROBE" -v error -select_streams v:0 \
        -show_entries stream=codec_name,width,height -of csv=p=0 "$file")" || return 1
    [[ "$result" == "$codec,$width,$height" ]] || {
        echo "Unexpected ffprobe video stream: $result, expected $codec,$width,$height" >&2
        return 1
    }
    frames="$("$FFPROBE" -v error -count_frames -select_streams v:0 \
        -show_entries stream=nb_read_frames -of csv=p=0 "$file")" || return 1
    [[ "$frames" =~ ^[0-9]+$ && "$frames" -ge 8 ]] || {
        echo "Expected at least 8 decoded frames, found $frames in $file" >&2
        return 1
    }
}
run_probe() {
    run_step "$1" REQUIRED check_video "$2" "$3" "$4" "$5"
}
# A nominal zero-exit FFmpeg invocation is insufficient if it emitted
# zero video frames. Framehash lines (excluding '#' metadata) prove actual
# decoded or filtered frames reached the system-memory output.
check_framehash() {
    local file="$1" observed
    [[ -s "$file" ]] || { echo "Missing or empty framehash: $file" >&2; return 1; }
    observed="$(awk '!/^#/ && NF { count++ } END { print count+0 }' "$file")" || return 1
    [[ "$observed" -eq 8 ]] || {
        echo "Expected 8 output frames, found $observed in $file" >&2
        return 1
    }
}
# 128x72 was rejected by the physical RTX A1000 NVENC and AV1 NVDEC:
# NVENC: "Frame Dimension less than the minimum supported value".
# AV1 NVDEC: minimum height 128, so 72 cannot initialize.
# Keep this tiny but comfortably supported 16:9 SD/HD sample consistent.
SOURCE_W=640
SOURCE_H=360
DOWNSCALE_W=320
DOWNSCALE_H=180
FFCOMMON=(-nostdin -hide_banner -loglevel verbose -y)
INPUT="$OUT/source.mkv"
if ! run_step synthetic_source REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
    -f lavfi -i "testsrc2=size=${SOURCE_W}x${SOURCE_H}:rate=8" -frames:v 8 -an \
    -c:v mpeg4 -q:v 4 "$INPUT"; then
    die "cannot generate the synthetic video; inspect $OUT"
fi
run_probe source_probe "$INPUT" mpeg4 "$SOURCE_W" "$SOURCE_H" || true
run_step cpu_source_decode REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
    -hwaccel none -i "$INPUT" -an -f null - || true

# NVENC encode and NVDEC decode are independent capabilities. Always
# generate a reference file with a SOFTWARE encoder to exercise NVDEC even if
# the hardware encoder cannot initialize. Never relabel NVENC FAIL as PASS.
for codec in h264 hevc; do
    encoder="${codec}_nvenc"
    if ! has_encoder "$encoder"; then
        record "nvenc_$codec" REQUIRED NOT_BUILT "FFmpeg has no $encoder encoder"
    else
        file="$OUT/$codec.mp4"
        if run_step "nvenc_$codec" REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
            -i "$INPUT" -an -frames:v 8 -pix_fmt yuv420p -c:v "$encoder" -preset p4 "$file"; then
            run_probe "probe_$codec" "$file" "$codec" "$SOURCE_W" "$SOURCE_H" || true
        fi
    fi

    # These tiny software references remove the NVENC-to-NVDEC test
    # dependency; encoded outputs are probed before CUDA decode.
    if [[ "$codec" == h264 ]]; then
        sw_encoder=libx264
    else
        sw_encoder=libx265
    fi
    if ! has_encoder "$sw_encoder"; then
        record "reference_$codec" REQUIRED NOT_BUILT "FFmpeg has no $sw_encoder encoder"
        record "nvdec_$codec" REQUIRED BLOCKED "software reference encoder unavailable"
        record "cpu_decode_$codec" REQUIRED BLOCKED "software reference encoder unavailable"
        continue
    fi
    ref_file="$OUT/${codec}-reference.mkv"
    if ! run_step "reference_$codec" REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
        -i "$INPUT" -an -frames:v 8 -pix_fmt yuv420p \
        -c:v "$sw_encoder" -preset ultrafast -crf 28 "$ref_file"; then
        record "nvdec_$codec" REQUIRED BLOCKED "software reference failed to encode"
        record "cpu_decode_$codec" REQUIRED BLOCKED "software reference failed to encode"
        continue
    fi
    if ! run_probe "probe_reference_$codec" "$ref_file" "$codec" "$SOURCE_W" "$SOURCE_H"; then
        record "nvdec_$codec" REQUIRED BLOCKED "reference stream failed ffprobe validation"
        record "cpu_decode_$codec" REQUIRED BLOCKED "reference stream failed ffprobe validation"
        continue
    fi
    if has_hwaccel cuda; then
        # GPU surfaces are downloaded explicitly: both the real CUDA path
        # and the 8 output frame count must pass to claim an NVDEC PASS.
        if run_step "nvdec_$codec" REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
            -hwaccel cuda -hwaccel_output_format cuda -i "$ref_file" -an \
            -vf "hwdownload,format=nv12" -f framehash "$OUT/nvdec_${codec}.framehash"; then
            run_step "nvdec_${codec}_framecheck" REQUIRED \
                check_framehash "$OUT/nvdec_${codec}.framehash" || true
        else
            record "nvdec_${codec}_framecheck" REQUIRED BLOCKED "CUDA reference decode failed"
        fi
    else
        record "nvdec_$codec" REQUIRED NOT_BUILT "CUDA hwaccel not built"
        record "nvdec_${codec}_framecheck" REQUIRED BLOCKED "CUDA hwaccel not built"
    fi
    run_step "cpu_decode_$codec" REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
        -hwaccel none -i "$ref_file" -an -f null - || true
done

# Ampere RTX A1000 has AV1 NVDEC but not AV1 NVENC. Encoder listing alone
# does not mean that the installed hardware can encode that codec.
if [[ "$GPU_NAME" == *"RTX A1000"* ]]; then
    record av1_nvenc OPTIONAL UNSUPPORTED_HARDWARE "Ampere RTX A1000 does not encode AV1"
elif has_encoder av1_nvenc; then
    run_step av1_nvenc OPTIONAL "$FFMPEG" "${FFCOMMON[@]}" -i "$INPUT" -an \
        -frames:v 8 -pix_fmt yuv420p -c:v av1_nvenc -preset p4 "$OUT/av1-nvenc.mkv" || true
else
    record av1_nvenc OPTIONAL NOT_BUILT "av1_nvenc not present in FFmpeg"
fi

AV1_INPUT="$OUT/av1-software.mkv"
if has_encoder libsvtav1; then
    av1_cmd=("$FFMPEG" "${FFCOMMON[@]}" -i "$INPUT" -an -frames:v 8 \
        -c:v libsvtav1 -preset 12 -crf 42 "$AV1_INPUT")
elif has_encoder libaom-av1; then
    av1_cmd=("$FFMPEG" "${FFCOMMON[@]}" -i "$INPUT" -an -frames:v 8 \
        -c:v libaom-av1 -threads 2 -cpu-used 8 -crf 42 -b:v 0 "$AV1_INPUT")
else
    av1_cmd=()
fi
if (( ${#av1_cmd[@]} == 0 )); then
    record av1_software_sample REQUIRED NOT_BUILT "no libsvtav1/libaom-av1 encoder"
    record av1_nvdec REQUIRED BLOCKED "no AV1 sample"
elif run_step av1_software_sample REQUIRED "${av1_cmd[@]}"; then
    run_probe av1_software_probe "$AV1_INPUT" av1 "$SOURCE_W" "$SOURCE_H" || true
    if has_hwaccel cuda; then
        if run_step av1_nvdec REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
            -hwaccel cuda -hwaccel_output_format cuda -i "$AV1_INPUT" -an \
            -vf "hwdownload,format=nv12" -f framehash "$OUT/av1-nvdec.framehash"; then
            run_step av1_nvdec_framecheck REQUIRED \
                check_framehash "$OUT/av1-nvdec.framehash" || true
        else
            record av1_nvdec_framecheck REQUIRED BLOCKED "AV1 CUDA decode failed"
        fi
    else
        record av1_nvdec REQUIRED NOT_BUILT "CUDA hwaccel not built"
        record av1_nvdec_framecheck REQUIRED BLOCKED "CUDA hwaccel not built"
    fi
    run_step av1_cpu_decode REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
        -hwaccel none -i "$AV1_INPUT" -an -f null - || true
else
    record av1_nvdec REQUIRED BLOCKED "AV1 software sample failed"
fi

# Select NVIDIA by name to avoid picking llvmpipe. For OpenCL, require
# an NVIDIA GPU device instead of silently selecting a CPU OpenCL provider.
VULKAN_DEVICE="${CFA4_VULKAN_DEVICE:-NVIDIA}"
if has_hwaccel vulkan; then
    if has_filter scale_vulkan; then
        run_step scale_vulkan REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
            -init_hw_device "vulkan=vk:$VULKAN_DEVICE" -filter_hw_device vk \
            -i "$INPUT" -an -frames:v 8 \
            -vf "format=nv12,hwupload,scale_vulkan=w=${DOWNSCALE_W}:h=${DOWNSCALE_H},hwdownload,format=nv12" \
            -f null - || true
    else
        record scale_vulkan REQUIRED NOT_BUILT "scale_vulkan filter not built"
    fi
    if has_filter libplacebo; then
        # FFmpeg's libplacebo filter can create its OWN Vulkan device for
        # software-input frames. Do not pass -filter_hw_device vk here:
        # FFmpeg 9's libplacebo device-import path can reject Vulkan queue
        # flags on libplacebo v7.360 (PL_API_VER < 365) with bare EINVAL.
        # This independent path must prove it picked the physical NVIDIA GPU,
        # not a software Vulkan device. scale_vulkan still uses explicit vk.
        if run_step libplacebo REQUIRED "$FFMPEG" \
            -nostdin -hide_banner -loglevel debug -y \
            -i "$INPUT" -an -frames:v 8 \
            -vf "libplacebo=w=${DOWNSCALE_W}:h=${DOWNSCALE_H}:format=yuv420p" \
            -f framehash "$OUT/libplacebo.framehash"; then
            run_step libplacebo_framecheck REQUIRED \
                check_framehash "$OUT/libplacebo.framehash" || true
            run_step libplacebo_adapter REQUIRED \
                grep -Fq "Device Name: $GPU_NAME" "$OUT/logs/libplacebo.log" || true
        else
            record libplacebo_framecheck REQUIRED BLOCKED "libplacebo framehash not produced"
            record libplacebo_adapter REQUIRED BLOCKED "libplacebo filter did not start"
        fi
    else
        record libplacebo REQUIRED NOT_BUILT "libplacebo filter not built"
        record libplacebo_framecheck REQUIRED BLOCKED "libplacebo filter not built"
    fi
else
    record scale_vulkan REQUIRED NOT_BUILT "Vulkan hwaccel not built"
    record libplacebo REQUIRED NOT_BUILT "Vulkan hwaccel not built"
fi
if has_hwaccel opencl && has_filter avgblur_opencl; then
    run_step avgblur_opencl REQUIRED "$FFMPEG" "${FFCOMMON[@]}" \
        -init_hw_device "opencl=cl:,device_type=gpu,device_vendor=NVIDIA" \
        -filter_hw_device cl -i "$INPUT" -an -frames:v 8 \
        -vf "format=yuv420p,hwupload,avgblur_opencl=sizeX=2:sizeY=2,hwdownload,format=yuv420p" \
        -f null - || true
else
    record avgblur_opencl REQUIRED NOT_BUILT "OpenCL or avgblur_opencl filter not built"
fi

{
    printf 'CFA4 R0 FFmpeg hardware audit (Ubuntu 26.04 / AMD64)\n'
    printf 'FFmpeg: %s\nGPU: %s\nGit SHA: %s\n' "$version" "$GPU_NAME" "$(git rev-parse HEAD)"
    printf 'Synthetic video: %sx%s, 8 frames @ 8 fps\n' "$SOURCE_W" "$SOURCE_H"
    printf 'FAIL cases: %d\nNOT_BUILT required cases: %d\nBLOCKED required cases: %d\n' "$failures" "$not_built" "$blocked"
    printf 'AV1_NVENC=UNSUPPORTED_HARDWARE is expected for RTX A1000.\n'
    printf 'Read results.tsv, logs/*.log and buildconf before closing issue #14.\n'
} > "$OUT/SUMMARY.txt"
cat "$OUT/SUMMARY.txt"
if (( failures > 0 )); then
    echo "FAIL: advertised functions failed; inspect $OUT/results.tsv" >&2
    exit 1
fi
if (( not_built > 0 || blocked > 0 )); then
    echo "INCOMPLETE: required capabilities missing or checks blocked; inspect $OUT/results.tsv" >&2
    exit 2
fi
echo "PASS: required FFmpeg hardware-media checks passed; unsupported AV1 NVENC reported separately."
echo "Evidence: $OUT (not uploaded automatically)"
