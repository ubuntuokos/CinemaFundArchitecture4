# cfa4-dirac

Pure-Rust (no `unsafe`) decoder for the **Dirac** video codec (core syntax: arithmetic coding,
motion compensation) and SMPTE ST 2042-1 **VC-2**, its intra-only, wavelet-based professional
subset. It is an L0 crate: it depends only on `thiserror`. Codec backlog items CB-7.2 (VC-2) and
CB-8.1 (Dirac core syntax) ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| BBC VC-2 conformance software | `vc2_conformance` 2.0.0 and `vc2_data_tables`, GPL-3.0-only, Copyright British Broadcasting Corporation | VC-2: the normative ST 2042-1 pseudocode (`bits.rs`, `header.rs`, `picture.rs`, `tables.rs`, `lib.rs`) |
| FFmpeg | n7.1.1, `libavcodec/dirac_arith.c`, `dirac_arith.h`, `diracdec.c`, `diracdsp.c`, LGPL-2.1-or-later | Dirac core syntax: the arithmetic decoder and its contexts (`arith.rs`), picture prediction parameters, block motion data, coefficient contexts and dequantisation offsets, overlapped block motion compensation, half-pel interpolation and sub-pel prediction (`core.rs`) |

`arith.rs` and `core.rs` keep FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`); the rest
of the crate is GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself is used
only as an external fixture generator and test oracle.

## Status

| Area | Status |
|---|---|
| Parse info, sequence header (all base video formats and presets, custom overrides), auxiliary and padding data, end of sequence | done |
| Low-delay pictures (slice sizes from a fraction, DC prediction) | done |
| High-quality pictures (prefix bytes, slice size scaler) | done |
| All seven wavelet filters, transform depth up to 10 | done |
| Asymmetric transforms (`dwt_depth_ho`, `wavelet_index_ho`, major version 3) | done |
| Default (Annex D) and custom quantisation matrices | done |
| 4:4:4, 4:2:2, 4:2:0; 8 to 16 bits; any frame size up to 16384 | done |
| Field coding (`picture_coding_mode` 1): fields woven into frames | done |
| Picture fragments (VC-2 v3) | rejected with `Error::Unsupported` |
| Dirac core syntax: intra pictures (arithmetic or exp-Golomb coding, codeblocks, quantiser deltas, any depth) | done |
| Dirac core syntax: inter pictures from one or two references, superblock splitting, DC blocks, every block size and motion vector precision (full, half, quarter, eighth pel), global motion, reference weights, zero residual | done (8-bit, arithmetic coding) |
| Dirac core inter pictures deeper than 8 bits or exp-Golomb coded, field pictures, Dirac low-delay pictures | rejected with `Error::Unsupported` (FFmpeg rejects them too) |
| Damaged input: errors, never panics; hostile sizes and integers capped | done |

## Where it differs from ffmpeg

ffmpeg's decoder departs from ST 2042-1 in three places; the BBC reference codec
(`vc2-reference`) settles them, and this crate follows the standard:

- Daubechies (9,7): ffmpeg drops the rounding of the shift after each level.
- Subbands one or two samples high: ffmpeg extends the edges differently.
- Field coding: ffmpeg's decoder rejects it (and its encoder signals `top_field_first` false
  while coding the top field first).

ST 2042-1:2017 Table 15.5 prints the second tap of the Fidelity filter as −10; the filter is
symmetric, and the BBC reference codec and ffmpeg both use 10, as does this crate.

Dirac core syntax follows FFmpeg sample for sample, including its edge handling (emulated edges
8 samples beyond the picture, 16-bit motion-compensation sums), with two exceptions where the
Dirac specification is followed instead:

- Reference weights other than the defaults: ffmpeg's C weighting scales most samples twice.
- Zero residual pictures: ffmpeg transforms the previous picture's coefficients again.

## Tests

- `tests/ffmpeg_oracle.rs`: 12 streams from ffmpeg's `vc2` encoder (four wavelets, depth 1–5,
  default / colour / flat matrices, slice sizes, odd sizes, 8 / 10 / 12 bits, 4:2:0 / 4:2:2 /
  4:4:4) bit-exact against ffmpeg's decoder; field-coded pictures checked field by field;
  truncated and bit-flipped streams.
- `tests/synthetic_oracle.rs`: 19 streams written by the test with random coefficients (low
  delay, all seven wavelets, custom matrices, prefix bytes and size scalers, 10-bit) against
  ffmpeg, and asymmetric transforms against a known result.
- `tests/core_oracle.rs`: 32 Dirac core syntax streams written by the test with random content
  against ffmpeg: intra pictures with every wavelet in arithmetic and exp-Golomb coding, inter
  pictures (I P B with one and two references) at every block preset and motion vector
  precision, custom blocks, global motion, 4:2:2 / 4:4:4, DC blocks; reference weights and zero
  residuals against the specification's result; 10-bit inter pictures rejected; 600 damaged
  streams without a panic. Three real Dirac files (Schrödinger and ffdirac, 419 pictures with
  inter pictures from one and two references) were also checked bit-exact while developing,
  directly and through AVI files frame by frame with seeks.
- `crates/codecs/tests/vc2_oracle.rs`: QuickTime, Matroska and AVI files through the media
  source, frame by frame, also backwards.
