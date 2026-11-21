# cfa4-avs3

Pure-Rust (no `unsafe`) decoder for **AVS3-P2** (IEEE 1857.10, GB/T 33475.2), Main-8 and Main-10
profiles. It is an L0 crate: it depends only on `thiserror`. Codec backlog item CB-18.3
([plan](../../docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md)).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| uavs3d | 1.1, commit `0e20d2c291853f196c68922a264bcd8471d75b68`, BSD-3-Clause, Peking University Shenzhen Graduate School, Peng Cheng Laboratory, Guangdong Bohua UHD Innovation Corporation | `source/decoder/` and `source/decore/` C code paths (no SIMD), re-expressed in every file of `src/` |

The ported files keep an SPDX BSD-3-Clause header and the notice in [`LICENSE.uavs3d`](LICENSE.uavs3d);
CFA4's own changes are GPL-3.0-or-later ([register](../../docs/licensing.md)). uavs3d is the decoder
CFA4 matches. The test encoder **uavs3e** (BSD-4-Clause, with an advertising clause) is only run as an
external program to make test streams; none of its code is in this crate or linked or shipped.

## Status

| Area | Status |
|---|---|
| Sequence (Main-8, Main-10), picture and patch headers, weighting-quantisation matrices | done |
| LBAC arithmetic decoding of every CU syntax element; quad, binary and extended-quad tree CUs up to 128×128, constrained-prediction-mode trees | done |
| Intra prediction, IPF, DT, TSCPM, IPCM, secondary transform, alternative 4×4 transform, PBT; inverse transforms 4×4 to 64×64 | done |
| Inter prediction: 8-tap luma / 4-tap chroma MC, AMVR / EMVR, HMVP, UMVE, SMVD, affine (4- and 6-parameter), temporal MV, I / P / B pictures | done |
| Deblocking, SAO, ALF (the shifted regions of uavs3d, patch borders included) | done |
| Reference picture lists, decode order index and output delay, output in display order, low-delay streams | done |
| 8-bit and 10-bit coding, odd picture sizes | done |
| Uniform patches | decoded as in uavs3d (no test stream: uavs3e cannot write more than one patch) |
| Non-uniform patches, library pictures, field coding, changing sequence parameters | `Error::Unsupported` |
| Raw elementary streams (`.avs3`) through `cfa4-codecs`, by content, random access from I pictures | done |
| Damaged streams: an error or a skipped picture, never a panic (checked with thousands of damaged streams) | done |

## Traps found while porting

- uavs3d's x86 SIMD paths differ from its C paths on extreme content (noise at high bit rates). The
  oracle is built C-only (`ENABLE_FUNCTION_X86` removed), which is what this crate follows.
- SAO and ALF filter regions shifted four samples up (and left) of the LCU: the loop filters run
  one LCU row behind the reconstruction. They are done here on whole pictures with the same regions.
- The LBAC `end` of a patch is exclusive and the end-of-picture bin after the last LCU is not asserted.
- At most one picture is output per decoded picture (the smallest order count among those whose
  output delay has passed); the end of the stream drains the rest.
- uavs3e settings whose streams its own reconstruction and uavs3d disagree on (`--hmvp 0`, 8×8
  minimum CU, patches) are not covered by the oracle tests.

## Tests

- `tests/uavs3d_oracle.rs`: streams encoded by uavs3e from a synthetic moving picture (random access,
  low delay, closed GOP, all intra, CTU 32 / 64 / 128, odd sizes, QP and weighted quantisation,
  each tool group off, with and without loop filters, 8- and 10-bit): every picture equal to uavs3d
  and, where uavs3e's reconstruction agrees with uavs3d, to that. Damaged streams and junk decode
  without a panic. The tools come from `scripts/ci/install-avs3-oracle.sh` (pinned commits, also
  run in CI); without them the tests skip.
- `crates/codecs/tests/avs3_media.rs`: encoded streams opened by content, every frame asked for in a
  shuffled order against a straight decode; AVS1 streams stay with `cfa4-cavs`.

`examples/avs3dec.rs` decodes a raw stream to planar YUV (`cargo run --release -p cfa4-avs3
--example avs3dec in.avs3 out.yuv`).
