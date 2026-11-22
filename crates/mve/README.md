# cfa4-mve

Pure-Rust (no `unsafe`) support for **Interplay MVE** (`.mve`): the video decoder (formats 0x06,
0x10 and 0x11; 8-bit palette and 16-bit RGB 5:5:5 pictures), the sound decoder (Interplay DPCM and
8 / 16-bit PCM) and the file's chunk layout. It is an L0 crate: it depends only on `thiserror`.
Codec backlog item CB-25.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/interplayvideo.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2003 The FFmpeg project (Mike Melanson) | the video decoder: block opcodes, the two decoding passes, `copy_from` |
| FFmpeg `libavcodec/dpcm.c` (the Interplay part) | n7.1.1, LGPL-2.1-or-later | the DPCM sound decoder |
| FFmpeg `libavformat/ipmovie.c` | n7.1.1, LGPL-2.1-or-later, by Mike Melanson | the chunk and opcode layout |

MVE has no complete public specification. Porting FFmpeg's decoder is covered by the owner's
general permission for FFmpeg ports (2026-11-14). The crate keeps FFmpeg's licence
(LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for the ported code; CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Video formats 0x06, 0x10, 0x11 with every block opcode, decoding and skip maps | done |
| 8-bit palette pictures (palette changes between pictures) and 16-bit RGB 5:5:5 | done |
| Hidden pictures (a packet without a "send buffer" opcode) change the decoder's state only | done |
| DPCM and PCM sound, mono and stereo | done |
| File chunks as ffmpeg's demuxer finds them (a damaged tail ends the packets, the rest is kept) | done |
| `.mve` through `cfa4-codecs`: picture by picture in any order (decoder states kept every 64 pictures), sound from the middle | done |
| Damaged input: errors or garbage blocks, never panics | done |

## Tests

- `tests/oracle.rs`: movies whose every syntax element is random within the bitstream's rules,
  against ffmpeg's decoders (pictures and sound); damaged files without a panic.
- `crates/codecs/tests/mve_file.rs`: a `.mve` file through the media source, pictures asked for in
  a jumping order across the checkpoints, and the sound from the middle.
