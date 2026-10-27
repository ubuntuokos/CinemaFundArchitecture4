# cfa4-j2k

Clean-room JPEG 2000 Part 1 and HTJ2K (Part 15) decoder, and a Part 1 encoder with the Digital
Cinema profiles, in pure Rust: the picture codec of Digital Cinema Packages
(SMPTE ST 429-4) and of JPEG 2000 in MXF / IMF. Layer L0: depends only on `thiserror`. No
`unsafe`, no panics on malformed input. No OpenJPEG, Kakadu or FFmpeg code was consulted for
Part 1. The HT block decoder (`src/ht.rs`, `src/ht_tables.rs`) is ported from OpenJPH
(BSD-2-Clause, `src/LICENSE-OpenJPH`, register in `docs/licensing.md` §5). FFmpeg (its `jpeg2000`
encoder and `libopenjpeg`), OpenJPEG and Grok are used only as external fixture generators and
test oracles.

## Specification

ITU-T T.800 | ISO/IEC 15444-1, the freely available ITU-T text: Annex A codestream
syntax, Annex B image / tile / precinct / code-block geometry, progression orders, packet headers
and tag trees, Annex C the MQ arithmetic decoder (Table C.2 state table), Annex D EBCOT
coefficient bit modelling (Tables D.1-D.3 contexts, D.7 vertically causal mode, D.4 code-block
styles), Annex E quantization, Annex F the inverse DWT (5/3 and 9/7 lifting with symmetric
extension, Table F.4), Annex G DC level shift and the RCT / ICT component transforms, Annex I
the JP2 file format.

## API

```rust
let img = cfa4_j2k::decode(&bytes)?;          // codestream (.j2k / .j2c) or JP2 file
for p in &img.components {
    // p.width × p.height samples in p.data, p.precision bits, sub-sampled by p.dx × p.dy
}
let h = cfa4_j2k::parse_header(cfa4_j2k::codestream(&bytes)?)?; // size, components, no decode

let j2c = cfa4_j2k::encode(&img, &cfa4_j2k::EncodeOptions::lossless())?;   // 5/3, exact
let dcp = cfa4_j2k::encode(&xyz12, &cfa4_j2k::EncodeOptions::cinema_2k(24))?; // DCI 2K frame
```

## Decoder

- Any number of tiles and tile-parts, components, sub-sampling, precisions 1-16 bits signed or
  unsigned, image and tile offsets.
- LRCP, RLCP, RPCL, PCRL and CPRL progressions and POC progression changes (main and tile
  headers), quality layers, user precincts, SOP / EPH markers, tile-specific COD / COC / QCD / QCC,
  packed packet headers in PPM (main header) and PPT (tile-part headers) markers.
- MQ decoder and the three coding passes with run-length mode; every code-block style: selective
  arithmetic coding bypass (raw segments), RESET, TERMALL, VSC, predictable termination and
  segmentation symbols.
- Region of interest by the Maxshift method (RGN, main and tile headers).
- HTJ2K (ITU-T T.814 | ISO/IEC 15444-15, code-block style 0x40): the HT cleanup pass (MEL,
  VLC / UVLC, MagSgn), the significance propagation and magnitude refinement passes, placeholder
  passes; HT packet headers (cleanup and refinement segment lengths). The CAP marker needs no
  handling. Ported to safe Rust from OpenJPH 0.28.1 (`ojph_block_decoder32.cpp`,
  `ojph_block_common.cpp`, `table0.h`, `table1.h`) with byte-wise readers.
- Reversible 5/3 in integers (lossless) and irreversible 9/7 in `f32`; RCT and ICT; scalar
  derived and expounded quantization; truncated streams reconstruct at the midpoint.
- Hostile input: every size and count is checked, allocations are capped (`MAX_SAMPLES`,
  code-block count), and errors are returned, never a panic.

Not supported yet (`Error::Unsupported`): Part 2 extensions (other ROI styles, arbitrary wavelet
kernels), mixed HT / Part 1 code-blocks (style 0x80) and HT code-blocks that receive passes in
more than one quality layer.

## Encoder

`encode(&Image, &EncodeOptions)` writes a codestream (`.j2c`): one tile, one quality layer, Part 1
code-blocks (style 0). Clean-room from T.800 and its DCI amendment; it reuses the decoder's
geometry, packet order and step-size code by parsing the header it has just written, so encoder
and decoder cannot disagree on them.

- DC level shift, RCT / ICT, forward 5/3 (integer) and 9/7 (`f32`) lifting, the exact inverses
  of the decoder's synthesis.
- Reversible: no quantization, lossless. Irreversible: dead-zone scalar quantization with
  expounded step sizes, the image step divided by each subband's synthesis norm (measured from
  the decoder's own synthesis of an impulse).
