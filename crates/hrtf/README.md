# cfa4-hrtf

HRTF sets and binaural (headphone) rendering for the CFA4 monitor: plan
[`CFA4-BINAURAL-MONITOR-001`](../../docs/plans/CFA4-BINAURAL-MONITOR-001.md), work package B-01.
Layer L1, no CFA4 dependencies, no `unsafe`, no allocation once a renderer is made, no panics on
any input.

## API

```rust
let set = Arc::new(cfa4_hrtf::HrtfSet::builtin(48_000)?);      // SADIE II KU100, Apache-2.0
let mut src = cfa4_hrtf::Binaural::new(set.clone(), Default::default())?;
src.process(cfa4_hrtf::position_of(90.0, 0.0, 1.0), 1.0, &mono, &mut left, &mut right);

let mut room = cfa4_hrtf::VirtualSpeakers::new(set, SpeakerLayout::surround_7_1_4(), Default::default())?;
room.process(&channels, &mut left, &mut right);                   // 12 channels → 2 ears
```

- `HrtfSet`: a `.maudhrtf` set (`assets/hrtf/README.md`), resampled to the output rate when it
  differs; `neighbours` / `blend` look a direction up on its rings (bilinear, four measured
  directions).
- `Binaural`: one source at a position that may move (metres, listener's frame: x right, y up,
  -z ahead). Minimum-phase responses blended around the direction, the onset delay read at a
  fraction of a sample (cubic Lagrange), the near field of a rigid-sphere head for sources closer
  than the set's distance (Duda and Martens 1998, with parallax), and a 128-frame (at 48 kHz)
  crossfade of the outputs when the direction changes.
- `VirtualSpeakers`: a channel layout (mono, 2.0, 5.1, 7.1, 7.1.2, 7.1.4 in WAV order, or any
  `Speaker::At` list) as fixed `Binaural` sources at the nominal loudspeaker directions; the LFE
  low-passed at 120 Hz into both ears at -10 dB (configurable).

## Origin

The binaural core is a Rust port of [Maul Audio](https://github.com/Rawframe-Project/maul-audio)
(MIT, Copyright (c) 2026 Sirac Ozmen), commit `aeb383458250759f92a3e724a5bd006505e45b4d`:
`src/hrtf*.{h,c}`, `file_fields.c`, `crc32.c`, `binaural.c`, `binaural_dsp.{h,c}`,
`near_field*.{h,c}` (the table copied unchanged). The ported files keep the MIT notice
(`LICENSE-MIT.maul-audio`); `speakers.rs` and `lib.rs` are CFA4's own (GPL-3.0-or-later).
Register row: `docs/licensing.md` §5.

## Tests

- `tests/binaural.rs`: the built-in set and its licence, malformed and mutated files are errors,
  the renderer reproduces the set's own left/right balance, ILD (> 10 dB at 4 kHz) and ITD (0.6 to
  0.95 ms, following the set's onset difference) for a lateral source, a source turning once a
  second makes no step steeper than a still one, the FIR equals a direct convolution, block size
  does not change the output, resampled sets keep their 1 kHz level, the near field is the identity
  at the set's distance and raises a close ear's lows, non-finite parameters are ignored, and the
  virtual loudspeakers place the centre, the surrounds and the LFE. 7.1.4 renders one second of
  audio in about 16 ms on one core of the build container (Xeon 2.1 GHz).
- `tests/maul_oracle.rs`: the same moving source rendered by Maul Audio's C library
  (`tests/oracle/maul_harness.c`, never linked or shipped): bit-identical on 2026-11-27 for 48 kHz
  with and without the near field and for the set resampled to 44.1 kHz (48000 frames each); the
  committed fixtures keep the first 4096 frames, compared within 1e-6 (1e-5 resampled: libm rounding
  of the resampler's f64 kernel differs by machine, 1.5e-6 on the CI runner).
