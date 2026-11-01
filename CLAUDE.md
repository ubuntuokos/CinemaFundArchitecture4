# CFA4 — instructions for agents

**CFA4 notice:** CFA4 is a Linux-only derivative of an upstream open-source video editor (see `NOTICE`).
Platform, licence and backend rules are in the CFA4 section at the top of [`AGENTS.md`](AGENTS.md); setup is in
[`docs/development/linux.md`](docs/development/linux.md). The inherited engineering context below does not
override them (pure-Rust-only frontends, upstream release processes, macOS/Windows/WASM targets).

## Start every session here
`plan/` is maintainer-local (gitignored, not in the repo). Each step names the public equivalent to use if you don't have it.
1. Read `plan/STATUS.md` (current milestone, next unchecked task, blockers). Public: [`ROADMAP.md`](ROADMAP.md), then [`docs/gaps.md`](docs/gaps.md) (ranked shortfalls) and [`docs/roadmap.md`](docs/roadmap.md) (current focus).
2. Read the task in `plan/execution-plan.md` §3, the relevant section of `plan/architecture.md`, and the README/docs of the crate you touch. Public: [`docs/architecture.md`](docs/architecture.md), [`docs/contributing.md`](docs/contributing.md), [`docs/testing.md`](docs/testing.md). Visual/behaviour reference: `plan/premiere/` (local only, never committed).
3. Follow the autonomous operation protocol (`plan/execution-plan.md` §7; public summary: [`docs/agents.md`](docs/agents.md) §5–7) **inside your own project** (owner decision, 2026-10-31): the owner-approved plan, ADR or issue your session was started on. Within it, pick the next task yourself in a logical order; **never take a task from another project**. Report an overlap or conflict with another project to the merge coordinator ([`docs/coordination.md`](docs/coordination.md) §2.8); the owner and the coordinator decide together, and that decision stands. Otherwise don't stop to ask unless it lists the decision as the user's.

## Non-negotiables
**Read [`AGENTS.md`](AGENTS.md) first; its rules override everything here.**
- **The rules in [`AGENTS.md`](AGENTS.md)**, each kept only there: never crash (§0; outranks feature work), assets
  and attribution (§1; no Adobe assets, `cargo xtask assets`), fonts (§1.9), clean-room (§2), and licence, media
  backends, GStreamer and GPU vendor neutrality (CFA4 section at the top).
- **Fonts, in practice:** `CFA4_FONTS_DIR=/path/to/extra-fonts cargo run -p cfa4` (used for Japanese in the UI
  and titles). Tests on its glyphs skip when it is empty, so run the gates with and without `CFA4_FONTS_DIR` when
  you touch fonts.
- **Pure Rust** in the product. OS media APIs only via Rust bindings in `crates/platform` (`AGENTS.md` §0.3,
  `docs/adr/0001-platform-ffi.md`), optional, behind traits, with our decoders as the fallback. The exceptions
  (FFmpeg, GStreamer via the `cfa4-route` router) are in the `AGENTS.md` CFA4 section. GStreamer build/test:
  `cargo test -p cfa4-gstreamer --features gstreamer` (needs `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`).
- **Layering** ([`docs/architecture.md`](docs/architecture.md) §1, enforced by `cargo xtask layers`): nothing below L5 depends on egui/eframe/winit/rfd/cpal. L0 codec/container crates depend only on `cfa4-bitstream`.
- **Exact time:** all time is `cfa4_time::Tick` (254 016 000 000/s). Never use f64 seconds for edit math.
- **Everything is a command** (`crates/engine`): id, label, menu path, shortcut, params, enabled(), run(). UI, CLI, control channel and MCP all dispatch by id.
- **Everything is agent-drivable:** every interactive widget registers an automation id; UI state is serde so the control channel can read/set it.
- **Quality gates** before every commit: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo xtask layers`, `cargo xtask assets`. `cargo xtask ci` runs all of them except the tests (run `cargo test --workspace` separately). `cargo xtask wasm` is opt-in legacy, not a gate (WASM is not a CFA4 target; [`docs/development/linux.md`](docs/development/linux.md)).
- **Python / Qt:** environment setup in [`docs/development/linux.md`](docs/development/linux.md) (`.venv`, `--require-hashes`). Checks as in CI: `.venv/bin/python -m ruff check python` and `.venv/bin/python -m pytest -q python/tests --ignore=python/tests/test_qt_gui.py`; the Qt GUI tests (`test_qt_gui.py`) run but don't block until the Qt6 GUI is finished ([`docs/coordination.md`](docs/coordination.md) §3.4).
- **Commits:** one task id per commit (`M2.4: CABAC residual decoding`). Only green states. End messages with the attribution line required by the environment.

## Running and looking at the app
- `cargo run -p cfa4 -- --control 9876` opens the desktop app with the JSON-lines control server (see `docs/control-protocol.md`).
- For UI work, **look at the result**: drive via the control channel and take `ui.screenshot`, compare with `plan/premiere/screenshots/` (maintainer-local; see [`docs/agents.md`](docs/agents.md) §3).
- Usage budget (owner decision 2026-10-31, #367): the project model is `opusplan` (`.claude/settings.json`). Start any task larger than a small fix in plan mode, so planning runs on Opus and execution on Sonnet. Run quality gates through the `cfa4-ci-runner` subagent and branch inventories through `cfa4-branch-lister` (both Haiku, read-only, `.claude/agents/`). PR evidence stays the unfiltered gate log.
- Parallel agents: separate git worktrees and `CARGO_TARGET_DIR=target/agent-<name>`; keep every `Cargo.toml` valid at all times (the `crates/*` glob means one broken manifest breaks everyone).
- Test fixtures: generate with ffmpeg into `target/fixtures/` (never commit media). See [`docs/testing.md`](docs/testing.md).
