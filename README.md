# CFA4 (CinemaFundArchitecture4)

A non-linear video editor written in pure Rust, native on macOS, Windows and Linux and in the browser
via WebAssembly. CFA4 is a modified version of [FilmCraft](https://github.com/storytold/filmcraft) by the
ArtCraft team (see [Origin](#origin)).

> **Goal of CFA4:** _to be written by the maintainer: what CFA4 is for and how it differs from FilmCraft._

![FilmCraft 0.10.0, the editor CFA4 is based on: Color workspace on an Apollo 11 documentary cut](docs/images/filmcraft-hero.png)

*Screenshot of FilmCraft 0.10.0, which CFA4 is based on. Footage is public domain, see
[ATTRIBUTION.md](ATTRIBUTION.md).*

## Origin

CFA4 started on 2026-10-23 from **FilmCraft 0.10.0** (upstream commit `cdf43fd`, released 2026-10-20),
Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors, licensed under **MIT OR Apache-2.0**
([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE), [NOTICE](NOTICE)). The upstream history is kept
in this repository. CFA4 is **not an ArtCraft product** and is not endorsed by the ArtCraft team.

What CFA4 changed so far (everything else is unchanged FilmCraft code):

- **Source detachment.** The two libraries FilmCraft pulled from `storytold/craft-libs` (`craft-ui`,
  `craft-docpath`) are vendored in [`vendor/craft-libs/`](vendor/craft-libs) as byte-identical copies of pinned
  upstream commits, each with a `SOURCE_MANIFEST.json` of file hashes. No Cargo dependency points at a Git
  remote any more; `python3 -m cfa4_devtools.dependency_audit` (with `PYTHONPATH=python`) checks this.
- **Brand marks removed.** The ArtCraft name and logos are trademarks and may not stay in a modified version, so
  they and the Discord/website links are gone. The project links in the app point at this repository.
- **Renamed to CFA4** wherever the app shows its name: window and dialog titles, menus, the About window,
  messages, the translations, the web page and the Linux package metadata. The app id is
  `io.github.ubuntuokos.cfa4`. The app icon is still FilmCraft's (MIT OR Apache-2.0, see
  [ATTRIBUTION.md](ATTRIBUTION.md)).
- Crate, binary and file-format names (`filmcraft-*`, `.fcproj`) are unchanged for compatibility, as are the
  names written into exported files (OMF/XMP/MXF tags, project generator, codec names). The macOS, Windows and
  FreeBSD package names still say FilmCraft.

## What it can do (inherited from FilmCraft 0.10.0)

Professional-style editing and trimming on an exact integer time base, a GPU compositor working in linear
light, colour grading with scopes, a large effects and transitions set, audio mixing with loudness meters,
titles and captions, and its own codecs (H.264, HEVC, ProRes, AAC, VP8/VP9, Opus, AV1, DNxHD/HR) and
containers, with export to H.264/ProRes/DNxHR and timeline interchange (FCP7 XML, FCPXML, OTIO, EDL, AAF,
OMF). Every action is a command that the UI, a CLI, a JSON control channel and an MCP server can all drive.
It is alpha software: see [ROADMAP.md](ROADMAP.md) and [docs/gaps.md](docs/gaps.md) for what is missing.

## Build and run

```
cargo run --release -p filmcraft                      # the desktop app, with a demo project
cargo run --release -p filmcraft -- --control 9876    # plus the JSON-lines control server
cargo run --release -p filmcraft-cli -- commands      # list every engine command
cargo run --release -p filmcraft-cli -- mcp           # MCP server (headless)
```

On Linux the desktop app needs the ALSA development files (for example `libasound2-dev`). Japanese text uses an
optional external font set (`CRAFT_FONTS_DIR`, see [docs/contributing.md](docs/contributing.md)); without it the
app uses its bundled and system fonts.

## Documentation

[CONTRIBUTING.md](CONTRIBUTING.md) and [docs/contributing.md](docs/contributing.md) (setup, quality gates),
[AGENTS.md](AGENTS.md) (rules for every contributor), [docs/architecture.md](docs/architecture.md),
[docs/testing.md](docs/testing.md), [docs/control-protocol.md](docs/control-protocol.md),
[docs/project-files.md](docs/project-files.md), [ROADMAP.md](ROADMAP.md).

## License and credits

MIT OR Apache-2.0, at your option. Required notices are in [NOTICE](NOTICE); the author, source and licence of
every bundled asset are in [ATTRIBUTION.md](ATTRIBUTION.md).
