# cfa4-indeo45

Pure-Rust (no `unsafe`) decoders for Intel Indeo Video Interactive 4 (`IV41`) and 5 (`IV50`). L0
crate, depends only on `thiserror`. Codec backlog item CB-33.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)); Indeo 2 is in `cfa4-indeo`, Indeo 3 in
`cfa4-indeo3`.

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/ivi.c`, `ivi_dsp.c`, `indeo4.c`, `indeo5.c`, `indeo4data.h`, `indeo5data.h` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2009 - 2012 Maxim Poliakovski, 2009 Konstantin Shishkov | Headers, wavelet bands and tiles, run/value codes, Huffman descriptors, transforms, motion compensation, recomposition |

Indeo has no public specification. Porting FFmpeg's decoders is covered by the owner's general
permission of 2026-11-14. The crate keeps FFmpeg's licence (`COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later. ffmpeg itself is only a test oracle.

`src/data.rs` is generated from FFmpeg's tables (`ivi.c` run/value maps, the quantisation matrices
and scan tables of the two data headers).

## Status

Decoder: `Decoder::new(Codec::Indeo4 | Codec::Indeo5)`, `decode(frame) -> Result<Option<Picture>>`
(planar YUV 4:1:0). Indeo 4 B-frame pairs come out as in ffmpeg (the P frame sent with an I frame
is returned for the following null packet).

Refused, as in ffmpeg: DCT transforms, custom scan patterns and quantisation matrices (Indeo 4),
YV12 and extended transform info (Indeo 5), password-protected clips, scalable streams other than
four luma bands. Pictures are limited to 4096 x 4096 (and 9.4 M pixels).

## Tests

FATE samples are fetched into `target/fixtures/fate/` (never committed), e.g.
`curl -O https://fate-suite.ffmpeg.org/iv41/indeo41-partial.avi`; tests skip without them.

- `tests/oracle.rs`: `iv41/indeo41-partial.avi` (100 frames) and `iv50/Educ_Movie_DeadlyForce.avi`
  (133 frames) equal ffmpeg's `indeo4` / `indeo5` decoders byte for byte; damaged frames never
  panic. The wavelet (scalable), B-frame and tiled paths have no FATE sample (ffmpeg has no Indeo
  encoder); they are ported line by line but not checked against ffmpeg.
