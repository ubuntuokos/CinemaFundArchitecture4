# CFA4 fuzz targets

Coverage-guided fuzzing of every parser that reads untrusted bytes
([CFA4-SECURITY-001](../docs/plans/CFA4-SECURITY-001.md) B2): each L0 container and codec crate, and
the project, interchange (EDL, FCP XML, FCPXML, OTIO, AAF, OMF), caption, credits and graphics-template
readers. One harness per target in [`src/lib.rs`](src/lib.rs) (`TARGETS`), one libFuzzer entry point
per target in [`fuzz_targets/`](fuzz_targets/).

This directory is its own Cargo workspace (excluded from the main one): libFuzzer needs a nightly
toolchain and a C++ compiler, which the quality gates do not.

## Run

```sh
fuzz/seeds.sh                                  # seed corpus into fuzz/corpus/ (ffmpeg; gitignored)
cargo +nightly fuzz run isobmff fuzz/corpus/isobmff -- -max_total_time=300
fuzz/run.sh 60                                 # every target, 60 s each
fuzz/run.sh 300 h264 hevc av1                  # some targets
(cd fuzz && cargo test --lib --release)        # stable smoke run: seeds + 400 mutations per target
```

Tools: `rustup toolchain install nightly`, `cargo +nightly install cargo-fuzz --locked`. Builds use
debug assertions (overflow checks), so an arithmetic overflow is a finding even where the release build
would wrap.

The nightly workflow [`cfa4-fuzz.yml`](../.github/workflows/cfa4-fuzz.yml) runs every target for 5
minutes in six shards and keeps the growing corpus in the Actions cache; `cargo xtask fuzz-targets`
(its first job) fails when an L0 parser crate has no target.

## A finding

A crash, overflow, timeout (10 s) or out-of-memory (2 GiB) input lands in `fuzz/artifacts/<target>/`.
Reproduce it with `cargo +nightly fuzz run <target> <file>`, fix the parser, and add the input (or a
synthetic minimal one) as a regression test in the crate that panicked, one that fails before the fix
(AGENTS.md §0.8). Never commit the artifact or corpus files themselves.

## Adding a target

Add the harness function and its `TARGETS` row to `src/lib.rs`, then `fuzz_targets/<name>.rs`
(three lines, as the others) and its `[[bin]]` in `Cargo.toml`; add seeds to `seeds.sh` and,
for formats without an ffmpeg writer, a minimal built-in seed in `src/smoke.rs`.
