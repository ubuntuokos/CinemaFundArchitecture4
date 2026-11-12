# CFA4 — instructions for agents

**CFA4 notice:** CFA4 is a Linux-only derivative of an upstream open-source video editor (see `NOTICE`).
Platform, licence and backend rules are in the CFA4 section at the top of [`AGENTS.md`](AGENTS.md); setup is in
[`docs/development/linux.md`](docs/development/linux.md); the owner's reference machine (hardware, OS, GPUs, measurement
notes) is in [`docs/development/reference-machine.md`](docs/development/reference-machine.md). The inherited engineering context below does not
override them (pure-Rust-only frontends, upstream release processes, macOS/Windows/WASM targets). The editor GUI is the
Qt 6 / PySide6 frontend in `python/cfa4_qt/` ([`docs/qt-gui.md`](docs/qt-gui.md)); all new UI work goes there. The egui
frontend (`crates/ui-egui`) stays only until the Qt6 GUI does everything it does ([`docs/qt-parity.md`](docs/qt-parity.md)).

## Start every session here
`plan/` is maintainer-local (gitignored, not in the repo). Each step names the public equivalent to use if you don't have it.
0. Read [`docs/development/reference-machine.md`](docs/development/reference-machine.md) (the owner's reference machine and measurement rules; [`AGENTS.md`](AGENTS.md), owner decision 2026-11-02). Never ask the owner for machine data.
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
- **Rust first** in the product. A non-Rust component (C, C++…) is allowed unless it slows CFA4 down, and may be ported to Rust step by step in later releases (owner decision 2026-11-07, `AGENTS.md` CFA4 section). OS media APIs only via Rust bindings in `crates/platform` (`AGENTS.md` §0.3,
  `docs/adr/0001-platform-ffi.md`), optional, behind traits, with our decoders as the fallback. The exceptions
  (FFmpeg, GStreamer via the `cfa4-route` router) are in the `AGENTS.md` CFA4 section. GStreamer build/test:
  `cargo test -p cfa4-gstreamer --features gstreamer` (needs `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`).
- **Layering** ([`docs/architecture.md`](docs/architecture.md) §1, enforced by `cargo xtask layers`): nothing below L5 depends on egui/eframe/winit/rfd/cpal. L0 codec/container crates depend only on `cfa4-bitstream`. The Qt6 GUI is outside the Rust crates: it drives the engine only through `cfa4-cli serve` ([`docs/qt-gui.md`](docs/qt-gui.md)).
- **Exact time:** all time is `cfa4_time::Tick` (254 016 000 000/s). Never use f64 seconds for edit math.
- **Calendar time goes through `chrono`** (workspace dependency; owner decision 2026-11-03). Never hand-roll UTC/calendar conversion (days-to-date algorithms, leap-year math); a crate owns only its output format string. The one exception is range extension: past the years chrono can represent, shift the date by whole 400-year Gregorian cycles (146 097 days, after which the calendar repeats), let chrono compute the month and day, and add the cycles back to the year.
- **Everything is a command** (`crates/engine`): id, label, menu path, shortcut, params, enabled(), run(). UI, CLI, control channel and MCP all dispatch by id.
- **Everything is agent-drivable:** every interactive widget registers an automation id; UI state is serde so the control channel can read/set it.
- **Quality gates** before every commit, as CI runs them: `cargo xtask ci` (= `cargo xtask fmt-check`, `cargo clippy --workspace --exclude cfa4-ui --all-targets --release -- -D warnings`, `cargo xtask layers`, `cargo xtask assets`), then the tests separately: `cargo test --workspace --exclude cfa4-ui --release`. The vendored `cfa4-ui` has its own test job (`scripts/dev/test-vendored-ui.sh`); `cargo xtask fmt-check` skips the vendor snapshots that plain `cargo fmt --check` would touch. `cargo xtask wasm` is opt-in legacy, not a gate (WASM is not a CFA4 target; [`docs/development/linux.md`](docs/development/linux.md)).
- **Python / Qt:** environment setup in [`docs/development/linux.md`](docs/development/linux.md) (`.venv`, `--require-hashes`). Checks as in CI: `.venv/bin/python -m ruff check python` and `.venv/bin/python -m pytest -q python/tests --ignore=python/tests/test_qt_gui.py`; the Qt GUI tests (`test_qt_gui.py`) run but don't block until the Qt6 GUI is finished ([`docs/coordination.md`](docs/coordination.md) §3.4).
- **Commits:** one task id per commit (`M2.4: CABAC residual decoding`). Only green states. End messages with the attribution line required by the environment.
- **Usage budget** (owner decision 2026-10-31, #367): the project model is `opusplan` (`.claude/settings.json`). Start any task larger than a small fix in plan mode, so planning runs on Opus and execution on Sonnet. Run quality gates through the `cfa4-ci-runner` subagent and branch inventories through `cfa4-branch-lister` (both Haiku, read-only, `.claude/agents/`). PR evidence stays the unfiltered gate log.

## Running and looking at the app
- **Qt6 GUI** (the editor; setup and options in [`docs/qt-gui.md`](docs/qt-gui.md)): `cargo build --release -p cfa4-cli`, then `.venv/bin/cfa4-qt --demo` (or `.venv/bin/cfa4-qt project.fcproj`). `.venv/bin/cfa4-qt --smoke --screenshot out.png` starts it, waits for the engine, saves a screenshot and quits.
- **egui app with the control server:** the server requires `CFA4_CONTROL_TOKEN` (64 hex characters), so set it first: `export CFA4_CONTROL_TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"`, then `cargo run -p cfa4 -- --control 9876` (protocol: [`docs/control-protocol.md`](docs/control-protocol.md)).
- For UI work, **look at the result**: in the Qt6 GUI with `--smoke --screenshot`, in the egui app via the control channel's `ui.screenshot`; compare with `plan/premiere/screenshots/` (maintainer-local; see [`docs/agents.md`](docs/agents.md) §3).
- Parallel agents: separate git worktrees and `CARGO_TARGET_DIR=target/agent-<name>`; keep every `Cargo.toml` valid at all times (the `crates/*` glob means one broken manifest breaks everyone).
- Test fixtures: generate with ffmpeg into `target/fixtures/` (never commit media). See [`docs/testing.md`](docs/testing.md).
