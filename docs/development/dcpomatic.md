# DCP-o-matic – installing the optional DCP mastering backend

[ADR 0011](../adr/ADR-0011-dcpomatic-external-mastering.md) · crate:
[`cfa4-dcpomatic`](../../crates/dcpomatic/README.md) · commands: `dcpomatic.status`,
`dcpomatic.makeDcp`.

CFA4 runs DCP-o-matic's command-line tools (`dcpomatic2_create`, `dcpomatic2_cli`) as separate
programs. They are not part of CFA4 and are not in Ubuntu's archive, so install them yourself.
Version **2.18.20 or newer** is needed.

## Install

- **Packages:** the DCP-o-matic project publishes `.deb` packages for current Ubuntu releases on
  its download page (<https://dcpomatic.com/download>). Install the one for your release with
  `sudo apt install ./dcpomatic_<version>_amd64.deb`; it puts both tools in `/usr/bin`.
- **From source:** follow DCP-o-matic's `BUILD.md` (waf, libdcp, libsub, FFmpeg, wxWidgets for
  the GUI tools). Point CFA4 at the build with
  `export CFA4_DCPOMATIC_BIN_DIR=/path/to/dcpomatic/build/src/tools` and, for a build installed
  into a prefix, `LD_LIBRARY_PATH=/prefix/lib`.

Check it:

```sh
cargo build --release -p cfa4-cli
echo '{"id":1,"method":"engine.execute","params":{"command":"dcpomatic.status","params":{}}}' \
  | target/release/cfa4-cli serve   # "available": true, "version": "2.18.x"
```

## Make a DCP

1. Export a master from CFA4 (`file.exportMedia`), e.g. ProRes 4444 in MOV with PCM audio, or a
   TIFF sequence plus a WAV.
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

It returns `{"job": N}`; follow it with `jobs.list` and stop it with `jobs.cancel`. With
`"wait": true` it returns `{"dcp": …, "filmDir": …, "bytes": …}` when done. The DCP is the folder
inside `output` that holds the asset map; the rest of `output` is DCP-o-matic's film directory
(settings and log).

Other parameters: `standard: "interop"`, `fps`, `audioChannels`, `videoBitRateMbps`, `threeD`
with `{"path": …, "eye": "left" | "right"}` content, mono audio files mapped with
`{"path": …, "channel": "L" | "R" | "C" | "Lfe" | "Ls" | "Rs" | "BsL" | "BsR" | "HI" | "VI", "gainDb": …}`,
`isdcfName: false` (folder named exactly `name`), `threads`, `configDir`.

Packages are unencrypted; encrypted DCPs and KDMs are not made by this backend (ADR 0011 §4).
Check the result with CFA4's DCP validation ([dcp-validation.md](dcp-validation.md)).
