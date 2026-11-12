# cfa4-dirac

Pure-Rust (no `unsafe`) decoder for SMPTE ST 2042-1 **VC-2**, the intra-only, wavelet-based
professional subset of the Dirac video codec. It is an L0 crate: it depends only on `thiserror`.
Codec backlog item CB-7.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)); Dirac core
syntax (CB-8.1) will extend this crate.

The decoder is a port of the normative ST 2042-1 pseudocode as published in the BBC VC-2
conformance software (`vc2_conformance` 2.0.0) and its data tables (`vc2_data_tables`), both
GPL-3.0-only, Copyright British Broadcasting Corporation ([register](../../docs/licensing.md)).
ffmpeg is used only as an external fixture generator and test oracle.

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
| Dirac core syntax (arithmetic coding, motion compensation) | rejected with `Error::Unsupported` (CB-8.1) |
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

## Tests

- `tests/ffmpeg_oracle.rs`: 12 streams from ffmpeg's `vc2` encoder (four wavelets, depth 1–5,
  default / colour / flat matrices, slice sizes, odd sizes, 8 / 10 / 12 bits, 4:2:0 / 4:2:2 /
  4:4:4) bit-exact against ffmpeg's decoder; field-coded pictures checked field by field;
  truncated and bit-flipped streams.
- `tests/synthetic_oracle.rs`: 19 streams written by the test with random coefficients (low
  delay, all seven wavelets, custom matrices, prefix bytes and size scalers, 10-bit) against
  ffmpeg, and asymmetric transforms against a known result.
- `crates/codecs/tests/vc2_oracle.rs`: QuickTime, Matroska and AVI files through the media
  source, frame by frame, also backwards.
