# cfa4-bink

Pure-Rust (no `unsafe`) decoders for **Bink 1** (RAD Game Tools): the video codec (revisions `b`
and `f` to `k`, 4:2:0 with an optional alpha plane), the **RDFT** and **DCT** audio codecs, and the
`.bik` container demuxer. It is an L0 crate: it depends only on `thiserror`. Codec backlog items
CB-23.1 (video) and CB-24.1 (audio) ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/bink.c`, `binkdsp.c`, `binkdata.h` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2009 Konstantin Shishkov, Daniel Verkamp, Peter Ross | the bundle and Huffman tree syntax, all block types, the integer IDCT, the scan, pattern and quantizer tables (`src/tables.rs`), the old `b` format |
| FFmpeg `libavcodec/binkaudio.c`, `wma_freqs.c` | n7.1.1, LGPL-2.1-or-later | the audio block syntax, bands and quantizer, overlap |
| FFmpeg `libavformat/bink.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2008-2010 Peter Ross, 2009 Daniel Verkamp | the header, the frame index, the per-frame packet layout |

Bink has no public specification. Porting FFmpeg's decoders is within the owner's general approval
of FFmpeg ports (2026-11-14); the crate keeps FFmpeg's licence (LGPL-2.1-or-later,
`COPYING.LGPL-2.1`) for the ported code and CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## API

- `container::Bink::parse(&[u8])` reads the header and frame index; `frame(i)` gives a frame's video packet and its audio packets, `layout(i)` their byte ranges. `container::probe` sniffs a file; an SMUSH wrapper is skipped.
- `video::Decoder::new(width, height, tag, flags)` and `decode(packet) -> Picture` (planes Y, U, V and A, cropped; `complete` is false when the packet ended before all planes were coded, the planes not coded are then the previous frame's).
- `audio::Decoder::new(sample_rate, channels, dct, revision)` and `decode(packet) -> Vec<f32>` (interleaved).

## Status

| Area | Status |
|---|---|
| Video revisions `b`, `f`, `g`, `h`, `i`, `k`, alpha | done |
| Revision `a` and Bink 2 (`KB2`) | not done (ffmpeg's demuxer does not take `a` either; ffmpeg has no Bink 2 decoder) |
| RDFT and DCT audio, old float header (`b`) | done |
| `.bik` and SMUSH-wrapped `.bik` through `cfa4-codecs` (first audio track) | done |
| Damaged input: errors, never panics | done |

## Tests

- `tests/video_oracle.rs`: ffmpeg cannot encode Bink, so a small encoder in `tests/common` writes the first frame of each stream (fill, pattern and raw blocks through the bundles), and for the old format the later frames too (skip, motion, DCT and residue blocks). The other frames are random bytes. Wherever ffmpeg decodes a frame, the picture is the same, and the frames each side refuses are the same. A frame that leaves pixels undefined (a skipped 16x16 part without its block; planes not coded) ends the comparison of that stream, as ffmpeg's content there is stale memory.
- `tests/audio_oracle.rs`: random coefficients through every run-length and width path, for RDFT and DCT, several rates and the old float header; the float output is within 100 dB SNR of ffmpeg's.
- `tests/robust.rs`: damaged and random files never panic; the container rules.
- `crates/codecs/tests/bink_oracle.rs`: a file through the media source against ffmpeg, frames last to first, and audio read out of order.
