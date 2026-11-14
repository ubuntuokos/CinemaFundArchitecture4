# cfa4-realaudio

Pure-Rust (no `unsafe`) RealAudio decoders: **Cook** (RealAudio G2, FourCC `cook`), the audio
codec of most RealMedia files; **SIPR** (ACELP.NET, `sipr`); **RealAudio 1.0** (14.4, `lpcJ`)
and **RealAudio 2.0** (28.8, `28_8`); **ATRAC3** (Sony, RealMedia `atrc` and WAV `.at3`),
which shares Cook's bit reader, prefix codes and IMDCT; **ATRAC3plus** (Sony's ATRAC3+, in
OpenMG `.oma` / `.aa3` and WAV `.at3`); and **ATRAC9** (PlayStation Vita and 4, WAV `.at9`). It
is an L0 crate: it depends only on `thiserror`. Codec backlog items CB-14.1, CB-14.2, CB-15.1,
CB-15.2 and CB-16.1
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)); the RealMedia container and its
packet deinterleaving are in [`cfa4-realmedia`](../realmedia/README.md), the WAV and OpenMG
readers in `cfa4-media`.

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, Sascha Sommer, Benjamin Larsson and the FFmpeg developers | `libavcodec/cook.c` (re-expressed in `cook.rs`), `cookdata.h` (generated into `tables.rs`), `libavutil/lfg.h` / `lfg.c` (the noise generator and its seed-0 state), `sinewin_tablegen.h` (the window), `get_bits.h` (the checked bit reader in `bits.rs`) |
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg project, Vitor Sessak, Vladimir Voroshilov, Reynaldo H. Verdejo Pinochet | `ra144.c` / `ra144dec.c` (`ra144.rs`, tables in `tables144.rs`), `ra288.c` / `ra288.h` (`ra288.rs`, `tables288.rs`), `sipr.c` / `sipr16k.c` / `siprdata.h` / `sipr16kdata.h` (`sipr.rs`, `tables_sipr.rs`), and the CELP helpers they use from `celp_filters.c`, `acelp_filters.c`, `acelp_vectors.c`, `acelp_pitch_delay.c`, `lsp.c`, `lpc_functions.h`, `libavutil/float_dsp.c` (`celp.rs`) |
| FFmpeg | n7.1.1, LGPL-2.1-or-later, Maxim Poliakovski, Benjamin Larsson | `atrac3.c`, `atrac3data.h`, `atrac.c`, `atrac.h` (`atrac3.rs`) |
| FFmpeg | n7.1.1, LGPL-2.1-or-later, Rostislav Pehlivanov | `atrac9dec.c` (`atrac9.rs`), `atrac9tab.h` (generated into `tables_atrac9.rs`, with the `av_lfg_init(0xFBADF00D)` state), `libavutil/lfg.c` `av_bmg_get` and `float_dsp.c` `vector_fmul_window` (`lfg.rs`, `atrac9.rs`) |
| FFmpeg | n7.1.1, LGPL-2.1-or-later, Maxim Poliakovski | `atrac3plus.c`, `atrac3plusdec.c`, `atrac3plusdsp.c`, `atrac3plus.h` (`atrac3p.rs`), `atrac3plus_data.h` and the tables of `atrac3plusdsp.c` (generated into `tables_atrac3p.rs`), `atrac.c` (gain compensation) |

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
| RealAudio 1.0 (14.4) | done: 20-byte frames of 160 samples, fixed point, FFmpeg's 16-bit samples bit for bit |
| RealAudio 2.0 (28.8) | done: 38-byte frames of 160 samples, FFmpeg's `f32` samples bit for bit (same `f32`/`f64` operation order) |
| SIPR | done: 5.0, 6.5, 8.5 kbit/s (8 kHz) and 16 kbit/s (16 kHz), FFmpeg's `f32` samples bit for bit |
| ATRAC3 | done: single channels and joint stereo, 1 to 8 channels, tonal components, gain control, QMF synthesis; WAV (14-byte extra data) and RealMedia (scrambled) streams; within 3·10⁻⁷ of FFmpeg (the IMDCT, as for Cook). ATRAC3 AL is not supported |
| ATRAC3plus | done: 1 to 8 channels (mono and stereo channel units), power compensation noise, gain control, GHA sine-wave tones, the 16-band PQF; within 1.2·10⁻⁷ of FFmpeg on its FATE samples. Channel unit extensions, GHA amplitude mode 0 (FFmpeg has neither) and ATRAC3plus AL are not supported |
| ATRAC9 | done: all six block layouts (mono, dual mono, stereo, quad, 5.1, 7.1 with LFE), all 16 sample rates, superframes of 1, 2 or 4 frames, intensity stereo, all five band extension modes (noise from FFmpeg's generator); within 5·10⁻⁷ of each packet's peak against FFmpeg |

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
- `tests/ra144_oracle.rs`: streams from ffmpeg's `real_144` encoder (sine, pink noise, chirp,
  near silence; these run in CI) and the FATE sample `real/ra3_in_rm_file.rm`: the same 16-bit
  samples as ffmpeg's `ra_144` decoder. Random frames without a panic.
