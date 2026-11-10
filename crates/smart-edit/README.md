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
// cut.allowed: packet copy is implemented for H.264 into MP4 / MOV (WP-04)
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
- **HEVC** (`index_hevc`, WP-07; base layer only): IDR = clean access point; a CRA or BLA picture
  is an access point (`cra` / `bla`), clean only when no RASL picture follows it. RASL pictures
  cross the access point (undecodable after a BLA or at the start of the stream); RADL and trailing
  pictures stay within the GOP. The picture type is the first slice segment's `slice_type` (it
  needs the PPS for `num_extra_slice_header_bits`; without it the type is `Unknown`, the access
  points are still found). A picture coded with other VPS / SPS / PPS payloads is a configuration
  change. The registry's HEVC `gop-index` and MP4 / MOV `packet-copy` rows are supported;
  re-encoding HEVC is not yet. A copy from an IDR stops before the RASL pictures of the next CRA
  (they are marked as crossing the access point), so in an open-GOP stream the safe cut-out is
  the end of the pictures before them.
- **AV1** (`index_av1`, WP-07; base spatial layer): one packet is one temporal unit. A unit that
  starts with a shown key frame is a clean access point (`key-frame`): it refreshes every
  reference slot. It needs a sequence header (from `av1C` or in-band). Hidden (forward) key frames
  and intra-only frames are intra but no cut points; a unit that only shows an existing frame
  decodes nothing new. Reference slots are not resolved. A changed sequence header is a
  configuration change. The registry's AV1 `gop-index` and MP4 / MOV `packet-copy` rows are
  supported (OBU framing, `av1C` kept); re-encoding AV1 is not yet. The executor classifies the
  copied temporal units again from the first one, so a copy from a key frame whose sequence
  header is neither in `av1C` nor in-band is refused.
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
  conversion, which is not registered yet. The built-in registry runs H.264 copy plans (WP-04) and
  H.264 boundary re-encodes (WP-06) into MP4 / MOV.

## Smart Cut executor (WP-04)

The executor lives in the export crate (`cfa4_export::smart_cut`), because this crate only plans.
`execute_mp4_copy(plan, stream_index, source, writer)` writes the copy segments of a lossless plan
from an MP4 / MOV source into a new MP4 / MOV file. It copies the sample payloads unchanged and keeps
the sample entry (`avcC` or `hvcC`), the durations and the composition offsets. Decode time starts at 0, and
an edit list starts the presentation at the first copied picture. `verify_mp4_copy` reads the result
back and compares every sample with its source sample (SCE-001 §18).

The executor refuses, before writing anything:
- a plan that re-encodes (`not-lossless`; the boundary re-encode executor below runs those);
- an index that no longer matches the source track (`stale-index`; for HEVC also an IDR, CRA or
  BLA picture where the index has none, or the other way round);
- copy segments that do not form one run in decode order;
- mixed sample entries;
- a plan, index and sample entry of different codecs;
- any codec or container other than H.264 or HEVC in MP4 / MOV.

The caller writes to a temporary file and renames it into place, like every export.

## Boundary re-encode executor (WP-06)

`cfa4_export::boundary_reencode` runs frame-accurate Smart Cut plans of one H.264 MP4 / MOV source.
`execute_mp4_boundary_reencode(plan, stream_index, source, encoding, writer)` copies the whole GOPs
as the copy executor does and decodes and re-encodes only the boundary pictures with the CFA4
H.264 encoder:

- **One stream:** the encoder uses parameter-set ids the source does not, so one `avcC` holds the
  source SPS/PPS unchanged followed by the encoder's. Every re-encoded segment starts with an IDR, a
  copy after one must start with an IDR, and neighbouring IDRs get different `idr_pic_id`s.
