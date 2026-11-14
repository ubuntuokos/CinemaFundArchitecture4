# cfa4-av2-decode

The out-of-process AV2 decoder helper of CFA4 ([ADR 0010](../../docs/adr/ADR-0010-av2-external-decoder.md),
A track of the [AV2 plan](../../docs/plans/AV2-CODEC-PLAN-2026-10-29.md)). A small C program that
wraps [dav2d](https://code.videolan.org/videolan/dav2d) (BSD-2-Clause) and speaks the CFA4 AV2
frame protocol on stdin/stdout. The engine side is the pure-Rust adapter
`crates/codecs/src/av2_external.rs`, which documents the protocol; a crash or hang of the helper
is an error there, never a crash of CFA4.

- `cfa4-av2-decode.c`: the helper (GPL-3.0-or-later).
- `dav2d.json`: the pinned dav2d commit (the one measured in the AV2-00 audit). Change it only
  together with a new conformance run.
- Build: `python3 scripts/dev/av2-decoder.py build` (into `target/av2-decoder/bin`; needs git,
  meson, ninja, nasm and a C compiler).
- Fuzzing: `python3 scripts/dev/av2-decoder.py build --sanitize` (ASan + UBSan, into
  `target/av2-decoder-asan`, never shipped), then `python3 scripts/dev/av2-decoder.py fuzz`
  ([`docs/testing.md`](../../docs/testing.md)).
- Use: build CFA4 with `--features cfa4-codecs/av2-external` and set `CFA4_AV2_DECODER` to the
  helper. This is AV2-LAB level; the router registration waits for G1 (plan X/7).

Packaging into the `cfa4` package (`/opt/cfa4`) with the dav2d licence and the AOM Patent License
in the notices is plan step AV2-10 (owner decision 2026-11-11: the helper is part of the `cfa4`
package).
