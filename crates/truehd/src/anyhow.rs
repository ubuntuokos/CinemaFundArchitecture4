//! CFA4: a minimal stand-in for the `anyhow` crate the upstream code used (`Error`, `Result`,
//! `anyhow!`, `bail!`, `ensure!`), so the crate has no dependency beyond `thiserror` and `log`.

use std::fmt;

type Inner = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A decoding error: the original error, or a message.
pub struct Error {
    inner: Inner,
}

#[derive(Debug)]
struct Message(String);

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Message {}

impl Error {
    pub fn msg(message: impl fmt::Display) -> Self {
        Self { inner: Box::new(Message(message.to_string())) }
    }

    /// The original error, when it has type `E`.
    pub fn downcast_ref<E: std::error::Error + 'static>(&self) -> Option<&E> {
        self.inner.downcast_ref::<E>()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl<E: std::error::Error + Send + Sync + 'static> From<E> for Error {
    fn from(error: E) -> Self {
        Self { inner: Box::new(error) }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

macro_rules! anyhow {
    ($msg:literal $(,)?) => {
        $crate::anyhow::Error::msg(format!($msg))
    };
    ($err:expr $(,)?) => {
        $crate::anyhow::Error::from($err)
    };
    ($fmt:literal, $($arg:tt)*) => {
        $crate::anyhow::Error::msg(format!($fmt, $($arg)*))
    };
}

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::anyhow::anyhow!($($arg)*))
    };
}

macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            $crate::anyhow::bail!($($arg)*);
        }
    };
}

#[allow(unused_imports)]
pub(crate) use {anyhow, bail, ensure};
