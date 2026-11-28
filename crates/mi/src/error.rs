//! The crate's error type: every failure is a value the caller can act on, never a panic.

use thiserror::Error;

/// What can go wrong in the Media Intelligence core.
#[derive(Debug, Error)]
pub enum MiError {
    /// A time anchor with an empty, reversed or negative range, or one that overflows.
    #[error("invalid time anchor: {0}")]
    InvalidAnchor(&'static str),
    /// A hash text that is not `<alg>:<64 hex digits>` of a known algorithm.
    #[error("invalid content hash: {0}")]
    InvalidHash(&'static str),
    /// The reader's length did not match the size the caller declared, or it ended early.
    #[error("media changed while it was read: {0}")]
    SizeMismatch(&'static str),
    /// Reading the media failed.
    #[error("media read failed: {0}")]
    Io(#[from] std::io::Error),
}
