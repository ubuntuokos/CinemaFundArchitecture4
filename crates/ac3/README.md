# cfa4-ac3

Clean-room AC-3 (Dolby Digital) and E-AC-3 (Dolby Digital Plus) audio decoder and encoder in pure Rust. Layer L0: depends only on
`thiserror`. Builds for `wasm32-unknown-unknown`; no `unsafe`. No GPL/LGPL code (FFmpeg,
liba52) was consulted; FFmpeg is used only as an external fixture generator and test oracle.

## Specification

ATSC A/52:2012 "Digital Audio Compression (AC-3, E-AC-3) Standard" (17 December 2012), the
publicly available standard: §5 bit stream syntax and semantics, §6 decoding overview, §7.1
exponents, §7.2 parametric bit allocation (Tables 7.6-7.16 extracted mechanically from the
standard's text), §7.3 mantissas and dither, §7.4 coupling, §7.5 rematrixing, §7.7.1 dynamic
range, §7.9 transforms and block switching. The window is computed as the Kaiser-Bessel derived
window (α = 5) and checked against Table 7.33 to 6·10⁻⁶. E-AC-3: Annex E §E2 bit stream syntax
(syncinfo, bsi, audfrm, audblk; Tables E2.1-E2.13, frame exponent strategies from Table E2.10),
§E3.3 modified parameters, §E3.4.2 AHT helper variables (to detect AHT), §E3.6 spectral extension
(attenuation Table E3.14, which is 2^(-(code + 1)(bin + 1) / 15), checked against the table).

## API

```rust
let mut dec = cfa4_ac3::Decoder::new();
let h = cfa4_ac3::parse_header(frame)?;   // rate, channels, frame size
let out = dec.decode(frame)?;                  // h.samples() per channel, WAV channel order
let (prog, used) = dec.decode_program(data)?;  // E-AC-3: substream 0 + its dependent substreams
dec.reset();                                    // before decoding from another position

let mut enc = cfa4_ac3::Encoder::new(48_000, 6, 448)?;   // or Encoder::new_eac3(rate, ch, kb/s)
let frames = enc.encode(&[l, r, c, lfe, ls, rs])?;     // planar f32, WAV order; whole syncframes
let tail = enc.flush()?;                               // pads; output lags input by 256 samples
```

## Decoder

- Every audio coding mode (1+1, 1/0, 2/0, 3/0, 2/1, 3/1, 2/2, 3/2) with or without LFE; 32 /
  44.1 / 48 kHz and every frame size; bsid ≤ 8 (9 and 10, the half / quarter-rate variants, are
  accepted).
- D15 / D25 / D45 exponents with reuse across blocks; the parametric bit allocation in the
  standard's fixed-point steps, delta bit allocation, the all-zero SNR offset special case;
  grouped 3-, 5- and 11-level mantissas (groups shared across exponent sets), 7- and 15-level
  and asymmetric mantissas up to 16 bits.
- Channel coupling (coupling bands, master coordinates, phase flags in 2/0), rematrixing (all
  four banding cases), dynamic range control (`dynrng`, `dynrng2`; applied at full scale), dither
  for zero-bit mantissas (uniform ±0.707, §7.3.4).
- 512-sample IMDCT and the pair of 256-sample IMDCTs for block-switched blocks, with radix-2
  inverse FFTs, KBD window and overlap-add.
- Output: f32 at full scale ±1; channels reordered from the coded order (L C R Ls Rs LFE) to the
  WAV / SMPTE order (L R C LFE Ls Rs). No downmix (the mixer handles layouts).
- Dither is reseeded per syncframe from the frame's CRC words, so a frame decodes to the same
  samples however it was reached (seeking, render caches).
- Errors (bad exponents, reserved codes, blocks running past the frame) return `Err`; garbage
  never panics.

## E-AC-3 (bsid 11-16)

- Independent substream 0 is decoded (stream types 0 and 2); further independent programs decode
  to no channels. `Header::is_primary` tells them apart.
- Programs beyond 5.1 (§E2.3.1, 7.1 and others): `Decoder::decode_program` decodes independent
  substream 0 together with the dependent substreams (stream type 1, substream ids 0-7) that
  follow it in the buffer, as an MP4 or Matroska access unit holds them. Each dependent channel
  takes a location from the custom channel map (`chanmap`, Table E2.5; bit 0 the MSB, pair bits
  take two channels) or, without one, from its acmod; it replaces the program's channel at that
  location (a 7.1 core's downmixed surrounds) or adds one. `Decoded::locations` names every
  channel; the output is in `Location` order, which keeps the WAV order for 5.1 and gives
  L R C LFE Lrs Rrs Ls Rs for 7.1 (the WAV / SMPTE order of 7.1, back pair before side pair, as
  the AAC and Opus paths of `cfa4-codecs` give it). A dependent substream that fails to decode (an unsupported
  tool, damage) or disagrees with its program's rate or length keeps the core's channels and
  silences its new locations, so the layout does not change mid-stream. Each dependent substream
  keeps its own overlap and exponent state. `decode` alone still returns the 5.1 core.
  `program_locations` gives the program's layout from the headers alone (the channel count an
  MPEG-TS stream decodes to).
- 1, 2, 3 or 6 blocks per syncframe (`Header::samples`), 32 / 44.1 / 48 kHz and the reduced rates
  16 / 22.05 / 24 kHz, every acmod with or without LFE.
- The whole bsi (mixing and informational metadata, custom channel maps, addbsi) is parsed and
  skipped; audfrm: per-block or frame-based exponent strategies (exponents may be reused across
  syncframes of fewer than six blocks; the decoder keeps them until `reset`), the syntax-enable
  flags and their defaults, SNR offset strategies 1-3, converter fields, block start information.
- Audio blocks share the AC-3 path: coupling with the default or transmitted band structure
  (`firstcplcos` / `firstcplleak` states), rematrixing with the Annex E band count, end mantissas
  derived from spectral extension.
- Spectral extension: band structure, coordinates, blending factors, translation with wrap,
  banded RMS, the border / wrap-point notch filter, noise blending and scaling (§E3.6.4).
- Refused with `Error::Unsupported`: the adaptive hybrid transform (when a channel uses it; `ahte`
  alone is fine) and enhanced coupling. Transient pre-noise processing data is read and not
  applied.

## Accuracy (vs FFmpeg as an external oracle)

AC-3 output is not bit-exact across decoders: zero-bit mantissas are filled with
decoder-specific dither. `tests/oracle.rs` decodes FFmpeg-encoded streams and compares with
FFmpeg's decode (worst channel SNR):

| Stream | SNR vs FFmpeg | SNR between two of our dither seeds |
|---|---|---|
| 2/0 48 kHz 192 kb/s (tones, chirp) | 66.1 dB | 76.0 dB |
| 2/0 44.1 kHz 96 kb/s with coupling and rematrixing | 84.5 dB | 81.7 dB |
| 1/0 32 kHz 96 kb/s | 94.1 dB | 95.7 dB |
| 3/2 + LFE 48 kHz 448 kb/s | 81.4 dB | 90.2 dB |
| 2/0 pink noise 128 kb/s (many zero-bit mantissas) | 27.7 dB | 27.9 dB |
| E-AC-3 2/0 48 kHz 192 kb/s (tones, chirp) | 64.8 dB | 74.7 dB |
| E-AC-3 2/0 44.1 kHz 64 kb/s with coupling and rematrixing | 65.5 dB | 66.1 dB |
| E-AC-3 1/0 32 kHz 64 kb/s | 91.0 dB | 90.1 dB |
| E-AC-3 3/2 + LFE 48 kHz 384 kb/s with mixing / informational metadata | 81.6 dB | 85.3 dB |
| E-AC-3 2/0 pink noise 96 kb/s | 48.6 dB | 47.9 dB |

The noise row shows the difference is dither: two of our own decodes with different dither
sequences differ as much as we differ from FFmpeg. In MPEG-2 TS (AVCHD-style `.mts`) and VOB the
maximum sample difference to FFmpeg is 2.3·10⁻³. E-AC-3 through `cfa4-codecs`
(`tests/eac3_oracle.rs`): 5.1 in MP4 and Matroska 91.3 dB, stereo in QuickTime 87.0 dB and
MPEG-TS 79.5 dB against FFmpeg's decode.

## Encoder

`Encoder::new` writes AC-3 (bsid 8) at a bit rate of Table 5.18 (32-640 kb/s), `new_eac3` E-AC-3
(bsid 16, one independent substream, six-block syncframes) at 32-1024 kb/s; 32 / 44.1 / 48 kHz,
mono, stereo, 3/0, 2/2, 3/2 and 5.1. Written from the same standard, mirroring the decoder:

- Forward MDCT as the exact transpose of the decoder's FFT-based IMDCT (checked against it bin
  by bin), long blocks only.
