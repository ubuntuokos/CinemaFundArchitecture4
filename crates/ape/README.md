# cfa4-ape

Pure-Rust Monkey's Audio (APE) decoder (codec backlog CB-5.2, `docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md`).

| | |
|---|---|
| Streams | format version 3.99 and later (every Monkey's Audio encoder since 2004), compression levels fast (1000) to insane (5000), 8-, 16-, 24- and 32-bit integers, 32-bit float, 1 to 32 channels |
| Containers (wired in `cfa4-codecs`) | `.ape` files (whole file decoded on open) |
| Not supported | files older than format 3.99 |

- `File::parse`: the descriptor, header and seek table (an ID3v2 tag in front is skipped).
- `Decoder::decode_frame`: one frame → planar `i32` (integers sign-extended, 8-bit as signed,
  floats as `f32` bits); `Info::to_f32` scales to [-1, 1).
- `decode_file`: a whole `.ape` file.

A frame that fails a check (the PCM CRC, the range coder, a truncated bitstream, a 16-bit
overflow) decodes to silence of its length, as in the reference decoder. Every count from the
file is bounded: up to 32 channels and 256 bytes per block, frames of at most 1 000 000 blocks
(10 000 000 at the insane level), and `decode_file` stops at 2³⁰ decoded values.

24-bit files from Monkey's Audio 5.01–8.51 summed some products in 64 bits; as in the reference
decoder, a 24-bit frame that fails its CRC is retried once in that "interim" arithmetic, which
then stays on for the file. No such file is in the tests (current encoders cannot write one).

## Provenance and licence

The decoding algorithm (range decoder, NN filters, two-stage predictor, channel unpacking, float
transform) is ported from the Monkey's Audio SDK 13.46 (BSD-3-Clause, Copyright 2000-2026 Matthew
T. Ashland; `LICENSE.MonkeysAudio`); the source files name the SDK files they follow. CFA4's
changes are GPL-3.0-or-later. Register entry: `docs/licensing.md` §5.

## Fidelity

Bit-exact (`tests/oracle.rs`, `crates/codecs/tests/ape_oracle.rs`) on files from the reference
`mac` encoder, built from the pinned SDK (`scripts/ci/install-monkeys-audio.sh`; the tests skip
without it):

- against `mac -d`: every compression level, 8 / 16 / 24 / 32-bit integers and float, mono, stereo,
  3, 4, 6, 7 and 8 channels, an odd rate, multi-frame files and a file shorter than a frame;
- against ffmpeg's APE decoder (an external oracle only) on 8-, 16- and 24-bit files;
- through the media source, front to back and after a seek.

A decode equals the source PCM; a damaged frame is muted while its neighbours stay exact.
