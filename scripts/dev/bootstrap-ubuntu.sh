#!/usr/bin/env bash
# CFA4 developer prerequisites for Ubuntu 26.04 and its flavours, x86_64/AMD64.
# Default: read-only checks. --install: install system development packages.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 development requires Linux x86_64 (AMD64)." >&2
  exit 2
fi

# Reject non-Ubuntu and non-26.04 hosts before checking or installing prerequisites.
. "$(dirname "${BASH_SOURCE[0]}")/../ci/require-ubuntu-2604.sh"

packages=(
  git curl ca-certificates build-essential pkg-config cmake ninja-build clang libclang-dev
  python3 python3-venv python3-pip python3-dev
  qt6-base-dev qt6-base-dev-tools qt6-declarative-dev qt6-tools-dev qt6-tools-dev-tools
  qt6-multimedia-dev libqt6svg6-dev
  libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev
  libx11-dev libx11-xcb-dev libxi-dev libxcursor-dev libxrandr-dev
  libvulkan-dev libegl1-mesa-dev libgl1-mesa-dev
  ffmpeg libheif-examples libheif-plugin-libde265 libheif-plugin-x265
)

case "${1:---check}" in
  --check)
    missing=0
    for cmd in rustc cargo rustup python3 pkg-config cmake ninja ffmpeg ffprobe heif-dec heif-enc; do
      if command -v "$cmd" >/dev/null 2>&1; then
        printf 'OK      %s: %s\n' "$cmd" "$(command -v "$cmd")"
      else
        printf 'MISSING %s\n' "$cmd"
        missing=1
      fi
    done
    if pkg-config --exists Qt6Core 2>/dev/null; then
      echo "OK      Qt6Core: $(pkg-config --modversion Qt6Core)"
    else
      echo "MISSING Qt6Core: install qt6-base-dev"
      missing=1
    fi
    if [[ "$missing" -ne 0 ]]; then
      echo "Install system packages with: bash scripts/dev/bootstrap-ubuntu.sh --install" >&2
      echo "Install rustup separately from https://rustup.rs/ if Rust is missing." >&2
    fi
    exit "$missing"
    ;;
  --install)
    echo "Installing Ubuntu 26.04 build prerequisites through apt (explicit opt-in)."
    if [[ "$EUID" -eq 0 ]]; then
      apt-get update
      DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${packages[@]}"
    else
      if ! command -v sudo >/dev/null 2>&1; then
        echo "sudo is required for --install unless run as root." >&2
        exit 2
      fi
      sudo apt-get update
      sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${packages[@]}"
    fi
    echo "System packages installed. Install Rust via https://rustup.rs/ if necessary."
    echo "Then run: rustup show && python3 -m venv .venv"
    ;;
  *)
    echo "Usage: bash scripts/dev/bootstrap-ubuntu.sh [--check|--install]" >&2
    exit 2
    ;;
esac
