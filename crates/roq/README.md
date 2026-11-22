# cfa4-roq

Pure-Rust (no `unsafe`) support for **id RoQ** (`.roq`): the video decoder, the DPCM audio decoder
and the file's chunk layout. It is an L0 crate: it depends only on `thiserror`. Codec backlog item
CB-25.1 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/roqvideo.c`, `roqvideo.h`, `roqvideodec.c` | n7.1.1, LGPL-2.1-or-later, id RoQ video decoder by Dr. Tim Ferguson | codebooks, 2×2 / 4×4 vectors, motion blocks, the two-buffer scheme |
| FFmpeg `libavcodec/dpcm.c` (the RoQ part) | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2003 The FFmpeg project (Mike Melanson) | the DPCM sound decoder |
| FFmpeg `libavformat/idroqdec.c` | n7.1.1, LGPL-2.1-or-later, by Mike Melanson | the chunk layout: info, codebook + VQ, sound |

RoQ has no complete public specification. Porting FFmpeg's decoder is covered by the owner's general
permission for FFmpeg ports (2026-11-14). The crate keeps FFmpeg's licence (LGPL-2.1-or-later,
`COPYING.LGPL-2.1`) for the ported code; CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Video: codebook and VQ chunks, motion (8×8 and 4×4), 2×2 and 4×4 vectors, partial codebooks | done |
| The pictures come out as ffmpeg's (full range YUV 4:4:4), including its two-buffer scheme: a block a packet leaves alone keeps the pixels of the picture two packets back | done |
| DPCM sound, mono and stereo, 22.05 kHz | done |
| File chunks as ffmpeg's demuxer finds them (a damaged tail ends the packets, the rest is kept) | done |
| `.roq` through `cfa4-codecs`: picture by picture in any order (decoder states kept every 128 pictures), sound from the middle | done |
| Pictures whose size is not a multiple of 16 | unsupported, as in ffmpeg |
| Damaged input: errors or garbage blocks, never panics | done |

## Tests

- `tests/oracle.rs`: 40 streams whose every syntax element is random within the bitstream's rules
  (every block code, motion bias, partial and updated codebooks) and files ffmpeg's `roqvideo` /
  `roq_dpcm` encoders and `roq` muxer write (mono, stereo, silent), against ffmpeg's decoders;
  random sound chunks, mono and stereo; 300 damaged files without a panic.
- `crates/codecs/tests/roq_file.rs`: a `.roq` file through the media source, pictures asked for
  in a jumping order across the checkpoints, and the sound from the middle.
