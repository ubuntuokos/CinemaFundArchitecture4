# CFA4 — CinemaFoundArchetecture4

CFA4 is a Linux x86_64 (AMD64)-targeted, further-developed derivative of the open-source **FilmCraft**
video editor. The inherited Rust multimedia engine remains in place; CFA4 development
combines **Rust / Cargo**, **Python**, and **Qt 6**.

> **Development status:** repository preparation and gradual Linux migration.
> Only Linux x86_64/AMD64 is supported for builds and releases. Android, ARM64, RISC-V, Windows, macOS and FreeBSD are **out of scope**.
> This repository is **not yet a completed Qt 6 video editor**.

## Getting started on Linux

1. Read [the Linux developer guide](docs/development/linux.md).
2. Run `bash scripts/dev/bootstrap-ubuntu.sh --check` (non-modifying checks).
3. Explicitly install prerequisites with `bash scripts/dev/bootstrap-ubuntu.sh --install` on Ubuntu/Debian.
4. Install Rust with rustup (1.95.0 is selected by `rust-toolchain.toml`).
5. Create Python virtualenv: `python3 -m venv .venv`.
6. Install developer tools: `.venv/bin/python -m pip install -e '.[dev,qt]'`.
7. Check environment: `.venv/bin/cfa4-doctor --strict`.
8. Run the Qt shell: `.venv/bin/cfa4-qt`.
9. Build Rust CLI: `cargo build --release --locked -p filmcraft-cli`.
10. Verify checks: `bash scripts/dev/verify-linux.sh`.

## Architecture and transition

| Component | Current status |
| --- | --- |
| Existing media engine and CLI | FilmCraft Rust crates, retained without a rewrite |
| Rust tooling | Pinned 1.95.0, Cargo workspace, existing xtask gates |
| Python tooling | Isolated venv, lint/tests, environment diagnostics |
| Qt 6 | Experimental PySide6 Widgets shell and offscreen CI smoke |
| Desktop editing GUI | Inherited egui implementation still in source; Qt port not complete |
| CI | Linux AMD64: automatic static checks; manually triggered full Rust/Python/Qt6/FFmpeg validation |
| Releases | Paused until trademark/packaging verification and a successful Linux build |

The Rust CLI is `filmcraft-cli`. In the first Qt iteration it is queried only for
its version; real editor operations require a future explicitly tested adapter.

## Upstream provenance and licensing

This repository derives from [FilmCraft](https://github.com/storytold/filmcraft)
by the ArtCraft Team and contributors. Retain the upstream MIT and Apache-2.0
license files, original notices and third-party attribution.

The **ArtCraft name and logos are not open-source assets**.
Refer to [the upstream brand license](docs/brand/LICENSE-brand.txt).
They must be removed or replaced before independently distributing modified CFA4 builds.
Existing inherited screenshots and application assets are pending this audit.

PySide6/Qt6 is an **optional** developer dependency and has its own LGPLv3/commercial
licensing obligations. The Qt shell does not bundle Qt for distribution.

## Project documentation

- [Linux development](docs/development/linux.md)
- [Existing engine architecture](docs/architecture.md)
- [Tests](docs/testing.md)
- [Known inherited gaps](docs/gaps.md)
- [Licenses and attribution](ATTRIBUTION.md)

The legacy `apps/filmcraft-web`, Windows/macOS/FreeBSD packaging/source components
are not removed in this preparation step to avoid breaking existing crate dependencies;
none are supported CFA4 release targets.
