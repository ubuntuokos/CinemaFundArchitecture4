/// Errors produced by the decoder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A section, a size or a compressed stream did not fit the data.
    #[error("invalid bitstream: {0}")]
    Invalid(String),
    /// A feature this decoder does not implement (BC7, BC6H …).
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// `ensure!(cond, "message")` returns `Error::Invalid` when `cond` is false.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err($crate::error::Error::Invalid(format!($($arg)+)));
        }
    };
}
pub(crate) use ensure;
