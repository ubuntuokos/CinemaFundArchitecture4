# cfa4-avs3

Pure-Rust (no `unsafe`) decoder for **AVS3-P2** (IEEE 1857.10, GB/T 33475.2), the baseline
profiles 0x20 (8-bit) and 0x22 (10-bit). It is an L0 crate: it depends only on `thiserror`. Codec
backlog item CB-19.1 ([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| uavs3d | 1.2, commit `0e20d2c291853f196c68922a264bcd8471d75b68`, BSD-3-Clause, Peking University Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation | the C code paths (no assembly), re-expressed in every file of `src/`; the header of each derived file keeps the upstream notice |

BSD-3-Clause is GPL-3.0-compatible ([register](../../docs/licensing.md)). uavs3d is also the
decoder CFA4 matches. FFmpeg has no AVS3 decoder of its own (only a wrapper of libuavs3d), so it
is not an oracle here. The encoder uavs3e (BSD-4-Clause) is a test tool only: it is built by
`scripts/ci/install-avs3-oracle.sh` to make the test streams and is never linked or shipped.

## Status

| Area | Status |
|---|---|
| Sequence, picture and patch headers, weighting-quantisation matrices, decoding-order-index handling (wrap at 256), reference picture lists | done |
| LBAC arithmetic decoding of every CU syntax element: quadtree, binary and extended quadtree splits (8×8 to 64×64 and up to 128×128 CTUs), skip / direct / UMVE / affine / AMVR / EMVR / SMVD modes, HMVP | done |
| Intra prediction (33 modes plus DC, planar, bilinear; IPF, TSCPM, intra DT, PBT) and inverse transforms (DCT-II, DST-VII / DCT-VIII, secondary transform, 4×4 alternative) with dequantisation and chroma QP offsets | done |
| Motion compensation (8-tap luma, 4-tap chroma), temporal, spatial and affine MV prediction, I / P / B pictures | done |
| Deblocking, SAO (with uavs3d's shifted blocks), ALF | done |
| Output in display order, low-delay streams, 8- and 10-bit coding at the sequence's sample precision | done |
| Several patches per picture | ported from uavs3d, **not oracle-tested** (uavs3e writes only one patch per picture) |
| Raw elementary streams (`.avs3`) through `cfa4-codecs`, by content (told apart from AVS1 and AVS2, which share start codes and profile numbers), random access from I pictures | done |
| Damaged streams: an error or a skipped picture, never a panic | done |
| Profiles beyond the baseline (Main 8 / 10 with 4:2:2 and the scalable and high-level tools) | `Error::Invalid` |

A picture whose reference is gone (a leading picture after a seek) is skipped instead of failing
the stream.

## Tests

- `tests/uavs3d_oracle.rs`: streams encoded by uavs3e from a synthetic moving picture, random
  access and low delay, CTU 32 / 64, odd sizes, QP extremes, adaptive and chroma QP, split-size
  limits, every coding tool on and off, deblocking and SAO / ALF off, the three weighting-matrix
  modes, 10-bit coding and a 300-picture run past the decoding-order-index wrap: every picture
  equal to uavs3d (10-bit streams use the `-10bit` builds and `--internal_bit_depth 10`). The
  encoder's own reconstruction is only reported (it differs for some sizes and tool settings).
  Damaged versions of the streams and random packets decode without a panic. The tools come from
  `scripts/ci/install-avs3-oracle.sh` (pinned commits, also run in CI); without them the tests
  skip.
- `crates/codecs/tests/avs3_media.rs`: encoded streams opened by content, every frame asked for in
  a shuffled order against a straight decode; AVS1 and AVS2 streams stay with their decoders.

`examples/decode.rs` decodes a raw stream to planar YUV (`cargo run --release -p cfa4-avs3
--example decode in.avs3 out.yuv`).
