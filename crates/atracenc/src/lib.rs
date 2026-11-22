//! Pure-Rust ATRAC encoders for CFA4's AT3 converter (Tools ▸ Audio Tools): **ATRAC3**
//! ([`atrac3`]; PSP / PS3 `.at3`, MiniDisc LP2 / LP4) and the AT3 RIFF WAVE writer ([`riff`]).
//! A port of atracdenc 0.2.3 (Daniil Cherednik, LGPL-2.1-or-later); see the README.
//!
//! It is an L0 crate: it depends only on `thiserror`. Every input is treated as hostile: samples
//! outside [-1, 1] and non-finite samples are clamped or silenced, never a panic.

pub mod atrac3;
mod bits;
mod mdct;
mod psy;
mod qmf;
pub mod riff;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("unsupported ATRAC encoding: {0}")]
    Unsupported(&'static str),
    #[error("the AT3 file would be too large ({0} bytes; RIFF holds at most 4 GiB)")]
    TooLarge(u64),
}

pub type Result<T> = std::result::Result<T, Error>;
