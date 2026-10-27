# cfa4-dts

DTS decoder: DTS Coherent Acoustics (the core), DTS-HD High Resolution and DTS-HD Master Audio,
and the bed of DTS:X.

## Origin and licence

This crate is the `dca` crate 0.9.0 of [harletty-bridge](https://github.com/harletty/harletty-bridge)
(commit `74cf6edaef6742777d3eb4da35387fd370f2e9f3`, 2026-10-21), taken over with the changes listed
below. That crate is a Rust port of FFmpeg's DTS decoder (`libavcodec/dca*.c`, `dcadsp.c`,
`dcadct.c`, `synth_filter.c`; Copyright (C) 2016 foo86 and the FFmpeg developers), which FFmpeg
licenses under the GNU LGPL 2.1 or later. harletty-bridge published its port as Apache-2.0
(`LICENSE.harletty-bridge`). Because the code is derived from FFmpeg, CFA4 treats it as
**LGPL-2.1-or-later** (`COPYING.LGPL-2.1`), the licence of what it was ported from. That licence is
compatible with CFA4's GPL-3.0-or-later.

Every ported file starts with a notice saying so, followed by its original harletty-bridge header.
`cfa4.rs`, the tests in `tests/` and `examples/` are CFA4's own (GPL-3.0-or-later). The crate is
listed in the reused-code register, [`docs/licensing.md`](../../docs/licensing.md) §5.

**Exception to the FFmpeg rule.** CFA4 otherwise uses FFmpeg only as an external test oracle.
Taking over this FFmpeg-derived code was decided by the project owner on 2026-10-26 (#83), since
DTS has no other open implementation to build on. No FFmpeg library is linked or shipped.

## Changes made for CFA4

- The AVX2 / AVX-512 builds of the filter banks and the lossless prediction
  (`synth_avx2.rs`, `synth_avx512.rs`, `xll_avx2.rs`, `cpu.rs`, the `target_feature` copies in
  `core.rs` and `dcadec/mod.rs`) are removed. They were `unsafe` and computed the same integers as
  the portable code that remains.
- The last `unsafe` block, a reinterpretation of `&mut [i32]` as `&mut [u32]`, is replaced: the
  bit reader stores raw fields into `i32` directly.
- No panicking calls outside tests: `try_into().unwrap()` on fixed slices became `first_chunk`, the
  `unreachable!` on the prediction order became a no-op arm. An underflow in the adaptive predictor
  for blocks shorter than the order is now saturated. The crate denies these with clippy lints.
- The core's LFE channel is synthesised with the fixed-point interpolator, like the fullband
  channels. The output then equals FFmpeg's bit-exact decode (`-flags +bitexact`) on every
  channel. The float LFE path and its table are removed.
- The unused `rustfft` dependency is dropped. Lints from the workspace are fixed.
- `cfa4.rs` (new) is the interface the rest of CFA4 uses, described below.
- Found on real streams (#131), ported from FFmpeg:
  - XLL recovery: the first frame of a stream with several channel sets and residual-coded
    channels (and the first after a reset) is the lossy core (`DCA_PACKET_RECOVERY`,
    `force_lossy_output`).
  - The downmix coefficients of a channel set are pre-scaled by the next set that downmixes into
    it (`prescale_down_mix`); a third set's downmix came out with slightly wrong coefficients.
  - The XCH rear centre of DTS-ES core streams (`parse_xch_frame`), lossy and as the XLL residual
    base, with the surround downmix taken back out of the lossy output (`dmix_sub_xch`).
  - 192 kHz XLL (#253): the second frequency band with its decimator history, joined to the
    first by `assemble_freq_bands` (`chs_assemble_freq_bands`); embedded downmixes are undone in
    both bands. Sets that differ in bands or rate are refused, as in FFmpeg.
  - DTS-HD Master Audio without a core (#254): extension substreams that no core frame precedes
    are decoded on their own when their lossless channels need no core (`decode_exss_into`), in
    raw streams and container packets; `packet_samples` counts their lossless frames. A substream
    that needs a core (a raw stream entered between a core frame and its substream) is skipped.

## What it decodes

`cfa4::Decoder::decode_packet` takes container packets (Matroska `A_DTS`, MP4 `dtsc` / `dtsh` /
`dtsl` or `mp4a` with object type 0xA9–0xAB, MPEG-TS / Blu-ray stream types 0x82, 0x85, 0x86).
`push` / `flush` take a raw stream in pieces. Both give planar `f32` in CFA4's WAV / SMPTE order
(L R C LFE back-L back-R … side-L side-R …), with the speakers named.

- DTS core: mono to 5.1, and 6.1 with the DTS-ES rear centre (XCH), 32–48 kHz, bit-exact
  against FFmpeg.
- DTS-HD Master Audio (XLL), lossless up to 7.1 (and 6.1 over a DTS-ES core) at 48 / 96 /
  192 kHz, also without a core.
- DTS-HD High Resolution: the core plus its XXCH channels. XBR-only streams play their core.
- DTS:X: the backward-compatible bed. The encoder has folded the heights and objects into it.
  Placing them on their own loudspeakers is #130.
- A frame that does not decode gives silence of its length, so timing holds.

Not decoded: DTS Express (LBR), 14-bit and little-endian DTS (DTS-CD, DTS in WAV), and raw `.dts`
file import (#132).

## Tests

- `tests/oracle.rs`: ffmpeg's DTS encoder writes mono, stereo, 5.0 and 5.1 streams and pink
  noise at 32–48 kHz. The decode is bit-exact against ffmpeg's bit-exact decode three ways: as a
  whole, in 1000-byte pieces, and frame by frame. A reset resumes at the next frame. Damaged
  streams (bit flips, truncation, scrambled bytes, random extension substreams) never panic, and
  neither do 3000 extension substreams that reach the lossless decoder with random channel sets.
- `a_real_dts_hd_stream_decodes_like_ffmpeg`: when `CFA4_DTSHD_SAMPLE` names a DTS-HD Master
  Audio stream (raw `.dts`, or a `.dtshd` file, whose `STRMDATA` chunk is read), it must be
  lossless and bit-exact against ffmpeg. No free encoder writes one, and none is committed. FFmpeg's
  FATE samples `dts/master_audio_7.1_24bit.dts`, `dts/dtsx_7.1.4_24bit.dts` and the
  `dts/dcadec-suite/xll_*` streams at 48, 96 and 192 kHz (5.1, 6.1 over DTS-ES, 7.1 in two and
  three channel sets, and `xll_51_24_48_none` without a core) all pass, decoded as a raw stream and
  frame by frame as container packets (#131, #253, #254).
- `crates/codecs/tests/dts_oracle.rs`: DTS in Matroska, MP4 and MPEG-TS, bit-exact, also on
  random access.
- The harletty-bridge unit tests. Those that need its private corpora skip without them.
