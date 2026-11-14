//! Pure-Rust RealAudio decoders: **Cook** (RealAudio G2, `cook`; mono, stereo, joint stereo and
//! multichannel) and **RealAudio 1.0** (14.4, `lpcJ`, [`ra144`]), as stored in RealMedia files
//! (`cfa4-realmedia` deinterleaves the packets).
//!
//! Cook follows FFmpeg's `cook` decoder (planar `f32` in `[-1, 1]`, the first two frames dropped
//! because they carry no audio); RealAudio 1.0 gives FFmpeg's 16-bit samples bit for bit.
//!
//! Codec backlog items CB-14.1 and CB-14.2. Every number in the stream is treated as hostile: a
//! damaged frame is an error, never a panic.

mod bits;
mod celp;
pub mod cook;
mod mdct;
pub mod ra144;
pub mod ra288;
pub mod sipr;
mod tables;
mod tables144;
mod tables288;
mod tables_sipr;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid RealAudio stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported RealAudio stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
