# cfa4-utvideo

Clean-room, pure-Rust (no `unsafe`) Ut Video decoder. It is an L0 crate: it depends only on
`thiserror`, and builds for `wasm32-unknown-unknown`. Codec backlog item CB-2.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources

Ut Video has no formal specification. The format facts come from the public MultimediaWiki
`Ut_Video` page: the extradata words (encoder version, source format, frame information size,
flags with the slice count), the frame layout (per plane 256 code lengths, slice end offsets,
slice data; a frame information word at the end with the predictor), the slice rows and the
code construction (the longest code is all zeros, the shortest all ones). What it leaves open
was established by black-box analysis of files that ffmpeg writes (ffmpeg is used only as an
external fixture generator and test oracle):

- the bit stream is a sequence of 32-bit little-endian words, each read from its top bit;
- codes of equal length go to the symbols in decreasing order;
- a code length of 0 marks the only symbol of a plane (no bits), 255 an absent symbol;
- slice boundaries fall on whole chroma rows (4:2:0 luma slices start on even rows);
- left prediction runs over the whole slice from 0x80; median: the first row left, the first
  sample of the second row the one above, then continuous over row ends;
- RGB planes are G, B - G + 0x80, R - G + 0x80 (then alpha).

No decoder source code (the original codec, FFmpeg or any other) was read.

## Status

| Area | Status |
|---|---|
| `ULY0` / `ULH0` (4:2:0), `ULY2` / `ULH2` (4:2:2), `ULY4` / `ULH4` (4:4:4), `ULRG`, `ULRA` | done (`ULY` BT.601, `ULH` BT.709) |
| No, left and median prediction; any slice count | done |
| Gradient prediction | implemented with the median's edges, not oracle-tested (ffmpeg does not write it) |
| Interlaced streams (flag 0x800) | rejected (`Error::Unsupported`): the field layout is not described and ffmpeg does not write it |
| 10-bit `UQY0` / `UQY2` / `UQRG` / `UQRA`, `UM..` formats | rejected (`Error::Unsupported`): not described, no encoder to test against |
| Damaged input: errors, never a panic | done |

Bit-exact with ffmpeg on 14 generated fixtures (`tests/ffmpeg_oracle.rs`: every format ffmpeg
writes, every predictor it writes, 1 to 16 slices, odd sizes, a constant plane); the same test
feeds truncated and bit-flipped frames and headers to the decoder.

In the app, `cfa4-codecs` decodes Ut Video from AVI and Matroska (`V_MS/VFW/FOURCC`); YUV comes
out as 8-bit 4:2:0 / 4:2:2 / 4:4:4 with the FourCC's matrix, RGB as RGBA.

## API

```rust
// fourcc: BITMAPINFOHEADER biCompression; extra: the header's bytes after the first 40
let mut dec = cfa4_utvideo::Decoder::new(*b"ULY0", width, height, &extra)?;
let pic = dec.decode(&chunk)?;   // every frame is a key frame
// pic.planes: Y, U, V or R, G, B (, A), top row first; pic.matrix for YUV
```
