// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-render 0.12.4.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Bitstream(crate::bitstream::Error),
    Decoder(crate::coding::Error),
    Buffer(crate::grid::OutOfMemory),
    Modular(crate::modular::Error),
    Frame(crate::frame::Error),
    Color(crate::color::Error),
    IncompleteFrame,
    FailedReference,
    UninitializedLfFrame(u32),
    InvalidReference(u32),
    NotReady,
    NotSupported(&'static str),
    /// CFA4: an internal invariant did not hold (a corrupt file the port did not anticipate);
    /// returned instead of panicking.
    Internal(&'static str),
}

impl From<crate::bitstream::Error> for Error {
    fn from(err: crate::bitstream::Error) -> Self {
        Self::Bitstream(err)
    }
}

impl From<crate::coding::Error> for Error {
    fn from(err: crate::coding::Error) -> Self {
        Self::Decoder(err)
    }
}

impl From<crate::grid::OutOfMemory> for Error {
    fn from(err: crate::grid::OutOfMemory) -> Self {
        Self::Buffer(err)
    }
}

impl From<crate::modular::Error> for Error {
    fn from(err: crate::modular::Error) -> Self {
        Self::Modular(err)
    }
}

impl From<crate::frame::Error> for Error {
    fn from(err: crate::frame::Error) -> Self {
        Self::Frame(err)
    }
}

impl From<crate::color::Error> for Error {
    fn from(err: crate::color::Error) -> Self {
        Self::Color(err)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Error::*;

        match self {
            Bitstream(err) => write!(f, "bitstream error: {err}"),
            Decoder(err) => write!(f, "entropy decoder error: {err}"),
            Buffer(err) => write!(f, "{err}"),
            Modular(err) => write!(f, "modular subimage decode error: {err}"),
            Frame(err) => write!(f, "frame error: {err}"),
            Color(err) => write!(f, "color management error: {err}"),
            IncompleteFrame => write!(f, "frame data is incomplete"),
            FailedReference => write!(f, "reference frame failed to render"),
            UninitializedLfFrame(lf_level) => {
                write!(f, "uninitialized LF frame for level {lf_level}")
            }
            InvalidReference(idx) => write!(f, "invalid reference {idx}"),
            NotReady => write!(f, "image is not ready to be rendered"),
            NotSupported(msg) => write!(f, "not supported: {msg}"),
            Internal(msg) => write!(f, "internal decoder error: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;

        match self {
            Bitstream(err) => Some(err),
            Decoder(err) => Some(err),
            Buffer(err) => Some(err),
            Modular(err) => Some(err),
            Frame(err) => Some(err),
            Color(err) => Some(err),
            _ => None,
        }
    }
}

impl Error {
    pub fn unexpected_eof(&self) -> bool {
        match self {
            Error::Bitstream(e) => e.unexpected_eof(),
            Error::Decoder(e) => e.unexpected_eof(),
            Error::Modular(e) => e.unexpected_eof(),
            Error::Frame(e) => e.unexpected_eof(),
            Error::Color(e) => e.unexpected_eof(),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
