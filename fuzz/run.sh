#!/usr/bin/env bash
# Run CFA4 fuzz targets with libFuzzer for a fixed time each (CFA4-SECURITY-001 B2).
# Usage: fuzz/run.sh [SECONDS_PER_TARGET] [TARGET...]   (default 60 s, every target)
# Needs: rustup nightly toolchain, cargo-fuzz (`cargo install cargo-fuzz`), a C++ compiler.
# Seeds come from fuzz/seeds.sh (run it first; missing seeds only make a target start from scratch).
# Crashes land in fuzz/artifacts/<target>/; the script exits 1 when any target found one.
# CFA4_FUZZ_CLEAN=1 deletes each target's binary after its run (CI disk space).
set -uo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
SECS="${1:-60}"
shift || true
if [[ $# -gt 0 ]]; then
    TARGETS=("$@")
else
    mapfile -t TARGETS < <(sed -n 's/^name = "\([a-z0-9_]*\)"$/\1/p' fuzz/Cargo.toml | grep -v '^cfa4-fuzz$')
fi
TRIPLE="$(rustc +nightly -vV | sed -n 's/^host: //p')"
# The workspace denies deprecated APIs (Cargo.toml [workspace.lints], CFA4-LINTS-001) against the
# pinned stable toolchain; nightly deprecates earlier (e.g. `fetch_update` → `try_update`), so lints
# only warn in the nightly fuzz build: the stable gates still enforce them.
export RUSTFLAGS="${RUSTFLAGS:-} --cap-lints=warn"
failed=()
for t in "${TARGETS[@]}"; do
    mkdir -p "fuzz/corpus/$t" "fuzz/artifacts/$t"
    echo "::group::fuzz $t (${SECS}s)"
    if ! cargo +nightly fuzz run --debug-assertions "$t" "fuzz/corpus/$t" -- \
        -max_total_time="$SECS" -timeout=10 -rss_limit_mb=2048 -max_len=1048576 \
        -artifact_prefix="fuzz/artifacts/$t/" -print_final_stats=1; then
        failed+=("$t")
    fi
    echo "::endgroup::"
    if [[ "${CFA4_FUZZ_CLEAN:-0}" == 1 ]]; then
        rm -f "fuzz/target/$TRIPLE/release/$t"
    fi
done
if [[ ${#failed[@]} -gt 0 ]]; then
    echo "fuzz: ${#failed[@]} target(s) failed: ${failed[*]}" >&2
    ls -1 fuzz/artifacts/*/ 2>/dev/null >&2
    exit 1
fi
echo "fuzz: ${#TARGETS[@]} target(s) ran ${SECS}s each without a finding"
