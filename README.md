# CFA4 — CinemaFoundArchetecture4

CFA4 is an Ubuntu 26.04 x86_64 (AMD64)-targeted, further-developed derivative of an open-source
video editor (see [Upstream provenance](#upstream-provenance)). The inherited Rust multimedia engine remains in place; CFA4 development
combines **Rust / Cargo**, **Python**, and **Qt 6**.

> **Goal and priorities:** read [VISION.md](VISION.md) first (in Hungarian). It defines what CFA4 is for: a Qt6 application on every graphical Ubuntu flavour that combines video editing, converting, publishing, subtitling, dubbing, audio editing and 2D/3D animation, with mostly local AI.

> **Development status:** repository preparation and gradual Linux migration.
> Only Ubuntu 26.04 and its desktop flavours on Linux x86_64/AMD64 are supported for builds and releases. Android, ARM64, RISC-V, Windows, macOS and FreeBSD are **out of scope**.
> This repository is **not yet a completed Qt 6 video editor**.

## Getting started on Ubuntu 26.04 AMD64

1. Read [the Ubuntu 26.04 developer guide](docs/development/linux.md).
2. Run `bash scripts/dev/bootstrap-ubuntu.sh --check` (non-modifying checks).
3. Explicitly install prerequisites with `bash scripts/dev/bootstrap-ubuntu.sh --install` on Ubuntu/Debian.
4. Install Rust with rustup (1.95.0 is selected by `rust-toolchain.toml`).
5. Create Python virtualenv: `python3 -m venv .venv`.
6. Install developer tools: `.venv/bin/python -m pip install -e '.[dev,qt]'`.
7. Check environment: `.venv/bin/cfa4-doctor --strict`.
8. Run the Qt shell: `.venv/bin/cfa4-qt`.
9. Build Rust CLI: `cargo build --release --locked -p cfa4-cli`.
10. Verify checks: `bash scripts/dev/verify-linux.sh`.

## Architecture and transition

| Component | Current status |
| --- | --- |
| Existing media engine and CLI | Inherited Rust crates (renamed `cfa4-*`), retained without a rewrite |
| Rust tooling | Pinned 1.95.0, Cargo workspace, existing xtask gates |
| Python tooling | Isolated venv, lint/tests, environment diagnostics |
| Qt 6 | Experimental PySide6 Widgets shell and offscreen CI smoke |
| Desktop editing GUI | Inherited egui implementation still in source; Qt port not complete |
| CI | Linux AMD64: automatic static checks; full Rust/Python/Qt6/FFmpeg validation only on manual dispatch or PR label |
| Releases | Paused until packaging verification and a successful Linux build |

The Rust CLI is `cfa4-cli`. In the first Qt iteration it is queried only for
its version; real editor operations require a future explicitly tested adapter.

## Licence

CFA4 is free software under the **GNU General Public License, version 3 or later**
(`GPL-3.0-or-later`, see [LICENSE](LICENSE)). The core application is free to use, change and
redistribute. Paid offerings (extended support, ready-made packages, separate-process extras,
content and services) are described in [docs/licensing.md](docs/licensing.md), which is also the
policy for reusing code from other projects.

## Upstream provenance

CFA4 is based on FilmCraft by the ArtCraft team (MIT OR Apache-2.0). That plain-text
statement and the upstream copyright notices in `LICENSE-MIT`, `LICENSE-APACHE`, [NOTICE](NOTICE),
`vendor/*/NOTICE` and `ATTRIBUTION.md` are retained because the licences require them. CFA4 is
not made, sponsored or endorsed by the upstream authors.

All upstream names, logos, icons and screenshots have been removed from the product, its
crates, packaging and documentation; `python/tests/test_independence.py` keeps them out.

GStreamer is an optional, dynamically linked system dependency (LGPL-2.1-or-later) for development
and experimental builds, see [ADR 0002](docs/adr/ADR-0002-gstreamer-backend.md).

PySide6/Qt6 is an **optional** developer dependency under LGPLv3 (compatible with CFA4's GPL)
with its own licensing obligations. The Qt shell does not bundle Qt for distribution.

## Project documentation

- [Linux development](docs/development/linux.md)
- [Existing engine architecture](docs/architecture.md)
- [Tests](docs/testing.md)
- [Known inherited gaps](docs/gaps.md)
- [Licenses and attribution](ATTRIBUTION.md)

The legacy `apps/cfa4-web`, Windows/macOS/FreeBSD packaging/source components
are not removed in this preparation step to avoid breaking existing crate dependencies;
none are supported CFA4 release targets.
