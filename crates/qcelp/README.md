# cfa4-qcelp

Pure-Rust (no `unsafe`) decoder for **QCELP-13K** speech (TIA/EIA/IS-733, "PureVoice": 8 kHz mono,
20 ms frames of 160 samples at full, half, quarter and eighth rate, plus blank frames) and reader
for **QCP** files (RFC 3625). It is an L0 crate: it depends only on `thiserror`. Codec backlog item
CB-38.2 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `libavcodec/qcelpdec.c`, `qcelpdata.h` (`decoder.rs`, `tables.rs`); `celp_filters.c`, `acelp_filters.c`, `acelp_vectors.c`, `lsp.c`, `libavutil/float_dsp.c` (`celp.rs`); `libavformat/qcp.c` (`qcp.rs`) |

Porting FFmpeg's decoder is an owner-approved exception to the FFmpeg-only-in-tests rule (owner
decision 2026-11-14, as DV-3 and Canopus). The ported code keeps FFmpeg's licence
(`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later ([register](../../docs/licensing.md)).

## Behaviour

- `Decoder::decode` takes one packet (the rate byte and the frame data: 35, 17, 8, 4 or 1 bytes; a
  packet one byte short has lost its rate byte, which is guessed from the size) and appends 160
  `f32` samples. Packets that cannot be decoded (unknown size, a rate byte above the size, reserved
  bits set, failed plausibility checks) are erasures and are concealed as FFmpeg does; decoding
  never fails.
- `parse` and `decode_file` read QCP files the way FFmpeg's demuxer does (variable or fixed packet
  size, the rate map, chunk padding); EVRC, SMV and 4GV files are recognised and reported as not
  QCELP.
- The float operations keep FFmpeg's order and its C promotion rules (a `float * double` product is
  computed in double), so the output matches FFmpeg bit for bit. The oracle for the C side must be
  built without fused multiply-add (`-ffp-contract=off`) on CPUs that have it.

## Tests

- `src/oracle_tests.rs`: bit-exact (float bits) against ffmpeg's `qcelp` decoder and `qcp` demuxer
  on synthetic QCP files (ffmpeg does not encode QCELP): random packets of every rate re-rolled
  until they decode as normal frames, mixed with raw random packets (erasures), packets that claim
  another rate, packets without the rate byte, packets of an unknown size and fixed-size files.
- `src/lib.rs`: damaged and truncated input never panics.
- `crates/codecs/tests/qcp_file.rs`: a `.qcp` file through the media source.
