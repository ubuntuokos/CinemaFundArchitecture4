# cfa4-cineform

GoPro CineForm (CFHD) decoder and encoder in pure Rust. Layer L0: depends only on `cfa4-bitstream`,
`thiserror` and (optional, feature `threads`, default on) `rayon`. Builds for
`wasm32-unknown-unknown`; no `unsafe`.

CineForm has no public standard; the reference is the open-source
[GoPro CineForm SDK](https://github.com/gopro/cineform-sdk) (Apache-2.0 OR MIT). Its codebooks
(`Common/table9.inc`, `Common/table17.inc`) were converted into `src/tables.rs`, and the tag
syntax, companding curves, peak tables and inverse 2/6 wavelet filters were re-expressed in safe
Rust from `Codec/codec.h`, `Codec/vlc.c`, `Codec/spatial.c` and
`Codec/InvertHorizontalStrip16s.c` (register entry in `docs/licensing.md` §5). No code was taken
from FFmpeg; ffmpeg is used only as a fixture generator and test oracle.

## Status

| Area | Status |
|---|---|
| Intra samples (spatial transform, 3 wavelet levels) | done, bit-exact vs ffmpeg |
| YUV 4:2:2 10-bit; RGB 4:4:4 and RGBA 4:4:4:4 12-bit | done, bit-exact vs ffmpeg |
| Band encodings: run-lengths with codebooks 9 / 17 / 18, difference coding, 16-bit bands, peak tables, solid lowpass | done |
| Display-height crop (e.g. 486 of 488 coded lines) | done |
| SDK and ffmpeg rounding of prescaled levels, SDK alpha curve ([`Flavor`], picked from the version tag) | done (SDK flavour: no SDK-written sample tested yet, #156) |
| Temporal (GOP / FIELDPLUS) samples | not yet (#154) |
| Bayer, YUVA 4:4:4:4, 3D (stereo) samples, uncompressed and lossless bands | not yet (#155) |
| Encoder: intra samples, YUV 4:2:2 10-bit, RGB 4:4:4 and RGBA 4:4:4:4 12-bit, six qualities (Film Scan 3 … Low) | done; ffmpeg decodes every sample exactly as our decoder |
| Encoder: widths that are not a multiple of 16 | not possible (no display-width field; ffmpeg's encoder refuses them too) |

Speed: about 33 ms to decode and 45 ms to encode a 1080p 4:2:2 frame on four cores (channels
run in parallel); the entropy coding of a band is sequential (#157).

## Encoder

`encode_frame` runs the exact inverse of the decoder's 2/6 filters (lowpass = pair sum,
highpass chosen so that the synthesis returns the pair), with the decoders' default prescale
(the middle levels' input is divided by 4), so without quantization the transform loses only
those prescale bits. Highpass bands are quantized (per-level quantizers set by the quality; a
band whose largest value would not fit the codebook gets a coarser quantizer instead of being
clipped), companded with codebook 17's cubic curve and run-length coded; the lowpass band is
written at 16 bits. The sample layout (tag order, level and band headers, the channel size
index) is the one decoders expect from the SDK and ffmpeg. Heights are coded as the next
multiple of 8 (last row repeated) and signalled as the display height.

## Tests

`tests/encoder.rs` encodes ffmpeg's `testsrc2` / `mandelbrot` frames at every quality and
several sizes (720×486 and 1080p included), muxes them into MOV and checks that ffmpeg's
`cfhd` decoder gives exactly our decoder's output, and that it is close to the source.

`tests/oracle.rs` encodes `testsrc2` / `mandelbrot` with ffmpeg's `cfhd` encoder at several
qualities and sizes into `target/fixtures/cineform/` and compares every sample with ffmpeg's
decode. Damaged (truncated, bit-flipped) streams must fail cleanly. Unit tests cover every
codeword, companding, peaks, the wavelet borders and hostile headers.

ffmpeg's encoder writes broken streams when a chroma lowpass band has an odd width (for example
720×486 4:2:2); ffmpeg cannot decode them either, so the crop test uses 704×486.
