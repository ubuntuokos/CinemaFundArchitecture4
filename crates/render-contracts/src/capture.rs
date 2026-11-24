//! Capture I/O interface (interface only: the plan's F7 implements it).

use cfa4_frame::VideoFrame;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("capture device unavailable: {0}")]
    Unavailable(String),
    #[error("no frame within the timeout")]
    Timeout,
}

/// A source of live frames. `Ok(None)` is a clean end of the signal; a missing frame is an
/// error, never a repeated or invented one.
pub trait CaptureSource {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, CaptureError>;
}
