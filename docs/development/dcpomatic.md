# DCP-o-matic – the DCP mastering backend (part of CFA4 only)

[ADR 0011](../adr/ADR-0011-dcpomatic-external-mastering.md) · crate:
[`cfa4-dcpomatic`](../../crates/dcpomatic/README.md) · commands: `dcpomatic.status`,
`dcpomatic.makeDcp`, `dcpomatic.makeKdm`.

> **Status (2026-11-13):** the approved plan [CFA4-DCPOMATIC-001 v1.0](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md)
> makes DCP-o-matic part of CFA4 only: the CFA4 package will ship its own pinned build (2.18.51) of the
> command-line tools in `/usr/lib/cfa4/dcpomatic/`, off `PATH`, with no menu entry or GUI (phases 2 and 5).
> Until then the draft code in PR #1173 still looks on `PATH` and in `CFA4_DCPOMATIC_BIN_DIR`.

CFA4 runs DCP-o-matic's command-line tools (`dcpomatic2_create`, `dcpomatic2_cli`, and for KDMs
`dcpomatic2_kdm_cli`) as separate programs.

## Build for development

Build the pinned tool chain from source as the [DCPOM-01 audit](../audit/DCPOM-01-DCPOMATIC-SURVEY-2026-11-13.md)
did (script: [`DCPOM-01-build.sh`](../audit/DCPOM-01-build.sh); about 12 minutes on 4 cores). The tools
need the patched OpenSSL binary `dcpomatic2_openssl` next to them. Point CFA4 at the build with
`export CFA4_DCPOMATIC_BIN_DIR=$DCPOM_DIR/prefix/bin` and
`LD_LIBRARY_PATH=$DCPOM_DIR/prefix/lib64`.

Check it:

```sh
cargo build --release -p cfa4-cli
echo '{"id":1,"method":"engine.execute","params":{"command":"dcpomatic.status","params":{}}}' \
  | target/release/cfa4-cli serve   # "available": true, "version": "2.18.x"
```

## Make a DCP

In the Qt GUI: **File ▸ DCP-o-matic…** (DCP and KDM tabs; the top line says whether DCP-o-matic is
installed). The same commands work over `cfa4-cli serve`, as below.

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

Check the result with CFA4's DCP validation ([dcp-validation.md](dcp-validation.md)).

## Encrypted DCP and KDMs

Add `"encrypt": true` to `dcpomatic.makeDcp`. The content keys stay in the film folder (`output`),
so **keep that folder**: every KDM is made from it. Then, for each projector:

```json
{"id": 3, "method": "engine.execute", "params": {"command": "dcpomatic.makeKdm", "params": {
  "film": "/work/MyFilm-dcpomatic",
  "certificate": "/certs/projector-leaf.pem",
  "output": "/work/kdms",
  "cinema": "Urania", "screen": "1",
  "validFrom": "2026-11-20 10:00:00",
  "validFor": {"count": 2, "unit": "weeks"}
}}}
```

Times are UTC (`now` or `YYYY-MM-DD HH:MM:SS`); give `validFor` or `validUntil`. Further options:
`formulation` (`modifiedTransitional1` default, `multipleModifiedTransitional1`, `dciAny`,
`dciSpecific`), `trustedDevices` (certificate files), `disableForensicMarkingPicture`,
`disableForensicMarkingAudio`. The answer lists the KDM files. To check a KDM in CFA4 itself, load it
with File ▸ Load KDM… (`dcp.loadKdm`) together with the recipient's private key.
