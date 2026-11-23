# cfa4-amv

Pure-Rust (no `unsafe`) decoder for **AMV** files (`.amv`): the chunk layout, the JPEG-variant video
and the IMA ADPCM audio of cheap MP4 players. L0 crate, depends only on `thiserror`. Codec backlog item
CB-32.1 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavformat/avidec.c` (AMV branch), `libavcodec/sp5xdec.c`, `sp5x.h`, `mjpegdec.c`, `mjpegdec_common.c`, `simple_idct.c`, `simple_idct_template.c`, `adpcm.c`, `adpcm_data.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) The FFmpeg project | Chunk walk (the `RIFF` / `LIST` sizes are zero, chunks are not padded), the fixed JPEG tables, scan decoding, the simple IDCT, ADPCM IMA AMV |

AMV has no public specification. Porting FFmpeg is covered by the owner's general permission of
2026-11-14; the crate keeps FFmpeg's licence (`COPYING.LGPL-2.1`) for the ported code, CFA4's
changes are GPL-3.0-or-later. ffmpeg itself is only a test oracle.

## Status

- `Amv::parse` reads the headers (picture size, frame period, the audio format) and the chunk
  table. `Decoder::new(width, height).decode(chunk)` gives the picture (4:2:0, full range, flipped
  right way up, cropped from the 16 × 16 macroblock grid). `decode_audio(chunk)` gives the 16-bit
  samples of one `01wb` chunk (each starts with its own predictor and step index).
- The video is ffmpeg's `sp5x_decode_frame` path for AMV: the chunk minus its first and last two
  bytes is the Huffman-coded scan of a baseline frame with two fixed quality-60 quantisation tables
  and the standard Huffman tables. The DC predictors start at the level shift, the dequantised
  coefficients are 16-bit like ffmpeg's, and the IDCT is ffmpeg's simple IDCT, so pictures match bit
  for bit.

Tests: bit for bit against ffmpeg on the FATE sample `amv/MTV_high_res_320x240_sample_Penguin_Joke_MTV_from_WMV.amv`
(local only, `target/fixtures/amv/fate/`; the test skips without it) and on files ffmpeg's own AMV
encoder writes (sizes that are and are not multiples of 16; CI); damaged chunks without a panic.
