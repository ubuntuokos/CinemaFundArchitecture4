#!/usr/bin/env bash
# Regenerate the hash-pinned Python lock for Linux AMD64 (#6, F1-01).
# The lock covers the `dev` and `qt` extras of pyproject.toml; PySide6 wheels are manylinux_2_39.
set -euo pipefail
cd "$(dirname "$0")/../.."
if ! command -v uv >/dev/null 2>&1; then
  echo "lock-python: uv is required (https://docs.astral.sh/uv/)" >&2
  exit 2
fi
out=requirements/linux-amd64.txt
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
# uv keeps the pins of an existing output file unless --upgrade/--upgrade-package is given.
if [[ -f "$out" ]]; then cp "$out" "$tmp"; fi
uv pip compile pyproject.toml --extra dev --extra qt --generate-hashes \
  --python-version 3.12 --python-platform x86_64-manylinux_2_39 --no-header -o "$tmp" "$@"
{
  echo "# Hash-pinned Python dependencies for CFA4 on Linux AMD64 (pyproject extras: dev, qt)."
  echo "# Regenerate with: bash scripts/dev/lock-python.sh   (add --upgrade to move versions)"
  echo "# Install: python -m pip install --require-hashes -r $out && python -m pip install --no-deps -e ."
  cat "$tmp"
} >"$out"
echo "Wrote $out"
