//! Pure-Rust RealAudio decoders. So far **Cook** (RealAudio G2, `cook`): mono, stereo, joint
//! stereo and multichannel Cook, as stored in RealMedia files (`cfa4-realmedia` deinterleaves
//! the packets). The output follows FFmpeg's `cook` decoder: planar `f32` in `[-1, 1]`, the
//! first two frames dropped (they carry no audio).
//!
//! Codec backlog item CB-14.1. Every number in the stream is treated as hostile: a damaged frame
//! is an error, never a panic.

mod bits;
pub mod cook;
mod mdct;
mod tables;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid RealAudio stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported RealAudio stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
