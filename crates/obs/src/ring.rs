//! The frame ring lives in `cfa4-io-core` (CFA4-DECKLINK-READY-001 V2.1); this module re-exports
//! it, so `cfa4_obs::ring` is unchanged for the host, the engine and the tests, and adds the one
//! function that knows the studio's [`VideoSettings`].

use crate::{Result, VideoSettings};
use std::path::{Path, PathBuf};

pub use cfa4_io_core::frame::bgra_to_rgba;
pub use cfa4_io_core::ring::{
    Extras, FLAG_BOTTOM_FIELD_FIRST, FLAG_INTERLACED, HEADER, MAGIC, MAX_FILE, RingError, RingFrame, RingReader, RingWriter, SLOT_HEADER, SLOTS, VERSION,
    create_format, create_sized, default_dir, remove,
};

/// Creates a ring for the studio's program frames (BGRA, no sound) in `dir`: a new private file,
/// never reused. Returns its path; the caller removes the file when done ([`remove`]).
pub fn create(dir: &Path, video: &VideoSettings) -> Result<PathBuf> {
    video.validate()?;
    Ok(create_sized(dir, video.width, video.height)?)
}
