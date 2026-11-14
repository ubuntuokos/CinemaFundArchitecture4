# cfa4-vvc-decode

The out-of-process H.266/VVC decoder helper of CFA4 ([ADR 0013](../../docs/adr/ADR-0013-vvc-integration.md),
VVC-02 of the [VVC plan](../../docs/plans/CFA4-VVC-PLAN-001-2026-11-07.md)). A small C program that
wraps Fraunhofer HHI's [VVdeC](https://github.com/fraunhoferhhi/vvdec) (BSD-3-Clause-Clear) and
speaks the CFA4 frame protocol v1 on stdin/stdout, the same protocol as the AV2 helper
(`packaging/av2-decode`, ADR 0010) with the hello tag `CFA4VVCD`. The engine side is the Rust
adapter `crates/codecs/src/external_decoder.rs` (protocol and process handling, shared with AV2)
and `crates/codecs/src/vvc_external.rs`; a crash or hang of the helper is an error there, never a
crash of CFA4.

- `cfa4-vvc-decode.c`: the helper (GPL-3.0-or-later). Requests are access units in Annex B form;
  `FLSH` drains the decoder at the end of a stream, `RSET` reopens it before a seek.
- `vvdec.json`: the pinned VVdeC release (the one measured in the VVC-00 report). Change it only
  together with a new conformance run.
- Build: `python3 scripts/dev/vvc-decoder.py build` (into `target/vvc-decoder/bin`; needs git,
  cmake and a C/C++ compiler).
- Use: build CFA4 with `--features cfa4-codecs/vvc-external` and set `CFA4_VVC_DECODER` to the
  helper. The router registration (`video.decode.vvc = ["vvdec", "ffmpeg", "vaapi"]`) waits for
  the G2 gate.

VVdeC carries no patent licence (VVC Advance, Via LA VVC): the helper ships only in the
`cfa4-restricted-extras` package, never in the base `cfa4` package (ADR 0013, plan step VVC-07).
