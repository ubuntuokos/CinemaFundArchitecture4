#!/usr/bin/env bash
# Regenerate the hash-pinned Python locks for Linux AMD64 (#6, F1-01; OTIO-1.3).
# requirements/linux-amd64.txt covers the `dev` and `qt` extras of pyproject.toml; PySide6 wheels are
# manylinux_2_39. requirements/otio-oracle.txt covers the `otio` extra: the OpenTimelineIO test oracle
# only (never imported by the product), installed by the CI `otio-oracle` job next to the main lock.
set -euo pipefail
cd "$(dirname "$0")/../.."
if ! command -v uv >/dev/null 2>&1; then
  echo "lock-python: uv is required (https://docs.astral.sh/uv/)" >&2
  exit 2
fi

# lock <output> <description> <install suffix> <extra>... -- <uv args>...
lock() {
  local out=$1 what=$2 then=$3
  shift 3
  local extras=()
  while [[ $# -gt 0 && $1 != -- ]]; do extras+=(--extra "$1"); shift; done
  [[ ${1:-} == -- ]] && shift
  local tmp
  tmp=$(mktemp)
  # uv keeps the pins of an existing output file unless --upgrade/--upgrade-package is given.
  if [[ -f "$out" ]]; then cp "$out" "$tmp"; fi
  uv pip compile pyproject.toml "${extras[@]}" --generate-hashes \
    --python-version 3.12 --python-platform x86_64-manylinux_2_39 --no-header -o "$tmp" "$@"
  {
    echo "# Hash-pinned Python dependencies for CFA4 on Linux AMD64 ($what)."
    echo "# Regenerate with: bash scripts/dev/lock-python.sh   (add --upgrade to move versions)"
    echo "# Install: python -m pip install --require-hashes -r $out$then"
    cat "$tmp"
  } >"$out"
  rm -f "$tmp"
  echo "Wrote $out"
}

lock requirements/linux-amd64.txt "pyproject extras: dev, qt" " && python -m pip install --no-deps -e ." dev qt -- "$@"
lock requirements/otio-oracle.txt "pyproject extra: otio, the OpenTimelineIO test oracle only" "" otio -- "$@"
