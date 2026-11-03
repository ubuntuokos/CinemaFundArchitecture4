// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-bitstream 1.1.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

//! This crate provides a JPEG XL bitstream reader and container format parser.
//!
//! # Bitstream reader
//!
//! [`Bitstream`] reads all the raw bits needed to decode JPEG XL codestream. It provides methods
//! to read data types that appear on the JPEG XL specification.
//!
//! # Container parser
//!
//! [`ContainerParser`] tries to parse the bytes fed into it, and emits various parser events
//! including codestream data and auxiliary box data.

#[allow(clippy::module_inception)]
mod bitstream;
pub mod consts;
pub mod container;
mod error;

pub use bitstream::{Bitstream, U, U32Specifier};
pub use container::{BitstreamKind, ContainerParser, ParseEvent};
pub use error::{BitstreamResult, Error};

/// Perform `UnpackSigned` for `u32`, as specified in the JPEG XL specification.
#[inline]
pub fn unpack_signed(x: u32) -> i32 {
    let bit = x & 1;
    let base = x >> 1;
    let flip = 0u32.wrapping_sub(bit);
    (base ^ flip) as i32
}

/// Perform `UnpackSigned` for `u64`, as specified in the JPEG XL specification.
#[inline]
pub fn unpack_signed_u64(x: u64) -> i64 {
    let bit = x & 1;
    let base = x >> 1;
    let flip = 0u64.wrapping_sub(bit);
    (base ^ flip) as i64
}
