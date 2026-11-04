// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Bitstream(crate::bitstream::Error),
    Decoder(crate::coding::Error),
    InvalidIccStream(&'static str),
    IccParseFailure(&'static str),
    UnsupportedColorEncoding,
    UnsupportedIccProfile,
    IccProfileEmbedded,
    InvalidEnumColorspace,
    CmsNotAvailable,
    CmsFailure(Box<dyn std::error::Error + Send + Sync + 'static>),
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

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Error::*;

        match self {
            Bitstream(err) => write!(f, "bitstream error: {}", err),
            Decoder(err) => write!(f, "entropy decoder error: {}", err),
            InvalidIccStream(s) => write!(f, "invalid ICC stream: {s}"),
            IccParseFailure(s) => write!(f, "parsing ICC profile failed: {s}"),
            UnsupportedColorEncoding => write!(f, "unsupported color encoding"),
            UnsupportedIccProfile => write!(f, "unsupported ICC profile"),
            IccProfileEmbedded => write!(f, "embedded ICC profile is signalled, use it instead"),
            InvalidEnumColorspace => write!(f, "unknown colorspace without embedded ICC profile"),
            CmsNotAvailable => write!(f, "color management system is not available"),
            CmsFailure(err) => write!(f, "color management system error: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;

        match self {
            Bitstream(err) => Some(err),
            Decoder(err) => Some(err),
            CmsFailure(err) => Some(&**err),
            _ => None,
        }
    }
}

impl Error {
    pub fn unexpected_eof(&self) -> bool {
        if let Error::Bitstream(e) | Error::Decoder(crate::coding::Error::Bitstream(e)) = self {
            return e.unexpected_eof();
        }
        false
    }
}

pub type Result<T> = std::result::Result<T, Error>;
