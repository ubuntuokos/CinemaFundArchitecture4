// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-vardct 0.11.1.
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

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Error::*;

        match self {
            Bitstream(err) => write!(f, "bitstream error: {err}"),
            Decoder(err) => write!(f, "entropy decoder error: {err}"),
            Buffer(err) => write!(f, "{err}"),
            Modular(err) => write!(f, "modular stream error: {err}"),
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
        }
    }
}

impl Error {
    pub fn unexpected_eof(&self) -> bool {
        match self {
            Error::Bitstream(e) => e.unexpected_eof(),
            Error::Decoder(e) => e.unexpected_eof(),
            Error::Modular(e) => e.unexpected_eof(),
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
