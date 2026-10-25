#!/usr/bin/env bash
# Developer validation; never claim success when prerequisites or tests are missing.
set -euo pipefail
cd "$(dirname "$0")/../.."

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 supports Ubuntu 26.04 Linux x86_64 (AMD64) only." >&2
  exit 2
fi
. scripts/ci/require-ubuntu-2604.sh

if [[ ! -x .venv/bin/python ]]; then
  echo "Missing .venv. Run: python3 -m venv .venv && .venv/bin/python -m pip install -e '.[dev,qt]'" >&2
  exit 2
fi

.venv/bin/python -m cfa4_devtools.health --strict
.venv/bin/python -m compileall -q python
.venv/bin/python -m pytest -q python/tests
.venv/bin/python -m ruff check python
QT_QPA_PLATFORM=offscreen .venv/bin/python -m cfa4_devtools.qt_app --smoke
CFA4_REQUIRE_ORACLES=1 cargo xtask ci

echo "CFA4: Linux developer checks passed."
