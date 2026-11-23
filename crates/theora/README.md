# cfa4-theora

Pure-Rust (no `unsafe`) decoder for **Theora** video (the VP3 family): the identification, comment
and setup headers, then key and inter frames. It is an L0 crate: it depends only on `thiserror`.
Matroska supplement ([plan](../../docs/plans/CODEC-BACKLOG-2026-10-27.md), item "Theora").

```rust,ignore
let mut dec = cfa4_theora::Decoder::new(identification, comment, setup)?;
let picture = dec.decode(packet)?; // visible region, top row first, 8-bit Y, Cb, Cr
```

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| Theora specification | Xiph.Org, 2011 (BSD-style licence), <https://www.theora.org/doc/Theora.pdf> | the format: headers, Huffman tables, superblock, macroblock and fragment coding, DC prediction, the transform, motion compensation, the loop filter |
| FFmpeg | n7.1.1, LGPL-2.1-or-later, the FFmpeg developers | `vp3.c`, `vp3dsp.c`, `vp3data.h` (re-expressed in `header.rs`, `frame.rs`, `dsp.rs`, `vlc.rs`), including FFmpeg's behaviour on streams the specification forbids |

The ported code keeps FFmpeg's licence (`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). Porting is an owner-approved exception to the
FFmpeg-only-in-tests rule (owner decision 2026-11-14).

## Status

| Area | Status |
|---|---|
| Key and inter frames, golden frame, four-vector and prior-vector modes, half-pel motion | done |
| 4:2:0, 4:2:2 and 4:4:4; picture region (cropping) | done |
| Loop filter, per-block quantizer indices, three quantizers per frame | done |
| Repeated frames (empty packets) | done |
| Streams before 3.2.0 (VP3 proper) | refused |
| Matroska `V_THEORA` through `cfa4-codecs` | done |
| Ogg `.ogv` video | yes (`cfa4-codecs` `ogg_video`; frame number = packet index) |
| Damaged headers and packets: an error, never a panic | done |

`synth` builds synthetic streams for tests: real headers with flat Huffman tables and random
quantizers, and frame payloads of random bytes, which parse as valid frames (every coding flag,
mode, vector and token is a bit field). The oracle test in `cfa4-codecs` compares 36 of them with
ffmpeg's decode, frame by frame; a local ffmpeg with the libtheora encoder adds real files.
