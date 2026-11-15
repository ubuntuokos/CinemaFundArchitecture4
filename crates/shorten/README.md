# cfa4-shorten

Pure-Rust (no `unsafe`) decoder for **Shorten** (`.shn`), Tony Robinson's lossless audio codec.
It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-9.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/shorten.c`, `golomb.h` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2005 Jeff Muizelaar | the stream header, the embedded WAVE / AIFF header parser, the Rice codes, the fixed (DIFF0–3) and QLPC predictors, the channel offset (mean) and bit shift |

Porting FFmpeg code is an owner exception (2026-11-14, as for Dirac). The crate keeps FFmpeg's
licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for the ported code; CFA4's changes are
GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself is used only as an external
test oracle.

## Status

| Area | Status |
|---|---|
| Versions 0 to 3, 1 to 8 channels | done |
| Fixed predictors DIFF0–3, QLPC, ZERO blocks, BITSHIFT, BLOCKSIZE | done |
| Embedded WAVE and AIFF / AIFC headers; 8-bit unsigned and 16-bit signed samples | done |
| `.shn` files through `cfa4-codecs` (also read from the middle) | done |
| Damaged input: errors, never panics | done |

## Tests

- `tests/oracle.rs`: 8 streams from a test encoder (versions 0–3, mono / stereo / 6 channels,
  8 and 16 bits, WAVE and AIFF, several block sizes, mean and LPC orders) decode losslessly and
  bit-exact against `ffmpeg -f shn`; the FATE sample `luckynight-partial.shn`, when it is in
  `target/fixtures/shorten/`, against ffmpeg; damaged streams without a panic.
- `crates/codecs/tests/shorten_file.rs`: a `.shn` file through the media source, from the start
  and from the middle.
