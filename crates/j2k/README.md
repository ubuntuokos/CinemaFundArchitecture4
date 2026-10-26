# cfa4-j2k

Clean-room JPEG 2000 Part 1 decoder in pure Rust: the picture codec of Digital Cinema Packages
(SMPTE ST 429-4) and of JPEG 2000 in MXF / IMF. Layer L0: depends only on `thiserror`. No
`unsafe`, no panics on malformed input. No OpenJPEG, Kakadu or FFmpeg code was consulted; FFmpeg
(its `jpeg2000` encoder and `libopenjpeg`) is used only as an external fixture generator and
test oracle.

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
```

## Decoder

- Any number of tiles and tile-parts, components, sub-sampling, precisions 1-16 bits signed or
  unsigned, image and tile offsets.
- LRCP, RLCP, RPCL, PCRL and CPRL progressions, quality layers, user precincts, SOP / EPH markers,
  tile-specific COD / COC / QCD / QCC.
- MQ decoder and the three coding passes with run-length mode; code-block styles RESET, TERMALL,
  VSC, predictable termination and segmentation symbols.
- Reversible 5/3 in integers (lossless) and irreversible 9/7 in `f32`; RCT and ICT; scalar
  derived and expounded quantization; truncated streams reconstruct at the midpoint.
- Hostile input: every size and count is checked, allocations are capped (`MAX_SAMPLES`,
  code-block count), and errors are returned, never a panic.

Not supported yet (`Error::Unsupported`): the selective arithmetic coding bypass code-block
style, POC progression changes, packed packet headers (PPM / PPT), region of interest (RGN),
Part 2 extensions and HTJ2K (Part 15).

In MXF the decoder runs through `cfa4-codecs` (`j2k.rs`): Y'CbCr, R'G'B' and gray components
become video frames, and the X'Y'Z' of the digital cinema profiles (`Header::digital_cinema`)
is converted to P3-D65 with the DCI transfer.

## Accuracy and tests

`tests/oracle.rs` generates 17 codestreams with ffmpeg:

- **Reversible 5/3, bit-exact to the encoder's input:** gray, RGB with RCT, YUV 4:2:0, gray 16-bit,
  64×48 tiles, all five progressions, SOP / EPH, a JP2 file, odd sizes (crop to non-multiples).
- **Irreversible 9/7, within ±1 of ffmpeg's decode:** YUV 4:2:0, RGB with ICT, quantised `-q:v 10`,
  three quality layers, and a DCI 2K cinema codestream (`libopenjpeg` `cinema2k`, 12-bit X'Y'Z',
  CPRL, a tile-part per component) at over 100 dB against ffmpeg.
- **Damaged streams:** bit flips, truncation and scrambled bytes of every fixture decode to an
  error or a picture, never a panic.

The code-block styles other than the default are implemented from the spec but not yet checked
against an encoder that writes them.

## Speed

Single-threaded, about 1.4 s for a 2K DCI frame on the development machine (release build).
Real-time DCP playback needs code-block parallelism and SIMD wavelets; that is follow-up work.
