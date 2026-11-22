# cfa4-adx

Pure-Rust (no `unsafe`) decoder for **CRI ADX** (`.adx`), the 4-bit ADPCM audio format of SEGA and
CRIWARE game middleware. It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-26.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/adxdec.c`, `libavcodec/adx.c`, `libavformat/adxdec.c` | n9.0.2, LGPL-2.1-or-later, Copyright (c) 2001, 2003 BERO, (c) 2011 Justin Ruggles | the header, the cutoff-derived predictor coefficients, the block decoder, the end-of-stream and partial-block rules |

ADX has no complete public specification; the format is documented only by implementations. Porting
FFmpeg code is an owner exception (general permission of 2026-11-14 for the remaining backlog codecs,
as for Dirac). The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself is used
only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Encoding 3 (18-byte blocks, 4-bit samples), 1 to 6 channels (the header version byte is ignored, as ffmpeg does) | done |
| Predictor from the header's cutoff and the sample rate (ffmpeg's `lrintf` rounding) | done |
| End-of-stream block (bit 15 of the scale), a trailing partial block group is dropped | done |
| `.adx` files through `cfa4-codecs` | done |
| Other encodings (fixed coefficients, exponential scale, AHX / MPEG audio), loop points, encrypted ADX | not supported: an error |
| Damaged input: errors, never panics | done |

## Tests

`tests/oracle.rs`, bit-exact against ffmpeg 9.0.2 (the CI oracle) and 6.1:

- 84 hand-built streams with random blocks: 1, 2, 3 and 6 channels, seven rate / cutoff pairs (down to a
  1 Hz rate and a 65535 Hz cutoff) and three scale ranges, so every nibble value and saturation occurs;
- an end block after the data, an end marker in the last channel's block mid-file, every partial-block cut;
- files written by ffmpeg's own ADX encoder (mono and stereo; it takes no more channels) decode the same
  as ffmpeg and carry the tone;
- header checks, and 3000 damaged files plus every truncation without a panic.
