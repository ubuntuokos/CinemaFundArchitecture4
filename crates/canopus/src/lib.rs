// The HQ, HQA and HQX decoding is ported from FFmpeg n7.1.1 (libavcodec/hq_hqa.c, hq_hqadsp.c,
// hq_hqadata.h, hqx.c, hqxdsp.c, hqxvlc.c), LGPL-2.1-or-later (`COPYING.LGPL-2.1`), an
// owner-approved exception (owner decision 2026-11-12, CB-8.2); see docs/licensing.md §5. CFA4
// changes GPL-3.0-or-later.

//! Pure-Rust decoder for the Canopus intra codecs: **HQ** and **HQA** (AVI FourCC `CUVC`, 8-bit
//! 4:2:2, HQA with alpha) and **HQX** (FourCC `CHQX`, 12-bit 4:2:2 or 4:4:4, optionally with
//! alpha). Every frame is a key frame; a damaged frame is an error, never a panic.

mod bits;
mod hq;
mod hqx;
#[allow(clippy::all)]
mod tables;

/// Errors of a frame.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("truncated frame")]
    Truncated,
    #[error("invalid frame: {0}")]
    Invalid(&'static str),
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Which codec a stream carries (from its FourCC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// `CUVC`: HQ or HQA frames.
    Hq,
    /// `CHQX`: HQX frames.
    Hqx,
}

impl Codec {
    /// The codec of an AVI / VfW FourCC.
    pub fn from_fourcc(fourcc: &[u8; 4]) -> Option<Self> {
        match &fourcc.map(|c| c.to_ascii_uppercase()) {
            b"CUVC" => Some(Self::Hq),
            b"CHQX" => Some(Self::Hqx),
            _ => None,
        }
    }
}

/// The planes of a picture: Y, Cb, Cr and, with alpha, A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Yuv422,
    Yuva422,
    Yuv444,
    Yuva444,
}

impl Layout {
    pub fn has_alpha(self) -> bool {
        matches!(self, Self::Yuva422 | Self::Yuva444)
    }
    pub fn is_444(self) -> bool {
        matches!(self, Self::Yuv444 | Self::Yuva444)
    }
}

/// Sample planes, each `width` (chroma of 4:2:2: `width.div_ceil(2)`) by `height`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Planes {
    /// HQ / HQA: 8 bits.
    U8(Vec<Vec<u8>>),
    /// HQX: 12 bits (0..=4095).
    U16(Vec<Vec<u16>>),
}

/// A decoded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    pub planes: Planes,
}

/// The largest picture we allocate (8K by 4K and a little, in pixels).
const MAX_PIXELS: u64 = 8192 * 4352;

/// FFmpeg's `av_image_check_size`, our allocation cap and its damaged-frame check (with the
/// default `discard_damaged_percentage` of 95: at least 2 bits per 20 macroblocks).
pub(crate) fn check_size(width: u32, height: u32, packet_size: usize) -> Result<()> {
    let (w, h) = (u64::from(width), u64::from(height));
    if w == 0 || h == 0 || (w + 128) * (h + 128) >= (i32::MAX as u64) / 8 || w * h > MAX_PIXELS {
        return Err(Error::Invalid("dimensions"));
    }
    let mbs = w.div_ceil(16) * h.div_ceil(16);
    if mbs * 5 / 100 > 4 * packet_size as u64 {
        return Err(Error::Invalid("frame too small for its size"));
    }
    Ok(())
}

/// A decoder of one stream; it keeps only its code tables.
pub struct Decoder {
    codec: Codec,
    hq: Option<hq::Tables>,
    hqx: Option<hqx::Tables>,
}

impl Decoder {
    pub fn new(codec: Codec) -> Self {
        match codec {
            Codec::Hq => Self { codec, hq: Some(hq::Tables::new()), hqx: None },
            Codec::Hqx => Self { codec, hq: None, hqx: Some(hqx::Tables::new()) },
        }
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Decode one frame (a whole AVI chunk, optionally led by an `INFO` block).
    pub fn decode(&self, packet: &[u8]) -> Result<Picture> {
        if packet.len() < 8 {
            return Err(Error::Truncated);
        }
        let le32 = |at: usize| packet.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let mut start = 0usize;
        if packet.get(..4) == Some(b"INFO") {
            let size = le32(4) as usize;
            if size > i32::MAX as usize || packet.len() - 8 < size {
                return Err(Error::Invalid("INFO size"));
            }
            start = 8 + size;
        }
        let data = packet.get(start..).unwrap_or_default();
        match (self.codec, &self.hq, &self.hqx) {
            (Codec::Hq, Some(t), _) => {
                if data.len() < 4 {
                    return Err(Error::Truncated);
                }
                match data.get(..4) {
                    Some([b'U', b'V', b'C', p]) => hq::decode_hq(t, data, *p),
                    Some(b"HQA1") => hq::decode_hqa(t, data),
                    _ => Err(Error::Invalid("not an HQ or HQA frame")),
                }
            }
            (Codec::Hqx, _, Some(t)) => hqx::decode(t, data, packet.len()),
            _ => Err(Error::Unsupported("codec")),
        }
    }
}

#[cfg(test)]
mod tests;
