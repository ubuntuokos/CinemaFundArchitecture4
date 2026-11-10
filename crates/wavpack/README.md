# cfa4-wavpack

Pure-Rust WavPack 4 / 5 decoder (codec backlog CB-5.1, `docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md`).

| | |
|---|---|
| Streams | lossless and hybrid (lossy; the `.wvc` correction file is not read), 8-, 16-, 24- and 32-bit integers, 32-bit float, 1 to 4096 channels, stream versions 0x402–0x410 |
| Containers (wired in `cfa4-codecs`) | `.wv` files (whole file decoded on open), Matroska `A_WAVPACK4` |
| Not supported | DSD (`DSD_FLAG`), pre-4.0 files, correction files |

- `Header::parse`: a block header with the reference reader's checks.
- `Info::probe`: the stream format (rate, channels and mask, bits, float) from the first audio block.
- `Decoder::decode_frame`: one block sequence (`INITIAL_BLOCK` … `FINAL_BLOCK`) → planar `i32`
  (integers right-justified, floats as `f32` bits); `Info::to_f32` scales to [-1, 1).
- `decode_file`: a whole `.wv` file; junk and tags between blocks are skipped.
- `matroska_blocks`: rebuilds the full blocks of a Matroska frame (Matroska strips the block
  headers down to `flags`, `crc` and a size).

A block that fails a check (CRC, the magnitude limit, malformed metadata, a truncated bitstream)
decodes to silence of its length, as in the reference decoder. Every count from the file is
bounded: blocks up to 16 MiB and 196 607 samples, at most 16 decorrelation terms, and
`decode_file` stops at 2³⁰ decoded values.

## Provenance and licence

The decoding algorithm (entropy decoder, decorrelation passes, extended-integer and float
reconstruction, hybrid error limits) is ported from the WavPack library 5.9.0 (commit
`0931f1f3`, BSD-3-Clause, Copyright (c) 1998 - 2025 David Bryant; `LICENSE.WavPack`); the source
files name the C files they follow. The block format follows "WavPack 4 & 5 Binary File / Block
Format" (D. Bryant, 2020). CFA4's changes are GPL-3.0-or-later. Register entry:
`docs/licensing.md` §5.

## Fidelity

Bit-exact (`tests/oracle.rs`, `crates/codecs/tests/wavpack_oracle.rs`):

- against ffmpeg's decoder (an external oracle only) on ffmpeg-encoded files: every compression
  level, joint and independent stereo, 8 / 16 / 24 / 32-bit and float, mono to 5.1, an odd rate;
- against the reference `wvunpack --raw` on files from the reference `wavpack` encoder: high and
  very high modes, extra modes 1–6, float and 32-bit integers, 6 and 8 channels, hybrid at
  several bit rates, with positive and negative noise shaping, hybrid float, 32-bit and 8-bit
  (skipped when the CLI is not installed; CI installs it);
- through the media sources in `.wv` and Matroska, front to back and after a seek, including
  reference-encoded hybrid and float streams remuxed into Matroska.

A lossless decode equals the source PCM; a damaged block is muted while its neighbours stay exact.
