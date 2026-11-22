//! Intel Indeo video decoders: Indeo 2 (`RT21`) here, Indeo 3, 4 and 5 in their own modules.
//! Rust re-expressions of FFmpeg n7.1.1 (LGPL-2.1-or-later), see `README.md`.
//!
//! Safe Rust only; damaged data is an error, never a panic.

use thiserror::Error;

pub mod ir2;
mod ir2data;

/// Indeo decoding errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("Indeo: invalid picture size {0}x{1}")]
    Size(u32, u32),
    #[error("Indeo: frame too short or damaged")]
    Invalid,
    #[error("Indeo: unsupported feature: {0}")]
    Unsupported(&'static str),
}

/// The pixel format of a decoded picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PixelFormat {
    /// Planar Y, U, V with chroma at a quarter of the width and height (rounded up).
    #[default]
    Yuv410,
}

/// A decoded picture: tightly packed planes.
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    /// Y, U, V.
    pub planes: Vec<Vec<u8>>,
    /// Whether this was a key (intra) frame.
    pub key: bool,
}

impl Picture {
    /// Width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if i == 0 { (w, h) } else { (w.div_ceil(4), h.div_ceil(4)) }
    }
}
