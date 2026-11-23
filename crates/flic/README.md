# cfa4-flic

Pure-Rust (no `unsafe`) decoder for **Autodesk Animator FLI / FLC** animations (`.fli`, `.flc`).
It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-35.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `libavcodec/flicvideo.c` (re-expressed in `src/lib.rs`), `libavformat/flic.c` (header, probe, chunk layout, frame rate) |

The format is documented informally (the `compuphase.com/flic.htm` page FFmpeg cites); FFmpeg's
decoder is the reference. Porting it is covered by the owner's general approval of FFmpeg ports
(2026-11-14). The ported code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are
GPL-3.0-or-later ([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| FLI (0xAF11, 1/70 s units), FLC (0xAF12) and DTA (0xAF44) files, 8-bit paletted pictures | done |
| Chunks: 256-level and 64-level palettes, byte run, line compressed, delta (FLI_DELTA, with the skip-lines and last-byte opcodes), black, copy, mini | done |
| Magic Carpet files (12-byte header, 6-bit palettes in every colour chunk) | done (no oracle stream) |
| Frame rate from the header speed; Terror from the Deep files by their audio block size | done (the audio is not decoded) |
| 1-, 15-, 16- and 24-bit pictures (FLX) | `Error::Depth` |
| Damaged frames: an error or a partly updated picture, never a panic | done |

Two small differences from ffmpeg: a run past the end of a row lands in padding (ffmpeg's frame
buffers have it too, with its own width), and the delta chunk's "last byte" opcode writes the
row's last pixel, where ffmpeg writes the last byte of its padded row, so it drops the pixel
unless the width fills ffmpeg's row alignment. Both only matter for streams the format does not
produce.

## Tests

- `tests/oracle.rs`: streams from the writer in `src/synth.rs` (feature `synth`; ffmpeg has no
  FLIC encoder) with every chunk type, FLI / FLC / DTA, odd sizes and widths that are multiples of
  64: every frame equal to ffmpeg's `flic` decoder (skipped without ffmpeg). Damaged streams
  decode without a panic.
- `crates/codecs/tests/flic_media.rs`: files opened by content, every frame asked for in a
  shuffled order (restarts from checkpoints) against a straight decode.
