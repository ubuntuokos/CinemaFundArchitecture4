# cfa4-avs2

Pure-Rust (no `unsafe`) decoder for **AVS2-P2** (IEEE 1857.4, GB/T 33475.2), Main and Main 10
profiles. It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-18.2
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| davs2 | 1.7, commit `b41cf117452e2d73d827f02d3e30aa20f1c721ac`, GPL-2.0-or-later, VCL, NELVT, Peking University (Falei Luo et al.) | `source/common/` C code paths (`--disable-asm`), re-expressed in every file of `src/` |

davs2 is the decoder CFA4 matches. Its GPL-2.0-or-later code is used under GPL-3.0-or-later, the
licence of CFA4 ([register](../../docs/licensing.md)). FFmpeg has no AVS2 decoder of its own (only
a wrapper of libdavs2), so it is not an oracle here.

## Status

| Area | Status |
|---|---|
| Sequence (Main, Main 10, Main picture), picture and slice headers, weighting matrices skipped, pseudo start codes | done |
| AEC arithmetic decoding of every CU syntax element: quadtree CUs 8×8 to 64×64, skip / direct modes (including MHP skip and WSM), AMP, NSQT, SDIP, DHP, PMVR | done |
| Intra prediction (33 modes), inverse transforms 4×4 to 64×64 (including the 64×64 wavelet path and the secondary transform), dequantisation, chroma QP offsets | done |
| Motion compensation (8-tap luma, 4-tap chroma, eighth-sample), temporal MV prediction, I / P / B / F pictures, up to four references | done |
| Deblocking (with filter offsets, cross-slice switch), SAO, ALF | done |
| Reference picture sets, POC from coding order and output delay, output in display order, low-delay streams | done |
| 8-bit and 10-bit coding (output at the sequence's sample precision, rounded as davs2 does) | done |
| Several slices per picture | done |
| Field pictures, background (G / Gb / S) pictures, adaptive frequency weighting quantisation | `Error::Unsupported` |
| Raw elementary streams (`.avs2`) through `cfa4-codecs`, by content (told apart from AVS1, which shares its start codes and profile 0x20), random access from I pictures | done |
| Damaged streams: an error or a skipped picture, never a panic | done |

Where the reference tools disagree, this crate follows the encoder: davs2 decides whether the LCU
row below belongs to another slice (SAO without cross-slice filtering) from the previous picture's
slice numbers, because in its row-by-row pipeline that row is not decoded yet. A picture whose
reference is gone (a leading picture after a seek) is skipped instead of failing the stream.

## Tests

- `src/recon.rs`: transform and motion-compensation edge cases.
- `tests/davs2_oracle.rs`: streams encoded by xavs2 1.4 from a synthetic moving picture with every
  preset and coding tool, random access and low delay (P and F pictures, DHP, WSM, MHP skip), all
  intra, odd sizes, LCU 16 / 32 / 64, QP and chroma offsets, loop-filter offsets, rate control,
  several slices and Main 10 (10-bit coding): every picture equal to davs2 1.7 and, where it
  writes one, to the encoder's reconstruction. Damaged versions of the streams and random packets
  decode without a panic. The tools come from `scripts/ci/install-avs2-oracle.sh` (pinned commits,
  also run in CI); without them the tests skip.
- `crates/codecs/tests/avs2_media.rs`: encoded streams opened by content, every frame asked for in
  a shuffled order against a straight decode; AVS1 streams stay with `cfa4-cavs`.

`examples/avs2dec.rs` decodes a raw stream to planar YUV (`cargo run --release -p cfa4-avs2
--example avs2dec in.avs2 out.yuv`).
