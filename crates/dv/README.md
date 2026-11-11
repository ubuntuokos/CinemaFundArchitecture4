# cfa4-dv

DV video decoder in pure Rust: consumer DV / DVCAM (IEC 61834-2, 25 Mbit/s), DVCPRO 25 and
DVCPRO 50 (SMPTE 314M) and DVCPRO HD (SMPTE 370M). Layer L0: depends only on `thiserror` and (optional, feature `threads`, default
on) `rayon`. No `unsafe`, no panics on damaged input.

The crate decodes DIF frames (step DV-1 of the DV/DVCPRO plan). `cfa4-codecs` imports them
(DV-2) from raw `.dv` / `.dif` files (`DifSource`), type-2 AVI, MOV / MP4 and MXF; 4:1:1 enters
the engine as 4:2:2, and the stream info's aspect, sampling and colour come from the first
frame (`cfa4_codecs::video::dv_stream_info`). DVCPRO 50 and DVCPRO HD are DV-3 (`pro.rs`). DV-4
adds the audio (`decode_audio`, raw `.dv` files play it) and type-1 AVI video; type-1 AVI audio is
still to do.

## Sources and licence

| Source | Edition / revision | Used for |
|---|---|---|
| libdv | 1.0.0 (Ubuntu `libdv_1.0.0.orig.tar.gz`, sha256 `a305734033a9c25541a59e8dd1c254409953269ea7c710c39e540bd8853389ba`), LGPL-2.1-or-later | AC VLC table (`vlc.c`), zigzag orders and area maps (`parse.c`, `quant.c`), quantisation step table and class offsets (`quant.c`), weighting constants (`weighting.c`), macroblock placement (`place.c`), the three-pass bit distribution (`parse.c`), audio shuffle tables, samples-per-frame minimums, AAUX source pack fields and 12-bit expansion (`audio.c`, `dv_types.h`) |
| FFmpeg | n7.1.1, `libavcodec/dv.c`, `dv_internal.h`, `dvdec.c`, LGPL-2.1-or-later | DVCPRO 50 / HD only (`pro.rs`): macroblock coordinates of 4:2:2 SD, 1080i and 720p, the skipped segments of 1080i50 / 720p50, the 720p half-frame shift, the DV100 quantiser steps and weights, the 8-block layout and field / last-row placement. Owner-approved exception to the FFmpeg-only-in-tests rule (2026-11-11), as for the DTS port (#83) |
| IEC 61834-2, SMPTE 314M, SMPTE 370M | not consulted directly (not purchased) | libdv implements these; the DIF structure, VAUX packs and the 2-4-8 transform follow their published structure |

`vlc.rs`, `quant.rs`, `layout.rs`, `segment.rs` and `audio.rs` are derived from libdv and `pro.rs`
from FFmpeg; they keep their licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`); the rest of the crate
is GPL-3.0-or-later. The port is
recorded in `docs/licensing.md` §5. The IDCTs (`idct.rs`) are written from the transform
definitions (orthonormal 8-8 DCT; 2-4-8 as two 4x8 DCTs of the field sum and difference), not
from libdv's AAN code. For 25 Mbit/s DV FFmpeg's code was not read; only `pro.rs` ports it.
ffmpeg is also the external fixture generator and test oracle.

## API

```rust
let mut dec = cfa4_dv::Decoder::new();
for range in cfa4_dv::frames(&data)? {          // raw DIF stream: fixed-size frames
    let pic = dec.decode(&data[range])?;
    // pic.y / pic.cb / pic.cr: tight 8-bit planes at 720x480 or 720x576
    // pic.profile: format, system, chroma (4:1:1 / 4:2:0), aspect, frame_rate(), sample_aspect()
    // pic.concealed, pic.errors: damaged macroblocks and coding errors
}
let profile = cfa4_dv::probe(&frame);           // header + VAUX packs, no decoding
```

`decode` takes one whole DIF frame (a `.dv` frame, or one AVI / MOV / MXF sample). Frames are
intra-only and independent.

## Decoder

- DIF frames of 10 (525/60) or 12 (625/50) sequences; header (DSF, APT) and VAUX source /
  source control packs (signal type, 4:3 / 16:9).
- 525/60 4:1:1, 625/50 4:2:0 (IEC 61834) and 625/50 4:1:1 (DVCPRO 25, APT 1). The 4:1:1
  right-edge macroblocks (16x16, chroma 4x16) are handled.
- 8-8 and 2-4-8 DCT, classes 0-3, QNO 0-15, the three-pass distribution of AC bits (block,
  macroblock, video segment).
- IDCT in `f32`, rounded, +128 offset, saturated. Output has no setup removal or range change.
- `Picture::chroma_422()` upsamples 4:1:1 chroma horizontally to 4:2:2 (even samples copied,
  odd ones the rounded mean of their neighbours): CFA4 has no 4:1:1 frame type (owner decision,
  2026-11-01), so DV-2 hands 4:1:1 pictures to the engine as 4:2:2.
- Video segments decode in parallel on rayon (feature `threads`): about 10 ms per 625 frame
  single-threaded, 2.4 ms on 4 threads (release build).
- A missing or wrong-type DIF video block conceals its macroblock with mid-grey
  (`Picture::concealed`); invalid codes and blocks without end of block are counted
  (`Picture::errors`, `Decoder::errors`). Truncated frames are refused.
- DVCPRO 50 (signal type 4): 4:2:2 at 720x480 / 720x576 in two channels, 16x8 macroblocks.
- DVCPRO HD (signal type 0x14, 0x18): 1080i (1280x1080 at 59.94 Hz, 1440x1080 at 50 Hz, four
  channels) and 720p (960x720 at 59.94 or 50 frames/s, two channels), 4:2:2, BT.709 in
  `cfa4-codecs`. Eight blocks per macroblock, DV100 quantiser and weights, 8x8 DCT only;
  field-coded macroblocks (DCT mode bit of Y0) and the 32x8 macroblocks of the last 1080 row
  are placed as FFmpeg does. The coded size is anamorphic: `Profile::sample_aspect` gives 3:2
  (1280) or 4:3 (1440, 960) for the 16:9 picture.
- Signal type 0x15 is recognised by `probe` and refused by `decode` with `Error::Unsupported`
  (FFmpeg has no profile for it either).

## Audio

`decode_audio(frame)` returns the frame's PCM per channel (`Audio`), `audio_format(frame)` only
its format. The AAUX source pack (`0x50`, first sequence, audio block 3) gives the rate (48,
44.1, 32 kHz), the quantisation and the sample count (`source_pack_samples` reads it from the 5
bytes at `SOURCE_PACK_OFFSET`). 16-bit linear audio is two channels; 12-bit non-linear audio is
expanded to 16 bits and has a second channel pair when the second half's source pack carries
audio. Samples marked damaged (`0x8000`) repeat the sample before them (`Audio::errors`).
Pre-emphasis is reported (`AudioFormat::emphasis`) but not undone; 20-bit audio is not supported.

## Tests

- Unit tests: VLC table completeness, scan/area tables, IDCT round trips against the forward
  transforms, macroblock placement coverage of every format, split codes, overflowing blocks,
  random data.
- `tests/audio_oracle.rs`: ffmpeg-written 16-bit PCM at 48 kHz (525 and 625), 44.1 and 32 kHz
  (625): every sample and every frame's sample count equal ffmpeg's decode; damaged samples,
  truncation and garbage never panic.
- `tests/oracle.rs`: six ffmpeg-encoded fixtures (525 4:1:1, 625 4:2:0 16:9, DVCPRO 25 625,
  interlaced 2-4-8 at 525 and 625, noise with heavy bit spilling) and six DVCPRO ones (50 at
  625, and at 525 with 2-4-8 and 16:9; HD 1080i50 interlaced, 1080i60, 720p50, 720p60 noise)
  decoded within ±3 of ffmpeg's decode, PSNR ≥ 48 dB (measured: max diff 2, PSNR 57-74 dB), no
  coding errors; bit flips,
  an erased DIF sequence and truncation never panic. `cargo xtask fixtures dv` generates the
  fixtures into `target/fixtures/dv`.
