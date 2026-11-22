# cfa4-acm

Pure-Rust (no `unsafe`) decoder for **Interplay ACM** audio (`.acm`). It is an L0 crate: it depends
only on `thiserror`. Codec backlog item CB-25.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/interplayacm.c` | n7.1.1, ISC, Copyright (c) 2004-2008 Marko Kreen, (c) 2008 Adam Gashlin, (c) 2015 Paul B Mahol (`COPYING.ISC-ACM`) | the block decoder: amplitude table, the column fillers, the inverse transform |
| FFmpeg `libavformat/acm.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2015 Paul B Mahol (`COPYING.LGPL-2.1`) | the 14-byte file header |

ACM has no complete public specification. Porting FFmpeg's decoder is covered by the owner's general
permission for FFmpeg ports (2026-11-14). CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Header, block shapes (`rows × 2^level`), 1 or more channels | done |
| Every column filler (zero, linear 3–16 bits, k13 k12 k24 k23 k35 k34 k45 k44, t15 t27 t37) | done |
| Inverse transform, 16-bit output, declared length | done |
| Bytes in any pieces; a block is decoded from one block's worth of bytes as ffmpeg does | done |
| A stream that breaks keeps the samples before the break | done |
| `.acm` files through `cfa4-codecs` (also read from the middle) | done |

## Tests

- `tests/oracle.rs`: streams whose every syntax element is random within the bitstream's rules
  (10 shapes, 6 rounds each) against ffmpeg's `interplayacm`; any packetisation; damaged streams
  without a panic.
- `crates/codecs/tests/acm_file.rs`: an `.acm` file through the media source.
