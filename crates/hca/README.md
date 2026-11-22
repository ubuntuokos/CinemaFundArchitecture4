# cfa4-hca

Pure-Rust (no `unsafe`) decoder for **CRI HCA** (High Compression Audio, `.hca`), the MDCT audio codec of
CRIWARE game middleware. It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-26.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/hcadec.c`, `libavcodec/hca_data.h`, `libavformat/hca.c` | n9.0.2, LGPL-2.1-or-later, Copyright (c) 2020 Paul B Mahol (demuxer) and the FFmpeg developers | the header chunks, the cipher, the ATH, the scale-factor / spectrum unpacking, intensity stereo, high-frequency reconstruction, the window and all tables |

The port is from **n9.0.2**, not n7.1.1: up to n7.1.2 ffmpeg does not skip the two reserved bytes of the
`comp` chunk, so it never sees the `ath` and `ciph` chunks behind it (fixed in n8.0). HCA has no public
specification. Porting FFmpeg code is an owner exception (general permission of 2026-11-14 for the
remaining backlog codecs). The crate keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for
the ported code; CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself
is used only as an external test oracle.

The inverse MDCT is not ported: ffmpeg uses its float FFT (`av_tx`, length 128, scale 1/8, the middle
half of the IMDCT); this crate computes the same transform as a DCT-IV through a 64-point FFT in double
precision, so the output equals ffmpeg's up to float rounding (about 3e-7 of the peak).

## Status

| Area | Status |
|---|---|
| Header chunks `fmt`, `comp` / `dec`, `vbr`, `ath`, `ciph`, `loop`, `rva`, `comm`, `pad`; masked tags | done |
| 1 to 16 channels, tracks, channel configurations, stereo / intensity bands, HFR groups | done |
| ATH type 0 and 1 (type 1 from the sample rate) | done |
| Cipher type 56 with key and subkey (`Key`, `cipher_table`); unencrypted streams | done |
| Cipher type 1 (fixed table, ffmpeg does not support it either) | not supported: an error |
| Encrypted stream without its key | error `KeyRequired` |
| A block without the sync word | silence for that block, counted by `decode_file` |
| Block CRC | not checked (ffmpeg checks it only with `-err_detect crccheck`) |
| `.hca` files through `cfa4-codecs` (unencrypted); keyed streams only through this crate's API | done |
| Damaged input: errors or silence, never panics | done |

## Tests

`tests/oracle.rs`, against ffmpeg 9.0.2 (the CI oracle; the chunk and key tests skip on ffmpeg before 8.0):

- 84 streams of random blocks behind a written header: mono to 16 channels in 1 or 2 tracks, every
  channel configuration, seven band layouts (stereo and HFR groups included), seven sample rates (96 kHz
  reaches the ATH curve's end); worst relative error 2.6e-7 of the peak;
- every header variant: versions 1.1 to 3.0, `comp` and `dec` chunks, ATH chunk types, the optional
  chunks, masked tags;
- encrypted streams (seven keys and subkeys; the stream is written with the inverse of the key's table),
  a key on an unencrypted stream, the missing-key and unsupported-cipher errors;
- damaged blocks, bad headers, and random damage and truncation without a panic.

No real-world HCA file was available (ffmpeg's FATE suite has none), so every stream is synthetic.
