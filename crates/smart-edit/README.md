# cfa4-smart-edit

Smart Cut / Smart Render planning for CFA4 (layer L3). Specification:
[`CFA4-SMART-CUT-RENDER-R1`](../../docs/plans/CFA4-SMART-CUT-RENDER-R1.md); starting point:
[WP-00 baseline audit](../../docs/audit/smart-edit-baseline.md).

The crate decides, it never executes: it says which parts of an export may be copied packet for
packet, which need their boundaries re-encoded and which must be fully rendered. Execution stays in
the existing export pipeline (`cfa4-export`, driven by `cfa4-engine`). No `unsafe`, no
dependencies beyond `serde`, `cfa4-route` (L1, for the backend order) and the L0 crates `cfa4-time`,
`cfa4-bitstream` and `cfa4-mpeg2v`; builds for `wasm32-unknown-unknown`.

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
- Which backend is tried first comes from the `cfa4-route` routing table (`BackendOrder`), not
  from this crate, with the router's own lookup: encoding and the boundary re-encode use the
  task `video.encode.<codec>`, decoding `video.decode` (the task ids `cfa4-platform` routes),
  each with its own row or else its family (shipped: hardware, FFmpeg, GStreamer, then `Native`,
  CFA4's own software codecs). The user's `routing.toml` steers the planner exactly as it steers
  the export routes (`Registry::set_backend_order`): a route the order leaves out is no
  candidate, and an entry or probe that names its concrete routes (`routes: ["nvenc"]`) is only
  admitted through one of them. Each route set keeps its own row (outcome and limits), so an
  unavailable `vaapi` probe does not hide an available `nvenc` one. Demux, GOP index, packet copy and mux have no routing task:
  `Native` (CFA4's own Rust crates) comes first there. No FFmpeg entry is registered: Smart Cut /
  Smart Render runs on CFA4's own Rust code (owner decision 2026-10-27,
  `docs/audit/smart-edit-baseline.md` §3) and ffmpeg is its validation oracle. ADR 0005 allows an
  FFmpeg ≥ 9.0.2 backend (`cfa4-ffmpeg`, through `cfa4-route`); its rows can be added once that
  crate exists and the owner decides so.
- Probes are applied in a canonical order; when two disagree about one capability (and route), the
  unavailable one wins; a probe for the whole backend covers all of its routes. The same entries
  and probes always give the same decisions and the same report.
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
  previous GOP (or cannot be decoded at all after a `broken_link`). The GOP header flags count
  even when the sequence header is not repeated. References are exact (latest
  one or two anchors); presentation order comes from the PTS, or from `temporal_reference`.
- **H.264:** IDR = clean access point; a non-IDR I picture is an access point only with a
  recovery point SEI (`recovery_frame_cnt` 0), and its leading pictures cross it; an I picture
  without one is `IntraOnly`, not a cut point; an IDR whose slice header cannot be read is none
  either. A picture coded with other SPS / PPS payloads than the one before it (a replaced
  parameter set or a switch to another `pic_parameter_set_id`) is a configuration change.
  Leading pictures are found from the PTS; without timestamps the pictures after a recovery point
  stay `Unknown`. Reference lists are not resolved (`references` is `None`): a picture after an
  intra picture but before any signalled access point is `Unknown`; before any intra picture it is
  `Undecodable`.
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
limits, probe order independence, backend order from the routing table, report round trip and hostile JSON.
