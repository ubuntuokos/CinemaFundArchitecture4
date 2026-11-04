# cfa4-jxl

JPEG XL (ISO/IEC 18181) still-image decoder, layer L0.

## Origin and licence

This crate is a port of [jxl-oxide](https://github.com/tirr-c/jxl-oxide) by Wonwoo Choi and the
jxl-oxide contributors (MIT OR Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`). Its crates
(jxl-oxide, jxl-render, jxl-frame, jxl-image, jxl-modular, jxl-vardct, jxl-bitstream, jxl-coding,
jxl-grid, jxl-color, jxl-oxide-common, jxl-threadpool) are merged into modules here; every ported
file keeps a provenance header. CFA4's changes are GPL-3.0-or-later and are registered in
[`docs/licensing.md`](../../docs/licensing.md) §5.

CFA4 changes:

- **No `unsafe`.** The aliasing `MutableSubgrid` became a list of row slices with hidden storage
  for Squeeze residuals (`grid/mutable_subgrid.rs`); SIMD paths use the generic code; the thread
  pool uses safe rayon scopes (feature `threads`).
- **No panics on input.** Every `unwrap`/`expect`/`panic!`/`unreachable!` on data from the file is
  an error; slice indexing on hostile numbers uses `get`; allocations are capped
  (`MAX_GRID_SAMPLES`, `cfa4::MAX_SAMPLES`). `cfa4::decode` additionally runs under
  `catch_unwind`.
- No lcms2/moxcms: colour output is the file's own enum colour encoding.

## Interface

`cfa4::sniff(bytes)` and `cfa4::decode(bytes) -> Picture` (first frame, interleaved `f32`
samples, channel count, bit depth, CICP code points). `crates/media/src/still/jxl.rs` turns it
into frames.

## Tests

- `tests/oracle.rs`: 17 `cjxl` files (VarDCT at several distances, progressive, resampled, EPF
  without Gabor, photon noise, patches, alpha, grey, 16-bit; Modular lossy and lossless at efforts
  1–9, grey + alpha) against `djxl` (Ubuntu package `libjxl-tools`); lossless is exact.
- `tests/fuzz.rs`: truncated and mutated files must return errors, never panic
  (`CFA4_JXL_FUZZ_ITERS`, default 1500 per file; 20 000 ran clean).

## Not yet

Animation (#138), ICC-profile colour and CMYK (#139), JPEG reconstruction and JXL in DNG (#140),
encoder (#141), speed and the remaining internal `assert!`s (#142).
