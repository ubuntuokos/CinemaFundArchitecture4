#!/usr/bin/env bash
# CFA4-SECURITY-001 B0: measure what Landlock + seccomp confinement costs, on the reference machine
# (docs/development/reference-machine.md). Installs nothing; writes a report under
# target/audit/security-overhead/<time>/.
# Usage: scripts/dev/security-overhead.sh [CORES] [FILES] [ROUNDS]
#   CORES   cores to pin to (default 0-7: eight physical cores on NUMA node 0 of the 88-thread machine)
#   FILES   small files per round (default 2000)
#   ROUNDS  rounds per child process (default 20)
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
CORES="${1:-0-7}"
FILES="${2:-2000}"
ROUNDS="${3:-20}"

OUT="$ROOT/target/audit/security-overhead/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT"
cargo build --release --locked -p cfa4-sandbox --example overhead
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/release/examples/overhead"

# The measurement record every report needs (reference-machine.md): CPU, pinned cores, load.
{
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "kernel: $(uname -r)"
    echo "cores: $CORES"
    echo "load: $(cut -d' ' -f1-3 /proc/loadavg)"
    echo "git: $(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
} >"$OUT/record.txt"
lscpu >"$OUT/lscpu.txt" 2>/dev/null || echo "lscpu not available" >"$OUT/lscpu.txt"

if command -v numactl >/dev/null; then
    numactl --cpunodebind=0 --membind=0 taskset -c "$CORES" "$BIN" "$FILES" "$ROUNDS" >"$OUT/overhead.json"
else
    taskset -c "$CORES" "$BIN" "$FILES" "$ROUNDS" >"$OUT/overhead.json"
fi
python3 - "$OUT/overhead.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
s = r["support"]
print(f"kernel {s['kernel']}, Landlock ABI {s['landlockAbi']}, seccomp {s['seccomp']}, AppArmor {s['apparmor']}")
for c in r["summary"]:
    print(f"{c['case']:>11}: apply {c['applyUs']:8.1f} µs, {c['nsPerFile']:9.1f} ns/file, overhead {c['overheadPct']:+6.2f}%")
PY
echo "Send back: $OUT (overhead.json, record.txt, lscpu.txt)"
