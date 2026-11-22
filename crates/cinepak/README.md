# cfa4-cinepak

Pure-Rust (no `unsafe`) decoder for **Cinepak** (`cvid`), the 1990s vector-quantisation video
codec of AVI and QuickTime. It is an L0 crate: it depends only on `thiserror`. Codec backlog item
CB-32.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/cinepak.c` | n7.1.1, LGPL-2.1-or-later, Copyright (C) 2003 The FFmpeg project (Ewald Snel); Cinepak colorspace support (c) 2013 Rl, Aetey Global Technologies AB | the frame and strip headers, the V1 / V4 codebook chunks (4- and 6-element vectors), the vector chunks, the Sega FILM extra bytes, the damaged-frame rules |

Cinepak has no public specification. Porting FFmpeg's decoder is covered by the owner's general
permission of 2026-11-14 (FFmpeg ports for the remaining backlog codecs; as Lagarith, Dirac and
Shorten). The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself is
used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Intra (0x30, 0x32) and inter (0x31) vector chunks, V1 and V4 blocks, skipped blocks | done |
| Full and partial codebook chunks of 4 (YUV-less) and 6 elements (chroma, clipped) | done |
| Strips with absolute and relative y; codebooks shared or per strip (frame flag bit 0) | done |
| 8-bit palette video (palette from the `BITMAPINFOHEADER`) | done |
| Sizes that are no multiple of 4; Sega FILM extra bytes after the header | done |
| Damaged data: a partly updated picture flagged `damaged`, never a panic (like ffmpeg) | done |
| `cvid` in AVI, Matroska and QuickTime through `cfa4-codecs` | done |
| Palette changes inside an AVI (`00pc` chunks), QuickTime `ctab` palettes (the 8-bit QuickTime FATE sample is not even opened by the MP4 reader) | not done |

## Tests

- `tests/oracle.rs`: a random-stream writer (every chunk id, partial codebooks, skipped blocks,
  several strips, odd sizes, palette video) and ffmpeg's own Cinepak encoder write AVI files; every
  picture equals ffmpeg's `cinepak` decoder byte for byte. Damaged frames decode without a panic.
- `crates/codecs/tests/cinepak_file.rs`: an AVI through the media source, in order and out of order.
- FATE samples (local only, `target/fixtures/fate/cvid/`): `laracroft-cinepak-partial.avi` and
  `pcitva15.avi` through the media source equal ffmpeg's decoding byte for byte.
