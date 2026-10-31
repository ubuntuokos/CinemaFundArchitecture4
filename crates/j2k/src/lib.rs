//! Clean-room JPEG 2000 Part 1 decoder (ITU-T T.800 | ISO/IEC 15444-1), the codec of Digital
//! Cinema Packages and of JPEG 2000 in MXF.
//!
//! - Codestreams (`.j2k`, `.j2c`) and JP2 files (`.jp2`, the `jp2c` box).
//! - Any number of tiles, tile-parts and components, component sub-sampling, precisions up to 16
//!   bits (signed or unsigned), image and tile offsets.
//! - All five progression orders (LRCP, RLCP, RPCL, PCRL, CPRL), quality layers, precincts,
//!   SOP / EPH markers, tile-specific COD / COC / QCD / QCC.
//! - EBCOT tier-1 decoding with the MQ arithmetic decoder and every code-block style (selective
//!   arithmetic coding bypass, RESET, TERMALL, vertically causal context, predictable
//!   termination, segmentation symbols).
//! - POC progression changes, packed packet headers (PPM / PPT) and Maxshift region of interest
//!   (RGN).
//! - Reversible 5/3 (bit-exact, lossless) and irreversible 9/7 wavelets, the reversible and
//!   irreversible multiple-component transforms, scalar derived and expounded quantization.
//!
//! Not supported (an [`Error::Unsupported`]): the Part 2 extensions (other ROI styles, wavelet
//! kernels), mixed HT / Part 1 code-blocks.
//!
//! [`encode`] writes Part 1 codestreams: lossless 5/3 or rate-controlled 9/7, and the Digital
//! Cinema 2K / 4K profiles ([`EncodeOptions::cinema_2k`], [`EncodeOptions::cinema_4k`]).
//! See the crate README for the specification and accuracy.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod codestream;
mod decode;
mod dwt;
mod encode;
mod ht;
mod ht_tables;
mod jp2;
mod mq;
mod t1;
mod t2;

pub use codestream::{ComponentInfo, Header, parse_header};
pub use decode::decode;
pub use encode::{EncodeOptions, Profile, Progression, encode};

/// Decoder errors. Malformed input gives an error, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated JPEG 2000 data")]
    Truncated,
    #[error("invalid JPEG 2000 data: {0}")]
    Invalid(&'static str),
    #[error("unsupported JPEG 2000 feature: {0}")]
    Unsupported(&'static str),
    #[error("JPEG 2000 image too large")]
    TooLarge,
}

pub type Result<T> = std::result::Result<T, Error>;

/// One decoded component, at its own (possibly sub-sampled) resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub width: u32,
    pub height: u32,
    /// Bits per sample (1-16).
    pub precision: u8,
    pub signed: bool,
    /// Sub-sampling against the reference grid (XRsiz, YRsiz).
    pub dx: u8,
    pub dy: u8,
    /// Samples in raster order, in the component's range (0 ..= 2^precision - 1, or the signed
    /// range).
    pub data: Vec<i32>,
}

/// A decoded image: one [`Plane`] per component, in codestream order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Image area on the reference grid (Xsiz - XOsiz, Ysiz - YOsiz).
    pub width: u32,
    pub height: u32,
    pub components: Vec<Plane>,
    /// A tile applied the inverse multiple-component transform (its COD, main or tile-part, set
    /// it): the first three components are R, G, B even when the main header says otherwise.
    pub mct: bool,
}

/// Samples an image may have across all components; larger images are refused before any
/// allocation ([`Error::TooLarge`]). 2^28 covers 8K 4:4:4 with alpha.
pub const MAX_SAMPLES: u64 = 1 << 28;

/// The codestream inside `data`: a raw codestream starts with SOC (FF 4F FF 51); a JP2 file
/// gives its `jp2c` box.
pub fn codestream(data: &[u8]) -> Result<&[u8]> {
    if data.starts_with(&[0xFF, 0x4F]) {
        return Ok(data);
    }
    jp2::codestream(data)
}

#[cfg(test)]
mod tests;
