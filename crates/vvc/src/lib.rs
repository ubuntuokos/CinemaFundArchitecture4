//! Clean-room H.266/VVC bitstream parsing, implemented from ITU-T Rec. H.266 (ISO/IEC 23090-3).
//!
//! This is the Rust side of the VVC integration (ADR 0013, plan VVC-02): it finds NAL units and
//! access units, tells random-access and non-reference pictures apart, and reads the leading
//! fields of the sequence parameter set (profile, level, chroma format, size, bit depth). It
//! decodes no pictures: decoding runs through the VVdeC helper or FFmpeg (ADR 0013). It is also
//! the first step of the native Rust decoder (plan B line).
//!
//! Every function takes untrusted input: nothing here may panic, and every loop is bounded by the
//! input size.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod au;
pub mod nal;
pub mod sps;

pub use au::{AccessUnitInfo, access_units, annexb_access_units, au_info};
pub use nal::{NalHeader, NalType};
pub use sps::Sps;

/// Errors from parsing a VVC bitstream.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("bitstream: {0}")]
    Bits(#[from] cfa4_bitstream::BitError),
    #[error("unsupported VVC stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Annex B start-code framing of `nals` (four-byte start codes), for feeding a decoder that
/// expects a byte stream with samples that were length-prefixed in a container.
pub fn to_annexb<'a>(nals: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    for n in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(n);
    }
    out
}
