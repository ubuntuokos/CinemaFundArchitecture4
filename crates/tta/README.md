# cfa4-tta

Pure-Rust (no `unsafe`) decoder for **TTA** (True Audio, TTA1), a lossless audio codec. It is an
L0 crate: it depends only on `thiserror`. Codec backlog item CB-9.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| libtta | libtta-c 2.3 (2015), LGPL-3.0, Copyright (c) 1999-2015 Aleksander Djuric | the header, frame layout, adaptive Rice decoder, hybrid filter, fixed predictor and channel decorrelation (`libtta.c`, `filter.h`) |

The crate keeps libtta's licence (LGPL-3.0, `COPYING.LGPL-3.0`) for the ported code; CFA4's
changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). libtta 2.3 does not handle
8-bit audio; its predictor shift there (4 instead of 5) was checked against ffmpeg's decoder,
which is used only as an external fixture generator and test oracle.

## Status

| Area | Status |
|---|---|
| TTA1 header (after an ID3v2 tag), header and seek table CRCs | done |
| 8-, 16- and 24-bit audio, 1 to 8 channels | done |
| Password-protected streams (format 2) | done, with the password ([`Decoder::new`]) |
| Frame CRC: a damaged frame decodes to silence (as libtta) | done |
| `.tta` files and Matroska `A_TTA1` through `cfa4-codecs` | done |
| Damaged input: errors or silence, never panics | done |

## Tests

- `tests/oracle.rs`: 10 files from ffmpeg's TTA encoder (8 / 16 / 24 bits, mono to 8 channels,
  rates from 245 Hz to 96 kHz, a file shorter than one frame, a stream of whole frames)
  bit-exact against ffmpeg's decoder; 400 damaged files without a panic.
- `crates/codecs/tests/tta_oracle.rs`: `.tta` and Matroska files through the media source,
  every sample against ffmpeg, also read from the middle.
