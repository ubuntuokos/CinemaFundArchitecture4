# cfa4-truehd

Dolby TrueHD and MLP (Meridian Lossless Packing, DVD-Audio) decoder.

## Origin and licence

This crate is a port of the [`truehd`](https://github.com/truehdd/truehdd) crate, version 0.7.2
(crates.io release), Copyright Rainbaby, licensed under the Apache License 2.0 (`LICENSE` in this
directory). It is listed in the reused-code register, [`docs/licensing.md`](../../docs/licensing.md)
§5. Apache-2.0 code may be combined into CFA4 (GPL-3.0-or-later); the files taken from upstream keep
the Apache-2.0 terms and carry a notice saying they were modified.

TrueHD has no public specification, so this is not a clean-room implementation from a spec: it
reuses an existing, permissively licensed open-source implementation, as `docs/licensing.md` §3
allows.

## Changes made for CFA4

- `utils/bitstream_io.rs` is rewritten over a plain byte slice; the `bitstream-io` dependency and
  its huffman tree macros are gone (the residual codes are decoded procedurally, same tables).
  Every read is bounds-checked.
- `anyhow.rs` replaces the `anyhow` crate (`Error`, `Result`, `anyhow!`, `bail!`, `ensure!`).
- The optional `evo-protection` feature (HMAC check of Evolution frames, `hmac` / `sha2`) is
  removed.
- No panicking calls outside tests (`unreachable!`, `unwrap` on a poisoned lock, the `unsafe`
  `get_unchecked` / `transmute` in `structs/oamd.rs` replaced by checked code); the crate denies
  them with clippy lints.
- Hostile-stream hardening: restart A headers with more than 14 matrix channels are rejected
  (the two dither channels must fit the 16-wide buffers), the restart A dither is widened to
  `i32` before `dither_shift` is applied, a segment whose blocks do not add up to one access unit
  is an error instead of a warning, and `decode_multi` refuses any block that would overrun its
  buffers. More than four substreams are rejected; Huffman residuals are reconstructed in 64 bits
  and range-checked; an Evolution frame that overruns its extra data is an error; and a major
  sync that reassigns the speakers refreshes the channel labels.
- `cfa4.rs` (new) is the interface the rest of CFA4 uses: [`cfa4::Decoder`] takes container
  packets and gives planar `f32` channels in CFA4's WAV / SMPTE order, plus the helpers the
  Matroska / MP4 sources use for packet timing and seek pre-roll.
- `render.rs` (new) renders the Atmos object presentation to loudspeakers.
- Formatted with the workspace's `rustfmt` settings; doc examples name `cfa4_truehd`.

## What it decodes

- FBA (TrueHD) and FBB (MLP) streams, 44.1–192 kHz, up to 24 bits.
- The 2-, 6- and 8-channel presentations; `cfa4::Decoder` picks the largest of them. Dolby Atmos
  streams carry a 7.1 (or 5.1) bed presentation, which is decoded.
- Dolby Atmos objects: `cfa4::Decoder::with_objects(layout)` decodes the 16-channel object
  presentation and renders its beds and dynamic objects with the Object Audio Metadata (OAMD) to
  2.0, 5.1, 7.1 or 7.1.4 (`render.rs`, new for CFA4; the module notes list its rendering choices).
  Programs with intermediate spatial format objects fall back to the bed presentation. Checked on
  synthetic OAMD only: no real Atmos stream has been tried yet (#124). Not rendered: binaural
  (#125), screen reference, divergence and trims (#127). E-AC-3 JOC is not decoded (#126).

## Tests

- `tests/oracle.rs`: ffmpeg's TrueHD and MLP encoders write streams (mono, stereo, 4.0, 5.1;
  44.1 / 48 / 96 kHz; 16 and 24 bit); the decode is bit-exact against ffmpeg's decoder and against
  the encoder's input, whole and in 1000-byte pieces. Damaged streams (bit flips, truncation,
  scrambled bytes) never panic.
- `crates/codecs/tests/truehd_oracle.rs`: TrueHD in Matroska and MP4, bit-exact, also on random
  access.
- `src/render/tests.rs`: the panner (each loudspeaker, constant power, zones, snap, elevation,
  beds), ramps across calls, OAMD payloads written bit by bit and parsed back, mangled payloads.
- `tests/oracle.rs` `a_real_atmos_stream_renders_like_its_bed_presentation`: runs on a real Atmos
  `.thd` named by `CFA4_ATMOS_SAMPLE` (skipped without one).
- The upstream unit tests.

No free encoder writes 7.1 TrueHD, so 8-channel presentations are covered only by the upstream
tests on their example stream.
