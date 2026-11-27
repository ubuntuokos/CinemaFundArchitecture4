# cfa4-sstp

Pure-Rust (no `unsafe`) decoder for the **MPEG-4 Simple Studio Profile** (SStP, ISO/IEC 14496-2
Amd. 1), the codec of Sony's HDCAM SR and SRMASTER ("Sony SR"). It is an L0 crate: it depends only
on `thiserror`. Codec backlog item CB-16.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | the studio parts of `mpeg4videodec.c`, `h263dec.c`, `ituh263dec.c`, `simple_idct_template.c` (32-bit, 10-bit IDCT) and `vlc.c` (re-expressed in `lib.rs`); `mpeg4data.h`, `mpegvideodata.c` (the tables in `tables.rs`, generated from them) |

FFmpeg's decoder is the reference CFA4 matches. Porting it is an owner-approved exception to the
FFmpeg-only-in-tests rule (as DV-3, CB-8.1 and CB-8.2). The ported code keeps FFmpeg's licence
(`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| Headers: visual object sequence, visual object, studio video object layer, GOP, user data, quantiser matrix extension | done |
| 10-bit 4:2:2, 4:4:4 and RGB (G, B, R planes), any size up to 8192 × 8192 | done |
| DCT macroblocks: studio DC and AC VLCs, intra DC precision 8–11 bits, DCT precision, MPEG or H.263 quantisation, non-linear quantiser scale, alternate scan, quantiser matrices | done |
| DPCM (lossless) macroblocks | done |
| Slices (resynchronisation after a damaged slice, as FFmpeg) | done |
| Pixel aspect ratio and frame rate from the video object layer | done |
| Bit depths other than 10, studio P / B pictures, non-rectangular shapes | refused (`Error::Unsupported`); FFmpeg decodes none of them either |
| Simple / Advanced Simple Profile (DivX, Xvid, 3GP) | not in this crate |
| Raw elementary streams (`.m4v`, by content) and MXF (MPEG-4 Visual essence) through `cfa4-codecs` | done |
| Damaged pictures: an error, never a panic | done |

The strict mode (`Decoder::set_strict`) refuses a picture whose decoding FFmpeg leaves undefined
or conceals: a read past the end, an integer overflow in the dequantiser, a macroblock that no
slice covers, slices out of order.

## Tests

- `src/lib.rs`: the code tables are complete, a flat DC block, random packets without a panic.
- `tests/sstp_oracle.rs`: ffmpeg's FATE sample `mpeg4/mpeg4_sstp_dpcm.m4v` when present in
  `target/fixtures/sstp` (local), and synthetic streams in 4:2:2, 4:4:4 (with quantiser
  matrices) and RGB, with every precision, scan and quantiser option, against ffmpeg's `mpeg4`
  decoder: every sample equal. Every macroblock is random bits the strict decoder accepts
  (`tests/synth`), built one macroblock at a time.
- `crates/codecs/tests/sstp_oracle.rs`: synthetic 4:2:2 and RGB streams opened by content as raw
  streams and through a byte reader, and the same pictures as MPEG-4 Visual essence in MXF:
  every frame bit-exact against ffmpeg in a shuffled order; damaged streams without a panic.
