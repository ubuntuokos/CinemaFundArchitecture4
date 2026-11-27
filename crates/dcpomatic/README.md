# cfa4-dcpomatic

DCP mastering backend that runs [DCP-o-matic](https://dcpomatic.com)'s command-line tools
(GPL-2.0-or-later, C++) as separate processes (plan
[CFA4-DCPOMATIC-001](../../docs/plans/CFA4-DCPOMATIC-001-FINAL-1.0.md), [ADR 0011](../../docs/adr/ADR-0011-dcpomatic-external-mastering.md)).
DCP-o-matic is never linked or vendored. CFA4 ships its own pinned, command-line-only build (2.18.51) in
`/usr/lib/cfa4/dcpomatic/` (`PRIVATE_DIR`), off `PATH`; a DCP-o-matic the user installed is never used. Without
the build the backend is unavailable and the rest of CFA4 is unaffected. CFA4's own DCP export
(`cfa4-export`) stays the default. Setup: [`docs/development/dcpomatic.md`](../../docs/development/dcpomatic.md).

## Use

```rust
use cfa4_dcpomatic::{Container, Content, ContentKind, Control, DcpRequest, Dcpomatic};

if let Ok(d) = Dcpomatic::locate() {
    let mut req = DcpRequest::new(vec![Content::new("/media/master.mov")], "/out/MyFilm", "My Film");
    req.content_kind = ContentKind::Feature;
    req.container = Some(Container::Flat);
    let out = d.make_dcp(&req, &Control::new().with_progress(|phase, f| println!("{phase:?} {f}")))?;
    println!("DCP in {}", out.dcp.display());
}
```

Engine commands: `dcpomatic.status`, `dcpomatic.makeDcp` and `dcpomatic.makeKdm`
(`crates/engine/src/dcpomatic.rs`). `DcpRequest::encrypt` makes an encrypted DCP, `DcpRequest::subtitles` adds
a subtitle file as an open subtitle track, and `Dcpomatic::make_kdm` (`KdmRequest`) makes KDMs from the film
directory for a projector certificate.

`Dcpomatic::locate` takes `dcpomatic2_create`, `dcpomatic2_cli` and the patched OpenSSL
`dcpomatic2_openssl` (which makes the signing chain), and `dcpomatic2_kdm_cli` if it is there, from `PRIVATE_DIR`, or, for development, from
`CFA4_DCPOMATIC_BIN_DIR`. `PATH` is never searched. The version must be at least **2.18.51**
(`MIN_VERSION`), read from `dcpomatic2_cli --version`.

## How a run works

1. `dcpomatic2_create --config <cfg> --output <film> --name … --no-encrypt --dcp-content-type …
   --standard SMPTE|interop [--twok|--fourk] [--container-ratio 185|239|119|133|137|138|166|178] [--dcp-frame-rate N]
   [--audio-channels N] [--video-bit-rate N] --twod|--threed [per content: --left-eye|--right-eye,
   --channel X, --gain dB] <absolute content paths> [subtitle files]` writes the film directory
   (`--encrypt` instead of `--no-encrypt` for an encrypted DCP).
2. `dcpomatic2_cli --config <cfg> --no-remote [--threads N] make-dcp <film>` encodes the DCP into it.
   Its job status (`Transcoding: 42%`) is read from its stdout and reported as progress.
3. The DCP is the subfolder of the film directory that holds `ASSETMAP.xml` (SMPTE) or `ASSETMAP`
   (Interop).

- Every tool starts with an argument array (no shell), in its own process group, stdin closed.
- The environment is cleared: only `PATH`, `LD_LIBRARY_PATH` and the locale are passed on. `HOME`,
  `TMPDIR` and the working directory are a private directory (mode 0700), removed afterwards.
- Output goes to files, so no pipe can fill up.
- Cancel: `SIGINT` to the group, `SIGKILL` after 5 s. Timeout (48 h by default, `with_timeout`):
  `SIGKILL`. In both cases the partly written film directory is removed. After an engine error it is
  kept, since DCP-o-matic's log in it says what went wrong.
- The request is checked before anything starts: the film directory must not exist, content must
  exist, frame rate one of 24/25/30/48/50/60, even channel count 2–16 large enough for the channel
  mapping, bit rate ≥ 10 Mbit/s, eyes only in 3D, UTF-8 paths.
- `--no-remote` always, so no frame goes to network encoders.
- KDMs: `dcpomatic2_kdm_cli --output <dir> --valid-from … --valid-duration "N unit" | --valid-to …
  --formulation … --projector-certificate <pem> [--cinema, --screen,
  --trusted-device-certificate, --disable-forensic-marking-*] create <film>`. The KDM tool has no
  `--config`, so its private `XDG_CONFIG_HOME` holds a `dcpomatic2` link to the configuration folder.
- DCP-o-matic's configuration (signing chain) lives in `${XDG_CONFIG_HOME:-~/.config}/cfa4/dcpomatic`
  unless `config_dir` says otherwise.

## Tests

`cargo test -p cfa4-dcpomatic` drives fake tools (shell scripts): the exact argument arrays, the
private cleared environment, progress, version check, refused requests, engine errors with their
last message, missing tools, cancel (a background child in the group stops too), timeout, the
search (private folder or the development override, never `PATH`; the OpenSSL is required) and the container
ids DCP-o-matic accepts, KDM arguments and subtitle files. The CI job `.github/workflows/dcpomatic.yml` builds
the pinned DCP-o-matic, checks a real DCP with a subtitle track (SMPTE and Interop) with `cfa4-dcp-validate`'s
`full-integrity` profile, and runs the engine's encrypted DCP → KDM → playback test
(`cfa4-engine`, `dcpomatic::real_tests`).
