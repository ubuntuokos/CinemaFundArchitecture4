# cfa4-smacker

Pure-Rust (no `unsafe`) demuxer and decoders for **Smacker** (`.smk`, RAD Game Tools, SMK2 and
SMK4): the container, the paletted video and the packed Huffman audio. It is an L0 crate: it
depends only on `thiserror`. Codec backlog item CB-24.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg `libavcodec/smacker.c` (`smackvid`, `smackaud`), `libavformat/smacker.c` | n9.0.2, LGPL-2.1-or-later, Copyright (c) 2006 Konstantin Shishkov | the header and frame table, palette updates, audio chunk splitting; the four header trees and their most-recent-value slots, the block types and runs, the SMK4 full-block modes; the packed audio decoder |
| The public Smacker description at wiki.multimedia.cx ("Smacker") | (documentation) | the format overview FFmpeg's files themselves cite |

Smacker has no standard; the format is documented only in that description and in FFmpeg's
decoder. Porting FFmpeg's decoder is an owner exception, as for TAK, Dirac and Shorten. The crate
keeps FFmpeg's licence (LGPL-2.1-or-later, `COPYING.LGPL-2.1`) for the ported code; CFA4's
changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). ffmpeg itself is used only as
an external test oracle. The decoder logic of n7.1.1 and n9.0.2 is the same except one check in
the audio decoder (the chunk must hold the first samples), which this crate follows.

## Status

| Area | Status |
|---|---|
| SMK2 and SMK4 headers, frame table, ring frame, pixel aspect from the interlace / double flags | done |
| Palette updates (kept, copied and new runs; the palette is cumulative) | done |
| Video: mono, full, skip and fill blocks, runs, the three SMK4 full-block modes, header trees with escape values | done |
| Audio: packed 8- and 16-bit, mono and stereo, up to seven tracks; raw PCM tracks | done |
| Bink audio tracks (RDFT, DCT) inside a Smacker file | not decoded (the Bink backlog item) |
| Damaged data: what ffmpeg does (a frame that runs out of data keeps its decoded blocks; a bad palette or audio size skips the rest of the frame), never a panic | done |
| `.smk` files through `cfa4-codecs` (`SmkSource`): any frame in any order (decoder snapshots), every track | done |

Behaviour kept from ffmpeg on purpose: a chunk that says "no data" is consumed one byte long and
ffmpeg decodes the rest of the packet again as a packet of its own; an 8-bit raw PCM track keeps
its first four bytes (a length) as data.

## Tests

- `tests/oracle.rs`: ffmpeg's FATE sample `wetlogo.smk`, when it is in `target/fixtures/smacker/`
  (or `$SMACKER_SAMPLES`), against ffmpeg (video frames as SHA-256 per frame, audio samples);
  60 files from the writer in `cfa4-testkit` (`SMACKER_RANDOM_FILES=800` checks 800) in which every
  syntax element is random (header trees with skipped trees and escapes, palette updates, SMK2 and
  SMK4, ring frames, up to seven tracks, packed and raw audio, frame data of random bytes, chunks
  ffmpeg refuses) against ffmpeg; damaged and truncated files without a panic.
- `crates/codecs/tests/smacker_file.rs`: the media source: pictures front to back, after jumps and
  in a file longer than the snapshot interval; audio per track, whole and in windows; damaged files.
- `fuzz/` target `smacker`.
