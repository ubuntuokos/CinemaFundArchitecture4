# CFA4 — instructions for agents

**CFA4 migration notice (overrides inherited platform/frontend instructions):** CFA4 is a
Linux-only derivative of an upstream open-source video editor (see `NOTICE`). Rust/Cargo remain the media engine; Python and
PySide6/Qt 6 are supported development components. Windows/macOS/FreeBSD/WASM release
targets are excluded. See `docs/development/linux.md` and the CFA4-specific top
section of `AGENTS.md`. The following instructions are inherited upstream
engineering context; references to pure-Rust-only frontends, upstream release processes
or macOS/Windows targets do not override this CFA4 direction.

## Start every session here
`plan/` is maintainer-local (gitignored, not in the repo). Each step names the public equivalent to use if you don't have it.
1. Read `plan/STATUS.md` (current milestone, next unchecked task, blockers). Public: [`ROADMAP.md`](ROADMAP.md), then [`docs/gaps.md`](docs/gaps.md) (ranked shortfalls) and [`docs/roadmap.md`](docs/roadmap.md) (current focus).
2. Read the task in `plan/execution-plan.md` §3, the relevant section of `plan/architecture.md`, and the README/docs of the crate you touch. Public: [`docs/architecture.md`](docs/architecture.md), [`docs/contributing.md`](docs/contributing.md), [`docs/testing.md`](docs/testing.md). Visual/behaviour reference: `plan/premiere/` (local only, never committed).
3. Follow the autonomous operation protocol (`plan/execution-plan.md` §7; public summary: [`docs/agents.md`](docs/agents.md) §5–7). Don't stop to ask unless it lists the decision as the user's.

## Non-negotiables
**Read [`AGENTS.md`](AGENTS.md) first; its rules override everything here.**
- **Never crash** ([`AGENTS.md`](AGENTS.md) §0). People depend on CFA4; a crash loses their work, so this outranks feature work. No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unimplemented!` or `unsafe` outside tests: return `Result<T, E>` and propagate with `?` (`ok_or`, `let … else`, `if let`). Treat every number from media, project files, presets or CLI/MCP/control params as hostile: `get()` instead of `[i]`, checked/saturating math, no division by possibly-zero values, capped allocations, bounded recursion. `lock().unwrap_or_else(PoisonError::into_inner)`; background jobs run under `catch_unwind`. The panic hook + UI `catch_unwind` (`crates/ui-egui/src/crash.rs`) are the last-resort guard, not a licence. Every crash fix gets a regression test that panicked before. In particular (§1): never use Adobe iconography, images or any other Adobe asset. Every asset must be openly licensed (OSS / public domain / Creative Commons, or original work by a contributor) and must have a `<file>.attribution` sidecar plus an entry in `ATTRIBUTION.md`. `cargo xtask assets` enforces this.
- **Fonts** live in an external font directory, never in this repo (no new font files here; [`AGENTS.md`](AGENTS.md) §1.9). Build with them: `CFA4_FONTS_DIR=/path/to/extra-fonts cargo run -p cfa4` (`crates/text/build.rs` embeds them as `cfa4_text::fonts::CFA4_FONTS`, used for Japanese in the UI and titles). Unset, `CFA4_FONTS` is empty and everything must still build, test and run; never add extra-fonts to a `Cargo.toml`. Tests on its glyphs skip when it is empty, so run the gates with and without `CFA4_FONTS_DIR` when you touch fonts.
- **Clean-room.** Never read/disassemble anything inside Adobe app bundles (names/listings only). Never copy Adobe icons, presets, LUTs, fonts. CFA4 is GPL-3.0-or-later: GPL-3.0-compatible code may be reused with intact headers and a register entry (`docs/licensing.md` §3, §5); never GPL-2.0-only (HandBrake), AGPL or proprietary code; dependencies must pass `cargo deny check licenses`. ffmpeg/ffprobe run only as external test oracles / fixture generators — never linked or shipped.
- **Pure Rust** in the product. OS media APIs only via Rust bindings in `crates/platform` (hardware decoding; the one crate allowed `unsafe`, see `AGENTS.md` §0.3 and `docs/adr/0001-platform-ffi.md`), optional, behind traits, with our decoders as the fallback. **CFA4 exception:** the optional GStreamer backend `crates/gstreamer` (safe gstreamer-rs API, no `unsafe`, feature `gstreamer` off by default, release builds without it; [`docs/adr/ADR-0002-gstreamer-backend.md`](docs/adr/ADR-0002-gstreamer-backend.md)). Build/test it with `cargo test -p cfa4-gstreamer --features gstreamer` (needs `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`).
- **Layering** ([`docs/architecture.md`](docs/architecture.md) §1, enforced by `cargo xtask layers`): nothing below L5 depends on egui/eframe/winit/rfd/cpal. L0 codec/container crates depend only on `cfa4-bitstream`.
- **Exact time:** all time is `cfa4_time::Tick` (254 016 000 000/s). Never use f64 seconds for edit math.
- **Everything is a command** (`crates/engine`): id, label, menu path, shortcut, params, enabled(), run(). UI, CLI, control channel and MCP all dispatch by id.
- **Everything is agent-drivable:** every interactive widget registers an automation id; UI state is serde so the control channel can read/set it.
- **Quality gates** before every commit: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo xtask layers`, `cargo xtask assets`, `cargo xtask wasm` (checks L0–L4 crates). `cargo xtask ci` runs them all.
- **Commits:** one task id per commit (`M2.4: CABAC residual decoding`). Only green states. End messages with the attribution line required by the environment.

## Running and looking at the app
- `cargo run -p cfa4 -- --control 9876` opens the desktop app with the JSON-lines control server (see `docs/control-protocol.md`).
- For UI work, **look at the result**: drive via the control channel and take `ui.screenshot`, compare with `plan/premiere/screenshots/` (maintainer-local; see [`docs/agents.md`](docs/agents.md) §3).
- Parallel agents: separate git worktrees and `CARGO_TARGET_DIR=target/agent-<name>`; keep every `Cargo.toml` valid at all times (the `crates/*` glob means one broken manifest breaks everyone).
- Test fixtures: generate with ffmpeg into `target/fixtures/` (never commit media). See [`docs/testing.md`](docs/testing.md).
