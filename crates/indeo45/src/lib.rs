//! Intel Indeo Video Interactive 4 (`IV41`) and 5 (`IV50`) decoders: a re-expression in safe Rust
//! of FFmpeg n7.1.1 `libavcodec/ivi.c`, `indeo4.c`, `indeo5.c` (LGPL-2.1-or-later, see
//! `COPYING.LGPL-2.1`; CFA4's changes are GPL-3.0-or-later).
//!
//! Both codecs code pictures as wavelet bands of 8x8 / 4x4 blocks (Haar or slant transform,
//! run/value Huffman codes), with motion compensation and (Indeo 4) B-frames; the output is YUV 4:1:0.

mod bits;
mod data;
mod dsp;
mod ivi;

pub use ivi::{Codec, Decoder, Error, Picture};
