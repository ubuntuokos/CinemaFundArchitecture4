# cfa4-atracenc

Pure-Rust (no `unsafe`) ATRAC encoders for CFA4's AT9/AT3 converter (Tools ▸ Audio Tools):
**ATRAC3** (Sony's ATRAC3 as in PSP / PS3 `.at3` files and MiniDisc LP2 / LP4) and the **AT3
writer** (RIFF WAVE, format 0x0270, Sony's 14 bytes of extra data and a `fact` chunk with the
sample count and the encoder delay). It is an L0 crate: it depends only on `thiserror`. Decoding
is in [`cfa4-realaudio`](../realaudio/README.md).

## Sources and licences

| Source | Version, licence | Used for |
|---|---|---|
| atracdenc | 0.2.3, LGPL-2.1-or-later, Daniil Cherednik | the ATRAC3 encoder (`atrac3denc`, `atrac3_bitstream`, `atrac_scale`, `atrac_psy_common`, `gain_processor`, `transient_detector`) in `atrac3.rs` and `psy.rs`, the QMF (`qmf.rs`), the MDCT (`mdct.rs`; the FFT is our own radix-2), the bit writer (`bits.rs`), the AT3 header (`at3.cpp`, in `riff.rs`) |
| Musepack (via atracdenc) | LGPL-2.1-or-later | the absolute threshold of hearing table (`ATHformula_Frank`, in `psy.rs`) |

The ported code keeps its licence (`COPYING.LGPL-2.1`); CFA4's changes are GPL-3.0-or-later
([register](../../docs/licensing.md)). The source converter (BMK-Studio AT9-AT3_Converter) only
runs Sony's SDK tools and has no licence, so nothing of it is used.

## Status

| Area | Status |
|---|---|
| ATRAC3 modes | 66 kbit/s (LP4, joint stereo), 105 kbit/s and 132 kbit/s (LP2): the three block sizes Sony's and FFmpeg's decoders accept in AT3 files. Mono uses half the frame (33, 52, 66 kbit/s) |
| Coding | four-band QMF, 256-line MDCT per band, gain control for attacks, bit allocation by the threshold of hearing and a loudness tracker, CLC/VLC chosen per frame. No tonal components (as atracdenc 0.2.3) |
| Polarity | the input is negated: with atracdenc's MDCT sign, Sony's and FFmpeg's decoders would otherwise play the stream back inverted |
| Delay | decoded audio is 1162 samples late; the `fact` chunk says so |
| Input | 44.1 kHz only (the converter resamples); non-finite samples are silence, the rest is clamped to [-1, 1]; never a panic |
| ATRAC3plus, ATRAC9 encoders | not yet (separate items) |

## Tests

- Unit tests: the bit writer, the MDCT against the textbook transform, rounding, the threshold of hearing,
  rates, frame sizes and hostile input.
- `tests/atrac3_roundtrip.rs`: every mode in mono and stereo, encoded and decoded by our ATRAC3
  decoder: the delay in the `fact` chunk is the best-matching shift and the SNR of a sweep with
  clicks and noise is above 18 dB (20–27 dB measured); silence stays silent, full-scale and
  non-finite input decode finite; cancelling stops. Where ffmpeg is present (CI), ffmpeg's `atrac3`
  decoder reads every file without a message and gives our decoder's samples (within 10⁻⁵).
