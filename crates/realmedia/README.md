# cfa4-realmedia

Pure-Rust (no `unsafe`) demuxer for **RealMedia** files (`.rm`, `.rmvb`). It is an L0 crate: it
depends only on `thiserror`. Codec backlog items CB-13.1 (with
[`cfa4-realvideo`](../realvideo/README.md)) and CB-14.1 (the audio part, with
[`cfa4-realaudio`](../realaudio/README.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `libavformat/rmdec.c` (the RealMedia part, re-expressed in `lib.rs`; the RealAudio header and packet deinterleaving in `audio.rs`), `libavformat/rmsipr.c` (the SIPR reordering in `audio.rs`) |

An owner-approved exception to the FFmpeg-only-in-tests rule, as `cfa4-realvideo`. The ported
code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later.

## Status

| Area | Status |
|---|---|
| `.RMF`, `PROP`, `MDPR`, `CONT`, `DATA` chunks; `INDX` index | done |
| `VIDO` video streams (FourCC, size, frame rate, codec extradata); `MLTI` (first substream) | done |
| Data packets: sync on the packet header, unknown streams and `INDX` chunks skipped | done |
| Video frames: whole, split over packets (FFmpeg's slice table), several in one packet | done |
| Audio streams | listed with their type-specific data; packets returned as stored |
| RealAudio headers (versions 3, 4, 5): codec, rate, channels, interleaving parameters, codec data | done (`audio::parse_audio_info`) |
| Deinterleaving into codec frames: `genr` (Cook, ATRAC3), `Int4` (28.8), `sipr`, `vbrs` / `vbrf` (AAC), `Int0` | done (`audio::Deinterleaver`); `genr`, `Int4`, `sipr` and `Int0` checked against ffmpeg (Cook, 28.8, SIPR, 14.4) |
| Old `.ra` files, RTSP/RDT, IVR | not supported |

A frame whose first part lies before the position given to `Demuxer::seek` comes out flagged
`partial`, as does an audio packet cut short by the end of the file (it keeps what is there, as
FFmpeg's `av_get_packet` does). Sizes in the file are capped (16 MiB per chunk or frame); a damaged packet is skipped
and the reader resynchronises.

## Tests

- `tests/demux.rs`: a hand-made file (header chunks, a frame split over two packets, a whole
  frame, two frames in one packet; seeking into a split frame) and 4000 damaged variants without
  a panic.
- `crates/realvideo/tests/oracle.rs` and `crates/codecs/tests/realmedia_oracle.rs`: files from
  ffmpeg's RealMedia muxer and ffmpeg's FATE sample, through the decoder.
- `crates/realaudio/tests/{cook,ra144,ra288,sipr}_oracle.rs`: the RealAudio FATE samples (and
  ffmpeg-encoded 14.4) through `audio`, and random RealAudio headers and packets without a panic.
