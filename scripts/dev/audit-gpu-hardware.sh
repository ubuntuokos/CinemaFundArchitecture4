#!/usr/bin/env bash
# CFA4 R0 GPU/WGPU hardware acceptance. Run locally on Ubuntu 26.04 AMD64.
# Unlike hosted CI, this gate FAILS if the 9 GPU-only FX tests cannot run.
# No system packages are installed and no GitHub Actions jobs are dispatched.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

die() { echo "CFA4 HARDWARE GPU AUDIT FAILED: $*" >&2; exit 1; }

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
    cargo test --locked --release -p filmcraft-gpu --lib fx_tests -- --nocapture --test-threads=1 \
    2>&1 | tee "$LOG"; then
    die "a strict GPU parity or fallback test failed; inspect $LOG"
fi

# The exact test count is deliberately pinned: disappearing tests, hidden
# skips, or missing physical adapter evidence must not turn into a green audit.
grep -Fq 'CFA4_GPU_HARDWARE_ADAPTER:' "$LOG" || die "no physical adapter recorded in $LOG"
grep -Fq 'test result: ok. 12 passed; 0 failed; 0 ignored;' "$LOG" || die "expected 12 passing FX tests; inspect $LOG"
if grep -Eqi 'SKIPPED:|no GPU adapter; skipping|test result:.*failed' "$LOG"; then
    die "skipped or failed GPU effects found in $LOG"
fi

echo "PASS: 12 GPU FX parity/fallback tests on a physical Vulkan NVIDIA/AMD adapter"
echo "Attach the metadata and test log to CFA4 GPU audit issue #12."
