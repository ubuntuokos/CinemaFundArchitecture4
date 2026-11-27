# cfa4-magicyuv

Pure-Rust (no `unsafe`) decoder for **MagicYUV** (`MAGY` and the per-format `M8..` / `M0..` /
`M2..` / `M4..` FourCCs), a lossless VfW video codec. It is an L0 crate: it depends only on
`thiserror`. Codec backlog item CB-2.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/magicyuv.c`, `vlc.c` (`ff_vlc_init_from_lengths`), `lossless_videodsp.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2016 Paul B Mahol | the frame header, the slice table, the run-length coded code lengths and their canonical codes, raw and Huffman slices, the left, gradient and median predictors, the RGB decorrelation |

MagicYUV has no public specification. Porting FFmpeg's decoder is an owner exception, as for
Dirac and Shorten. The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for
the ported code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)).
ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Version 7: 8-bit RGB, RGBA, YUV 4:4:4 / 4:2:2 / 4:2:0, YUVA 4:4:4, grey | done |
| 10-bit YUV 4:4:4 / 4:2:2 / 4:2:0 and grey; 10-, 12- and 14-bit RGB and RGBA | done |
| Slices, Huffman and raw slices, left / gradient / median prediction, interlaced | done |
| Colour matrix (BT.601 / BT.709) and full range flags | done |
| Other versions, slices narrower than the frame | not done (ffmpeg has none either) |
| AVI and Matroska through `cfa4-codecs` | done |
| Damaged input: errors, never panics | done |

## Tests

- `tests/oracle.rs`:
  - 63 AVI files from ffmpeg's encoder (7 formats, 3 predictions, 3 sizes and slice counts, a noise source for raw slices) decode to ffmpeg's pictures.
  - A test encoder writes 34 streams covering every format, progressive and interlaced, all three predictions, raw slices and complete and incomplete codes. They decode back to the source and to ffmpeg's pictures.
  - Damaged frames decode without a panic.
- `crates/codecs/tests/magicyuv_oracle.rs`: AVI and Matroska files from ffmpeg, frame by frame, also out of order.
