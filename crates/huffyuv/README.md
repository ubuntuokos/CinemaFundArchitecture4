# cfa4-huffyuv

Pure-Rust (no `unsafe`) Huffyuv (`HFYU`) decoder, clean-room except for the Huffyuv MT (`HYMT`) slice
layout (below). It is an L0 crate: it depends only
on `thiserror`, and builds for `wasm32-unknown-unknown`. Codec backlog item CB-1.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources

**Two more parts are not clean-room (CB-36.1 follow-up, owner decision 2026-11-23: "Igen, FFmpeg-port"):**
the built-in "classic" tables of Huffyuv 1.x (`src/classic.rs`: run-length coded code lengths and the
explicit codes, which do not follow the code assignment of 2.x tables) and the 4:2:0 layouts (12 bits
per pixel, an ffmpeg encoder extension), both re-expressed from FFmpeg n9.0.2
`libavcodec/huffyuvdec.c` (LGPL-2.1-or-later, `COPYING.LGPL-2.1`; the owner's general FFmpeg-port
approval of 2026-11-14). The MultimediaWiki page says only that a 1.x decoder "must know the classic
tables" and has nothing on 4:2:0.

**Huffyuv MT (`HYMT`, CB-36.1)** is the other part not clean-room: the sliced packet layout (the 16-byte
trailer with slice count, height and table offset, independent slices, slice 0 at the bottom) is
re-expressed from FFmpeg n9.0.2 `libavcodec/huffyuvdec.c` (LGPL-2.1-or-later, `COPYING.LGPL-2.1`; the
owner's general FFmpeg-port approval of 2026-11-14 and the decision of 2026-11-23). Enable it with
`Decoder::set_slices(true)`; it applies to YUY2 only (as in FFmpeg). The rest of the crate
still rests on the public description and black-box analysis below.

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
| RGB without decorrelation (old encoders) | done (bit-exact on the sample `rgb_predleftnodecorr.avi`) |
| Huffyuv 1.x files using the built-in "classic" tables (no extradata, or `biBitCount` low bits set) | done: YUY2, RGB24 and RGBA, predictors from the low bits of `biBitCount` as in ffmpeg |
| 4:2:0 (12 bits per pixel, 2.x header): left, gradient and median predictors, progressive and interlaced | done, ported from ffmpeg: odd frame heights are refused |
| Adaptive tables (extradata flag `0x40`) and FFmpeg's `FFVH` variant (high bit depths, version 3 extradata) | not supported (`Error::Unsupported`) |
| Damaged input: errors, never a panic | done |

Bit-exact with ffmpeg on 12 generated fixtures (`tests/ffmpeg_oracle.rs`: every predictor for
YUY2, RGB24 and RGBA, odd sizes, interlaced and tall progressive frames); the same test feeds
truncated and bit-flipped frames and headers to the decoder. About 40 fps for 1080p YUY2 on one
core (release build).

`tests/synthetic_oracle.rs` checks what ffmpeg cannot encode against its decoder, with frames of random
bits that are valid streams of random residuals (the tables used are complete prefix codes): 4:2:0 in
every predictor, progressive, interlaced and by the 1.x height rule, in seven frame sizes; and the
classic tables for YUY2, RGB24 and RGBA, every predictor code, three frame sizes, interlaced by
the height rule (all bit-exact; ffmpeg has no median predictor for RGB). `tests/fate_samples.rs`
decodes 12 real files from ffmpeg's sample archive (`V-codecs/HuffYUV/`: `yuv_v1`, `rgb_v1`,
`huffyuv`, `cons-huffyuv`, every predictor, interlaced RGB) bit-exact against ffmpeg when they are in
`target/fixtures/fate/huffyuv/` (never committed; skipped when absent). The archive's 4:2:0 file
`archive/video/huffyuv/hfyu_small.avi` cannot be used: its header holds no valid Huffman table
(the first one has 255 entries), ffmpeg refuses it too ("Error reading huffman table").

Huffyuv MT is checked against ffmpeg's `hymt` decoder on packets assembled from ffmpeg-encoded
slices (`tests/hymt_oracle.rs`: left and gradient prediction, four slices, plus hostile trailers);
ffmpeg's median path ignores the slice offset, so a multi-slice median stream has no reference.
There is no real HYMT file to test with.

In the app, `cfa4-codecs` decodes Huffyuv and Huffyuv MT from AVI and Matroska (`V_MS/VFW/FOURCC`); YUY2 comes
out as 8-bit 4:2:2, 12-bit streams as 8-bit 4:2:0, RGB as RGBA.

## API

```rust
// bit_count: BITMAPINFOHEADER biBitCount; extra: the header's bytes after the first 40
let mut dec = cfa4_huffyuv::Decoder::new(width, height, bit_count, &extra)?;
let pic = dec.decode(&chunk)?;   // every frame is a key frame
// pic.planes: Y, U, V (U, V half width) or R, G, B (, A), top row first
```
