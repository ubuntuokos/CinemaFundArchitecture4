# cfa4-hap

Clean-room, pure-Rust (no `unsafe`) Hap video decoder. It is an L0 crate: it depends only on
`thiserror`. Codec backlog item CB-3.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources

- The Hap specification (Vidvox, `HapVideoDRAFT.md`): section headers (4- and 8-byte forms),
  top-level section types, multiple-image sections, decode instructions with chunk compressor,
  size and offset tables, FourCCs.
- The Snappy format description (Google).
- The OpenGL `EXT_texture_compression_s3tc` (DXT1, DXT5) and `EXT_texture_compression_rgtc`
  (RGTC1) extensions.
- *Real-Time YCoCg-DXT Compression* (van Waveren and Castaño, 2007): scaled YCoCg in DXT5
  (Co, Cg, scale, Y in R, G, B, A).

The specifications give the texture interpolation as exact fractions. The integer rounding was
established by black-box comparison with ffmpeg (used only as an external fixture generator and
test oracle) over every 5- and 6-bit endpoint value and random texture blocks:

- 5:6:5 endpoints expand as `(v * 263 + 16) >> 5` (5 bits) and `(v * 259 + 32) >> 6` (6 bits);
- colour thirds and halves and alpha sevenths and fifths truncate;
- scaled YCoCg: `scale = (B >> 3) + 1`, `Co = (R - 128) / scale`, `Cg = (G - 128) / scale`
  (truncating), then `R = Y + Co - Cg`, `G = Y + Cg`, `B = Y - Co - Cg`, clamped.

No decoder source code (Vidvox's, FFmpeg's or any other) was read.

## Status

| Area | Status |
|---|---|
| `Hap1` (RGB DXT1), `Hap5` (RGBA DXT5), `HapY` (scaled YCoCg DXT5) | done |
| `HapM` (scaled YCoCg DXT5 + RGTC1 alpha), `HapA` (RGTC1 alpha only, as a grey matte, like ffmpeg) | done |
| No second-stage compression, Snappy, chunked decode instructions (with or without offsets) | done |
| Images of a multiple-image section in either order (the spec allows any order) | done; ffmpeg accepts only colour first |
| `Hap7` (BC7), `HapH` (BC6H) | rejected (`Error::Unsupported`); ffmpeg does not decode them either |
| Frame sizes that are not a multiple of 4 | done (edge blocks cropped) |
| Damaged input: errors, never a panic | done |

Tests: unit tests for Snappy, the block decoders and the section layout (with truncated and
bit-flipped frames); `crates/codecs/tests/hap_oracle.rs` is bit-exact with ffmpeg on QuickTime and
AVI files ffmpeg writes (Hap, Hap Alpha, Hap Q, uncompressed and Snappy, chunks) and on files
whose textures were replaced with random blocks (every DXT1 and DXT5 mode, YCoCg scales) or
rewritten as Hap Alpha-Only and Hap Q Alpha.

In the app, `cfa4-codecs` decodes Hap from QuickTime / MP4 and AVI to RGBA. Hap in Matroska
(`V_QUICKTIME`) is not read yet: the Matroska reader does not map that codec id.

## API

```rust
let mut dec = cfa4_hap::Decoder::new(*b"Hap1", width, height)?;
let pic = dec.decode(&sample)?;   // every frame is a key frame
// pic.rgba: width * height * 4 bytes, top row first; pic.alpha: whether alpha carries data
```
