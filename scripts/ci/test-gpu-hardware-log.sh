#!/usr/bin/env bash
# Offline regression coverage for the physical GPU audit's log parser.
# No Rust compile, Vulkan runtime, or GPU is required.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECKER="$ROOT/scripts/dev/audit-gpu-hardware.sh"
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

cat >"$TMP/ok.log" <<'EOF'
running 12 tests
test fx_tests::gpu_effects_match_cpu_exactly ... CFA4_GPU_HARDWARE_ADAPTER: name=NVIDIA RTX A1000 backend=Vulkan device_type=DiscreteGpu vendor=0x10de
ok
test fx_tests::layers_with_effects_fall_back_to_the_cpu_without_a_stage ... ok
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 29 filtered out; finished in 3.76s
EOF

expect_pass() {
    if ! bash "$CHECKER" --verify-log "$1" >"$TMP/stdout" 2>"$TMP/stderr"; then
        echo "GPU log parser incorrectly rejected a valid trace:" >&2
        cat "$TMP/stderr" >&2
        exit 1
    fi
}

expect_fail() {
    if bash "$CHECKER" --verify-log "$1" >"$TMP/stdout" 2>"$TMP/stderr"; then
        echo "GPU log parser incorrectly accepted an invalid trace: $1" >&2
        exit 1
    fi
}

expect_pass "$TMP/ok.log"

cp "$TMP/ok.log" "$TMP/skipped.log"
echo 'SKIPPED: this adapter cannot create the GPU Rgba32Float effect stage' >> "$TMP/skipped.log"
expect_fail "$TMP/skipped.log"

cp "$TMP/ok.log" "$TMP/no_adapter.log"
sed -i '/CFA4_GPU_HARDWARE_ADAPTER:/d' "$TMP/no_adapter.log"
expect_fail "$TMP/no_adapter.log"

cp "$TMP/ok.log" "$TMP/count.log"
sed -i 's/test result: ok. 12 passed; 0 failed;/test result: ok. 11 passed; 0 failed;/' "$TMP/count.log"
expect_fail "$TMP/count.log"

cp "$TMP/ok.log" "$TMP/failed_result.log"
echo 'test result: FAILED. 11 passed; 1 failed; 0 ignored;' >> "$TMP/failed_result.log"
expect_fail "$TMP/failed_result.log"

cp "$TMP/ok.log" "$TMP/failed_case.log"
echo 'test fx_tests::gpu_effects_match_cpu_exactly ... FAILED' >> "$TMP/failed_case.log"
expect_fail "$TMP/failed_case.log"

cp "$TMP/ok.log" "$TMP/no_gpu.log"
echo 'no GPU adapter; skipping' >> "$TMP/no_gpu.log"
expect_fail "$TMP/no_gpu.log"

expect_fail "$TMP/nonexistent.log"

echo 'PASS: GPU hardware audit log parser accepts 0 failed and rejects missing adapters, skips and real test failures.'
