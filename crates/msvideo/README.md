# cfa4-msvideo

Pure-Rust decoders for **Microsoft Video 1** (`MSVC`, `CRAM`, `WHAM`) and **Microsoft RLE**
(`BI_RLE8`, `BI_RLE4`, `mrle`, `WRLE`), codec backlog CB-35.1. Ported from FFmpeg n9.0.2's
`libavcodec/msvideo1.c`, `msrle.c` and `msrledec.c` (LGPL-2.1-or-later, `COPYING.LGPL-2.1`; an
owner-approved exception to the FFmpeg-only-in-tests rule, see `docs/licensing.md` §5). CFA4
changes are GPL-3.0-or-later.

| Codec | Pictures | API |
|---|---|---|
| Video 1 | 8-bit palette, RGB555; 4x4 blocks of 1, 2 or 8 colours and skip codes | [`video1::Decoder`] |
| RLE | 4-, 8- and 24-bit run-length pictures, and uncompressed 1-, 4-, 8- and 24-bit pictures | [`rle::Decoder`] |

Both are inter-frame: a delta picture changes part of the previous one, so a decoder keeps its
picture between packets and returns the whole picture as RGBA8 (top row first) after each.
`cfa4-codecs` wires them into AVI (`strf` compression `MSVC` / `CRAM` / `WHAM`, 1, 2, `mrle`,
`WRLE`) and Matroska (VfW FourCC); the media source decodes from the nearest key frame.

## Fidelity

`tests/oracle.rs` compares every picture with ffmpeg's (external oracle only, never linked) as
RGB24: generated streams that use every block and run type at several sizes (a first picture that
covers everything, then delta pictures with skips), and random bytes as delta pictures (Video 1
and 4-bit RLE; an 8- or 24-bit run may overshoot into the row padding of ffmpeg's buffers, whose
size depends on the build, so those are not compared on random data). RGB555 is widened by
repeating the top bits, as ffmpeg's conversion does.

## What is not covered

- Palette-change chunks (`##pc`) in AVI: the palette in the stream header is used throughout
  (ffmpeg applies a change to the pictures after it).
- RLE at 16 and 32 bits and RLE-coded 1-bit pictures: ffmpeg's `msrle` decoder has none either
  (a 1-bit picture stays as it was unless the packet is an uncompressed picture).

## Robustness

No `unsafe`, no panics on any packet: a damaged or short packet leaves the rest of the picture as
it was, as in ffmpeg (a Video 1 packet below the minimum size and an RLE packet below two bytes
are refused).
