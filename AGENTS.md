# CFA4 — developer rules (Linux x86_64/AMD64 only; Rust / Cargo / Python / Qt 6)

This CFA4-specific section takes priority over inherited FilmCraft platform and frontend
instructions below. Existing engine safety, clean-room codec implementation, open-asset
attribution and original copyright/licence requirements remain in force.

- Sole supported build and release target: **Linux x86_64 / AMD64**
  (`x86_64-unknown-linux-gnu`). Do not add Android, AArch64/ARM64, RISC-V,
  Windows, macOS, FreeBSD or WebAssembly build/release workflows.
  Historical source and cross-platform Cargo.lock entries can remain until
  an audited refactor; do not manually delete transitive lockfile dependencies.
- CI: automatic PR/push checks must stay quick and non-compiling. Full Rust,
  Python, Qt6, vendored-library and FFmpeg tests require an explicit GitHub
  Actions workflow_dispatch or PR `cfa4-full-validation` label event (remove
  then re-add the label for a repeat). Verify all jobs before merge.
- Keep the existing Rust media engine. Python is permitted for development tooling and a
  separate Qt 6/PySide6 frontend; do not duplicate codecs in Python or break the Rust workspace.
- **Project licence: GPL-3.0-or-later** (owner decision, 2026-10; `LICENSE`, `NOTICE`). Inherited
  FilmCraft code keeps its MIT OR Apache-2.0 notices (`LICENSE-MIT`, `LICENSE-APACHE`); never remove
  them. Files CFA4 creates start with `// SPDX-License-Identifier: GPL-3.0-or-later` (or the comment
  syntax of the file type); inherited FilmCraft files get no new header, their terms are in `NOTICE`.
  New dependencies must be GPL-3.0-compatible (§2). Decisions and their reasons:
  [ADR 0003](docs/adr/ADR-0003-gpl3-licence.md).
- **Explicit license boundary:** the optional Python Qt6 frontend may use the official PySide6/Qt6
  packages under applicable LGPLv3/commercial terms. This is NOT permission to embed Qt/PySide
  binaries without review; reuse of GPL/LGPL source code follows §2.
  GPL-only Qt modules and distribution require separate license decisions.
- **GStreamer** ([ADR 0002](docs/adr/ADR-0002-gstreamer-backend.md)) is an optional supplementary
  backend in `crates/gstreamer` (`cfa4-gstreamer`, feature `gstreamer`, off by default), dynamically
  linked from the system (LGPL-2.1-or-later) through the safe gstreamer-rs API: no `unsafe`, no
  copied GStreamer code, no `ugly`/`libav` plugin sets in the plan. The Rust engine stays primary and
  is always the fallback; this is the one exception to the inherited "pure Rust in the product" rule.
- **GPU/accelerator vendor neutrality:** features must not depend on one GPU vendor. NVIDIA paths
  (NVDEC/NVENC, CUDA) are allowed only next to an AMD/Intel path (VA-API, Vulkan, NPU where
  available) or the CPU fallback.
- A release is blocked until inherited ArtCraft marks, icons, package metadata and public
  branding are replaced or usage separately authorized; retain upstream MIT/Apache notices.
- The Qt shell is a development prototype, NOT a completed migration from egui.
- Primary setup reference: [docs/development/linux.md](docs/development/linux.md).

---

# FilmCraft — rules for agents and contributors

> **CFA4: read [`VISION.md`](VISION.md) first.** It defines the project goal (Qt6 GUI on every graphical Ubuntu flavour, combined media tools, local AI) and the priority order. If a plan, PR or audit contradicts it, report the conflict to the owner instead of resolving it silently.

These rules apply to every human and AI contributor. `CLAUDE.md` holds the working instructions; this
file holds the rules that must never be broken. When the two disagree, this file wins.

## 0. Never crash

