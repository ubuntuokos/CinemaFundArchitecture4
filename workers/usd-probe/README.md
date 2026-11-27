# cfa4-usd-probe (G-USD-1 probe worker, S5)

Separate-process OpenUSD probe (C++, TOST-1.0 library) for the sandbox and fuzz gate SB-G6 of
`docs/plans/CFA4-OPENUSD-SOURCEBUILD-001-0.1.md`. It is **not** the U2 worker: it only inspects a stage (up axis, units, time
range, prim count and depth, root prims, layer count) and writes small test scenes. The Rust side is `crates/usd-probe`
(`cfa4-usd-probe`): it runs one command per process, bounds time, memory and output, and turns every worker failure into an `Err`.

Build (optional, never part of `cargo` or CI; nothing is downloaded):

    scripts/dev/fetch-usd-source.sh onetbb --yes && scripts/dev/fetch-usd-source.sh usd --yes
    scripts/dev/build-usd.sh                                   # target/usd-install, about 12 minutes on 4 cores
    cmake -S workers/usd-probe -B target/usd-probe -G Ninja -DCFA4_USD_PREFIX=$PWD/target/usd-install
    cmake --build target/usd-probe
    export CFA4_USD_PROBE=$PWD/target/usd-probe/cfa4-usd-probe CFA4_USD_ROOT=$PWD/target/usd-install
    cargo test -p cfa4-usd-probe --release
    CFA4_USD_FUZZ_ITERS=7500 CFA4_USD_FUZZ_ITERS_CONFINED=2500 cargo test -p cfa4-usd-probe --release hostile

Without the probe the real-worker tests return early (`Error::Unavailable`).

## SB-G6 status (2026-11-27, this container, OpenUSD 26.11.1)
| Criterion | State |
|---|---|
| (1) mutation fuzz, USDA/USDC/USDZ seeds | done: 7 500 mutated files (truncation, byte flips, 8 bytes of 0xFF as wrong sizes) unconfined and 2 500 confined; the host never panicked or hung, every worker failure was an `Err`. No finding, so no regression test was added. Not coverage-guided (libFuzzer/AFL++ is an open option, see plan §10) |
| (2) memory and time limits | done: `RLIMIT_AS` (`with_memory_limit_mb`, default 4096) and the host time limit both give an `Err` (tests) |
| (3) Landlock + seccomp | done: the fake probe shows no read outside the grant, no network, no program start; the real probe runs confined (inspect of .usda/.usdc/.usdz) and a reference out of the file's folder is not composed (4 prims unconfined, 2 confined) |
| (4) recursion and depth | done for what the worker controls: it stops at 1 000 000 prims or depth 512 and reports `truncated` (700-deep test); a sublayer cycle is cut by USD itself (warning on stderr, the stage opens). Deep reference chains and instancing blow-ups are only bounded by the memory and time limits, not by a count |
| (5) fuzz findings as tests | none to add |

Limits of this result: one machine, one toolchain (gcc 13.3); the plugin path is locked by the host (empty environment) but a
hostile `plugInfo.json` inside the install cannot be tested here; **writing** under the sandbox is not designed (USD writes through a
temporary file next to the output, which the single-file output grant refuses), so test files are written unconfined; USD
prints warnings and errors on stderr, which the host discards.
