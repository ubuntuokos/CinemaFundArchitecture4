# cfa4-ffv1

Clean-room, pure-Rust (no `unsafe`) FFV1 decoder, implemented from IETF RFC 9043, *FFV1 Video
Coding Format Versions 0, 1, and 3*. It is an L0 crate: it depends only on `thiserror`, and
builds for `wasm32-unknown-unknown`.

No code was taken or ported from FFmpeg or any other implementation; ffmpeg is used only as an
external fixture generator and test oracle. Codec backlog item CB-1.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Status

| Area | Status |
|---|---|
| Versions 0, 1 and 3 (parameters in key frames, or in the version 3 configuration record) | done |
| Version 2 (experimental, never standardised) | rejected with `Error::Unsupported` |
| Range coder with the default or a custom state transition table (`coder_type` 1 / 2), coded initial states | done |
| Golomb Rice coding with run mode (`coder_type` 0) | done |
| YCbCr with any chroma subsampling, gray (no chroma planes), transparency plane | done |
| RGB through the JPEG 2000 RCT, with or without transparency | done |
| 8 to 16 bits per sample | done |
| Slices (located from their footers), slice and configuration record CRC-32 | done |
| Non-key frames (context states carried over), sample aspect ratio, picture structure | done |
| Damaged input: errors (never panics); decoding resumes at the next key frame | done |

Bit-exact with ffmpeg on 26 generated fixtures (`tests/ffmpeg_oracle.rs`: every version, both
coders and state tables, slices with CRCs, odd sizes, gray 8 / 16, YCbCr 4:2:0 / 4:2:2 / 4:4:4 /
4:1:0 at 8, 10 and 16 bits, alpha, RGB 8 / 10 / 12 / 16 bits with and without alpha); the same
test feeds truncated and bit-flipped frames and configuration records to the decoder.

In the app, `cfa4-codecs` decodes FFV1 from AVI (`FFV1` FourCC, the record in the
`BITMAPINFOHEADER` extra bytes) and Matroska (`V_MS/VFW/FOURCC` and `V_FFV1`). Subsamplings the
frame type can't hold (4:1:0, 4:1:1, 4:4:0) are upsampled to 4:4:4, and RGB above 8 bits is
rounded to 8 bits.

## API

```rust
let mut dec = cfa4_ffv1::Decoder::new(width, height, &config_record)?; // empty for versions 0 / 1
let pic = dec.decode(&sample)?;     // one container sample per call
// pic.planes: Y, Cb, Cr (or R, G, B) as u16, pic.alpha, pic.bits, pic.format
let key = cfa4_ffv1::is_keyframe(&sample);
dec.reset();                        // after a seek
```

Width and height come from the container: FFV1 does not store them.

## Testing

```sh
cargo test -p cfa4-ffv1            # needs ffmpeg for the oracle tests (skipped otherwise)
FFV1_ONLY=gray cargo test -p cfa4-ffv1 matches_ffmpeg -- --nocapture
```
