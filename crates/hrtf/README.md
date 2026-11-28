# cfa4-hrtf

HRTF sets and binaural (headphone) rendering for the CFA4 monitor: plan
[`CFA4-BINAURAL-MONITOR-001`](../../docs/plans/CFA4-BINAURAL-MONITOR-001.md), work packages B-01
(HRTF sets, binaural core, virtual loudspeakers) and B-02 (the spatial scene and its object audio
adapters). Layer L1; depends on `cfa4-truehd` and `cfa4-iab` (L0) for the adapters and on
`cfa4-audio-dsp` (same layer, allowed in `xtask` `SAME_LAYER`) for the room's Surround Reverb. No
`unsafe`, no allocation while rendering (scheduling a scene update may allocate), no panics on any
input.

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
- `SceneRenderer` (`scene`): beds and moving objects, each a mono signal with timed updates
  (`schedule(at, object, SceneState, ramp)`, then `render`). Room-cube positions (the convention of
  `cfa4_truehd::render` and `cfa4_iab::render`) map to directions around a listener at the centre
  of the floor, warped so that the cube's front corners are at ±30°, the side walls at ±90° and the
  rear corners at ±135°; walls 2 m away (configurable), so objects inside the room get the near
  field. Size spreads an object over four decorrelated directions; Dolby's headphone render modes:
  off (plain stereo pan), near (dry), mid and far (small and larger room: the Surround Reverb fed
  with the binaural signal; can be turned off). The LFE as in `VirtualSpeakers`.
- `atmos`: `schedule_truehd` / `truehd_state` turn the events of `cfa4_truehd::cfa4::ObjectDecoder`
  into scene updates (bed labels to loudspeaker directions, snap to the nearest 7.1.4 loudspeaker,
  elevation off to ear height; the OAMD headphone element's mode is not mapped, see the module
  notes); `IabScene` renders IAB frames: the Beds (with activation and BedRemaps) through
  `cfa4_iab::render_beds` to 7.1.4 as fixed loudspeakers, the activated Objects with their own
  position, gain and spread per sub-block, matched across frames by MetaID.
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
- `tests/scene.rs`: an object circling the listener in 40 ramped updates has no clicks, objects
  play from their side, a bed channel matches its virtual loudspeaker within 0.5 dB, the LFE is
  low-passed into both ears at -10 dB, off mode pans without HRTF, the mid and far rooms add a
  tail and turn off, size 1 lowers the interaural coherence (0.89 → 0.25) at the same level,
  hostile states never panic, TrueHD states map as documented, an IAB frame's object moves from
  left to right across frames. With `CFA4_ATMOS_SAMPLE` set to a raw `.thd`: on 2026-11-27 the
  Altitude16 Dolby Atmos sample (first 17 s) placed all 16 objects and rendered in about 1.4 s
  with the rooms on, its two ears 3 dB above the energy of the 7.1.4 loudspeaker render.
- `tests/maul_oracle.rs`: the same moving source rendered by Maul Audio's C library
  (`tests/oracle/maul_harness.c`, never linked or shipped): bit-identical on 2026-11-27 for 48 kHz
  with and without the near field and for the set resampled to 44.1 kHz (48000 frames each); the
  committed fixtures keep the first 4096 frames, compared within 1e-6 (1e-5 resampled: libm rounding
  of the resampler's f64 kernel differs by machine, 1.5e-6 on the CI runner).
