# cfa4-dcpomatic

DCP mastering backend that runs [DCP-o-matic](https://dcpomatic.com)'s command-line tools
(GPL-2.0-or-later, C++) as separate processes ([ADR 0011](../../docs/adr/ADR-0011-dcpomatic-external-mastering.md)).
DCP-o-matic is not part of CFA4: it is never linked, vendored or shipped. Install it yourself
([`docs/development/dcpomatic.md`](../../docs/development/dcpomatic.md)); without it the backend is
unavailable and the rest of CFA4 is unaffected. CFA4's own DCP export (`cfa4-export`) stays the default.

## Use

```rust
use cfa4_dcpomatic::{Container, Content, ContentKind, Control, DcpRequest, Dcpomatic};

if let Some(d) = Dcpomatic::locate() {
    let mut req = DcpRequest::new(vec![Content::new("/media/master.mov")], "/out/MyFilm", "My Film");
    req.content_kind = ContentKind::Feature;
    req.container = Some(Container::Flat);
    let out = d.make_dcp(&req, &Control::new().with_progress(|phase, f| println!("{phase:?} {f}")))?;
    println!("DCP in {}", out.dcp.display());
}
```

Engine commands: `dcpomatic.status` and `dcpomatic.makeDcp` (`crates/engine/src/dcpomatic.rs`).

`Dcpomatic::locate` finds `dcpomatic2_create` and `dcpomatic2_cli` in `CFA4_DCPOMATIC_BIN_DIR`, or
else on `PATH`. The oldest supported DCP-o-matic is **2.18.20** (`MIN_VERSION`), read from
`dcpomatic2_cli --version`.

## How a run works

1. `dcpomatic2_create --config <cfg> --output <film> --name … --no-encrypt --dcp-content-type …
   --standard SMPTE|interop [--twok|--fourk] [--container-ratio 185|239|190] [--dcp-frame-rate N]
   [--audio-channels N] [--video-bit-rate N] --twod|--threed [per content: --left-eye|--right-eye,
   --channel X, --gain dB] <absolute content paths>` writes the film directory.
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
- `--no-encrypt` always (ADR 0011 §4); `--no-remote` always, so no frame goes to network encoders.
- DCP-o-matic's configuration (signing chain) lives in `${XDG_CONFIG_HOME:-~/.config}/cfa4/dcpomatic`
  unless `config_dir` says otherwise.

## Tests

`cargo test -p cfa4-dcpomatic` drives fake tools (shell scripts): the exact argument arrays, the
private cleared environment, progress, version check, refused requests, engine errors with their
last message, missing tools, cancel (a background child in the group stops too) and timeout.
