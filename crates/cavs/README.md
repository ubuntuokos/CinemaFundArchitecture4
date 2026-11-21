# cfa4-cavs

Pure-Rust (no `unsafe`) decoder for **AVS1-P2** (GB/T 20090.2, "Chinese AVS"), Jizhun (base)
profile. It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-18.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers (Stefan Gehrer) | `cavs.c`, `cavsdec.c`, `cavsdsp.c` (C code paths), `cavs_parser.c`, the `cavsvideo` probe (re-expressed in `lib.rs`, `dec.rs`, `dsp.rs`, `bits.rs`); `cavsdec.c`, `cavs.c`, `cavsdata.c` (the tables in `tables.rs`) |

FFmpeg's decoder is the reference CFA4 matches. Porting it is an owner-approved exception to the
FFmpeg-only-in-tests rule (2026-11-14, as DV-3, CB-8.1 and CB-8.2). The ported code keeps
FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| Sequence header (Jizhun profile), I / P / B picture headers, slices, stream revision detection | done |
| Intra 8×8 luma and chroma prediction with FFmpeg's mode substitutions | done |
| P macroblocks (skip runs, 16×16 / 16×8 / 8×16 / 8×8, two references), B macroblocks (skip, direct, forward, backward, symmetric, 16×8 / 8×16 / 8×8 sub-types) | done |
| Motion vector prediction and scaling, quarter-sample luma and eighth-sample chroma MC (edges as FFmpeg's edge emulation) | done |
| 2D-VLC residuals with escapes, dequantisation, 8×8 inverse transform, in-loop deblocking | done |
| Packetisation as FFmpeg's parser, output order as FFmpeg's decoder (`low_delay`, reference delay, flush) | done |
| Interlaced streams | as FFmpeg decodes them (the 576-line FATE sample matches); slice weighting parameters are not read, as in FFmpeg |
| AVS+ (Guangdian profile), AVS2, AVS3 | not in this crate (FFmpeg's decoder refuses the Guangdian profile too) |
| Raw elementary streams through `cfa4-codecs` (by content), random access from I pictures | done |
| Damaged pictures: an error, never a panic | done |

Bit-exactness is against FFmpeg's C code paths (`-cpuflags 0`). FFmpeg's x86 SIMD inverse
transform works in 16 bits throughout and differs from its C code when coefficients overflow,
which conformant streams do not do.

## Tests

- `src/lib.rs`, `src/bits.rs`: packetisation, picture types, exp-Golomb codes, hostile input.
- `src/synth.rs` (feature `synth` for other crates' tests): a generator of random but valid
  streams written from the decoder's tables (every macroblock type and sub-type, legal intra modes,
  motion vectors, residual codes and escapes, skip runs, slices, quantiser and deblocking
  settings, `low_delay` and B-picture GOPs). Its test decodes 24 of them (more with
  `CAVS_SYNTH_CASES=n`; 400 pass locally) here and with ffmpeg: every picture equal, in order.
- `tests/fate_oracle.rs`: ffmpeg's FATE samples `cavs/bunny.mp4` and `cavs/cavs.mpg` as raw
  streams when present in `target/fixtures/cavs/fate` (local): all 230 pictures equal.
- `crates/codecs/tests/cavs_media.rs`: synthetic streams and the FATE samples opened by content,
  every frame asked for in a shuffled order against a straight decode; damaged streams without a
  panic.