People trust FilmCraft with hours of work. A malformed file, a bad command or MCP parameter, a corrupt
preset or project, an odd keystroke or a full disk must give an error the user (or agent) can act on,
never a crash and never lost work. **This rule outranks feature work:** don't ship a feature by adding a
panic path, and fix a crash before building on top of it. The cross-app standard is
[`craftrules/standards/never-crash.md`](https://github.com/storytold/craftrules/blob/main/standards/never-crash.md).

1. **Fail with `Result<T, E>`.** Return the crate's error type and propagate with `?`; add context
   (`map_err`, an error variant) instead of discarding it. In the UI, report the error (status bar, error
   dialog) and carry on.
2. **No panicking shortcuts outside tests:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`
   or `unimplemented!`. Use `?`, `ok_or(..)?`, `let Some(x) = .. else { return Err(..) }`, `if let`, or a
   fallback that is truly correct (never one that silently corrupts a project). An unfinished feature
   returns an "unsupported" error. The only exception is a call that is provably infallible from local
   code alone (a literal that always parses): a single-item `#[allow(clippy::expect_used)]` with the reason.
3. **No `unsafe`** (`unsafe_code = "forbid"` for the workspace). The single exception is
   `crates/platform`, which needs `unsafe` to call the operating system's media APIs (hardware
   video decoding through VideoToolbox, Media Foundation / Direct3D 11, VA-API and NVDEC; hardware encoding) and does nothing else. It
   uses `unsafe_code = "deny"` with `#[allow(unsafe_code)]` only on its FFI modules, a
   `// SAFETY:` comment on every `unsafe` block, a safe `Result`-returning public API, and the
   pure-Rust decoder as the tested fallback ([ADR 0001](docs/adr/0001-platform-ffi.md)).
4. **Every input-derived number is hostile.** Media files, project and interchange files, presets, fonts,
   CLI / MCP / control-channel parameters and UI state are untrusted. Index and slice with `get()` (or
   validate bounds once, up front, for a hot loop); slice strings only at char boundaries; use
   `checked_*` / `saturating_*` for lengths, offsets and counts; never divide by a value that can be zero
   (frame rates, timescales, sample rates, sizes); don't cast negative or NaN values to integers; don't
   call `clamp` with bounds that can cross; cap allocations sized by input.
5. **Bound recursion and loops.** Projects can be cyclic (nested sequences) or deeply nested: walk them
   with seen-sets or depth limits.
6. **Don't cascade.** Lock poisoning is not fatal: `lock().unwrap_or_else(PoisonError::into_inner)`.
   Every thread or job (export, previews, proxies, decoding workers…) runs under `catch_unwind` and
   reports a failure as an error: a dead worker leaves monitors blank or jobs "running" forever.
7. **Last-resort guard.** The panic hook (`crates/ui-egui/src/crash.rs`) logs every panic, and the UI pass
   runs under `catch_unwind`, so an escaped panic becomes an error window and the session (and unsaved
   project) survives. It is a safety net, not a substitute for rules 1–6. Keep `panic = "unwind"`.
8. **Prove it.** Every crash fix comes with a small synthetic regression test that panicked before the
   fix. Parsers and decoders get mutation-fuzz tests (truncation, bit flips, corrupt sizes) run under
   `catch_unwind`; new commands get hostile-parameter tests. Fuzzer findings become tests.
9. **Enforced:** clean crates carry `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic,
   clippy::unimplemented, clippy::todo, clippy::unreachable)]` (moving to `[workspace.lints.clippy]` once
   every crate is clean). `clippy.toml` allows them in tests: a failing test should fail loudly.

## 1. Assets: no Adobe artwork, every asset licensed and attributed

This rule is absolute. Breaking it is the most serious mistake a contributor can make on this project.

