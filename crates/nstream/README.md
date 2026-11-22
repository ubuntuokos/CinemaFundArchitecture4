# cfa4-nstream

Pure-Rust (no `unsafe`) support for Nintendo **BRSTM** (Wii), **BFSTM** (Wii U) and **BCSTM** (3DS)
audio streams: the header and the block layout, 8-bit and 16-bit PCM and Nintendo DSP ADPCM. It is
an L0 crate: it depends only on `thiserror`. Codec backlog items CB-29.2 (DSP ADPCM and BRSTM) and
CB-30.1 (BFSTM; BCSTM is the same layout, ffmpeg's `bfstm` demuxer reads it as well)
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavformat/brstm.c` (the `brstm` and `bfstm` demuxers) | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2012 Paul B Mahol | header, coefficient tables, `ADPC` / `SEEK` history, block walk |
| FFmpeg `libavcodec/adpcm.c` (the `adpcm_thp` / `adpcm_thp_le` part) | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2001-2003 The FFmpeg project, THP ADPCM decoder by Marco Gerards | DSP ADPCM |
| FFmpeg `libavcodec/pcm.c` (planar PCM) | n7.1.1, LGPL-2.1-or-later | PCM blocks |

These formats have no public specification. Porting FFmpeg's code is covered by the owner's general
permission for FFmpeg ports (2026-11-14). The crate keeps FFmpeg's licence (LGPL-2.1-or-later,
`COPYING.LGPL-2.1`) for the ported code; CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| BRSTM header (`RSTM`, `HEAD`), BFSTM and BCSTM header (`FSTM` / `CSTM`, `INFO`), both byte orders | done |
| 8-bit PCM, 16-bit PCM (both byte orders), DSP ADPCM (1–14 channels) | done |
| Same samples as ffmpeg's demuxers and decoders (including the half-filled last block and a file cut inside a block) | done |
| `.brstm`, `.bfstm`, `.bcstm` through `cfa4-codecs` (sound decoded whole at open) | done |
| Loop points (read, not used by playback), other codecs (ffmpeg refuses them too) | read / unsupported |
| Damaged input: errors or garbage, never panics | done |

## Tests

- `tests/oracle.rs`, against ffmpeg's demuxers and decoders: the real FATE samples
  `brstm/lozswd_partial.brstm`, `bfstm/spl-forest-day.bfstm` and `bfstm/loz-mm-mikau.bcstm` (not
  committed: they sit in `target/fixtures/nstream/fate/`, from <https://fate-suite.ffmpeg.org/>;
  those tests skip without them) sample for sample; BRSTM files from a writer of ours with 8-bit
  and 16-bit PCM in both byte orders, 1–3 channels and half-filled last blocks (always run);
  damaged files without a panic.
- `crates/codecs/tests/nstream_file.rs`: the samples through the media source, read from the
  middle.
- a fuzz target (`fuzz/fuzz_targets/nstream.rs`).
