# cfa4-vvc

Clean-room H.266/VVC bitstream parsing from ITU-T Rec. H.266 (ISO/IEC 23090-3), edition 2
(04/2022) syntax, for the VVC integration ([ADR 0013](../../docs/adr/ADR-0013-vvc-integration.md),
[VVC plan](../../docs/plans/CFA4-VVC-PLAN-001-2026-11-07.md) VVC-02). L0: depends only on
`cfa4-bitstream`. Pictures are decoded by the VVdeC helper or FFmpeg (ADR 0013); this crate is also
the first step of the native Rust decoder (plan B line).

| Stage | Status |
|---|---|
| NAL unit header and types (§ 7.3.1.2, Table 5) | done |
| Access units, multi-layer aware (§ 7.4.2.4.3); Annex B byte ranges | done |
| Picture facts: IRAP / IDR / GDR, `ph_non_ref_pic_flag` (§ 7.3.2.8, § 7.3.7.1) | done |
| SPS leading fields up to `sps_bitdepth_minus8`: profile, tier, level, GCI, sublayer levels, conformance window, subpicture layout skipped (§ 7.3.2.4, § 7.3.3) | done |
| Rest of the SPS (VUI colour description), PPS, slice header, decoding | not started |

Tests: unit tests per module; `tests/encoded_streams.rs` checks access units, IRAP pictures and
the SPS geometry of a VVenC stream (generated with an FFmpeg that has `libvvenc`; skipped without
it, as `libvvenc` is a restricted component); `tests/fuzz_hostile.rs` truncates and mutates a
synthetic stream and the generated fixture. Nothing may panic on hostile input.

The GCI and subpicture skipping were cross-checked with FFmpeg's `trace_headers` bitstream filter
(test oracle only) on SPS units spliced from a VVenC stream.
