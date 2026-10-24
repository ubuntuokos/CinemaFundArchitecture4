# CFA4 — Linux development environment

**Sole build and release target:** Ubuntu 26.04 and its Ubuntu-based desktop flavours, Linux x86_64 (AMD64; Rust target `x86_64-unknown-linux-gnu`).
Kubuntu, Ubuntu Studio and other Ubuntu 26.04 flavours share the host/version gate; other Linux distributions are not CFA4 release targets.
**Unsupported targets:** Android, ARM/AArch64, RISC-V, Windows, macOS, FreeBSD and browser/WASM.
Historical sources and target-specific transitive entries in Cargo.lock may remain until separately audited; they are not build or release targets.

## Responsibility boundaries

- **Rust 1.95 / Cargo:** existing FilmCraft media engine, codec implementations, project model, rendering, export and `filmcraft-cli`.
- **Python 3.12+:** developer tooling and orchestration. Project CI exercises Python 3.13; newer Linux distributions can use a supported version.
- **Qt 6 / PySide6:** new Linux-native GUI development path. The initial `cfa4-qt` application is a deliberately minimal **development shell**, not a port of the full video editor.
- **Existing egui desktop app:** retained during migration to avoid breaking the Rust media engine. Do not mistake it for the finished Qt interface.
- **FFmpeg 9:** the primary media backend (planned `cfa4-ffmpeg` crate, GPL-3 build, [ADR 0004](../adr/ADR-0004-ffmpeg9-primary-backend.md)); GStreamer is the secondary backend and the Rust codecs the fallback. The pinned FFmpeg/ffprobe 9.0.2 CLI (`scripts/ci/install-ffmpeg-9.0.2.sh`) remains a separate test oracle.
- **Python-to-Rust integration:** `filmcraft-cli` subprocess, JSON command interfaces or documented local control protocol. Do not silently replace the engine with Python implementations.

## Bootstrap (Ubuntu 26.04 AMD64)

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
bash scripts/ci/check-linux-amd64.sh  # fast, no Rust compilation
bash scripts/dev/verify-linux.sh    # full local checks, explicit opt-in
~~~

`cfa4-doctor` is read-only and reports the actual installed versions. `--strict` fails on
missing Linux/Rust/Cargo/Python/Qt/FFmpeg prerequisites. The GUI shell queries
`filmcraft-cli --version` when the executable exists; it does not yet edit videos.

The CI workflow `.github/workflows/ci-linux.yml` runs only the inexpensive
`platform-scope` shell/configuration gate on pull requests and main pushes.
To avoid repeated unnecessary builds, all Rust, FFmpeg, vendored crate, CLI,
Python and Qt6 jobs run **only** on explicit GitHub Actions `workflow_dispatch`
or when the `cfa4-full-validation` label is **added** to a pull request.
Until this workflow exists on the default branch, use the PR label; for a later
rerun, remove the label and add it again. Ordinary pushes to labeled PRs do not
trigger expensive jobs. Before a merge or release, explicitly run the entire
workflow and confirm all mandatory jobs pass; a green static gate alone
does not qualify as full CI validation. An unrun, skipped, or failing job is
**not** proof that the full CI passed.

The older `cargo xtask wasm` code remains in source for historical compatibility,
but WASM is **not** a supported CFA4 target or quality gate.

## HEIF / HEIC still images (local libheif, Ubuntu 26.04 AMD64)

CFA4 recognizes `.heic`, `.heif` and `.hif` image files. Its Rust
`filmcraft-media` still loader checks HEIF file brands **before** the MP4/MOV
openers, including random-access media browser imports. The local
`heif-dec` executable decodes the image to a temporary PNG and CFA4
imports its first picture as an 8-bit RGBA still. In `filmcraft-export`,
the `still::StillFormat::{Heic, Heif}` **single-frame export** option uses
`heif-enc` to turn a generated 8-/16-bit PNG into HEVC-based HEIC bytes.
A 16-bit source is passed to libheif for HEVC 10-bit encoding; this does
not yet provide end-to-end HDR metadata preservation.

The adapter calls a locally installed subprocess; it introduces **no
new Cargo dependency**, no linked GPL/LGPL media codec library and **no
cloud service**. Image conversion uses a private, automatically removed
temporary directory with file-size guards. A missing `heif-dec` or
`heif-enc` returns a descriptive error rather than silently skipping
the feature.

Install the Ubuntu 26.04 packages (or run the development bootstrap
with `--install`):

~~~bash
sudo apt-get install --no-install-recommends \
  libheif-examples libheif-plugin-libde265 libheif-plugin-x265
heif-dec --version
heif-enc --version
~~~

The `.deb` package **recommends** these Ubuntu packages rather than
bundling their binaries. Other distributions are not CFA4 release targets; Ubuntu 26.04 HEIF runtime dependencies
remain subject to the release audit.

Run the targeted Rust oracles on a machine with the plugins installed:

~~~bash
cargo test --locked -p filmcraft-media --test heif_oracle
cargo test --locked -p filmcraft-export --lib heic_heif_single_frame_exports_round_trip
~~~

**Scope and outstanding work:** first-picture import and single-frame
export only; multiple primary/auxiliary pictures, burst sequences,
full ICC/EXIF metadata transfer, HDR colour metadata and native
10-/12-bit display-frame retention are **not yet implemented**. Existing
media import currently normalizes these pictures to 8-bit sRGB.
`libheif`, `libde265` and `x265` runtime/plugin licensing, patents
and package redistribution require an independent release review.
Installing system packages or passing the HEIF CI oracles does **not**
constitute distribution approval.

## AppImage local test tooling (Ubuntu 26.04 AMD64 only)

The Linux packaging script intentionally never downloads `appimagetool`.
For an **explicit local packaging test**, install a previously reviewed and
version-pinned x86_64 appimagetool binary, verify its expected SHA-256 from an
independent trusted release source, then pass the approved digest to the
packaging command. Replace the placeholders below with locally verified values:

~~~bash
export APPIMAGETOOL=/absolute/path/to/reviewed/appimagetool
export CFA4_APPIMAGETOOL_SHA256=<approved_64_hex_sha256>
CFA4_ALLOW_LOCAL_PACKAGING=1 bash packaging/linux/package.sh --formats appimage
~~~

Without the tool/digest, use `--formats 'deb tar'` for a local test,
subject to their own prerequisites. Never treat a hash copied from the same
untrusted download as provenance verification. Packaging remains developer-only
and **not approved for publication**.

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
