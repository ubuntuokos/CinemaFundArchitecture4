//! Clean-room pure-Rust FFV1 decoder, implemented from RFC 9043 ("FFV1 Video Coding Format
//! Versions 0, 1, and 3").
//!
//! Supported: versions 0, 1 and 3; Golomb Rice and range coding with the default or a custom
//! state transition table; YCbCr with or without chroma planes, any chroma subsampling and a
//! transparency plane; RGB through the JPEG 2000 RCT with or without transparency; 8-16 bits per
//! sample; slices, non-key frames (context states carried over), coded initial states and the
//! slice and configuration record CRCs. Version 2 (experimental) is rejected.
//!
//! ```no_run
//! let config: Vec<u8> = Vec::new(); // the container's codec private data (version 3)
//! let mut dec = cfa4_ffv1::Decoder::new(1920, 1080, &config).unwrap();
//! let frame: Vec<u8> = Vec::new(); // one container sample
//! let pic = dec.decode(&frame).unwrap();
//! println!("{}x{} {} bits", pic.width, pic.height, pic.bits);
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod bits;
mod decoder;
mod error;
mod header;
mod range;
mod tables;

pub use decoder::{Decoder, is_keyframe};
pub use error::{Error, Result};

/// How the planes of a [`Picture`] are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Luma only (`chroma_planes == 0`).
    Gray,
    /// Y, Cb, Cr; chroma planes are `ceil(w / 2^log2_h)` x `ceil(h / 2^log2_v)`.
    Yuv { log2_h: u32, log2_v: u32 },
    /// R, G, B (after the reversible colour transform), all full size.
    Rgb,
}

/// A decoded frame: planes tight (stride = plane width), samples in the low `bits` bits.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bits: u32,
    pub format: Format,
    /// Y (, Cb, Cr) or R, G, B.
    pub planes: Vec<Vec<u16>>,
    /// The transparency plane (full size), when the stream has one.
    pub alpha: Option<Vec<u16>>,
    pub key: bool,
    /// Sample aspect ratio from the slice header (version 3), when known.
    pub sar: Option<(u32, u32)>,
    /// `picture_structure` (0 unknown, 1 top field first, 2 bottom field first, 3 progressive).
    pub picture_structure: u8,
}
