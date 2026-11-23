# cfa4-mace

Pure-Rust decoders for Apple **MACE 3:1** and **MACE 6:1** (QuickTime `MAC3`, `MAC6`), codec
backlog CB-38.1. Ported from FFmpeg n9.0.2's `libavcodec/mace.c` (LGPL-2.1-or-later,
`COPYING.LGPL-2.1`; an owner-approved exception to the FFmpeg-only-in-tests rule, see
`docs/licensing.md` §5). CFA4 changes are GPL-3.0-or-later.

[`Decoder::decode`] takes a packet (whole frames: one byte per channel for 6:1, two for 3:1; a
partial tail is dropped, as ffmpeg does) and returns planar 16-bit channels (mono or stereo). The
per-channel predictor state carries from packet to packet, so `cfa4-codecs` saves decoder states
at intervals and a seek decodes the same samples as playback from the start.

## Fidelity

`tests/oracle.rs` writes random bytes into an AIFF-C file (`MAC3` / `MAC6`) that ffmpeg decodes
(external oracle only, never linked) and compares every sample, mono and stereo. The FATE
`qt-surge-suite/surge-{1,2}-8-MAC{3,6}.mov` files are compared when they are in
`target/fixtures/fate/qt/` (local only).
