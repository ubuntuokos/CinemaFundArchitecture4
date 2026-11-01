# cfa4-dv

DV video decoder in pure Rust: consumer DV / DVCAM (IEC 61834-2, 25 Mbit/s) and DVCPRO 25
(SMPTE 314M). Layer L0: depends only on `thiserror` and (optional, feature `threads`, default
on) `rayon`. No `unsafe`, no panics on damaged input.

This is step DV-1 of the DV/DVCPRO plan: the crate decodes DIF frames. Wiring into the
importers (raw `.dv` / `.dif`, AVI, MOV, MXF; 4:1:1 enters as 4:2:2) is DV-2, DVCPRO 50
and DVCPRO HD are DV-3, DV audio is DV-4.

## Sources and licence

| Source | Edition / revision | Used for |
|---|---|---|
| libdv | 1.0.0 (Ubuntu `libdv_1.0.0.orig.tar.gz`, sha256 `a305734033a9c25541a59e8dd1c254409953269ea7c710c39e540bd8853389ba`), LGPL-2.1-or-later | AC VLC table (`vlc.c`), zigzag orders and area maps (`parse.c`, `quant.c`), quantisation step table and class offsets (`quant.c`), weighting constants (`weighting.c`), macroblock placement (`place.c`), the three-pass bit distribution (`parse.c`) |
| IEC 61834-2, SMPTE 314M | not consulted directly (not purchased) | libdv implements these; the DIF structure, VAUX packs and the 2-4-8 transform follow their published structure |

`vlc.rs`, `quant.rs`, `layout.rs` and `segment.rs` are derived from libdv and keep its licence
(LGPL-2.1-or-later, `COPYING.LGPL-2.1`); the rest of the crate is GPL-3.0-or-later. The port is
recorded in `docs/licensing.md` §5. The IDCTs (`idct.rs`) are written from the transform
definitions (orthonormal 8-8 DCT; 2-4-8 as two 4x8 DCTs of the field sum and difference), not
from libdv's AAN code. FFmpeg's DV code was not read; ffmpeg is used only as an external fixture
generator and test oracle.

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
- DVCPRO 50 (signal type 4) and DVCPRO HD (0x14, 0x15, 0x18) are recognised by `probe` and
  refused by `decode` with `Error::Unsupported`.

## Tests

- Unit tests: VLC table completeness, scan/area tables, IDCT round trips against the forward
  transforms, macroblock placement coverage of every format, split codes, overflowing blocks,
  random data.
- `tests/oracle.rs`: six ffmpeg-encoded fixtures (525 4:1:1, 625 4:2:0 16:9, DVCPRO 25 625,
  interlaced 2-4-8 at 525 and 625, noise with heavy bit spilling) decoded within ±3 of ffmpeg's
  decode, PSNR ≥ 48 dB (measured: max diff 2, PSNR 57-65 dB), no coding errors; bit flips,
  an erased DIF sequence and truncation never panic. `cargo xtask fixtures dv` generates the
  fixtures into `target/fixtures/dv`.
