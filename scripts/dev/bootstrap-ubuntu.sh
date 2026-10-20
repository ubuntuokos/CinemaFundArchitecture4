#!/usr/bin/env bash
# Linux-only CFA4 developer prerequisite bootstrap (Ubuntu/Debian).
# Default: read-only checks. --install: install system development packages.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "CFA4 development is supported on Linux only." >&2
  exit 2
fi

if [[ -r /etc/os-release ]]; then
  . /etc/os-release
else
  echo "Cannot identify Linux distribution." >&2
  exit 2
fi

case " $ID ${ID_LIKE:-} " in
  *ubuntu*|*debian*) ;;
  *) echo "This package installer supports Ubuntu/Debian only. See docs/development/linux.md." >&2; exit 2 ;;
esac

packages=(
  git curl ca-certificates build-essential pkg-config cmake ninja-build clang libclang-dev
  python3 python3-venv python3-pip python3-dev
  qt6-base-dev qt6-base-dev-tools qt6-declarative-dev qt6-tools-dev qt6-tools-dev-tools
  qt6-multimedia-dev libqt6svg6-dev
  libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev
  libx11-dev libx11-xcb-dev libxi-dev libxcursor-dev libxrandr-dev
  libvulkan-dev libegl1-mesa-dev libgl1-mesa-dev
  ffmpeg
)

case "${1:---check}" in
  --check)
    missing=0
    for cmd in rustc cargo rustup python3 pkg-config cmake ninja ffmpeg ffprobe; do
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
    echo "Installing Ubuntu/Debian build prerequisites through apt."
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
