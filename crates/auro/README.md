# cfa4-auro

Auro-3D: detects an Auro-Codec side channel in 24-bit PCM and restores the layout it carries
(8.0, 9.1, 10.1, 11.1, 13.1 …) from the carrier (a 4.0, 5.1 or 7.1 DTS-HD Master Audio, FLAC,
WAV or LPCM track). Issue #299.

## How it works

An Auro-3D encoder folds the height channels into a carrier mix and replaces the lowest 3 to 14
bits of every carrier sample, block by block, with a side channel. Each block (1000 or up to 4096
samples) starts with a sync (bit 0 of sixteen samples set), a CRC-16 over the block, the block size
and the number of borrowed bits. Its payload holds the ADOL configuration (which original layout
was folded into which carrier: `ChannelConfig`), and per carrier channel which streams it folds,
the predictor seeds, the gains, a residual codebook and Golomb-Rice coded indices into it. A
decoder clears the borrowed bits, extrapolates one stream from its own history while deriving the
others from the sum, and lets the residuals correct the extrapolation, so every folded stream
comes back.

Only a bit-exact carrier can be decoded: a lossy decode (a DTS core, AC-3, AAC) has lost the low
bits. CFA4's own DTS-HD Master Audio decoder (`cfa4-dts`) and FLAC/PCM paths are bit-exact.

## Specification

There is no public specification of Auro-Codec. The block layout and the configuration tables
follow the public description in almirus/Orua-D3 (MIT) and MediaInfoLib PR #2531; the residual
layer is harletty-bridge's analysis, checked against a published original/encoded pair. The
mechanism is the one described in US 8,620,465 B2 ("Method and encoder for combining digital data
sets…", Auro Technologies). **Patent:** the project owner decided on 2026-10-27 (#299) to build both
detection and height-channel decoding while the patent is in force, accepting the risk, and noted
that it expires on 2027-10-15 (Google Patents estimates 2030-09-27; not a legal conclusion).

## Origin and licence

`src/` (except `cfa4.rs`) and `tests/synthetic.rs` are the `auro` crate 0.9.0 of
[harletty-bridge](https://github.com/harletty/harletty-bridge) (commit
`74cf6edaef6742777d3eb4da35387fd370f2e9f3`, 2026-10-21, `auro/`), Apache-2.0
(`LICENSE.harletty-bridge`), compatible with CFA4's GPL-3.0-or-later. Every ported file starts
with a notice saying so, followed by its original header. `src/synth.rs` adapts the block writer
of those tests (Apache-2.0). `src/cfa4.rs` and `examples/` are CFA4's own (GPL-3.0-or-later). The
crate is listed in the reused-code register, [`docs/licensing.md`](../../docs/licensing.md) §5.

## Changes made for CFA4

- No panicking calls outside tests (AGENTS.md §0): the `unreachable!` in the detector, slice
  indexing that hostile input could push out of bounds (the payload words, the Rice output, the
  unmix inputs, the stored CRC, the decoded outputs), the `debug_assert!` on the push size, and an
  underflow in `Unfolder::ready` are replaced by checks that reject the block or the call.
  `Unfolder::push` returns `false` for an unknown carrier or an oversized push.
  `decode::DecodeError::Truncated` and `rice::RiceError::Output` are new. The crate denies
  `unwrap`/`expect`/`panic!`/`unreachable!` with clippy lints.
- `cfa4.rs` (new): the interface the rest of CFA4 uses. `detect` / `unfold` / `unfold_with` work
  on planar carriers in CFA4's WAV channel order; `output_order` puts the restored streams in
  CFA4's order, the bed in WAV order, then the heights, then the top:

  | Layout | Channels |
  |---|---|
  | 4.0_4H (8.0) | L R Ls Rs HL HR HLs HRs |
  | 5.1_4H (9.1) | L R C LFE Ls Rs HL HR HLs HRs |
  | 5.1_5H_1T (11.1) | L R C LFE Ls Rs HL HR HC HLs HRs T |
  | 7.1_4H (11.1, 7.1.4) | L R C LFE Lb Rb Ls Rs HL HR HLs HRs |
  | 7.1_5H_1T (13.1) | L R C LFE Lb Rb Ls Rs HL HR HC HLs HRs T |

  `pcm24_from_f32` / `f32_from_pcm24` convert CFA4's `f32` samples losslessly.
- `synth.rs` (new, feature `test-util`): synthetic carriers for tests in other crates.
- Formatting follows the workspace `rustfmt.toml`.

## Where CFA4 uses it

`cfa4_media::auro::wrap` (in `crates/media/src/auro.rs`) runs on every source CFA4 opens: an audio
stream that opens with exact 24-bit PCM and a block sync is checked by the detector, and a carrier
then plays as its restored layout ("DTS + Auro-3D 13.1", 14 channels). Requests are decoded from a
carrier window one block wider on both sides, so random access is exact.

## Tests

- `cargo test -p cfa4-auro`: the ported unit and synthetic tests, the CFA4 interface, and mutation
  fuzz of whole carriers and of the stream layer past the CRC (no panics).
- `cargo test -p cfa4-media auro`: the wrapped source on a synthetic carrier (layout, windows,
  resampling, hostile requests, plain and lossy audio left alone).
- `CFA4_AURO_DIR=/path/to/samples/auro cargo test -p cfa4-codecs --test auro_oracle`: real carriers
  (Amplitude16 DTS-HD MA 13.1 in Matroska, FLAC 8.0 / 9.1 / 11.1 / 13.1 at 48 and 96 kHz, FLAC in
  MP4, and the 9.1 / 11.1 / 13.1 PCM channel tests). Each opens with its restored layout, every
  sample equals what `unfold` restores from ffmpeg's bit-exact decode of the carrier, also on
  random access, and in the channel tests the noise reaches each loudspeaker alone. The samples
  are not in the repository.
- `cargo run -p cfa4-auro --release --example auro_probe -- carrier.s32 <channels> <rate>` prints
  the detection and the level of every restored channel per second.

Not covered: Auro-CX (MP4 `a3ds`, a separate codec, #322); raw `.dts` files and `A_MS/ACM`
Matroska PCM do not open in CFA4 yet (#321, #320).
