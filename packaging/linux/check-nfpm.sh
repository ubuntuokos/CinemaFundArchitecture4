#!/usr/bin/env bash
# Find the pinned nfpm (packaging/linux/nfpm.lock) and print its absolute path; refuse any other.
#   check-nfpm.sh            NFPM, else `nfpm` on PATH, else the install-nfpm.sh default location
# The SHA-256 is checked before the binary is ever run; only then its --version must name the
# pinned version and module hash. Exit 0 and the path on stdout, or exit 2 with the reason.
# Build the pinned binary with scripts/release/install-nfpm.sh (never downloaded by the packaging).
set -euo pipefail
LOCK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/nfpm.lock"
lock() { sed -n "s/^$1=\\([^[:space:]]*\\)\$/\\1/p" "$LOCK"; }
version="$(lock CFA4_NFPM_VERSION)"
module_sum="$(lock CFA4_NFPM_MODULE_SUM)"
expected="$(lock CFA4_NFPM_SHA256)"
if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ || ! "$expected" =~ ^[0-9a-f]{64}$ || "$module_sum" != h1:* ]]; then
  echo "error: $LOCK is incomplete" >&2
  exit 2
fi
default="${CFA4_TOOLS_DIR:-${HOME:-/nonexistent}/.cache/cfa4/tools}/nfpm-$version/nfpm"
tool="${NFPM:-$(command -v nfpm || true)}"
if [[ -z "$tool" && -x "$default" ]]; then tool="$default"; fi
if [[ -z "$tool" || ! -f "$tool" || ! -x "$tool" ]]; then
  echo "error: the pinned nfpm $version is not installed (scripts/release/install-nfpm.sh builds it; NFPM names it)" >&2
  exit 2
fi
tool="$(cd "$(dirname "$tool")" && pwd)/$(basename "$tool")"
actual="$(sha256sum -- "$tool" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  echo "error: nfpm SHA-256 mismatch for $tool ($actual, pinned $expected); refusing an unverified nfpm." >&2
  echo "       Build the pinned $version with scripts/release/install-nfpm.sh." >&2
  exit 2
fi
info="$("$tool" --version 2>&1 || true)"
if ! grep -Eq "^GitVersion:[[:space:]]+$version\$" <<<"$info" || ! grep -Fq "$module_sum" <<<"$info"; then
  echo "error: $tool does not report nfpm $version ($module_sum)" >&2
  exit 2
fi
echo "$tool"
