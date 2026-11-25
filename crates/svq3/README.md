# cfa4-svq3

Pure-Rust (no `unsafe`) decoder for **Sorenson Video 3** (`SVQ3`), the QuickTime codec that
Apple's H.264 draft grew out of. It is an L0 crate: it depends only on `thiserror`. Codec backlog
item K3 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/svq3.c`, `h264data.c`, `h264pred.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2003 The FFmpeg Project | the sequence header, the slice layout and its watermark scrambling, the interleaved Exp-Golomb codes, intra 4x4 / 16x16 and chroma prediction (with the SVQ3 variants), the dequantiser and integer transforms, motion vector prediction, full / half / third-pel motion compensation, B pictures and the output delay |
| `puff.c` by Mark Adler | zlib licence | the small zlib inflater (`src/inflate.rs`) for the watermark logo whose CRC is the scrambling key |

SVQ3 has no public specification. Porting FFmpeg's decoder is an owner exception, as for Dirac and
Shorten. The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself is
used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| I, P and B pictures, 16x16 down to 4x4 partitions, full / half / third-pel motion | done |
| Intra 4x4 and 16x16 prediction, chroma DC / plane | done |
| Watermark-scrambled slices (zlib logo in the sequence header) | done |
| Output delayed by one reference picture (`has_b_frames`), `flush`, `reset` | done |
| Missing references: a grey stand-in picture, as ffmpeg does | done |
| Media-key encryption of the stream | not done (an error, as in ffmpeg) |
| `SVQ3` in AVI, Matroska and QuickTime (`stsd` entry) through `cfa4-codecs` | done |
| QuickTime files with a compressed `moov` (`cmov`) | not done: the container reader lacks it |
| Damaged input: errors, never panics | done |

## Tests

- `tests/oracle.rs`: the FATE samples `Vertical400kbit.sorenson3.mov`, `svq3_decoding_regression.mov`
  and `svq3_watermark.mov` (never committed; fetch into `target/fixtures/fate/svq3/`, tests skip
  without them) are remuxed to AVI with `-c copy`; every output byte equals ffmpeg's. Damaged
  packets and sequence headers give errors, never panics; the picture types, output delay and
  `reset` are checked.
- `crates/codecs/tests/svq3_file.rs`: the samples through the media source (QuickTime and AVI),
  the same pictures as ffmpeg and cold seeks.
- The unit tests cover the zlib inflater and the tables.

## Two ffmpeg behaviours that are copied on purpose

- With no reference picture (a stream that starts on a P picture) ffmpeg predicts from a fake frame
  whose lower chroma rows stay 0; so does this decoder.
- In the half-pel averaging of 2-wide blocks ffmpeg's predictor is a plain put; so is this one.
