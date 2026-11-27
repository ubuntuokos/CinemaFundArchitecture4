# cfa4-indeo

Pure-Rust (no `unsafe`) decoders for the Intel Indeo family. L0 crate, depends only on
`thiserror`. Codec backlog items CB-33.1 (Indeo 2 / 3) and CB-33.2 (Indeo 4 / 5)
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/indeo2.c`, `indeo2data.h` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2007 Konstantin Shishkov | Indeo 2 (`RT21`): code table, delta tables, intra and inter planes |

Indeo has no public specification. Porting FFmpeg's decoders is covered by the owner's general
permission of 2026-11-14. The crate keeps FFmpeg's licence (`COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later. ffmpeg itself is only a test oracle.

## Status

| Codec | Status |
|---|---|
| Indeo 2 (`RT21`, module `ir2`) | decoder done |
| Indeo 3 (`IV31`/`IV32`) | not done |
| Indeo 4 (`IV41`), Indeo 5 (`IV50`) | not done |

## Tests

FATE samples are fetched into `target/fixtures/fate/` (never committed), e.g.
`curl -O https://fate-suite.ffmpeg.org/rt21/VPAR0026.AVI`; tests skip without them.

- `tests/oracle.rs`: Indeo 2 `rt21/VPAR0026.AVI` (130 frames, intra and inter) equals ffmpeg's
  `indeo2` decoder byte for byte; damaged frames never panic.
