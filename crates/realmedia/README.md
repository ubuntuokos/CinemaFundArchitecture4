# cfa4-realmedia

Pure-Rust (no `unsafe`) demuxer for **RealMedia** files (`.rm`, `.rmvb`). It is an L0 crate: it
depends only on `thiserror`. Codec backlog item CB-13.1, with
[`cfa4-realvideo`](../realvideo/README.md).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `libavformat/rmdec.c` (the RealMedia part, re-expressed in `lib.rs`) |

An owner-approved exception to the FFmpeg-only-in-tests rule, as `cfa4-realvideo`. The ported
code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later.

## Status

| Area | Status |
|---|---|
| `.RMF`, `PROP`, `MDPR`, `CONT`, `DATA` chunks; `INDX` index | done |
| `VIDO` video streams (FourCC, size, frame rate, codec extradata); `MLTI` (first substream) | done |
| Data packets: sync on the packet header, unknown streams and `INDX` chunks skipped | done |
| Video frames: whole, split over packets (FFmpeg's slice table), several in one packet | done |
| Audio streams | listed with their type-specific data; packets returned as stored (no deinterleaving) |
| Old `.ra` files, RTSP/RDT, IVR | not supported |

A frame whose first part lies before the position given to `Demuxer::seek` comes out flagged
`partial`. Sizes in the file are capped (16 MiB per chunk or frame); a damaged packet is skipped
and the reader resynchronises.

## Tests

- `tests/demux.rs`: a hand-made file (header chunks, a frame split over two packets, a whole
  frame, two frames in one packet; seeking into a split frame) and 4000 damaged variants without
  a panic.
- `crates/realvideo/tests/oracle.rs` and `crates/codecs/tests/realmedia_oracle.rs`: files from
  ffmpeg's RealMedia muxer and ffmpeg's FATE sample, through the decoder.
