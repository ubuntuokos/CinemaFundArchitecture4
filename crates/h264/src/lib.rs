//! Clean-room pure-Rust H.264 / AVC decoder, implemented from ITU-T Rec. H.264 (ISO/IEC 14496-10).
//!
//! Supported: progressive (frame) coding, 8-bit 4:2:0, Baseline / Main / High profiles — CAVLC and CABAC,
//! I/P/B slices, 8x8 transform, custom scaling matrices, weighted prediction, spatial/temporal direct,
//! multiple slices, long-term references and all MMCOs, deblocking.
//!
//! ```no_run
//! let mut dec = cfa4_h264::Decoder::new();
//! let stream = std::fs::read("video.h264").unwrap();
//! for pic in dec.decode(&stream, 0).unwrap() {
//!     println!("{}x{} poc {}", pic.width, pic.height, pic.poc);
//! }
//! for pic in dec.flush() {
//!     let _ = pic;
//! }
//! ```

// Index loops over fixed-size blocks read more clearly than iterator chains in codec code.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod cabac;
mod cabac_mb;
#[rustfmt::skip]
mod cabac_tables;
mod cavlc;
#[rustfmt::skip]
mod cavlc_tables;
mod deblock;
mod decoder;
pub mod dpb;
mod error;
mod inter;
mod intra;
mod mbtypes;
pub mod params;
mod picture;
pub use picture::set_plane_allocator;
pub mod slice;
mod slicedec;
#[cfg(test)]
mod synth_tests;
mod tables;
mod transform;

pub use decoder::{DecodeStats, Decoder, default_threads};
pub use error::{Error, Result};

/// Colour description from the VUI (ITU-T H.273 code points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorInfo {
    /// `video_full_range_flag`.
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

/// One decoded plane: `U8` for 8-bit pictures, `U16` for High 10 and above (values within
/// `0..1 << bit_depth`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plane {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl Plane {
    pub fn len(&self) -> usize {
        match self {
            Plane::U8(v) => v.len(),
            Plane::U16(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The samples when the plane is 8-bit.
    pub fn as_u8(&self) -> Option<&[u8]> {
        match self {
            Plane::U8(v) => Some(v),
            Plane::U16(_) => None,
        }
    }
    /// The samples when the plane is 16-bit.
    pub fn as_u16(&self) -> Option<&[u16]> {
        match self {
            Plane::U16(v) => Some(v),
            Plane::U8(_) => None,
        }
    }
    /// The samples as raw bytes: 8-bit planes as they are, deeper ones little-endian (the layout
    /// ffmpeg calls `yuv420p10le`).
    pub fn to_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        match self {
            Plane::U8(v) => std::borrow::Cow::Borrowed(v),
            Plane::U16(v) => std::borrow::Cow::Owned(v.iter().flat_map(|s| s.to_le_bytes()).collect()),
        }
    }
    /// Sample `i`, whatever the storage.
    #[inline]
    pub fn get(&self, i: usize) -> u16 {
        match self {
            Plane::U8(v) => v.get(i).copied().unwrap_or(0).into(),
            Plane::U16(v) => v.get(i).copied().unwrap_or(0),
        }
    }
}

/// A decoded, cropped picture in planar 4:2:0 of [`Picture::bit_depth`] bits.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Cropped luma width.
    pub width: u32,
    /// Cropped luma height.
    pub height: u32,
    pub chroma_width: u32,
    pub chroma_height: u32,
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
    pub y_stride: usize,
    pub uv_stride: usize,
    /// Bits per sample (8, or 9..=14 for High 10 and above).
    pub bit_depth: u8,
    /// Presentation timestamp passed to [`Decoder::decode`] with the access unit of this picture.
    pub pts: i64,
    /// Picture order count.
    pub poc: i32,
    /// IDR picture.
    pub key: bool,
    pub color: ColorInfo,
    /// Sample aspect ratio (0, 0 when unspecified).
    pub sar: (u16, u16),
    /// Decoded in draft mode ([`Decoder::set_draft`]): a non-reference picture whose deblocking
    /// filter was skipped. Its samples are approximate (not the conforming output); no other
    /// picture is affected.
    pub draft: bool,
}
