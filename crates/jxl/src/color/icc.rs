// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

//! Functions related to ICC profiles.
//!
//! - [`read_icc`] and [`decode_icc`] can be used to read embedded ICC profile from the bitstream.
//! - [`colour_encoding_to_icc`] can be used to create an ICC profile to embed into the decoded
//!   image file, or to be used by the color management system for various purposes.

mod decode;
mod parse;
mod synthesize;

pub use decode::{decode_icc, read_icc};
pub use parse::icc_tf;
pub(crate) use parse::parse_icc;
pub(crate) use parse::parse_icc_raw;
pub use synthesize::colour_encoding_to_icc;

#[derive(Debug)]
#[non_exhaustive]
pub struct IccHeader {
    pub color_space: [u8; 4],
    pub rendering_intent: crate::color::RenderingIntent,
}

#[derive(Debug)]
struct IccTag {
    tag: [u8; 4],
    data_offset: u32,
    len: u32,
}
