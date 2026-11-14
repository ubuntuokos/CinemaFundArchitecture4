# DCP-o-matic – the DCP mastering backend (part of CFA4 only)

Plan: [CFA4-DCPOMATIC-001 v1.0](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md) · [ADR 0011](../adr/ADR-0011-dcpomatic-external-mastering.md) ·
crate: [`cfa4-dcpomatic`](../../crates/dcpomatic/README.md) · commands: `dcpomatic.status`, `dcpomatic.makeDcp`.

> **Status (2026-11-14, phase DCPOM-2):** the engine and its commands are in. Encryption and KDMs come in
> DCPOM-3, the Qt6 interface (Export window, Tools ▸ Video Tools ▸ Publish, File ▸ Export) in DCPOM-4, and the
> package that ships the build in DCPOM-5.

DCP-o-matic is a function of CFA4, not a separate program. CFA4 ships its own pinned, command-line-only build
(2.18.51) in `/usr/lib/cfa4/dcpomatic/`, off `PATH`, with no menu entry, `.desktop` file or GUI, and runs
`dcpomatic2_create` and `dcpomatic2_cli` from there as separate processes. A DCP-o-matic the user installed
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
`isdcfName: false` (folder named exactly `name`); `threads`; `configDir`. `encrypt: true` is refused until
DCPOM-3.

DCP-o-matic's configuration and signing chain live in `~/.config/cfa4/dcpomatic` (it adds a `2.18/`
subfolder), so the user's own DCP-o-matic settings are never touched.

Check the result with CFA4's DCP validation ([dcp-validation.md](dcp-validation.md)); the phase 2 acceptance
is the `full-integrity` profile, for SMPTE and Interop.
