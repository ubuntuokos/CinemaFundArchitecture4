# cfa4-thp

Pure-Rust (no `unsafe`) support for **Nintendo THP** movies (`.thp`): the file layout, the JPEG
video decoder and the THP ADPCM sound decoder. It is an L0 crate: it depends only on `thiserror`.
Codec backlog item CB-28.1 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavformat/thp.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2007 Marco Gerards, (c) 2009 David Conrad | header, component list and the frame chain |
| FFmpeg `libavcodec/mjpegdec.c` (the `thp` decoder path) | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2000, 2001 Fabrice Bellard, (c) 2003 Alex Beregszaszi, (c) 2003-2004 Michael Niedermayer | baseline Huffman JPEG, DC prediction, restart handling |
| FFmpeg `libavcodec/simple_idct_template.c` (8-bit) | n7.1.1, LGPL-2.1-or-later (after mpeg2dec's idct_mmx.c by Aaron Holtzman) | the IDCT ffmpeg's JPEG decoder uses |
| FFmpeg `libavcodec/adpcm.c` (the THP part) | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2001-2003 The FFmpeg project | the ADPCM sound decoder |

THP has no public specification. Porting FFmpeg's code is covered by the owner's general permission
for FFmpeg ports (2026-11-14). The crate keeps FFmpeg's licence (LGPL-2.1-or-later,
`COPYING.LGPL-2.1`) for the ported code; CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Header, components (one video, at most one audio), frame chain as ffmpeg's demuxer reads it (a damaged tail ends the packets) | done |
| JPEG video: baseline 8-bit, 4:2:0 / 4:2:2 / 4:4:0 / 4:4:4 and grey; scan data without byte stuffing; restart intervals without markers (byte alignment and DC reset) | done |
| The pictures come out bit for bit as ffmpeg's `thp` decoder with its `simple_idct` | done |
| THP ADPCM sound, mono and stereo, with the history kept from packet to packet | done |
| `.thp` through `cfa4-codecs`: any frame in any order (every frame is a sync sample), sound from the middle | done |
| Progressive or 12-bit JPEG, other samplings | unsupported (an error) |
| Damaged input: errors or garbage, never panics | done |

## Tests

- `tests/oracle.rs`: files from a writer of ours (`tests/support/mod.rs`) that picks every syntax
  element at random (every sampling, sizes that are no multiple of 16, restart intervals, random
  coefficient tables and nibbles) against ffmpeg's `thp` demuxer and decoders; 300 damaged files
  without a panic. The coefficients stay moderate: with extreme ones ffmpeg's SIMD IDCT saturates
  16-bit intermediate values where the C code (and this port) wraps.
- `crates/codecs/tests/thp_file.rs`: a `.thp` file through the media source, frames asked for in a
  jumping order, and the sound from the middle.
