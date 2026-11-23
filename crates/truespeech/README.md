# cfa4-truespeech

Pure-Rust decoder for **DSP Group TrueSpeech** (WAVE format tag `0x0022`), codec backlog CB-37.2:
8 kHz mono speech, 32-byte frames of 240 samples. Ported from FFmpeg n9.0.2's
`libavcodec/truespeech.c` (LGPL-2.1-or-later, `COPYING.LGPL-2.1`; an owner-approved exception to
the FFmpeg-only-in-tests rule, see `docs/licensing.md` §5). CFA4 changes are GPL-3.0-or-later.

[`Decoder::decode`] takes a packet of whole frames (a trailing partial frame is ignored, a packet
under one frame is an error) and returns 16-bit samples. The filter and pitch memories carry from
frame to frame, so a decoder must see every frame in order; `cfa4-codecs` and `cfa4-media` save
decoder states at intervals so that a seek decodes the same samples as playback from the start.

## Fidelity

`tests/oracle.rs` writes random 32-byte frames into a WAV with tag 0x0022 and compares the samples
with ffmpeg's (external oracle only, never linked) byte for byte. The FATE sample
`truespeech/a6.wav` is compared when it is in `target/fixtures/fate/` (local only).