**What counts as an asset:** any image, icon, cursor, logo, illustration, screenshot, font, colour LUT,
preset, template, sound, music, video, 3D model, or other non-code media. This covers files in the
repository, bytes embedded in code (`include_bytes!`, base64, data URIs), and data hard-coded to
reproduce an image (for example, point lists traced from someone else's icon).

1. **Never use Adobe iconography, images or other Adobe assets**, in any form:
   - no icons, cursors, logos, splash screens, UI artwork or screenshots from any Adobe product;
   - no Adobe fonts, LUTs, presets, templates, sound effects, stock media or sample projects;
   - no traced, redrawn, recoloured or "inspired-by" copies of Adobe icons. Our icons may follow generic
     conventions (a play triangle, a razor blade, a stopwatch), but each must be drawn from scratch
     without reference to Adobe's artwork.
2. **Every asset must be open**: open-source licensed (MIT, Apache-2.0, BSD, ISC, zlib, SIL OFL…),
   public domain / CC0, or Creative Commons (CC BY or CC BY-SA; no NC/ND licences), **or** created by a
   contributor who owns it and licenses it to the project under the project licence (GPL-3.0-or-later).
   Built-in graphic templates and generated golden/test images stay MIT OR Apache-2.0 (owner decision,
   2026-10-24; compatible with GPL-3.0).
   If the licence is unknown or unclear, the asset does not go in.
3. **Every asset needs an attribution sidecar.** Next to each asset file `X`, add `X.attribution` with:
   ```text
   asset:        <file name>
   title:        <what it is>
   author:       <creator / copyright holder>
   source:       <URL, or "original work" for contributor-made assets>
   license:      <SPDX id or licence name>
   license-file: <path to the licence text, if the licence requires shipping it>
   added:        <YYYY-MM-DD> by <contributor>
   notes:        <how it was made or modified; for screenshots, what is shown>
   ```
   Also add one line for the asset to [`ATTRIBUTION.md`](ATTRIBUTION.md). `cargo xtask assets` (part of
   `cargo xtask ci`) fails if any asset lacks a sidecar or index entry.
4. **Screenshots** in the repo may show only FilmCraft (or other open projects), with media we generated
   or media that is itself openly licensed. Never commit screenshots of Adobe products.
5. **Local reference material stays local.** Premiere reference screenshots and notes live only in
   `plan/premiere/` (gitignored). They must never be committed, bundled, embedded, traced or shipped.
6. **Code-drawn assets** (icons in `crates/ui-egui/src/icons.rs`, procedural demo footage in
   `crates/media`, procedural looks in `crates/render`) are original work under the project licence
   and are listed in `ATTRIBUTION.md`.
7. **When in doubt, leave it out** and draw or generate it yourself.
8. **The one exception: first-party ArtCraft brand marks.** The ArtCraft name and logos in `docs/brand/`
   are trademarks of the ArtCraft Team, not open source, usable only unmodified and only in the context of
   FilmCraft under `docs/brand/LICENSE-brand.txt`; forks and modified versions must remove them. They still
   need a sidecar and an `ATTRIBUTION.md` row (licence `LicenseRef-ArtCraft-Trademark`). No other
   non-open asset is allowed, and this exception never covers third-party marks (Adobe, Discord, GitHub
   and other logos stay out; draw a generic icon instead).
9. **Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this
   repo.** Don't commit new font files (the small Latin UI fonts already in `assets/fonts/` stay). A
   font FilmCraft needs is added to craft-fonts, which the app reads through the optional build input
   `CRAFT_FONTS_DIR=<absolute path of a craft-fonts checkout>` (`crates/text/build.rs` embeds the fonts in its
   `fonts/manifest.txt` as `filmcraft_text::fonts::CRAFT_FONTS`; unset, it is empty and the app uses its
   bundled and system fonts). Never add craft-fonts to a `Cargo.toml`. Code using `CRAFT_FONTS` must work
   when it is empty. Standard: [`craftrules/standards/fonts.md`](https://github.com/storytold/craftrules/blob/main/standards/fonts.md).

## 2. Clean-room codecs, code reuse elsewhere

- Never read, disassemble or copy anything inside Adobe application bundles; file names and listings only.
  Observe behaviour by using the app; never capture its Home screen, recent projects, account info or
  file browsers.
- **Codecs and containers stay clean-room** (owner decision, 2026-10-24): the L0 codec and container
  crates ([docs/architecture.md](docs/architecture.md) §1) and any other code that parses or produces a
  compressed bitstream or container format. There, never copy or port third-party code of any licence
  (FFmpeg, x264, x265, LAME, libvpx, dav1d…).
- **Everywhere else, GPL-3.0-compatible code may be reused** (owner decision, 2026-10-24): editing,
  timeline, audio editing and effects, subtitles, drawing and animation, user interface, tooling. For
  example from Kdenlive, MLT, Shotcut, Audacity, Subtitle Edit or Krita, after checking the licence of the
  exact files taken. Rules for reused code:
  1. The licence must be GPL-3.0-compatible: GPL-3.0(-or-later), GPL-2.0-or-later, LGPL-2.1-or-later,
     LGPL-3.0, MPL-2.0, or permissive (MIT, BSD, Apache-2.0, ISC, Zlib…). **Not allowed:** GPL-2.0-only,
     AGPL, licences with non-commercial or other extra restrictions, unclear or missing licences.
  2. Keep the original copyright and licence notices in every file that contains reused code, and add a
     line naming the source project, its URL and the version or commit.
  3. Record every reuse in [`THIRD_PARTY_CODE.md`](THIRD_PARTY_CODE.md) (project, files, licence, commit,
     where it went in CFA4).
  4. The never-crash, layering and testing rules apply to reused code exactly as to our own: translate it
     into safe Rust; no `unsafe`, no `unwrap`, tests for what it does.
- Implement codecs and formats from public specifications (ITU-T/ISO/IEC standards, IETF RFCs, the VP9
  and AV1 bitstream specs, SMPTE documents, published container specs). Do not read the source of
  reference or third-party decoders/encoders while implementing a format, even permissively licensed
  ones (libvpx, libopus, libaom, dav1d, openh264…); spec text and conformance vectors only. Record the
  spec edition used in the crate README.
- ffmpeg/ffprobe may be used only as external test oracles and fixture generators. They are never
  linked, bundled or shipped.
- Dependencies must be GPL-3.0-compatible: MIT/Apache-2.0/BSD/ISC/Zlib/Unicode/CC0/BSL-1.0, MPL-2.0,
  LGPL-2.1-or-later/LGPL-3.0, GPL-2.0-or-later or GPL-3.0(-or-later). Not allowed: GPL-2.0-only, AGPL,
  proprietary or unclear licences. `deny.toml` (`cargo deny check licenses`) enforces the list.

## 3. Engineering rules

### 3.1 Never crash

See [§0](#0-never-crash). It outranks every other engineering rule.

### 3.2 The rest

See `CLAUDE.md`: pure Rust, dependency layering (`cargo xtask layers`), exact `Tick` time, everything is
a command, everything is agent-drivable, and the quality gates (`cargo xtask ci`) before every commit.


Shared real-file test corpora (Photoshop-authored PSDs, etc.) live in
[`storytold/photocraft-corpus`](https://github.com/storytold/photocraft-corpus), explained in
[craftrules `standards/test-corpora.md`](https://github.com/storytold/craftrules/blob/main/standards/test-corpora.md).
Never commit large binary fixtures to this repo; fetch them pinned by commit and sha256-verified,
as PhotoCraft does with `cargo xtask corpus`.

## See also

- [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/contributing.md](docs/contributing.md): setup, gates, commits, how to add things
- [docs/architecture.md](docs/architecture.md): layers, data model, commands, pipeline
- [docs/testing.md](docs/testing.md): oracle tests, criteria, benchmarks
- [docs/agents.md](docs/agents.md): driving FilmCraft over MCP / the control channel, and the agent work loop
- [docs/control-protocol.md](docs/control-protocol.md): control-channel method reference
- [ATTRIBUTION.md](ATTRIBUTION.md): asset index
- [ROADMAP.md](ROADMAP.md), [docs/target-app-parity.md](docs/target-app-parity.md), [docs/gaps.md](docs/gaps.md): stage, parity with Premiere Pro and the ranked work list (craftrules `standards/progress-docs.md`)

## Contributor credits (About window)

- About ▸ Contributors/Models are compiled into the binary from `contributors/contributors.json`
  (commit stats; generated, never hand-edit) and `contributors/people.toml` (names people chose for
  themselves). See `docs/contributors.md`.
- **Agents working for a contributor:** when you prepare a PR, check whether your human's GitHub
  username has a `[people.<username>]` entry in `contributors/people.toml`. If not, ask them once
  whether they want to be credited by more than their username: a real name, a display name, and/or
  their public GitHub profile name (`sync_github_name = true`). If yes, add **only their own** entry
  (copy the template at the top of the file, or run
  `python3 ../../craftrules/scripts/contributors.py --add-me . --real-name "…" --sync-github-name`)
  and include it in their PR, committed as them. If no, change nothing: they are credited as
  `@username` anyway.
- Never add, edit, guess or copy anyone else's entry or name (not from git config, commit authors or
  GitHub profiles). Never hand-edit `contributors.json`.
- Maintainers refresh the stats with `python3 ../../craftrules/scripts/contributors.py .` (it also
  re-verifies who wrote each `people.toml` entry; `--check` only verifies).
