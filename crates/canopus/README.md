# cfa4-canopus

Pure-Rust (no `unsafe`) decoder for the **Canopus** intra video codecs: **HQ** and **HQA** (AVI
FourCC `CUVC`) and **HQX** (`CHQX`). It is an L0 crate: it depends only on `thiserror`. Codec
backlog item CB-8.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `hq_hqa.c`, `hq_hqadsp.c`, `hqx.c`, `hqxdsp.c` (re-expressed in `hq.rs`, `hqx.rs`, `bits.rs`); `hq_hqadata.h`, `hqxvlc.c` (the tables in `tables.rs`, generated from them) |

There is no public specification of these formats; FFmpeg's decoders are the reference. Porting
them is an owner-approved exception to the FFmpeg-only-in-tests rule (owner decision 2026-11-12,
as DV-3 and #83). The ported code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are
GPL-3.0-or-later ([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| HQ: 8-bit 4:2:2, every one of the 22 profiles (sizes and slice permutations) | done |
| HQA: 8-bit 4:2:2 with alpha, any size | done |
| HQX: 4:2:2 / 4:4:4, with or without alpha, DC precision 9–11 bits, 12-bit output | done |
| Interlaced macroblocks (field DCT) | done |
| `INFO` block in front of a frame | skipped (aspect ratio and field order are not read) |
| AVI (`CUVC`, `CHQX`) and Matroska (VfW) through `cfa4-codecs` | done |
| Damaged frames: an error, never a panic | done |

A damaged HQX slice leaves its macroblocks black (FFmpeg leaves them uninitialised); a damaged
HQ or HQA slice ends the frame there, as in FFmpeg.

## Tests

- `src/tests.rs`: synthetic frames (random codes of every table, every HQ profile class, HQA
  sizes from 16×16 to 720×486, HQX in all four formats and three DC precisions, progressive and
  interlaced, up to 1920×1080) decoded by us and by ffmpeg's `hq_hqa` and `hqx` decoders: every
  sample equal; 2000 damaged frames without a panic.
- `crates/codecs/tests/canopus_oracle.rs`: an HQA frame muxed by ffmpeg into AVI and Matroska,
  and ffmpeg's FATE samples (`canopus/hq.avi`, `hq25i.avi`, `hqa.avi`, `hqx422.avi`,
  `hqx422a.avi`) when present in `$CANOPUS_SAMPLES` or `target/fixtures/canopus/fate`: every
  frame bit-exact against ffmpeg.
