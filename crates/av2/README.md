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
| Tile layout `tile_params()` (§ 5.18.7.3–5.18.7.7) | done (sequence and frame header) |
| Frame header of intra frames (`frame_header_info()` § 5.18 through `film_grain_config()`: frame size, quantizer, segmentation, quantizer matrices, delta q, lossless, deblocking, GDF, CDEF, loop restoration modes and unit sizes, CCSO, tx mode, film grain id/seed) and `tile_group_obu()` up to the tile data (§ 5.19: tile range, tile sizes) | done; all 17 corpus key-frame headers have the reference decoder's length, `base_q_idx` and tile grid, and their tile data fills the OBU exactly. Not yet: inter, open-loop key, switch/RAS, leading, TIP and bridge frames, multi-frame headers, frame-level Wiener filter banks (each returns `Unsupported`) |
| Stream scope check (`probe_annexb`: no MSDO, no extended or embedded layers) | done |
| Inter frame headers, operating point set, tile decoding | not started |

Tests: unit tests per module; `tests/fuzz_hostile.rs` truncates and mutates a synthetic stream and,
when present, the AVM corpus in `target/fixtures/av2/vectors` (`scripts/dev/av2-oracle.py corpus`)
and checks that the corpus sequence headers parse completely (geometry, tiles, film grain) and that
the key-frame headers match the values the instrumented AVM v1.0.0 decoder reports (`NumFrameHeaderBits`,
`base_q_idx`, tile grid). Nothing may panic on hostile input.
