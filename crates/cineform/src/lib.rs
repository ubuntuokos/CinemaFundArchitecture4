//! Pure-Rust GoPro CineForm (CFHD) decoder.
//!
//! CineForm is a wavelet intermediate codec: in an intra sample every channel is a three-level
//! 2/6 spatial wavelet pyramid whose highpass bands are run-length / variable-length coded. The
//! bitstream is a sequence of 32-bit tag/value pairs.
//!
//! The decoder follows the GoPro CineForm SDK (Apache-2.0 / MIT, see `docs/licensing.md` §5),
//! re-expressed in safe Rust: the tag syntax, the run/value codebooks, the companding curves
//! and the inverse wavelet filters match the SDK. Output is verified bit-exact against ffmpeg's
//! `cfhd` decoder on streams from ffmpeg's encoder.
//!
//! Supported: intra samples (one frame per sample, the form every encoder writes for
//! editing) in 4:2:2 YUV at 10 bits, 4:4:4 RGB and 4:4:4:4 RGBA at 12 bits, 16-bit and
//! run-length band encodings with all three codebooks, peak tables and solid lowpass bands.
//! The SDK and ffmpeg decoders round prescaled levels differently and treat alpha differently;
//! [`decode_frame`] picks the behaviour of the encoder that wrote the sample ([`Flavor`]).
//!
//! See the crate README for the limitations.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod decode;
mod tables;
mod tags;
mod vlc;
mod wavelet;

pub use decode::{decode_frame, decode_frame_as, probe};

/// Codec errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated CineForm data")]
    Truncated,
    #[error("invalid CineForm data: {0}")]
    Invalid(&'static str),
    #[error("unsupported CineForm feature: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Internal representation of the encoded image (`CODEC_TAG_ENCODED_FORMAT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncodedFormat {
    /// Y, Cb, Cr channels, chroma at half width.
    Yuv422,
    /// Bayer sensor data (four quarter-size channels).
    Bayer,
    /// G, R, B channels at full size.
    Rgb444,
    /// G, R, B, A channels at full size.
    Rgba4444,
    /// Y, Cb, Cr, A channels at full size.
    Yuva4444,
}

impl EncodedFormat {
    fn from_tag(v: u16) -> Result<EncodedFormat> {
        Ok(match v {
            0 | 1 => EncodedFormat::Yuv422,
            2 => EncodedFormat::Bayer,
            3 => EncodedFormat::Rgb444,
            4 => EncodedFormat::Rgba4444,
            5 => EncodedFormat::Yuva4444,
            _ => return Err(Error::Invalid("unknown encoded format")),
        })
    }

    /// Number of coded channels.
    pub fn channels(self) -> usize {
        match self {
            EncodedFormat::Yuv422 | EncodedFormat::Rgb444 => 3,
            EncodedFormat::Bayer | EncodedFormat::Rgba4444 | EncodedFormat::Yuva4444 => 4,
        }
    }
}

/// Stream parameters from the sample header (see [`probe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels (a multiple of 8; see `display_height`).
    pub height: u32,
    /// Height to show (≤ `height`); equal to `height` when not signalled.
    pub display_height: u32,
    pub format: EncodedFormat,
    /// Source precision in bits (8, 10 or 12).
    pub precision: u8,
    /// Number of coded channels.
    pub channels: u8,
    /// `true` for an intra (single-frame) sample, `false` for the frames of a temporal group.
    pub intra: bool,
    /// Version of the CineForm SDK that wrote the sample (`major << 12 | minor << 8 | revision`),
    /// absent in streams from other encoders.
    pub encoder_version: Option<u16>,
}

impl Header {
    /// The decoder flavour matching the encoder: [`Flavor::Sdk`] for SDK-written samples (they
    /// carry a version tag), [`Flavor::Ffmpeg`] otherwise.
    pub fn flavor(&self) -> Flavor {
        if self.encoder_version.is_some() { Flavor::Sdk } else { Flavor::Ffmpeg }
    }
}

/// The two CineForm decoders differ in two details; output matches the chosen one bit for bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flavor {
    /// The GoPro CineForm SDK: prescaled wavelet levels keep their lowest bit, and RGBA alpha is
    /// stored with the SDK's alpha curve (`16..=239` scaled into 12 bits), which is removed.
    Sdk,
    /// ffmpeg's `cfhd`: prescaled levels drop their lowest bit, alpha is stored linearly.
    Ffmpeg,
}

/// One decoded plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    /// Row-major samples of `bit_depth` significant bits.
    pub data: Vec<u16>,
}

/// A decoded frame. Planes are Y, Cb, Cr (chroma at half width) for 4:2:2 and R, G, B (and
/// full-range A) for RGB (the coded channel orders are Y, Cr, Cb and G, R, B, A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Display width.
    pub width: u32,
    /// Display height.
    pub height: u32,
    pub format: EncodedFormat,
    /// Significant bits per sample of every plane.
    pub bit_depth: u8,
    pub planes: Vec<Plane>,
}
