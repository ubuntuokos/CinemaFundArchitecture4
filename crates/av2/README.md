# cfa4-av2

Clean-room AV2 bitstream parsing from the AV2 Bitstream & Decoding Process Specification v1.0.0
(native track B of `docs/plans/AV2-CODEC-PLAN-2026-10-29.md`, step AV2-04). L0: depends only on
`cfa4-bitstream`.

| Stage | Status |
|---|---|
| Annex B framing, `leb128()` (§ 4.10.5, Annex B.2) | done |
| OBU header and extension (§ 5.2.2, Table 6.1) | done |
| Sequence header, leading fields up to the cropping window (§ 5.4.1) | done |
| Sequence header tool syntax: decoder model, layer dependency maps, partition, segment, intra, inter, screen content, transform/quantizer/entropy, filter and tile configuration, film grain flag, extension flag and trailing bits (§ 5.4.1–5.4.13) | done; all 13 AVM corpus headers end exactly at their trailing bits |
| Tile layout `tile_params()` (§ 5.18.7.3–5.18.7.7) | done (used by the sequence header; the frame header will reuse it) |
| Stream scope check (`probe_annexb`: no MSDO, no extended or embedded layers) | done |
| Frame header, operating point, tile decoding | not started |

Tests: unit tests per module; `tests/fuzz_hostile.rs` truncates and mutates a synthetic stream and,
when present, the AVM corpus in `target/fixtures/av2/vectors` (`scripts/dev/av2-oracle.py corpus`)
and checks that the corpus sequence headers parse completely (geometry, tiles, film grain). Nothing may panic on hostile input.
