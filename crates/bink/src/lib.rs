// Ported from FFmpeg n7.1.1 libavcodec/bink.c, binkdsp.c, binkdata.h, binkaudio.c (Copyright (c)
// 2009 Konstantin Shishkov, 2009 Daniel Verkamp, 2011 Peter Ross) and libavformat/bink.c
// (Copyright (c) 2008-2010 Peter Ross, 2009 Daniel Verkamp), LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust **Bink 1** (RAD Game Tools) decoders: the video codec (revisions `a`/`b` and
//! `f`..`k`, with alpha) and the RDFT and DCT audio codecs, plus the `.bik` container demuxer.
//! Bink 2 (`KB2`) is not supported. Video output is the same as ffmpeg's `binkvideo`, byte for
//! byte; audio matches `binkaudio_rdft` / `binkaudio_dct` to float rounding.

pub mod audio;
mod bits;
pub mod container;
mod tables;
pub mod video;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid Bink data: {0}")]
    Invalid(&'static str),
    #[error("unsupported Bink stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
