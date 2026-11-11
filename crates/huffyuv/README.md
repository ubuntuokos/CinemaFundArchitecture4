# cfa4-huffyuv

Clean-room, pure-Rust (no `unsafe`) Huffyuv (`HFYU`) decoder. It is an L0 crate: it depends only
on `thiserror`, and builds for `wasm32-unknown-unknown`. Codec backlog item CB-1.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources

Huffyuv has no formal specification. The format facts come from Roberto Togni's public
description of the bitstream (`huffyuv.txt` on multimedia.cx, kept as the MultimediaWiki
`HuffYUV` page): the extradata fields, the run-length coded code lengths, the predictors, the
symbol order per colour format and the interlaced layout. What it leaves open was established
by black-box analysis of files that ffmpeg writes (ffmpeg is used only as an external fixture
generator and test oracle):

- codes are assigned from the longest length down, within a length in increasing symbol order,
  the running code halving at each shorter length;
- the bit stream is a sequence of 32-bit little-endian words, each read from its top bit;
- the raw first pixel reads as V Y1 U Y0 (YUY2), R G B x (RGB24) or A R G B (RGBA) in that order;
- gradient: the up-left sample of the first pixel of the second row is 0; median: the first two
  YUY2 pairs of the second row use the left predictor;
- extradata byte 2: 0x10 interlaced, 0x20 progressive (neither: the 1.x rule, taller than 288).

No decoder source code (the original codec, FFmpeg or any other) was read.

## Status

| Area | Status |
|---|---|
| YUY2 (4:2:2), RGB24, RGBA (32 bits per pixel) | done |
| Left, gradient and median prediction; RGB green decorrelation | done |
| Interlaced frames (fields side by side) | done |
| Huffman tables stored in the header (Huffyuv 2.x, every ffmpeg file) | done |
| RGB without decorrelation (old encoders) | implemented from the description, not oracle-tested (ffmpeg always decorrelates) |
| Huffyuv 1.x files using the built-in "classic" tables | rejected (`Error::Unsupported`): the tables are not publicly described |
| FFmpeg's `FFVH` variant (4:2:0, high bit depths, adaptive tables) | not supported (no description) |
| Damaged input: errors, never a panic | done |

Bit-exact with ffmpeg on 12 generated fixtures (`tests/ffmpeg_oracle.rs`: every predictor for
YUY2, RGB24 and RGBA, odd sizes, interlaced and tall progressive frames); the same test feeds
truncated and bit-flipped frames and headers to the decoder. About 40 fps for 1080p YUY2 on one
core (release build).

In the app, `cfa4-codecs` decodes Huffyuv from AVI and Matroska (`V_MS/VFW/FOURCC`); YUY2 comes
out as 8-bit 4:2:2, RGB as RGBA.

## API

```rust
// bit_count: BITMAPINFOHEADER biBitCount; extra: the header's bytes after the first 40
let mut dec = cfa4_huffyuv::Decoder::new(width, height, bit_count, &extra)?;
let pic = dec.decode(&chunk)?;   // every frame is a key frame
// pic.planes: Y, U, V (U, V half width) or R, G, B (, A), top row first
```
