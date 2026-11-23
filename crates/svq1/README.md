# cfa4-svq1

Pure-Rust (no `unsafe`) decoder for Sorenson Video 1 (`SVQ1`). L0 crate, depends only on
`thiserror`. Codec backlog item CB-34.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)); Sorenson Video 3 is a separate crate.

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/svq1dec.c`, `svq1.h`, `svq1.c`, `svq1_cb.h`, `svq1_vlc.h`, the motion code table `ff_mvtab` of `h263data.c`, the half-pel `put_pixels` of `hpeldsp` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2002 The Xine project, The FFmpeg project | Frame header, quad-tree vector decoding, multi-stage codebooks, motion vector prediction, half-pel motion compensation |

SVQ1 has no public specification. Porting FFmpeg's decoder is covered by the owner's general
permission of 2026-11-14. The crate keeps FFmpeg's licence (`COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later. ffmpeg itself is only a test oracle.

`src/data.rs` is generated from FFmpeg's tables (the inter and intra codebooks, the VLC tables,
the frame size table).

## Status

`Decoder::new(width, height, extradata)`, `decode(frame) -> Result<Picture>` (planar YUV 4:1:0).
Key frames, delta frames (skip, one-vector and four-vector blocks, intra blocks), the
swapped-header variants, and ffmpeg's workaround for streams whose first frames carry temporal
reference 0 (only without container extradata). Embedded messages and checksums are skipped,
not verified.

Tests: bit-exact against ffmpeg on the FATE samples `svq1/ct_ending_cut.mov` and
`svq1/marymary-shackles.mov` (remuxed to AVI with `-c copy`; local only, in
`target/fixtures/fate/`, tests skip without them), damaged frames without a panic.
