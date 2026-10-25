#!/usr/bin/env bash
# CFA4 R0 GPU/WGPU hardware acceptance. Run locally on Ubuntu 26.04 AMD64.
# Unlike hosted CI, this gate FAILS if the 9 GPU-only FX tests cannot run.
# No system packages are installed and no GitHub Actions jobs are dispatched.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

die() { echo "CFA4 HARDWARE GPU AUDIT FAILED: $*" >&2; exit 1; }

# Validate an existing cargo test --nocapture transcript, without compiling
# again. The `0 failed` text in a SUCCESS summary MUST NOT be treated as a
# failure. Match actual FAILED results or explicit hardware-stage skips only.
check_fx_log() {
    local log="$1"
    [[ -r "$log" ]] || die "unreadable GPU FX test log: $log"
    grep -Fq 'CFA4_GPU_HARDWARE_ADAPTER:' "$log" || die "no physical GPU adapter recorded in $log"
    grep -Fq 'test result: ok. 12 passed; 0 failed; 0 ignored;' "$log" || die "expected 12 passing FX tests in $log"
    if grep -Eqi 'SKIPPED:|no GPU adapter; skipping|(^|[[:space:]])test result:[[:space:]]*FAILED([[:space:];.]|$)|^test fx_tests::[^[:space:]]+[[:space:]]+\.\.\.[[:space:]]+FAILED([[:space:]]|$)' "$log"; then
        die "skipped or failed GPU effect tests found in $log"
    fi
}

# The existing hardware trace can be rechecked after updating this script.
# This does not independently verify the recorded commit/host metadata.
if [[ "${1:-}" == "--verify-log" ]]; then
    [[ "$#" -eq 2 ]] || die "usage: $0 --verify-log PATH"
    check_fx_log "$2"
    echo "PASS: existing 12/12 physical GPU FX log has no skipped or failed tests"
    echo "Note: verify the companion hardware metadata and commit SHA separately."
    exit 0
fi
[[ "$#" -eq 0 ]] || die "usage: $0 [--verify-log PATH]"

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || die "Linux x86_64 (AMD64) is required"
[[ -f /etc/os-release ]] || die "cannot identify Ubuntu OS"
# shellcheck source=/dev/null
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] || die "Ubuntu 26.04 is required (found ${PRETTY_NAME:-unknown})"

for program in git cargo rustc vulkaninfo; do
    command -v "$program" >/dev/null 2>&1 || die "missing $program (install vulkan-tools for vulkaninfo)"
done
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || die "tracked working tree is not clean; commit or stash changes before collecting evidence"

# The strict test helper must be present in this exact source checkout. An older
# branch without it would silently report GPU effect tests as successful skips.
grep -Fq 'CFA4_REQUIRE_GPU_FX' crates/gpu/src/fx_tests.rs || die "strict GPU test prerequisite absent in this checkout"
grep -Fq 'CFA4_GPU_HARDWARE_ADAPTER:' crates/gpu/src/blend_tests.rs || die "strict physical Vulkan adapter probe absent"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUTPUT_DIR="${CFA4_GPU_AUDIT_DIR:-$ROOT/target/audit/gpu}"
mkdir -p "$OUTPUT_DIR"
METADATA="$OUTPUT_DIR/hardware-$STAMP.txt"
LOG="$OUTPUT_DIR/fx-parity-$STAMP.log"

{
    printf 'Date UTC: %s\n' "$(date -u --iso-8601=seconds)"
    printf 'Commit: %s\n' "$(git rev-parse HEAD)"
    printf 'Branch: %s\n' "$(git branch --show-current)"
    printf 'OS: %s\n' "${PRETTY_NAME:-unknown}"
    printf 'Kernel: %s\n' "$(uname -r)"
    printf 'Session: %s\n' "${XDG_SESSION_TYPE:-unknown}"
    rustc --version
    cargo --version
    if command -v nvidia-smi >/dev/null 2>&1; then
        nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
    fi
    vulkaninfo --summary
} >"$METADATA" 2>&1 || die "hardware preflight failed; inspect $METADATA"

echo "Hardware metadata: $METADATA"
echo "GPU FX test log: $LOG"
echo "Running only the 12 GPU FX/CPU-fallback tests, not the full workspace..."
if ! CFA4_REQUIRE_GPU_FX=1 WGPU_BACKEND=vulkan \
    cargo test --locked --release -p cfa4-gpu --lib fx_tests -- --nocapture --test-threads=1 \
    2>&1 | tee "$LOG"; then
    die "a strict GPU parity or fallback test failed; inspect $LOG"
fi

# No accepted failure or silent skip in the Rust test transcript.
check_fx_log "$LOG"

echo "PASS: 12 GPU FX parity/fallback tests on a physical Vulkan NVIDIA/AMD adapter"
echo "Attach the metadata and test log to CFA4 GPU audit issue #12."
