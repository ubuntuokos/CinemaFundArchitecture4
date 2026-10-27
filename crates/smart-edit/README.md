# cfa4-smart-edit

Smart Cut / Smart Render planning for CFA4 (layer L3). Specification:
[`CFA4-SMART-CUT-RENDER-R1`](../../docs/plans/CFA4-SMART-CUT-RENDER-R1.md); starting point:
[WP-00 baseline audit](../../docs/audit/smart-edit-baseline.md).

The crate decides, it never executes: it says which parts of an export may be copied packet for
packet, which need their boundaries re-encoded and which must be fully rendered. Execution stays in
the existing export pipeline (`cfa4-export`, driven by `cfa4-engine`). No `unsafe`, no
dependencies beyond `serde` and the L0 crates `cfa4-time`, `cfa4-bitstream` and `cfa4-mpeg2v`; builds for `wasm32-unknown-unknown`.

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
  `Gstreamer`, `Ffmpeg`. No FFmpeg entry is registered: execution is CFA4's own Rust code
  (owner decision 2026-10-27, `docs/audit/smart-edit-baseline.md` §3) and ffmpeg stays an
  external validation oracle (AGENTS.md §2).
- Probes are applied in a canonical order; when two disagree about one capability, the unavailable
  one wins. The same entries and probes always give the same decisions and the same report.
- A Smart Render also needs an encoder whose limits (profile, chroma subsampling, bit depth) fit
  the source stream; unknown limits refuse it.

## Media / GOP index (WP-02)

```rust
use cfa4_smart_edit::{h264_index::*, media_index::*, mpeg2_index::*, timestamp::Rational};

// packets in decode order, straight from the container (MPEG-TS units, MP4 samples…)
let ix = index_h264(0, Rational::MPEG_90K, NalFormat::AnnexB, &[], packets)?;
// or index_mpeg2(0, timebase, packets)
ix.clean_access_points();   // IDR / closed-GOP pictures: safe lossless cut-in points
ix.gops;                    // access point + picture count + kind (IDR, closed, open, recovery point)
ix.frames[i].dependency_status; // independent, within GOP, crosses the access point, undecodable
let cached = MediaIndex::new("clip", fingerprint, vec![ix]);
cached.check_valid(&fingerprint_now)?; // size, modification time, content id, index version
```

The index comes from the bitstream, not from the container's keyframe flag:

- **MPEG-2:** sequence header + I picture = access point; `closed_gop` (or no leading B
  pictures) makes it clean, otherwise it is an open GOP whose leading B pictures refer to the
  previous GOP (or cannot be decoded at all after a `broken_link`). References are exact (latest
  one or two anchors); presentation order comes from the PTS, or from `temporal_reference`.
- **H.264:** IDR = clean access point; a non-IDR I picture is an access point only with a
  recovery point SEI (`recovery_frame_cnt` 0), and its leading pictures cross it; an I picture
  without one is `IntraOnly`, not a cut point. SPS / PPS payload changes per id are configuration
  changes. Reference lists are not resolved (`references` is `None`).
- Timestamps stay on the stream's rational timebase (`MediaTime`, exact conversion to `Tick`);
  missing PTS / DTS stay `None`; DTS going backwards and repeated PTS are reported.

## Tests

`cargo test -p cfa4-smart-edit`: synthetic MPEG-2 GOP structures (open, closed, broken link, missing
sequence header, configuration change, stream starting mid-GOP), H.264 from `cfa4-h264enc` and
hand-built NAL units (recovery point SEI, parameter-set changes, length-prefixed framing),
mutation fuzzing of both analysers, cache validity; `tests/index_oracle.rs` indexes ffmpeg-made
MPEG-TS references (MPEG-2 open / closed GOP, x264 closed / open GOP) and checks every picture
type and the presentation order against ffprobe. Capability registry: built-in table validity, the WP-03 gate (no Smart Cut allowed
before it is implemented, for every codec and container pair), composite decisions, encoder
limits, probe order independence, backend order, report round trip and hostile JSON.
