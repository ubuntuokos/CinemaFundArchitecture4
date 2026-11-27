# cfa4-realvideo

Pure-Rust (no `unsafe`) decoder for **RealVideo 1.0** (`RV10`) and **RealVideo 2.0** (`RV20`),
the H.263-based codecs of RealMedia files, and, in the `rv34` module, **RealVideo 3.0** (`RV30`)
and **RealVideo 4.0** (`RV40`), the H.264-like ones. It is an L0 crate: it depends only on
`thiserror`. Codec backlog items CB-13.1 and CB-13.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)); the container is
[`cfa4-realmedia`](../realmedia/README.md).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `rv10.c`, `ituh263dec.c`, `h263.c`, `h263dsp.c`, `mpegvideo.c`, `mpegvideo_dec.c`, `mpegvideo_motion.c`, `mpv_reconstruct_mb_template.c`, `mpeg4video.c`, `simple_idct_template.c` (re-expressed in `lib.rs`); `h263data.c`, `mpegvideodata.c`, `h263dsp.c`, `rv10.c` (the tables in `tables.rs`) |
| FFmpeg | n7.1.1, LGPL-2.1-or-later, Mike Melanson, Konstantin Shishkov, Michael Niedermayer and the FFmpeg developers | `rv34.c`, `rv30.c`, `rv40.c` (`rv34/mod.rs`, `rv34/vlc.rs`), `rv34dsp.c`, `rv30dsp.c`, `rv40dsp.c`, `h264qpel_template.c`, `h264chroma_template.c` (`rv34/dsp.rs`), `h264pred_template.c` (`rv34/pred.rs`); `rv34vlc.h`, `rv34data.h`, `rv30data.h`, `rv40data.h`, `rv40vlc2.h` (generated into `rv34/tables.rs`) |

RealNetworks never published these formats; FFmpeg's decoder is the reference. Porting it is an
owner-approved exception to the FFmpeg-only-in-tests rule (as the Dirac core, CB-8.1, and
Shorten, CB-9.2; for RV30/RV40 as for RV10/RV20). The ported code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes
are GPL-3.0-or-later ([register](../../docs/licensing.md)).

## Status

| Area | Status |
|---|---|
| RV10: I and P pictures, RV10 escapes, version 3 DC coding in I-frames | done (version 3 has no test sample yet) |
| RV20: I / P / B pictures, AIC intra prediction, modified quantisation, loop filter, direct mode | done |
| 4MV, unrestricted (long) vectors, half-pel motion with and without rounding | done |
| Reference picture resampling sizes (RV20 `rpr`), slices, the slice table of a packet | done |
| RV10 OBMC (micro version 2), PB-frames | refused (`Error::Unsupported`), as in FFmpeg |
| Damaged slices: the missing macroblocks are copied from the previous picture | done |
| RV30 / RV40 (`rv34`): I / P / B pictures, 4x4 and 16x16 intra prediction with the RV30 / RV40 mode coding, third-pel (RV30) and quarter-pel (RV40) motion, B direct mode, weighted bi-prediction | done |
| RV30 / RV40: both loop filters, picture size changes (RV30 `rpr`, RV40 dimension codes), slices | done |
| RV30 / RV40 damaged slices: the missing macroblocks are copied from the previous picture (FFmpeg's error resilience is not replicated) | done |

`Decoder::decode` takes a packet in FFmpeg's layout (slice count, slice table, slice data, as
`cfa4-realmedia` assembles it) and returns pictures in display order: with B-frames a reference
picture comes out when the next one is decoded, `Decoder::flush` returns the last one (FFmpeg
drops it). `Picture::decode_index` maps a picture back to its packet.

`rv34::Decoder` works the same way and, like FFmpeg's `rv30` / `rv40`, outputs the last reference
picture on `flush`.

The inverse DCT is FFmpeg's simple IDCT (the C template; its x86 SSE2 / AVX versions give the
same results on these streams).

## Tests

- `tests/oracle.rs`: streams from ffmpeg's `rv10` / `rv20` encoders (several sizes, quantisers
  and GOPs) and, when present, ffmpeg's FATE sample `real/G2_with_SVT_320_240.rm` (RV20 with
  B-frames and the loop filter; `target/fixtures/real`, from
  <https://fate-suite.ffmpeg.org/real/>), decoded by us and by ffmpeg with
  `-idct simple -flags +bitexact -fps_mode passthrough`: every picture equal; damaged packets
  (random bytes, cuts) and random data without a panic.
- `tests/rv34_oracle.rs`: ffmpeg's FATE samples `real/rv30.rm` (RV30, 109 frames) and
  `real/spygames-2MB.rmvb` (RV40 with B-frames, 521 frames) in `target/fixtures/real`, decoded
  by us and by ffmpeg with `-flags +bitexact -threads 1 -fps_mode passthrough`: every picture
  equal (ffmpeg has no RV30/RV40 encoder, so these run where the samples are present); damaged
  packets and random data without a panic.
- `crates/codecs/tests/realmedia_oracle.rs`: the same through the media source, last frame first.
