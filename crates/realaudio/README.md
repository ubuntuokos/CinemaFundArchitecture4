# cfa4-realaudio

Pure-Rust (no `unsafe`) RealAudio decoders. So far **Cook** (RealAudio G2, FourCC `cook`), the
audio codec of most RealMedia files. It is an L0 crate: it depends only on `thiserror`. Codec
backlog item CB-14.1 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)); the container
and its packet deinterleaving are in [`cfa4-realmedia`](../realmedia/README.md).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, Sascha Sommer, Benjamin Larsson and the FFmpeg developers | `libavcodec/cook.c` (re-expressed in `cook.rs`), `cookdata.h` (generated into `tables.rs`), `libavutil/lfg.h` / `lfg.c` (the noise generator and its seed-0 state), `sinewin_tablegen.h` (the window), `get_bits.h` (the checked bit reader in `bits.rs`) |

RealNetworks never published the format; FFmpeg's decoder is the reference. Porting it is an
owner-approved exception to the FFmpeg-only-in-tests rule (as RealVideo, CB-13.1 / CB-13.2). The
ported code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| Mono, stereo (two half-frames), joint stereo with coupling, multichannel (several sub-packets) | done |
| 256 / 512 / 1024 samples per channel, envelope and SQVH coding, noise fill, gain control | done |
| The inverse MDCT | in `f64` (`mdct.rs`); FFmpeg's `av_tx` result depends on its SIMD code, so samples match it to rounding (within 2·10⁻⁷ on the samples below) |
| Output | planar `f32` in `[-1, 1]`; the first two frames give no output, as in FFmpeg |
| Damaged frames | an error for the frame, never a panic |

The noise fill uses FFmpeg's lagged Fibonacci generator over the whole stream, so the exact
samples depend on everything decoded before. `Decoder` is `Clone`: a clone is a checkpoint that
decodes on exactly like the original (the media source keeps one every 32 superblocks).

## Tests

- `tests/cook_oracle.rs`: ffmpeg's FATE samples `real/ra_cook.rm` (stereo), `real/rv30.rm` (mono,
  with RealVideo 3.0) and `real/spygames-2MB.rmvb` (stereo, with RealVideo 4.0) in
  `target/fixtures/real` (from <https://fate-suite.ffmpeg.org/real/>), deinterleaved and decoded
  by us and decoded by ffmpeg: the same sample count, every sample within 10⁻⁶. ffmpeg has no
  Cook encoder, so these run where the samples are present. Damaged frames, random stream
  parameters and random RealAudio headers without a panic.
- `crates/codecs/tests/realmedia_oracle.rs`: the same samples through the media source, read in
  order and with jumps backwards.