- Tier-1: the MQ encoder (C.2) and the three coding passes, with the exact truncation length of
  every pass (the bits the decoder needs down to the interval's precision) and the distortion
  it removes, measured at the decoder's midpoint reconstruction.
- Rate control (PCRD): each code-block's passes on their convex hull; a slope threshold found by
  binary search over the hull slopes against the frame limit, and first a per-component floor
  against the component limit. Sizes are measured by writing the packet headers, not estimated.
- Tier-2: tag trees, Lblock, Table B.4 pass counts, bit stuffing; any progression and precincts;
  one tile-part per component (and per progression change); TLM.
- Guard bits are chosen so that every subband's largest index fits; samples outside a
  component's range are clamped.
- `EncodeOptions::cinema_2k(fps)` / `cinema_4k(fps)`: Rsiz 3 / 4, 12-bit X'Y'Z' (the caller
  converts), 9/7, 5 / 6 levels, 32×32 code-blocks, CPRL, precincts 128 at resolution 0 and 256
  above, a tile-part per component (4K: the 4K resolution in its own progression by POC, so six
  tile-parts), TLM; at most 250 Mbit/s per frame and 200 Mbit/s per component (1 302 083 and
  1 041 666 bytes at 24 fps). The image and options are checked against the profile.

Not yet (#117): several quality layers, tiles, HTJ2K encoding, other code-block styles, SOP /
EPH / PLT, IMF and broadcast profiles. DCP / MXF export is #115, speed #116.

## Accuracy and tests

`tests/oracle.rs` generates 17 codestreams with ffmpeg:

- **Reversible 5/3, bit-exact to the encoder's input:** gray, RGB with RCT, YUV 4:2:0, gray 16-bit,
  64×48 tiles, all five progressions, SOP / EPH, a JP2 file, odd sizes (crop to non-multiples).
- **Irreversible 9/7, within ±1 of ffmpeg's decode:** YUV 4:2:0, RGB with ICT, quantised `-q:v 10`,
  three quality layers, and a DCI 2K cinema codestream (`libopenjpeg` `cinema2k`, 12-bit X'Y'Z',
  CPRL, a tile-part per component) at over 100 dB against ffmpeg.
- **Damaged streams:** bit flips, truncation and scrambled bytes of every fixture decode to an
  error or a picture, never a panic.

- **Code-block styles, POC and ROI (OpenJPEG, when `opj_compress` is installed):** bypass, RESET,
  TERMALL, VSC, predictable termination, segmentation symbols and combinations of them, POC
  progression changes (also per tile) and Maxshift ROI, 5/3 bit-exact to the input and 9/7 within
  ±1 of `opj_decompress`. Skipped where the tools are missing.
- **HTJ2K (Grok `grk_compress -M 64` and OpenJPEG 2.5's decoder, when installed):** 5/3 bit-exact
  to the input with 64×64, 32×16 and 4×4 code-blocks, tiles with RPCL, precincts, VSC; 9/7 and
  rate-limited streams within ±1 (5/3: exactly) of `opj_decompress`; damaged copies never panic.
  Grok writes cleanup passes only, so the refinement passes are checked on samples dropped into
  `target/fixtures/j2k/ht-samples/` (OpenJPH's `test.j2c`: ±1 against OpenJPEG; leaving out the
  SigProp / MagRef passes makes it fail by 25 / 5).
- **Packed headers:** an ffmpeg tiled, layered stream rewritten with its packet headers in PPM and
  in PPT markers decodes to the same picture; truncated and damaged copies never panic.

- **Encoder (`src/encode/tests.rs`, `tests/oracle.rs`):** lossless round trips exact for 1-4
  components, 1-16 bits, signed, 4:2:0 sub-sampling, 0-7 levels, all progressions and precincts;
  every truncation point of every code-block decodes to the distortion the encoder counted;
  byte limits per frame and per component hold; DCI 2K / 4K structure (Rsiz, POC, tile-parts,
  TLM). OpenJPEG (`opj_decompress`) and ffmpeg's decoder read our streams: lossless ones exactly,
  lossy, rate-limited and DCI ones within ±1 of our decode.

## Speed

`cargo run --release -p cfa4-j2k --example j2k_bench -- FILE…` times decodes. On the 4-core
development machine, a 2K DCI frame (`dci_2k.j2k`, 586 kB) decodes in about 150 ms
(about 360 ms on one thread; it was 1.16 s before #78):

- **Tier-1:** each sample's flags carry its neighbours' significance and signs, so the zero and
  sign coding contexts are table lookups. A per-stripe-column summary lets the passes skip
  columns with nothing to code. The MQ decoder keeps state and MPS in one table index.
- **Wavelet:** rows are lifted on their low- and high-pass halves before interleaving, and
  columns as whole-row operations in 64-column strips.
- **Threads** (the default `threads` feature, rayon): code-blocks, components, wavelet rows and
  strips, and the output planes run in parallel.

Players also decode several frames at once, since every picture is intra coded.

`cargo run --release -p cfa4-j2k --example j2k_encode_bench -- 2k|4k` times the encoder on a
synthetic 12-bit frame with grain: a DCI 2K frame takes about 0.75 s, a 4K frame about 3 s on
the same machine, filled to the byte limit (most of it tier-1; #116).
