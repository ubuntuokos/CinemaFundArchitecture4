#!/usr/bin/env bash
# Ubuntu 26.04 packages for the pinned FFmpeg 9.0.2 test oracle build and the
# Rust release tests that call it. Shared by ci-linux.yml (rust-release-tests)
# and ci-cache-warm.yml, so both build FFmpeg and the Rust dependencies the
# same way and the caches they write and read stay identical.
set -euo pipefail
sudo apt-get update
# Inspect ALL apt candidates before installation: report every unavailable
# package in a single fast failure, not one by one across costly CI runs.
oracle_dev_packages=(
  libdav1d-dev libsvtav1enc-dev libaom-dev
  libvpx-dev libmp3lame-dev libopus-dev libwebp-dev libopenjp2-7-dev zlib1g-dev
  libheif-examples libheif-plugin-libde265 libheif-plugin-x265 libjxl-tools
  webp
)
missing_packages=()
for pkg in "${oracle_dev_packages[@]}"; do
  candidate="$(apt-cache policy "$pkg" 2>/dev/null | awk '/Candidate:/ {print $2; exit}' || true)"
  if [[ -z "$candidate" || "$candidate" == "(none)" ]]; then
    missing_packages+=("$pkg")
  else
    echo "FFmpeg oracle package $pkg: $candidate"
  fi
done
if (( ${#missing_packages[@]} > 0 )); then
  printf '::error::Missing Ubuntu 26.04 FFmpeg package: %s\n' "${missing_packages[@]}" >&2
  exit 1
fi
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config clang cmake ninja-build \
  libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev \
  libx11-dev libx11-xcb-dev libxi-dev libxcursor-dev libxrandr-dev \
  libvulkan-dev libegl1-mesa-dev libgl1-mesa-dev nasm curl xz-utils libjxl-tools \
  "${oracle_dev_packages[@]}"
# JPEG 2000 / HTJ2K oracles (the tests skip without them)
sudo apt-get install -y --no-install-recommends libopenjp2-tools grokj2k-tools \
  || echo "::warning::libopenjp2-tools / grokj2k-tools unavailable: OpenJPEG and HTJ2K oracle tests skip"
# Speex reference encoder / decoder (speexenc, speexdec 1.2.1, built from source) for the Speex
# oracle tests (they skip without them)
bash "$(dirname "$0")/install-speex.sh" \
  || echo "::warning::speex tools unavailable: Speex oracle tests skip"
# AMR oracle `cfa4-amr-oracle` (opencore-amr + vo-amrwbenc, built from the pinned upstream tags;
# the AMR oracle tests skip without it)
bash "$(dirname "$0")/install-amr-oracle.sh" \
  || echo "::warning::cfa4-amr-oracle build failed: the AMR oracle tests skip"
