# cfa4-tak

Pure-Rust (no `unsafe`) decoder for **TAK** (Tom's lossless Audio Kompressor, `.tak`). It is an
L0 crate: it depends only on `thiserror`. Codec backlog item CB-10.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/takdec.c`, `tak.c`, `tak.h`, `takdsp.c`, `tak_parser.c`, `libavformat/takdec.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2012, 2015 Paul B Mahol | the file header and metadata blocks, the frame header and STREAMINFO, the CRC-24, frame finding, the residual codes, LPC subframes, channel decorrelation (modes 1–7, multichannel) |

TAK has no public specification. Porting FFmpeg's decoder is an owner exception, as for the Dirac
core and Shorten. The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for
the ported code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)).
ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Mono / stereo and multichannel codecs, 8 / 16 / 24 bits, 1 to 6 channels | done |
| LPC subframes, windowed residual codes, sample shifts, decorrelation modes 1–7 | done |
| `.tak` files (STREAMINFO, LAST_FRAME, MD5, ENCODER blocks; an ID3v2 tag before; bare frames) | done |
| Frames found as ffmpeg's parser finds them (sync code and header CRC) | done |
| A frame that does not decode becomes silence of its length (ffmpeg drops it) | done |
| `.tak` files through `cfa4-codecs` (also read from the middle) | done |
| Damaged input: errors or silence, never panics | done |

## Tests

- `tests/oracle.rs`: ffmpeg's FATE sample `luckynight-partial.tak`, when it is in
  `target/fixtures/tak/`, against ffmpeg; 8 streams of real audio coded losslessly by the test's
  encoder (8 / 16 / 24 bits, mono to 6 channels, short and long last frames) against the source
  and ffmpeg; 64 streams whose every syntax element is random within the bitstream's rules (LPC
  subframes, escape codes, sample shifts, every decorrelation mode, multichannel decorrelation)
  against ffmpeg; damaged streams without a panic.
- `crates/codecs/tests/tak_file.rs`: a `.tak` file through the media source, from the start and
  from the middle.
