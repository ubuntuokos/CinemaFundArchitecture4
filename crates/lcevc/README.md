# cfa4-lcevc

MPEG-5 Part 2 LCEVC (ISO/IEC 23094-2) in CFA4: detection of the enhancement layer and an own
Rust decoder for it (codec backlog CB-17.2). Layer L0, no dependencies, no `unsafe`, no panics on
damaged input.

LCEVC adds an enhancement layer on top of a base video (H.264, HEVC, …): the base is upscaled
once or twice (with an optional predicted-average correction) and residuals of two levels of
quality are added, the full-resolution ones through a temporal buffer.

## Decoder

`Decoder::decode(nal, base)` takes one LCEVC access unit (an Annex B NAL unit, start code
included) and the decoded base picture, and returns the enhanced picture with its conformance
window. `extract_lcevc` takes the access unit out of a base sample's T.35 SEI.
`Decoder::upscale_only` gives the upscaled base for a picture whose enhancement is missing or
damaged, and `Decoder::probe` the output size from an IDR access unit.

It is a port of V-Nova's LCEVCdec 4.3.1 (<https://github.com/v-novaltd/LCEVCdec>, commit
`e295d8438ba554bc541fb269baa9cbd567b626a8`): the scalar paths of
`src/enhancement/src/` (`config_parser.c`, `chunk_parser.c`, `huffman.c`, `entropy.c`,
`dequant.c`, `transform.c`, `transform_unit.c`, `decode.c`), `src/common/src/ln_fp_scaled.c`,
`src/pipeline_cpu/src/tasks_cpu.cpp` and the `pixel_processing` upscale and conversion loops.

Supported: every bitstream feature LCEVCdec's CPU pipeline decodes for progressive pictures
without tiles: 4:0:0 / 4:2:0 / 4:2:2 / 4:4:4, 8 to 14 bits (base and enhancement depths can
differ), DD (2×2) and DDS (4×4) transforms, 0D / 1D / 2D scaling per level, all upsample kernels
including the adaptive cubic, predicted average, prefix-coded and RLE-only chunks, the temporal
buffer with reduced signalling and refresh, deblocking, quantisation matrices (all modes),
dequantisation offsets, chroma step-width multiplier, user data in the residuals.

Not supported (the picture comes out as the upscaled base): tiled enhancement, field
(interlaced) pictures, dithering, S-filter (sharpening), and the `lvc1` track / TS
`stream_type` 0x36 carriages (detected, the base plays).

## Licence and patents

The ported files (`src/bits.rs`, `config.rs`, `entropy.rs`, `residual.rs`, `decoder.rs`) stay
under LCEVCdec's BSD-3-Clause-Clear licence, as LCEVCdec requires
([`LICENSE.LCEVCdec`](LICENSE.LCEVCdec)); the rest of the crate is GPL-3.0-or-later. The
licence grants **no patent rights**; LCEVC is patent-encumbered (patent register:
`docs/patent-review.md`).

## Detection

From V-Nova's public carriage documentation
(<https://docs.v-nova.com/technologies/mpeg-5.lcevc/carriage/overview-of-lcevc-carriages>):

- **T.35 SEI in the base stream**: SEI payload type 4 (`user_data_registered_itu_t_t35`) in an
  H.264 SEI NAL unit (type 6) or an HEVC prefix SEI NAL unit (type 39), starting with the UK
  country code `0xB4` and the register bytes `00 50 00`. `sample_has_lcevc` scans one access
  unit, Annex B or length-prefixed; emulation prevention bytes are removed first.
- **`lvc1` tracks** in MP4 / MOV (ISO/IEC 14496-15): the MP4 reader skips them when it chooses
  the picture track and reports the enhancement.
- **TS `stream_type` 0x36** (ISO/IEC 13818-1).

In `cfa4-codecs`, a T.35 SEI found in the first 8 samples of an H.264 or HEVC track (MP4 / MOV,
Matroska, MPEG-TS / PS / elementary streams) wraps the base decoder in `LcevcDecoder`: the
stream reports the enhanced size and its label gains ` + LCEVC`. The other carriages keep
` + LCEVC (enhancement not decoded)`.

## Tests

- Unit tests: SEI in H.264 and HEVC, both framings, other T.35 payloads (HDR10+), several
  messages in one SEI, emulation prevention, extraction, damaged-input fuzzing of the decoder.
- `tests/lcevc_oracle.rs`: a generator writes random LCEVC streams over random base pictures
  (every chroma format, plane mode, transform, scaling mode and kernel, both entropy codings,
  temporal signalling, deblocking, all quantisation-matrix modes, user data, 8 / 10 bits) and
  the output must equal LCEVCdec 4.3.1's bit for bit. The oracle (`lcevc_oracle`, built by
  `scripts/ci/install-lcevc-oracle.sh` from LCEVCdec with the driver in `tests/oracle/`) runs
  with SIMD and dithering off; the test skips without `CFA4_LCEVC_ORACLE`.
  `CFA4_LCEVC_SYNTH_ROUNDS` and `CFA4_LCEVC_SYNTH_SEED` set the number of streams and the seed.
- `cfa4-codecs` `tests/lcevc_detect.rs`: ffmpeg encodes H.264 (libx264) and HEVC (libx265), the
  test inserts an LCEVC T.35 SEI before every picture, ffmpeg remuxes to MP4, Matroska and
  MPEG-TS; the stream must report the enhanced size and decode, and the unmodified file must not
  be flagged.
