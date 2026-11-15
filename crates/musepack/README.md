# cfa4-musepack

Pure-Rust (no `unsafe`) decoder for **Musepack** audio, stream versions 7 (`MP+`) and 8
(`MPCK`). It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-10.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| libmpcdec | `musepack_src_r475` (the Musepack SV8 reference), BSD-3-Clause, Copyright (c) 2005-2009 The Musepack Development Team | stream headers and SV8 blocks (`mpc_demux.c`, `streaminfo.c`), the SV7 and SV8 bitstreams, requantisation, noise substitution (`mpc_decoder.c`, `requant.c`), the synthesis filter in its floating-point build (`synth_filter.c`), Huffman tables and bit decoders (`huffman.c`, `mpc_bits_reader.c`) |

The ported code keeps libmpcdec's licence (`LICENSE.libmpcdec`); CFA4's changes are
GPL-3.0-or-later ([register](../../docs/licensing.md)). The operation order of the float
build's synthesis and requantisation is kept, so the output is libmpcdec's. `huffman.rs` was
generated from `huffman.c`.

## Status

| Area | Status |
|---|---|
| SV7: header, frames, gapless length (true gapless and old streams) | done |
| SV8: `SH` header with CRC, `AP` audio blocks, key frames, `SE` end, unknown length | done |
| Mid/side stereo, noise substitution (`Res = -1`), every quantiser, mono SV8 | done |
| ID3v2 tag before the stream | done |
| `.mpc` files through `cfa4-codecs` (`AudioFileSource`, decoded at open) | done |
| Damaged input: decodes as far as it can, never panics | done |
| Replay gain, chapters, seek tables | not used (the file is decoded whole) |
| SV4–SV6 | not supported (as libmpcdec) |

## Tests

- `tests/oracle.rs`: ffmpeg's two FATE samples (`inside-mp7.mpc`, `inside-mp8.mpc`, in
  `$MUSEPACK_SAMPLES` or `target/fixtures/musepack/fate`) within 1 LSB at 16 bits of ffmpeg's
  fixed-point decoder; 2000 damaged copies without a panic.
- `tests/libmpcdec_oracle.rs`: exactly libmpcdec's floating-point samples (the same `f32`
  bits), through the tools `scripts/ci/install-musepack-oracle.sh` builds from the pinned
  musepack_src_r475 archive (in CI): SV8 streams the reference encoder `mpcenc` makes at
  qualities 0–10, 32 / 44.1 / 48 kHz, mono and stereo; and the FATE samples when present.
- `crates/codecs/tests/musepack_oracle.rs`: the same files through the media source, also
  read from the middle.
