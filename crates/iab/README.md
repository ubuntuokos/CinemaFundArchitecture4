# cfa4-iab

Clean-room Immersive Audio Bitstream (IAB) decoder, renderer and writer in pure Rust. IAB is the
object-based sound format of SMPTE Digital Cinema Packages (ST 429-18) and IMF (ST 2067-201).
Layer L0: depends only on `cfa4-bitstream` and `thiserror`. No `unsafe`, no panics on malformed
input.

## Specification

SMPTE ST 2098-2, *Immersive Audio — Bitstream Specification*, from the public committee draft of
its 2026 revision ([github.com/SMPTE/st2098-2](https://github.com/SMPTE/st2098-2)). The code was
written from the draft's syntax tables and Annex B pseudocode; none of its text is copied here.
No other IAB implementation was consulted.

- Clauses 5, 7-10: Plex coding, the IABitstream frame (preamble and IAFrame segments), the
  element tree (IAFrame, BedDefinition, BedRemap, ObjectDefinition, ObjectZoneDefinition19,
  AudioDataPCM, AudioDataDLC, AuthoringToolInfo, UserData, FrameSequence), every frame rate
  (Tables 18, 19, 24, 31, 33, 34, including the five-frame cadence of 30000/1001 and 60000/1001).
- Annex B: DLC lossless coding: lattice to direct-form predictors (Q20), Rice / Golomb and PCM
  residuals, the 96 kHz extension layer over the half-band interpolated 48 kHz base layer, the
  final shift. The arithmetic is the normative 32 / 64-bit integer arithmetic; it wraps where
  hostile coefficients would overflow.

One reading of the draft: in the 96 kHz Rice residual pseudocode the unary quotient and sign are
only applied when RiceRemBits is non-zero, unlike the 48 kHz layer. That looks editorial, so both
layers decode as the 48 kHz one does (quotient and sign always apply).

## API

```rust
let frame = cfa4_iab::parse(&bytes)?;                    // one IABitstream frame (or IAFrame)
let feeds = cfa4_iab::render(&frame, &cfa4_iab::Layout::surround_7_1())?; // [speaker][sample]
let bytes = cfa4_iab::write(&frame)?;                    // PCM or DLC per asset
for f in cfa4_iab::split(&stream) { /* each IABitstream frame of a concatenation */ }
```

Decoded assets are 32-bit left-justified samples (`AudioData::samples`, full scale ±2^31);
`Frame` keeps every bitstream code (gains, positions, spread, zones), so `write(&parse(b)?)`
reproduces the frame.

## Renderer

ST 2098-2 leaves rendering to the cinema processor. `render` follows the bitstream's activation
rules (UseCase of conditional Beds and Objects, BedRemap, 10.4.2 / 10.6.1) and places sound
simply:

- Bed channels go to the loudspeaker with the same ChannelID, or are panned from their nominal
  position (`channel_position`); LFE goes to the LFE (dropped on stereo).
- Objects are panned by distance with constant power (weights 1 / (d² + ε)², far loudspeakers
  under 5 % dropped), spread widens the source, snap sends it to the nearest loudspeaker within
  the tolerance, and the nine zone gains of Table 25 scale their loudspeakers.
- Gains ramp linearly across each sub-block. Decorrelation is not applied and the 19 zones of
  ObjectZoneDefinition19 are not used.

Layouts: 2.0, 5.1, 7.1 (L R C LFE Lrs Rrs Lss Rss) and 7.1.4, in WAV / SMPTE channel order.

## Writer

`write` produces one IABitstream frame with an empty preamble. PCM assets are written at the
frame's bit depth. DLC assets are lossless: the encoder strips the low bits every sample shares
(ShiftBits), tries lattice predictors of order 2-16 (Levinson-Durbin, 10-bit reflection codes)
against none, and codes each sub-block as Rice or PCM, whichever is smaller. At 96 kHz the base
layer is a smoothed, eight-sample-aligned decimation; any base layer keeps the coding lossless.

## Tests

`src/tests.rs` and `src/dlc.rs`: Plex examples of 5.2, PCM and DLC frames at 48 and 96 kHz and
several rates round-trip exactly, the 1001 cadences sum to the right sample counts, conditional
Beds and BedRemap follow the UseCase, Objects pan by position with snap, zones and gain ramps,
hostile DLC codes and mutated or truncated frames are errors, never panics. `cfa4-mxf` and
`cfa4-codecs` test IAB tracks in MXF (frame- and clip-wrapped, with and without an index) and
their playback as 7.1. No third-party IAB stream has been tested yet.