- Exponents per block, shared across the following blocks while the audible spectrum (bins above
  about -90 dB) moves less than one step per bin on average; the shared set holds each bin's
  smallest exponent. D45 for a set of one block, D25 for two or three, D15 beyond; differences
  limited to ±2. Bandwidth (chbwcod) by the bits per channel.
- Rematrixing (2/0) per band, decided for the whole syncframe.
- The decoder's own parametric bit allocation with the A/52 default parameters; one SNR offset per
  syncframe, the largest whose mantissas fit, found by binary search on counted (not written)
  mantissas; AC-3 keeps blocks 0-1 inside the first 5/8 of the frame.
- Grouped and asymmetric mantissas, both AC-3 CRC words (crc1 solved so the first 5/8 checks to
  zero), the E-AC-3 CRC.
- NaN, infinities and samples beyond full scale are replaced or clamped; a frame whose exponents
  cannot fit falls back to one D45 set per channel.

Tests: every channel mode and rate through our decoder; ffmpeg decodes our streams with
`-err_detect crccheck+explode`, and on the same input and bit rate our SNR matches or beats
ffmpeg's own AC-3 / E-AC-3 encoder (stereo 192 kb/s: 83 vs 51 dB; 5.1 448 kb/s: 75.5 vs 75.5 dB;
stereo 96 kb/s at 44.1 kHz: 61.6 vs 65.5 dB, where ffmpeg couples). Speed
(`cargo run --release -p cfa4-ac3 --example ac3_encode_bench`): 5.1 at about 23× real time on
one thread. Not written yet (#120): coupling, block switching, spectral extension, 7.1, DRC
words, 1+1; export wiring is #119.

No free encoder writes dependent substreams, so the 7.1 tests make them: FFmpeg encodes a 5.1 and
a 2/2 stream, and `make_dependent_substream` (feature `test-util`) turns each 2/2 syncframe into a
dependent substream with a custom channel map (Ls, Rs, Lrs / Rrs): decoding the syncframe records
the bit ranges of the fields only independent substreams carry, which are removed, and crc2 is
recomputed. FFmpeg decodes the result as 7.1 and is the oracle: 72-86 dB per channel in the raw
stream (`src/dependent_tests.rs`), 75.6 dB worst channel in MP4 and Matroska through
`cfa4-codecs`.

## Gaps

- FFmpeg's AC-3 encoder never block-switches, so the 256-sample transforms are implemented from
  §7.9.4.2 but not checked against an oracle.
- No downmixing, no heavy compression (`compr`) or dialogue normalisation (both are metadata
  for playback systems), no CRC check (corrupt frames are caught by consistency checks).
- FFmpeg's E-AC-3 encoder writes six-block syncframes with frame-based exponent strategies,
  coupling with the default band structure and none of the optional per-block syntax. Spectral
  extension (no free encoder writes it), fewer blocks per syncframe, the reduced rates and the
  per-block syntax (shared with AC-3) are implemented from the standard and covered by unit tests
  on synthetic data, not by an oracle.
- E-AC-3: no adaptive hybrid transform, enhanced coupling or transient pre-noise processing;
  Dolby Atmos object audio (JOC, ETSI TS 103 420) is not decoded, its 5.1 / 7.1 bed plays.
- Dependent substreams are joined only when they follow their independent substream in the same
  buffer (MP4, Matroska, MPEG-TS access units, raw streams).
- The chanmap location table and the dependent-substream bsi differences follow A/52:2012
  Annex E; FFmpeg's decode agrees on the synthetic 7.1 stream, but no Dolby-encoded 7.1 file has
  been tested.
