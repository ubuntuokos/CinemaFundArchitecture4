# cfa4-lagarith

Pure-Rust (no `unsafe`) decoder for **Lagarith** (`LAGS`), Ben Greenwood's lossless VfW video
codec. It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-3.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/lagarith.c`, `lagarithrac.c`, `lagarithrac.h`, `lossless_videodsp.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2009 Nathan Caldwell, David Conrad | the frame header, the probability header and its prefix code, the range decoder and its fixed-point probability scaling, the zero-run escape, the zero-run-only planes, the RGB, YUY2 and YV12 predictors |

Lagarith has no public specification. Porting FFmpeg's decoder is an owner exception, as for
Dirac and Shorten. The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for
the ported code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)).
ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Range-coded RGB24 (types 2, 4) and RGBA (8), YUY2 (3), YV12 (10) frames | done |
| Zero-run escape (1–3 zeros), uncompressed, zero-run-only and solid planes | done |
| Solid grey, colour and RGBA frames (5, 6, 9) | done |
| Obsolete arithmetic RGB (7), reduced-resolution (11) and uncompressed (1) frames | not done (ffmpeg has none either) |
| `LAGS` in AVI and Matroska through `cfa4-codecs` | done |
| Damaged input: errors, never panics | done |

## Tests

- `tests/oracle.rs`: a test encoder (the predictors, the probability header and a range coder)
  writes 9 AVI files of every frame type and plane mode, with even and odd sizes. Every frame
  decodes back to the source and byte for byte to ffmpeg's `lagarith` decoder. Zero-run-only
  planes on random data give the same pictures as ffmpeg, and the same refusals. Damaged frames
  decode without a panic.
- `crates/codecs/tests/lagarith_file.rs`: an AVI through the media source, in order and out of
  order.