- **Compatibility first (SCE-001 §8):** Baseline, Main or High profile, 8-bit 4:2:0, progressive,
  4-byte NAL lengths, an even picture size, free parameter-set ids, a constant frame rate in the
  re-encoded pictures, no SEI user data (captions, unknown unregistered SEIs) in them and no
  in-band parameter sets; anything else is refused
  (`incompatible`) before a byte is written, never spliced unsafely. HDR mastering-display and
  content-light SEIs are carried into the re-encoded pictures. `check_mp4_boundary_reencode` makes
  the same checks without decoding, so a planner or the UI can show the refusal first (the
  registry's `StreamFormat` does not describe interlacing, frame rate or SEI).
- **Timing:** source durations are kept; the re-encoded pictures (no reordering) get the composition
  delay of the copied run, so the presentation has no gap at a splice.
- **Validation:** `verify_mp4_boundary_reencode` checks the copied samples against the source, the
  presentation timeline against the plan, decodes the whole output and compares every re-encoded
  picture with its source picture (luma PSNR, `BoundaryEncoding::min_psnr`).

Rendered segments (gaps, effects) and timeline plans are refused here (`render-segment`,
`timeline-plan`); they belong to the timeline Smart Render executor below. Writing and verifying decode the boundaries picture by picture (the
encoder's VUI comes from the source SPS), so no boundary is held in memory whole.

## Timeline Smart Render executor (WP-06)

`cfa4_export::timeline_render` runs a `plan_smart_render` plan into one H.264 MP4 / MOV stream.
`execute_mp4_timeline(plan, sources, frames, output, writer)` takes every source the plan reads
(`TimelineSource`: its index and MP4 track, by `source_id`), the caller's renderer for the `Render`
segments (`RenderedFrames`: 8-bit 4:2:0 pictures of the output size) and the output picture size,
frame duration and encoding (`TimelineOutput`):

- **Copied runs** keep their payloads. Every copied run must have the same `avcC`, timescale and
  composition delay; a run starts at an IDR unless it continues the previous run of the same source.
- **Re-encoded runs** (boundaries, `incompatible-stream`) are decoded from their source as in the
  boundary executor; their aspect ratio and colour must match the copied stream.
- **Rendered segments** (gaps, effects, transitions…) are encoded from the caller's pictures; the
  executor knows nothing of effects. With nothing copied the output is High profile at the
  timeline's frame rate.
- **One stream, constant frame rate:** the output `avcC` is the copied SPS/PPS followed by the
  encoder's, every encoded run starts with an IDR with an `idr_pic_id` its neighbours do not use,
  and every copied or re-encoded source picture must last exactly one output frame (the planner
  renders a clip at another frame rate).

`check_mp4_timeline` makes the same refusals without decoding or rendering.
`verify_mp4_timeline` compares the copied samples with their sources, checks that the output shows
one picture per frame without gaps, decodes the whole output and compares every re-encoded picture
with its source picture (luma PSNR); rendered pictures are checked to decode. The registry's
`timeline-render` rows are supported for H.264 into MP4 / MOV.

**Engine (`export.plan` / `export.start` with `"scope": "timeline"`).** `crates/engine/src/smart_render.rs`
turns the sequence's video (`"sequence"`, else the active one) into an `EditDecisionList` and plans
it. A range is copied only when nothing could change its pictures: exactly one clip is visible; it
is an H.264 MP4 / MOV file at the sequence's frame size, pixel aspect and frame rate, Rec. 709 with a
plain colour pipeline; it plays at 100 % with every effect at its defaults (Motion, Opacity and Time
Remapping included), no transition, no Interpret Footage override, and its edits are on the frame
grid. Any other range is rendered, with its `RenderCause`s (`source` for stills, generators,
graphics or files Smart Cut cannot read). The rendered pictures come from the export pipeline
(`cfa4_export::Pipeline`, full-resolution media). `export.plan` reports the plan, a summary
(copied, re-encoded and rendered frames) and `timelineRender: {ready, problem}`. `export.start`
needs `"confirm": true` when anything is re-encoded or rendered, verifies the output and only then
writes it. The output is the video from the sequence start to the end of its last video clip, without
audio.

`h264_stream_format(avc)` reads the profile, chroma format and bit depth from the source SPS for
the planner's `SmartCutRequest::format`. The engine commands use it: `export.plan` reports, for a
re-encoding plan, whether the executor can run it (`boundaryReencode: {ready, problem}`), and
`export.start` with `"confirm": true` runs it, checks the output and only then writes the file
(`"crf"` sets the quality of the re-encoded pictures, 16 by default).

## Smart Render planner (WP-05)

`plan_smart_render(&registry, &indexes, &edit_decision_list, &request, policy)` plans the export of
a whole timeline. The result is the same `ExportExecutionPlan`, and each segment names its source and
timeline item (`source`). The engine builds the `EditDecisionList` from a project snapshot:

- output ranges on the frame grid;
- each range filled by a source clip or a gap;
- the reasons the timeline changes a clip's pictures (`RenderCause`: effect, transition, composite,
  overlay, transform, speed, frame rate, colour conversion, proxy / nested).

The planner sorts every output range into one of these kinds:
- **Rendered** (`Render`):
  - a gap;
  - a changed clip, with its causes;
  - a clip in another codec;
  - a clip whose pictures do not fill its range exactly (another frame rate).
- **Copied with re-encoded boundaries:** an unchanged clip, planned like a frame-accurate Smart
  Cut. Each boundary segment's `decode` span starts at the clean access point before it and holds
  every picture its output pictures predict from.
- **Re-encoded for the seam** (`IncompatibleStream`): a copyable run whose codec configuration
  differs from the output stream, or is unknown. The output stream's configuration is fixed by the
  request or by the first copied run.

Unchanged clips that continue each other in the same source are joined first, so a razor cut alone
costs no re-encode. Output times are on the tick timebase (`TICK_TIMEBASE`). Execution is WP-06.

## Tests

`cargo test -p cfa4-smart-edit`: synthetic MPEG-2 GOP structures (open, closed, broken link, missing
sequence header, configuration change, stream starting mid-GOP), H.264 from `cfa4-h264enc` and
hand-built NAL units (recovery point SEI, parameter-set changes, length-prefixed framing),
mutation fuzzing of both analysers, cache validity; `tests/index_oracle.rs` indexes ffmpeg-made
MPEG-TS references (MPEG-2 open / closed GOP, x264 closed / open GOP) and checks every picture
type and the presentation order against ffprobe. Capability registry: built-in table validity, the Smart Cut gate (Smart Cut
allowed only where it is implemented, for every codec and container pair), composite decisions, encoder
limits, probe order independence, backend order from the routing table, report round trip and hostile JSON.
Smart Cut planner: whole-GOP copies, first / last picture, mid-GOP cuts in both modes with the
safe points, head and tail boundaries, leading pictures, open GOPs, configuration changes,
undecodable pictures, invalid ranges and timestamps, copy guarantees, encoder limits, JSON round
trip, an encoded H.264 stream, and random indexes and requests (the segments always cover the
range; copied packets output exactly their pictures).
Smart Cut executor (`cargo test -p cfa4-export smart_cut`): GOP-boundary cuts and whole streams into MP4
and MOV, with and without B-frames, read back and re-indexed. The tests also check refusals (re-encode plans,
stale indexes, malformed plans) and that a changed payload or a truncated file fails verification.
FFmpeg (oracle only) decodes the copy to exactly the source pictures of the range (framemd5).
HEVC (`smart_cut_hevc_tests`): GOPs of the CFA4 encoder copied unchanged with their `hvcC`, a stale
IDR refused, and x265 closed and open GOPs with B pictures decoded by FFmpeg to the source pictures.
Smart Render planner: whole-GOP clips, boundaries with their reference pictures, joined splits, gaps
and changed clips, codec configuration seams, other codecs and frame rates, refusals, registry
blockers, JSON round trip, and random timelines (the segments cover the output, decoding starts at a
clean access point and reaches every output picture).
