#!/usr/bin/env bash
# F2-01 #22: measure the runtime dependencies of the CFA4 release binaries on the
# Ubuntu 26.04 AMD64 reference machine (objdump/ldd/dpkg-query; installs nothing).
# Usage: scripts/dev/audit-runtime-deps.sh [--build]
#   --build  run `cargo build --release --locked -p cfa4 -p cfa4-cli` first
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
bash scripts/ci/require-ubuntu-2604.sh

if [[ "${1:-}" == "--build" ]]; then
    cargo build --release --locked -p cfa4 -p cfa4-cli
fi
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/release"
for name in cfa4 cfa4-cli; do
    [[ -x "$BIN/$name" ]] || {
        echo "Missing $BIN/$name: build it first, or rerun with --build." >&2
        exit 2
    }
done
for tool in objdump ldd dpkg-query; do
    command -v "$tool" >/dev/null || echo "warning: $tool not found; its part of the report stays empty" >&2
done

OUT="$ROOT/target/audit/runtime-deps/$(date -u +%Y%m%dT%H%M%SZ)"
PYTHONPATH="$ROOT/python${PYTHONPATH:+:$PYTHONPATH}" \
    python3 -m cfa4_devtools.runtime_deps "$BIN/cfa4" "$BIN/cfa4-cli" -o "$OUT"
echo "Send back: $OUT/runtime-deps.json (and runtime-deps.md)"
