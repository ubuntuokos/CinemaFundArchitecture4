# cfa4-mobiclip

Pure-Rust (no `unsafe`) support for **Nintendo MobiClip** movies (`.moflex`): the MOFLEX container,
the MobiClip video decoder and the FastAudio, IMA ADPCM (MOFLEX) and PCM sound decoders. It is an
L0 crate: it depends only on `thiserror`. Codec backlog item CB-28.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavformat/moflex.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2015-2016 Florian Nouwt, (c) 2017 Adib Surani, (c) 2020 Paul B Mahol | block headers, stream list, bit-packed packet chain |
| FFmpeg `libavcodec/mobiclip.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2022 Paul B Mahol | the video decoder and its tables |
| FFmpeg `libavcodec/fastaudio.c` | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2020 Paul B Mahol | the FastAudio decoder |
| FFmpeg `libavcodec/adpcm.c` (the MOFLEX part) | n7.1.1, LGPL-2.1-or-later, Copyright (c) 2001-2003 The FFmpeg project | IMA ADPCM (MOFLEX) |

MobiClip has no public specification. Porting FFmpeg's code is covered by the owner's general
permission for FFmpeg ports (2026-11-14). The crate keeps FFmpeg's licence (LGPL-2.1-or-later,
`COPYING.LGPL-2.1`) for the ported code; CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). ffmpeg itself is used only as an external test oracle.

## Status

| Area | Status |
|---|---|
| Container: block headers, stream list (video, audio, data), bit-packed packets; a damaged tail ends the packets | done |
| Video: intra pictures (nine prediction modes, 4×4 and 8×8 blocks), predicted pictures (half-pel motion, 4×4 / 8×8 residues), YCgCo streams | done |
| The pictures come out bit for bit as ffmpeg's `mobiclip` decoder | done |
| Sound: FastAudio (float), IMA ADPCM MOFLEX, PCM s16le | done |
| `.moflex` through `cfa4-codecs`: random access from the nearest intra picture | done |
| YCgCo streams (moflex bit 0) are converted to RGB as full-range YCgCo; no sample of one is known | unverified |
| Damaged input: errors or garbage, never panics | done |

## Tests

- `tests/oracle.rs`, against ffmpeg's `moflex` demuxer and decoders, on the FATE sample
  `mobiclip/mobiclip-reference-rect-partitions.moflex` (not committed: it sits in
  `target/fixtures/mobiclip/fate/`, from <https://fate-suite.ffmpeg.org/mobiclip/>; the tests skip
  without it): both pictures bit for bit, the FastAudio sound exactly; 150 files with random bit
  flips in the packets (every file ffmpeg accepts without a complaint and we accept too gives the
  same pictures: this reaches syntax the sample does not use); damaged files without a panic.
- `crates/codecs/tests/mobiclip_file.rs`: the sample through the media source.
