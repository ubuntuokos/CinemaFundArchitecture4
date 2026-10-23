#!/usr/bin/env bash
# Read-only CFA4 Ubuntu 26.04 AMD64 packaging prerequisites: no builds/downloads.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FORMATS="deb appimage flatpak tar"
if (( $# )); then
  if [[ $# == 2 && "$1" == "--formats" ]]; then
    FORMATS="$2"
  else
    echo "Usage: $0 [--formats 'deb appimage flatpak tar']" >&2
    exit 2
  fi
fi
if [[ -z "$FORMATS" ]]; then
  echo "error: no CFA4 package formats requested" >&2
  exit 2
fi
set -f
for format in $FORMATS; do
  case "$format" in
    deb|appimage|flatpak|tar) ;;
    *) echo "error: unsupported CFA4 package format: $format" >&2; exit 2 ;;
  esac
done
has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

bash "$ROOT/scripts/ci/require-ubuntu-2604.sh"
missing=0
fail() { echo "MISSING: $*" >&2; missing=$((missing + 1)); }
check_command() {
  if command -v "$1" >/dev/null 2>&1; then
    echo "OK: $1"
  else
    fail "reviewed tool '$1' is not installed"
  fi
}

for tool in tar python3 sha256sum; do
  check_command "$tool"
done
if has tar || has deb || has appimage; then
  for tool in cargo rustc install strip sed; do
    check_command "$tool"
  done
fi
if has deb; then
  check_command nfpm
  check_command dpkg-deb
fi
if has flatpak; then
  check_command flatpak
  check_command flatpak-builder
  check_command ostree
  if command -v flatpak >/dev/null 2>&1; then
    for name in org.freedesktop.Platform org.freedesktop.Sdk; do
      ref="$name//25.08"
      if flatpak info --arch=x86_64 --show-ref "$ref" >/dev/null 2>&1; then
        echo "OK: preinstalled $ref"
      else
        fail "Flatpak $ref is missing; provision reviewed runtime and SDK beforehand"
      fi
    done
  fi
fi
if has appimage; then
  tool="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [[ -z "$tool" || ! -f "$tool" || ! -x "$tool" ]]; then
    fail "reviewed appimagetool not executable (set APPIMAGETOOL)"
  fi
  expected="${CFA4_APPIMAGETOOL_SHA256:-}"
  if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
    fail "CFA4_APPIMAGETOOL_SHA256 must be an independently approved 64-hex SHA-256"
  elif [[ -n "$tool" && -f "$tool" && -x "$tool" ]] && command -v sha256sum >/dev/null; then
    actual="$(sha256sum -- "$tool" | awk '{print $1}')"
    if [[ "$actual" == "${expected,,}" ]]; then
      echo "OK: pinned local appimagetool checksum"
    else
      fail "appimagetool SHA-256 mismatch; refusing unverified tool"
    fi
  fi
fi

if (( missing )); then
  echo "CFA4 local four-format build BLOCKED: $missing missing prerequisite(s)." >&2
  echo "No dependencies were installed and no packages were built." >&2
  exit 2
fi
echo "PASS_PREREQUISITES_ONLY: reviewed local CFA4 Ubuntu 26.04 AMD64 tools detected."
echo "RELEASE HOLD: actual builds, install/runtime tests and legal approval remain separate."
