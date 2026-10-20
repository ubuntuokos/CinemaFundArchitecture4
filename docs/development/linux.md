# CFA4 — Linux development environment

**Supported target platform:** Linux (Ubuntu/Debian development bootstrap; other distributions may install equivalent packages manually).
**Unsupported:** Windows, macOS, FreeBSD and browser/WASM as release targets. Their historical source files may remain until a separate refactor.

## Responsibility boundaries

- **Rust 1.95 / Cargo:** existing FilmCraft media engine, codec implementations, project model, rendering, export and `filmcraft-cli`.
- **Python 3.12+:** developer tooling and orchestration. Project CI exercises Python 3.13; newer Linux distributions can use a supported version.
- **Qt 6 / PySide6:** new Linux-native GUI development path. The initial `cfa4-qt` application is a deliberately minimal **development shell**, not a port of the full video editor.
- **Existing egui desktop app:** retained during migration to avoid breaking the Rust media engine. Do not mistake it for the finished Qt interface.
- **FFmpeg / ffprobe:** test oracle tools, not linked runtime dependencies of the existing Rust codec implementation.
- **Python-to-Rust integration:** `filmcraft-cli` subprocess, JSON command interfaces or documented local control protocol. Do not silently replace the engine with Python implementations.

## Bootstrap (Ubuntu / Debian)

For package inspection, without installing anything:

~~~bash
bash scripts/dev/bootstrap-ubuntu.sh --check
~~~

To install the Linux build prerequisites (explicit opt-in; apt needs elevated privileges):

~~~bash
bash scripts/dev/bootstrap-ubuntu.sh --install
~~~

Then install the official Rust toolchain via https://rustup.rs/ if it is not installed.
The repository's `rust-toolchain.toml` selects Rust 1.95.0 with rustfmt/clippy.
No command in this repository silently installs Rust.

Create the Python environment in the repository root (do not install into the system Python):

~~~bash
python3 -m venv .venv
.venv/bin/python -m pip install --upgrade pip
.venv/bin/python -m pip install -e '.[dev,qt]'
~~~

The Qt system headers/tools (Qt Widgets, Qt Multimedia, Qt Designer, CMake) are installed by apt.
PySide6 provides its own Qt shared libraries via the wheel, so do not combine the wheel's
libraries with the distro's Qt binaries in a single process.

## Developer tools / commands

~~~bash
.venv/bin/cfa4-doctor --json
.venv/bin/cfa4-doctor --strict
cargo metadata --format-version 1 --no-deps
cargo build --locked -p filmcraft-cli
cargo build --locked -p filmcraft
cargo xtask ci
QT_QPA_PLATFORM=offscreen .venv/bin/cfa4-qt --smoke
.venv/bin/cfa4-qt
bash scripts/dev/verify-linux.sh
~~~

`cfa4-doctor` is read-only and reports the actual installed versions. `--strict` fails on
missing Linux/Rust/Cargo/Python/Qt/FFmpeg prerequisites. The GUI shell queries
`filmcraft-cli --version` when the executable exists; it does not yet edit videos.

The CI workflow `.github/workflows/ci-linux.yml` has two independent jobs:
Rust (Cargo tests/lints/layering/assets + mandatory oracle tools) and Python/Qt6
(compile, ruff, pytest and offscreen widget smoke). The Rust test suite is large and
may expose existing upstream failures. **An unrun or failing job is not a passed job.**

The older `cargo xtask wasm` code remains in source for historical compatibility,
but WASM is **not** a supported CFA4 target or quality gate.

## Python / Qt license and origin requirements

Existing FilmCraft Rust code is MIT OR Apache-2.0 and retains attribution.
PySide6 / parts of Qt6 have LGPLv3 (or commercial) terms, with modules possibly GPL-only.
Keep PySide6 dynamically loaded in an isolated Python environment and do not bundle
the Qt runtime, redistribute LGPL components or introduce GPL-only Qt modules
without an explicit distribution/license compliance review.
Read https://www.qt.io/development/open-source-lgpl-obligations .

**Trademark blocker:** ArtCraft marks present in inherited screenshots, brand assets,
application icons and packages are not open licensed. Independently distributing the modified
CFA4 product requires removing/replacing them, consistent with `docs/brand/LICENSE-brand.txt`.
The Linux bootstrap and CI do not represent release approval; publication remains blocked
until branding, application IDs and license attribution have been audited.

## Remaining tasks (intentionally not claimed as implemented)

1. Integrate Rust engine commands with a Qt controller (error/timeouts and project lifecycle).
2. Port the actual NLE GUI from egui to Qt6 in incremental, tested modules.
3. Establish Linux-only package identifiers, icon/brand assets and packaging pipeline.
4. Run full real-hardware import/edit/export checks on Ubuntu target systems.
5. Audit/remove unused macOS/Windows/FreeBSD source paths and dependencies safely after tests pass.
6. Add offline/vendored dependency support and version/lock policy for Python packages.

See also `docs/audit/BASELINE-2026-10-20.md` on the separate audit branch / draft PR
until it is merged.
