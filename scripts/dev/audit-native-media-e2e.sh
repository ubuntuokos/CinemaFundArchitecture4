#!/usr/bin/env bash
# R0 #16: physical Ubuntu 26.04/Wayland native editor media audit.
# Deliberately NO automatic full Rust workspace, package installation or CI.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || {
    echo "CFA4 R0 native E2E requires Linux AMD64" >&2
    exit 2
}
# Optional explicit request for the TWO ignored hardware-focused Rust tests.
# This compiles only cfa4-platform's chosen test target if needed.
if [[ "${1:-}" == "--rust-gates" ]]; then
    shift
    export CFA4_MEDIA_FIXTURE_DIR="${CFA4_MEDIA_FIXTURE_DIR:-$ROOT/target/audit/ffmpeg/20261022T153037Z}"
    export CFA4_NATIVE_E2E_OUTPUT_DIR="${CFA4_NATIVE_E2E_OUTPUT_DIR:-$ROOT/target/audit/native-media/rust-oracles}"
    mkdir -p "$CFA4_NATIVE_E2E_OUTPUT_DIR"
    echo "Running only CFA4 native NVDEC/NVENC Linux oracles (no full release workspace)..."
    cargo test --locked -p cfa4-platform --test r0_native_media_e2e \
        -- --ignored --nocapture --test-threads=1 2>&1 |
        tee "$CFA4_NATIVE_E2E_OUTPUT_DIR/r0-native-rust.log"
    if grep -Fq 'SKIPPED:' "$CFA4_NATIVE_E2E_OUTPUT_DIR/r0-native-rust.log"; then
        echo "Native Rust media audit cannot count a skipped or empty test as PASS" >&2
        exit 1
    fi
    grep -Fq 'test result: ok. 2 passed; 0 failed; 0 ignored;' \
        "$CFA4_NATIVE_E2E_OUTPUT_DIR/r0-native-rust.log" || {
        echo "Native Rust media audit needs two real passing oracles" >&2
        exit 1
    }
    echo "PASS: two strict physical CFA4 Rust NVDEC/NVENC media oracles"
fi

PYTHONPATH="$ROOT/python${PYTHONPATH:+:$PYTHONPATH}" \
    python3 -m cfa4_devtools.native_media_e2e "$@"