- `tests/ra288_oracle.rs`: the FATE sample `real/ra_288.rm`, bit for bit.
- `tests/sipr_oracle.rs`: the FATE samples `sipr/sipr_{5k0,6k5,8k5,16k}.rm` (from
  <https://fate-suite.ffmpeg.org/sipr/>, in `target/fixtures/sipr`), bit for bit, all four modes.
  ffmpeg has no 28.8 or SIPR encoder, so these run where the samples are present.
- `tests/atrac3_oracle.rs`: the FATE samples `atrac3/mc_sich_at3_{066,105,132}_small.wav` (from
  <https://fate-suite.ffmpeg.org/atrac3/>, in `target/fixtures/atrac3`), every sample within 10⁻⁶
  of ffmpeg. Unit tests: bad parameters refused, random frames in six layouts without a panic.
- `tests/atrac3p_oracle.rs`: the FATE samples `atrac3p/at3p_sample1.oma` and
  `sonateno14op27-2-cut.aa3` (from <https://fate-suite.ffmpeg.org/atrac3p/>, in
  `target/fixtures/atrac3p`, local) within 10⁻⁵ of ffmpeg (worst 1.2·10⁻⁷); and, in CI (ffmpeg has
  no ATRAC3plus encoder), random frames that the decoder's strict mode accepts (fully defined
  decoding: no code outside the tables, no read past the frame, every channel unit present) in
  OpenMG files with all seven channel layouts, the multichannel frames assembled from valid mono
  and stereo units, within 10⁻⁵ of each frame's peak (worst 4·10⁻⁷). Unit tests: the Huffman
  codes are complete, bad parameters refused, random frames in eight layouts without a panic.
- `tests/atrac9_oracle.rs`: there is no public ATRAC9 sample and no free encoder, so the streams
  are synthetic: packets the decoder's strict mode accepts (no read past the packet), grown block
  by block from random bits, in all six block layouts and at 11 to 176 kHz, written as WAV `.at9`
  files and decoded by ffmpeg: within 10⁻⁵ of each packet's peak (worst 4.3·10⁻⁷). Every
  scalefactor mode, band extension mode, both coefficient codebook sets, the LFE block and
  reused block parameters occur. Unit tests: the prefix codes are complete, configurations
  checked, random packets in every layout without a panic.
- `crates/codecs/tests/realmedia_oracle.rs`: the same samples (and a 14.4 stream from ffmpeg's
  encoder) through the media source, read in order and with jumps backwards; ATRAC3 both in WAV
  and in RealMedia files built from the WAV frames (ffmpeg's muxer cannot write ATRAC3; ffmpeg
  decodes the built files too).
- `crates/codecs/tests/openmg_oracle.rs`: ATRAC3plus through the media source, random valid frames
  in OpenMG and WAV `.at3` files (CI) and the FATE samples (local), read in order and with jumps
  backwards across decoder checkpoints.
- `crates/codecs/tests/atrac9_oracle.rs`: ATRAC9 through the media source, synthetic stereo and
  5.1 `.at9` files of 200 packets with a short last packet, read in order and with jumps
  backwards across decoder checkpoints.
