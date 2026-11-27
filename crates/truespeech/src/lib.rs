//! DSP Group TrueSpeech (WAVE format tag 0x0022): an 8 kHz mono speech codec, 32 bytes per frame
//! of 240 samples. Ported from FFmpeg n9.0.2's `libavcodec/truespeech.c` (LGPL-2.1-or-later,
//! `COPYING.LGPL-2.1`); CFA4 changes are GPL-3.0-or-later.

mod decoder;
mod tables;

pub use decoder::Decoder;

/// Bytes of one frame.
pub const FRAME_BYTES: usize = 32;
/// Samples of one frame.
pub const FRAME_SAMPLES: usize = 240;
/// The only sample rate of the codec.
pub const SAMPLE_RATE: u32 = 8000;

/// A TrueSpeech decoding error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The packet cannot hold a frame.
    #[error("TrueSpeech: packet of {0} bytes is smaller than one {FRAME_BYTES}-byte frame")]
    TooSmall(usize),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;
