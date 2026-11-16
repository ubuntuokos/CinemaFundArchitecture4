# cfa4-evc

Pure-Rust (no `unsafe`) decoder for the **Baseline profile of MPEG-5 EVC** (Essential Video
Coding, ISO/IEC 23094-1). It is an L0 crate: it depends only on `thiserror`. Codec backlog item
CB-17.1 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| xevd (Samsung, MPEG-5 EVC reference decoder) | v0.8.0 (commit `8fa744e865e9bdb94a5d5da6a4ab18268cdc4bb4`), BSD-3-Clause | the Baseline decoder of `src_base` (`xevd.c`, `xevd_eco.c`, `xevd_util.c`, `xevd_itdq.c`, `xevd_ipred.c`, `xevd_mc.c`, `xevd_df.c`, `xevd_picman.c`, `xevd_recon.c`), re-expressed in Rust; `xevd_tbl.c` (the tables in `tables.rs`, generated from it) |
| xeve (Samsung, MPEG-5 EVC reference encoder) | v0.7.1 (commit `fe229fec19e14907c60d65626ebd59f41517d9b3`), BSD-3-Clause | the arithmetic encoder of `xeve_eco.c`, mirrored in the test stream generator (`tests/synth`) |

The ported code keeps the BSD-3-Clause licence (`COPYING.BSD-3`); CFA4's changes are
GPL-3.0-or-later ([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| NAL units with 4-byte lengths (raw `.evc`, MP4 `evc1`), SPS (VUI read for the sample aspect ratio, timing and reordering), PPS, slice header | done |
| 64×64 coding tree units, quad-tree splits down to 4×4, any picture size up to level 6.2 | done |
| Intra: the five Baseline modes (DC, horizontal, vertical, up-left, up-right), most probable mode coding, constrained intra prediction | done |
| Inter: skip, motion vector prediction from the left, up, up-right and collocated units, motion vector differences, direct mode for B pictures, quarter-sample luma (8-tap) and eighth-sample chroma (4-tap) interpolation, bi-prediction | done |
| Run-length coefficient coding, dequantisation (with the chroma QP mapping tables of the SPS), inverse DCT up to 64×64, CU QP deltas | done |
| Deblocking filter | done |
| Reference structure from the temporal id (sub-GOPs, reference picture gap), IDR and CRA pictures, output in order count (bumping), cropping | done |
| 4:0:0 and 4:2:0, 8 to 12 bits (equal luma and chroma depth) | done |
| Main profile tools (binary / ternary splits, split unit coding order, ALF, DRA, adaptive MVR, affine, …), reference picture lists, several tiles or slices per picture | not in this crate (the Main profile is not in the backlog item) |
| 4:2:2, 4:4:4, different luma and chroma depths | refused (`Error::Unsupported`) |
| Damaged streams: an error, never a panic | done |

xevd parses a whole slice before reconstructing it; the port decodes and reconstructs one coding
unit at a time, which reads the same neighbour data.

## Tests

- `src/`: the inverse transform against the matrix product, a flat DC block, Exp-Golomb codes.
- `tests/decode.rs`: synthetic streams (`tests/synth`: the arithmetic coder of xeve and random
  coding decisions for every syntax element) in low-delay P and random-access B structures, in
  4:2:0 and 4:0:0, 8, 10 and 12 bits, with cropping, CU QP deltas and constrained intra
  prediction. A picture of intra units without residual is flat mid-grey, skip units with the
  collocated motion vector copy their reference, pictures come out in order count; damaged
  streams (truncated, bits flipped, random slices) without a panic.
- `crates/codecs/tests/evc_media.rs`: raw streams opened by content with frames in a shuffled
  order, `evc1` samples with an `evcC` record through the MP4 decoder factory, and an MP4 that
  ffmpeg remuxed from a raw stream (ffmpeg has no EVC decoder of its own).

A bit-exact comparison with xevd needs the reference decoder built as a test oracle, which waits
for the owner's approval.
