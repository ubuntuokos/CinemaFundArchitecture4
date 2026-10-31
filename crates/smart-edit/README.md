# cfa4-smart-edit

Smart Cut / Smart Render planning for CFA4 (layer L3). Specification:
[`CFA4-SMART-CUT-RENDER-R1`](../../docs/plans/CFA4-SMART-CUT-RENDER-R1.md); starting point:
[WP-00 baseline audit](../../docs/audit/smart-edit-baseline.md).

The crate decides, it never executes: it says which parts of an export may be copied packet for
packet, which need their boundaries re-encoded and which must be fully rendered. Execution stays in
the existing export pipeline (`cfa4-export`, driven by `cfa4-engine`). No `unsafe`, no
dependencies beyond `serde`, builds for `wasm32-unknown-unknown`.

## Capability registry (WP-01)

```rust
use cfa4_smart_edit::capabilities::*;

let mut reg = Registry::native();              // the state the baseline audit documented
reg.apply_probes(probes)?;                     // run-time checks (hardware encoders…)
let d = reg.decide(Key::codec(Operation::Encode, Codec::Hevc), Policy::default());
// d.allowed, d.backend, d.level, d.reasons (one line per backend considered)
let cut = reg.smart_cut(Codec::H264, Container::Mp4, Container::Mp4, Policy::default());
// cut.allowed is false until GOP indexing (WP-02) and packet copy (WP-04) are implemented
let json = serde_json::to_string(&reg.report())?; // machine-readable, stable order
```

Rules:

- A capability without an entry is unsupported. Only `Supported` (tested) entries allow an
  operation; `Experimental` ones only with `Policy { allow_experimental: true }`.
- Backends are tried in a fixed order: `Native` (CFA4's own Rust crates), `Platform` (hardware),
  `Gstreamer`, `Ffmpeg`. No FFmpeg entry is registered: Smart Cut / Smart Render runs on CFA4's
  own Rust code (owner decision 2026-10-27, `docs/audit/smart-edit-baseline.md` §3) and ffmpeg
  is its validation oracle. ADR 0005 allows an FFmpeg ≥ 9.0.2 backend (`cfa4-ffmpeg`, through
  `cfa4-route`); its rows can be added once that crate exists and the owner decides so.
- Probes are applied in a canonical order; when two disagree about one capability, the unavailable
  one wins. The same entries and probes always give the same decisions and the same report.
- A Smart Render also needs an encoder whose limits (profile, chroma subsampling, bit depth) fit
  the source stream; unknown limits refuse it.

## Tests

`cargo test -p cfa4-smart-edit`: built-in table validity, the WP-03 gate (no Smart Cut allowed
before it is implemented, for every codec and container pair), composite decisions, encoder
limits, probe order independence, backend order, report round trip and hostile JSON.
