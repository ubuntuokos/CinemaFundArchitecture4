# CFA4 F1 — Linux identity and upstream independence

Status: **in development, not approved for redistribution**. F1 is tracked in
[issue #4](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/4).
The base engine remains FilmCraft, version 0.10.0, and retains upstream notices and
MIT OR Apache-2.0 license. This is a further-developed derivative, not a claim of
original authorship of the inherited code.

## New Linux identity (pre-release)

| Property | CFA4 |
|---|---|
| Repository | https://github.com/ubuntuokos/CinemaFundArchitecture4 |
| Desktop/Flatpak/AppStream ID | `io.github.ubuntuokos.cfa4` |
| Package name | `cfa4` |
| Installed app binary | `cfa4` (temporarily built from existing Rust `filmcraft` target) |
| Installed command tool | `cfa4-cli` (temporarily built from `filmcraft-cli`) |
| Original project format | `.fcproj`, `application/x-filmcraft` retained for backward compatibility |
| Platforms | **Linux only** |
| Frontend | PySide6/Qt6 in development; inherited egui still exists and is not rebranded internally |
| Native source / engine | Rust/Cargo, original crate names retained for compatibility |
| Upstream credits | MIT/Apache-2.0/NOTICE and per-asset attribution retained |

The Linux desktop entry, AppStream metadata, Flatpak source/bundle manifests,
deb/rpm manifest and installation scripts use the CFA4 application identity.
A new minimal, independently composed SVG icon is used for *Linux packaging assets*
(`assets/cfa4/hicolor/scalable/apps/io.github.ubuntuokos.cfa4.svg`), and it is
included in `ATTRIBUTION.md`.

The earlier `ai.storyteller.filmcraft.*` Linux packaging manifests are removed
from the F1 branch. Windows/macOS/FreeBSD build workflows were removed in PR #3.

## Safety / license release gate

**No public release is permitted yet.** The original FilmCraft desktop binary still
contains inherited app naming and iconography, including resources that may be subject
to separate brand rights. The repository also retains inherited screenshots and artwork
for migration inspection. A custom Linux package cannot be distributed until all
embedded marks and artwork have been reviewed or replaced.

The local Linux packaging scripts reject calls by default. Explicit
`CFA4_ALLOW_LOCAL_PACKAGING=1` is permitted **only for local packaging smoke tests**
and must not be interpreted as redistribution authorization. Do not add a release
workflow before the review and a Linux install/use acceptance test.

PySide6/Qt6 carries separate LGPL/commercial terms. Do not embed or redistribute Qt
without package-level license compliance, including the relevant dynamically linked
libraries and their notices.

## Supply-chain dependencies still pending

`storytold/craft-libs` is currently referenced in `Cargo.toml` with exact
commit SHAs for `craft-ui` and `craft-docpath`. Pinning prevents moving-tag
surprises but is **not** a guarantee against source unavailability or supply-chain
compromise. F1 still requires a license/attribution assessment and a decision to
vendor (with recorded hashes) or replace these dependencies.

The old `storytold/craft-common-actions@main` workflows were removed from the
Linux-only development branch. `storytold/craft-fonts` is optional for the engine,
not a forced dependency of the default Rust build.

## Acceptance / remaining work

- [x] Create independently named Linux install metadata.
- [x] Use a different SVG for the *package icon*, without reusing the inherited icon.
- [x] Block experimental packaging by default.
- [x] Keep the existing `.fcproj` project file type while migrating.
- [ ] Complete `cargo xtask ci` and independent CLI smoke tests on the F1 branch.
- [ ] Replace original FilmCraft naming/icons embedded in Rust runtime and egui.
- [ ] Provide actual Qt6 video editing GUI, not only a smoke shell.
- [ ] Validate installer and package metadata on supported Linux distributions.
- [ ] Review legal rights and attribution for all inherited runtime assets.
- [ ] Document and verify source availability/license of pinned `craft-libs`.
- [ ] Decide whether a dedicated `.cfa4proj` format and migration tool is required.

The default source remains developable even while packaging is blocked. F1
is **partially implemented** and must not be marked complete before these
remaining conditions have direct test or license evidence.

## F1-01 audit és reprodukálhatóság

A közvetlen Git-források változatlan pinjeit, két upstream revíziójuk licenc- és
NOTICE-bizonyítékát, valamint az offline/migrációs kockázatokat a
[docs/audit/f1-01-dependencies.md](../audit/f1-01-dependencies.md) rögzíti.
Az átmeneti, hálózat nélküli Git-source drift-kapu a
`python -m cfa4_devtools.dependency_audit` parancs; F1-02-ben a tényleges
vendorizációval együtt kell átalakítani, hogy ne őrizze meg a régi Git-forrásokat.
