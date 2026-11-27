//! Microsoft Video 1 (`MSVC`, `CRAM`, `WHAM`) and Microsoft RLE (`BI_RLE8`, `BI_RLE4`, `mrle`,
//! `WRLE`) decoders for AVI and Matroska (VfW) files, ported from FFmpeg's `msvideo1.c`,
//! `msrle.c` and `msrledec.c`.
//!
//! Both codecs are inter-frame: a delta picture changes only part of the previous one, so a
//! decoder keeps the picture between packets. [`video1::Decoder`] and [`rle::Decoder`] return the
//! whole picture as RGBA8 after every packet.

pub mod rle;
pub mod video1;

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("unsupported {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Largest frame edge accepted.
pub const MAX_DIMENSION: u32 = 16384;

/// A decoded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, R, G, B, A (opaque), top row first.
    pub rgba: Vec<u8>,
}

/// A palette of 256 entries as the file stores them: four bytes each, blue first.
#[derive(Clone)]
pub(crate) struct Palette([[u8; 4]; 256]);

impl Palette {
    /// FFmpeg reads as many whole entries as the extradata holds (up to 256); the rest stay black.
    pub(crate) fn from_extra(extra: &[u8]) -> Self {
        let mut p = [[0u8; 4]; 256];
        for (e, c) in p.iter_mut().zip(extra.chunks_exact(4)) {
            e.copy_from_slice(c);
        }
        Self(p)
    }

    pub(crate) fn rgb(&self, i: u8) -> [u8; 3] {
        let [b, g, r, _] = self.0[usize::from(i)];
        [r, g, b]
    }
}

pub(crate) fn check_size(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::Invalid("frame size"));
    }
    Ok(())
}
