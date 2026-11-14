# DCP-o-matic – the DCP mastering backend (part of CFA4 only)

Plan: [CFA4-DCPOMATIC-001 v1.0](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md) · [ADR 0011](../adr/ADR-0011-dcpomatic-external-mastering.md) ·
crate: [`cfa4-dcpomatic`](../../crates/dcpomatic/README.md) · commands: `dcpomatic.status`, `dcpomatic.makeDcp`, `dcpomatic.makeKdm`.

> **Status (2026-11-14, phase DCPOM-3):** the engine and its commands are in, with encrypted DCPs, KDMs and
> subtitle tracks. The Qt6 interface (Export window, Tools ▸ Video Tools ▸ Publish, File ▸ Export) comes in
> DCPOM-4, and the package that ships the build in DCPOM-5.

DCP-o-matic is a function of CFA4, not a separate program. CFA4 ships its own pinned, command-line-only build
(2.18.51) in `/usr/lib/cfa4/dcpomatic/`, off `PATH`, with no menu entry, `.desktop` file or GUI, and runs
`dcpomatic2_create`, `dcpomatic2_cli` and `dcpomatic2_kdm_cli` from there as separate processes. A DCP-o-matic the user installed
(on `PATH` or anywhere else) is never used. If the folder is missing a tool (`dcpomatic2_create`,
`dcpomatic2_cli`, or the patched OpenSSL `dcpomatic2_openssl`), `dcpomatic.status` says `"available": false`
and why; the rest of CFA4 works as before.

## Build for development

Build the pinned tool chain from source as the [DCPOM-01 audit](../audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md)
did (script: [`DCPOM-01-build.sh`](../audit/DCPOM-01-build.sh), about 12 minutes on 4 cores; the CI job
`dcpomatic.yml` does the same with a cache). Build from a full clone with tags: the version string comes from
`git describe`. Copy the patched `dcpomatic2_openssl` next to the tools. Then point CFA4 at the build (the
development override for `/usr/lib/cfa4/dcpomatic/`):

```sh
export CFA4_DCPOMATIC_BIN_DIR=$DCPOM_DIR/prefix/bin
export LD_LIBRARY_PATH=$DCPOM_DIR/prefix/lib64:$DCPOM_DIR/prefix/lib
cargo build --release -p cfa4-cli
echo '{"id":1,"method":"engine.execute","params":{"command":"dcpomatic.status","params":{}}}' \
  | target/release/cfa4-cli serve   # "available": true, "version": "2.18.51"
```

## Make a DCP

1. Export a master from CFA4 (`file.exportMedia`), e.g. ProRes 4444 in MOV with PCM audio, or a TIFF
   sequence plus a WAV.
2. Run `dcpomatic.makeDcp` (here over `cfa4-cli serve`, [qt-gui.md](../qt-gui.md)):

```json
{"id": 2, "method": "engine.execute", "params": {"command": "dcpomatic.makeDcp", "params": {
  "content": ["/work/master.mov"],
  "output": "/work/MyFilm-dcpomatic",
  "name": "My Film",
  "contentKind": "feature",
  "container": "flat",
  "resolution": "2k",
  "standard": "smpte"
}}}
```

It returns `{"job": N}`; follow it with `jobs.list` and stop it with `jobs.cancel`. With `"wait": true` it
returns `{"dcp": …, "filmDir": …, "bytes": …}` when done. The DCP is the folder inside `output` that holds the
asset map; the rest of `output` is DCP-o-matic's film directory (settings and log).

Other parameters: `standard: "interop"`; `container`: `flat` (1.85), `scope` (2.39), `1.19`, `1.33`, `1.37`,
`1.38`, `1.66`, `1.78` (DCP-o-matic 2.18.51 has no 1.90 container); `fps`; `audioChannels`;
`videoBitRateMbps`; `threeD` with `{"path": …, "eye": "left" | "right"}` content; mono audio files mapped with
`{"path": …, "channel": "L" | "R" | "C" | "Lfe" | "Ls" | "Rs" | "BsL" | "BsR" | "HI" | "VI", "gainDb": …}`;
`isdcfName: false` (folder named exactly `name`); `threads`; `configDir`; `subtitles` (below); `encrypt`
(below). Flags must be `true` or `false` and numbers whole numbers: anything else is refused, never read as the
default. Two jobs never write into the same `output` folder at once.

## Subtitles

`"subtitles": ["/work/film.srt"]` (or one path) adds a subtitle file (`.srt`, `.ssa`, `.ass`, or a DCP subtitle
`.xml`) as an open subtitle track: not burnt into the picture, starting at the beginning of the film. Only one
file is taken (`dcpomatic2_create` would place a second one after the end of the first). In a
SMPTE DCP it is a `sub_*.mxf` track file, in an Interop DCP an XML file in its own folder; the CPL lists it
either way.

## Encrypted DCPs and KDMs

`"encrypt": true` encrypts the picture, sound and subtitle track files. The film directory, whose
`metadata.xml` holds the content keys, is made readable only by its owner (mode 0700). The answer then has
`"encrypted": true` and a `keepFilmDir` note: the content keys exist only in the film folder (`filmDir`), and
every KDM is made from it, so keep that folder (privately) for as long as KDMs are needed.

A KDM for one projector (its media block's certificate, a PEM file the cinema sends):

```json
{"id": 3, "method": "engine.execute", "params": {"command": "dcpomatic.makeKdm", "params": {
  "film": "/work/MyFilm-dcpomatic",
  "certificate": "/work/cinema/projector.pem",
  "output": "/work/kdm",
  "cinema": "Uránia", "screen": "1",
  "validFrom": "2026-11-20 10:00:00",
  "validFor": {"count": 7, "unit": "days"}
}}}
```

- Times are UTC (`YYYY-MM-DD HH:MM:SS`); `validFrom` defaults to `now`. Give either `validFor` (`hours`,
  `days`, `weeks`, `years`) or `validUntil`.
- `formulation`: `modifiedTransitional1` (default), `multipleModifiedTransitional1`, `dciAny`, `dciSpecific`
  (with `trustedDevices`: certificate files).
- `disableForensicMarkingPicture`, `disableForensicMarkingAudio`: only when the distributor asks for it.
- It returns `{"kdms": [...]}`, the KDM file(s) written into `output`. The signing chain is the one in the configuration folder
  (`configDir`, by default `~/.config/cfa4/dcpomatic`), the same one that signed the DCP.

To play the encrypted DCP in CFA4 itself, load the KDM with `dcp.loadKdm` (`{"kdm": …, "privateKey": …}`, the
projector's private key). The phase 3 acceptance test does exactly this with a test certificate:
`cargo test -p cfa4-engine --lib dcpomatic::real_tests` (needs `CFA4_DCPOMATIC_BIN_DIR`) makes an encrypted DCP
with a 1 kHz tone, a KDM for a self-signed test projector, checks that the sound is silent without keys, then
loads the KDM and checks that the tone plays.

DCP-o-matic's configuration and signing chain live in `~/.config/cfa4/dcpomatic` (it adds a `2.18/`
subfolder), so the user's own DCP-o-matic settings are never touched.

Check the result with CFA4's DCP validation ([dcp-validation.md](dcp-validation.md)); the acceptance is the
`full-integrity` profile, for SMPTE and Interop, with a subtitle track.
