# cfa4-indeo3

Pure-Rust (no `unsafe`) Intel Indeo 3 (`IV31`, `IV32`) video decoder. L0 crate, depends only on
`thiserror`. Codec backlog item CB-33.1 (second half; Indeo 2 is in `cfa4-indeo`)
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/indeo3.c`, `indeo3data.h` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2009 - 2011 Maxim Poliakovski, 2009 Benjamin Larsson, 2010 Vitor Sessak | Frame header, binary-tree cells, vector quantisation tables, run-length codes, motion compensation |

Indeo has no public specification. Porting FFmpeg's decoder is covered by the owner's general
permission of 2026-11-14. The crate keeps FFmpeg's licence (`COPYING.LGPL-2.1`) for the ported
code; CFA4's changes are GPL-3.0-or-later. ffmpeg itself is only a test oracle.

The decoder gives planar YUV 4:1:0 (`Picture`); `cfa4-codecs` shows it as 4:2:0 by repeating
chroma samples (the frame type has no 4:1:0). Frames are limited to 16..=640 x 16..=480 with sizes
that are multiples of 4, as in ffmpeg. Sync (null) frames give `Ok(None)`.

## Tests

FATE samples are fetched into `target/fixtures/fate/` (never committed), e.g.
`curl -O https://fate-suite.ffmpeg.org/iv32/OPENINGH.avi`; tests skip without them.

- `tests/oracle.rs`: `iv32/OPENINGH.avi` equals ffmpeg's `indeo3` decoder byte for byte (all
  frames, key and delta); damaged frames never panic.
