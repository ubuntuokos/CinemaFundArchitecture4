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
  admitted through one of them. Each route keeps its own row (outcome and limits), so an
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

## Smart Cut planner (WP-03)

```rust
use cfa4_smart_edit::{copy_planner::*, plan::*};

let req = SmartCutRequest {
    stream_id: 1,
    in_point, out_point,                 // Tick, on the stream's PTS timeline; out is exclusive
    mode: CutMode::FrameAccurate,        // or CutMode::GopBoundary: copy only
    guarantee: CopyGuarantee::BitExact,  // or Compatible; never switched silently
    source_container: Container::Mp4,
    output_container: Container::Mp4,
    format: Some(stream_format),         // for the boundary encoder's limits
};
let cut = plan_smart_cut(&registry, &media_index, &req, Policy::default())?;
cut.plan.segments;              // StrictCopy / CompatibleCopy / BoundaryReencode, in output order
cut.plan.requires_confirmation(); // something is re-encoded: ask the user first
cut.executable; cut.blockers;   // whether the registry allows every step now, and why not
cut.safe_points;                // nearest lossless cut-in / cut-out around the request
```

The planner reads nothing but its arguments and writes nothing. Its output is the common
`ExportExecutionPlan` of CFA4-SCE-001 v1.5 §8 (`plan.rs`), which the Smart Render planner
(WP-05) will extend to timelines.

- The output holds every picture shown during `[in_point, out_point)`.
- A **copy span** starts at a clean access point (IDR, closed GOP) and ends where the packets
  read so far in decode order output exactly a run of consecutive pictures. Every picture in it
  must be independent or refer only within its GOP, and no codec configuration change may fall
  inside. Leading pictures of a closed GOP are copied with it; an open GOP is no copy start.
- **GOP-boundary mode** copies only. A request that is not at such points is refused
  (`NotAtSafePoints`) with the nearest safe points before and after. The cut is never moved
  silently.
- **Frame-accurate mode** copies the longest copy span inside the range. The pictures before and
  after it are re-encoded, decoded from the latest clean access point before them; a range with
  no copy span is re-encoded as a whole. Pictures that cannot be decoded refuse the plan.
- **Executability** comes from the registry (`smart_cut`, plus `smart_render` with the encoder
  limits when something is re-encoded) and from the copy guarantee. A bit-exact copy cannot change
  the packet framing (H.264 from MP4 into MPEG-TS), and a compatible copy needs a proven
  conversion, which is not registered yet. Until packet copy (WP-04) and the boundary re-encode
  (WP-06) exist, the built-in registry returns every plan with `executable: false`.


`cargo test -p cfa4-smart-edit`: synthetic MPEG-2 GOP structures (open, closed, broken link, missing
sequence header, configuration change, stream starting mid-GOP), H.264 from `cfa4-h264enc` and
hand-built NAL units (recovery point SEI, parameter-set changes, length-prefixed framing),
mutation fuzzing of both analysers, cache validity; `tests/index_oracle.rs` indexes ffmpeg-made
MPEG-TS references (MPEG-2 open / closed GOP, x264 closed / open GOP) and checks every picture
type and the presentation order against ffprobe. Capability registry: built-in table validity, the WP-03 gate (no Smart Cut allowed
before it is implemented, for every codec and container pair), composite decisions, encoder
limits, probe order independence, backend order from the routing table, report round trip and hostile JSON.
Smart Cut planner: whole-GOP copies, first / last picture, mid-GOP cuts in both modes with the
safe points, head and tail boundaries, leading pictures, open GOPs, configuration changes,
undecodable pictures, invalid ranges and timestamps, copy guarantees, encoder limits, JSON round
trip, an encoded H.264 stream, and random indexes and requests (the segments always cover the
range; copied packets output exactly their pictures).
