//! Our pure-Rust Microsoft Video 1 and Microsoft RLE decoders (`cfa4-msvideo`): AVI and Matroska
//! (VfW FourCC) `MSVC` / `CRAM` / `WHAM` and `BI_RLE8` / `BI_RLE4` / `mrle` / `WRLE`.

use cfa4_frame::VideoFrame;
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry formats; the `CodecConfig::Unknown` `raw` of both is a `BITMAPINFOHEADER`
/// whose bit count (byte 14) and trailing palette (after byte 40) set the decoder up.
pub const MSVC: FourCc = FourCc(*b"MSVC");
pub const MRLE: FourCc = FourCc(*b"MRLE");

/// Which of the two codecs a FourCC names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video1,
    Rle,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video1 => "Microsoft Video 1",
            Self::Rle => "Microsoft RLE",
        }
    }

    fn format(self) -> FourCc {
        match self {
            Self::Video1 => MSVC,
            Self::Rle => MRLE,
        }
    }
}

/// The codec a `biCompression` / VfW FourCC names: `MSVC`, `CRAM`, `WHAM` are Video 1; `mrle`,
/// `WRLE` and the values 1 and 2 (`BI_RLE8`, `BI_RLE4`) are RLE.
pub fn kind(fourcc: &[u8; 4]) -> Option<Kind> {
    let mut f = *fourcc;
    f.make_ascii_uppercase();
    match &f {
        b"MSVC" | b"CRAM" | b"WHAM" => Some(Kind::Video1),
        b"MRLE" | b"WRLE" | [1, 0, 0, 0] | [2, 0, 0, 0] => Some(Kind::Rle),
        _ => None,
    }
}

/// The palette an AVI header carries: FFmpeg's demuxer takes the last `2^bits` entries' worth of
/// bytes after the header (less a trailing `BottomUp` marker) for pictures of 8 bits or fewer.
pub fn palette_bytes(bit_count: u16, extra: &[u8]) -> &[u8] {
    if bit_count == 0 || bit_count > 8 {
        return &[];
    }
    let size = ((1usize << bit_count) * 4).min(extra.len());
    let mut end = extra.len();
    if extra.len() >= size + 9 && extra.ends_with(b"BottomUp\0") {
        end -= 9;
    }
    extra.get(end.saturating_sub(size)..end).unwrap_or_default()
}

/// A `BITMAPINFOHEADER` carrying only what the decoder reads: the bit count and the palette.
pub fn bitmap_info_header(bit_count: u16, extra: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 40];
    if let Some(s) = b.get_mut(14..16) {
        s.copy_from_slice(&bit_count.to_le_bytes());
    }
    b.extend_from_slice(palette_bytes(bit_count, extra));
    b
}

/// A sample entry for a stream of `kind`, `w` x `h`, with `BITMAPINFOHEADER` `bih`.
pub fn sample_entry(kind: Kind, bih: &[u8], w: u16, h: u16) -> SampleEntry {
    let f = kind.format();
    SampleEntry::video(f, CodecConfig::Unknown { fourcc: f, raw: bih.to_vec() }, w, h)
}

enum Inner {
    Video1(cfa4_msvideo::video1::Decoder),
    Rle(cfa4_msvideo::rle::Decoder),
}

pub struct MsVideoDecoder {
    inner: Inner,
    name: &'static str,
}

impl MsVideoDecoder {
    pub fn new(kind: Kind, width: u32, height: u32, bih: &[u8]) -> Result<Self> {
        let bits = bih.get(14..16).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let extra = bih.get(40..).unwrap_or_default();
        let unsupported = |e: cfa4_msvideo::Error| CodecError::Unsupported(format!("{}: {e}", kind.label()));
        Ok(match kind {
            Kind::Video1 => Self {
                inner: Inner::Video1(cfa4_msvideo::video1::Decoder::new(width, height, bits, extra).map_err(unsupported)?),
                name: "CFA4 Microsoft Video 1",
            },
            Kind::Rle => {
                Self { inner: Inner::Rle(cfa4_msvideo::rle::Decoder::new(width, height, bits, extra).map_err(unsupported)?), name: "CFA4 Microsoft RLE" }
            }
        })
    }
}

impl VideoDecoder for MsVideoDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = match &mut self.inner {
            Inner::Video1(d) => d.decode(sample),
            Inner::Rle(d) => d.decode(sample),
        }
        .map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: VideoFrame::rgba8(pic.width, pic.height, pic.rgba), draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        match &mut self.inner {
            Inner::Video1(d) => d.reset(),
            Inner::Rle(d) => d.reset(),
        }
    }
    fn name(&self) -> &str {
        self.name
    }
}

pub fn msvideo_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    let kind = match e.format {
        MSVC => Kind::Video1,
        MRLE => Kind::Rle,
        _ => return None,
    };
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let bih = match &e.codec {
        CodecConfig::Unknown { raw, .. } => raw.as_slice(),
        _ => &[],
    };
    Some(MsVideoDecoder::new(kind, w, h, bih).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourccs() {
        assert_eq!(kind(b"MSVC"), Some(Kind::Video1));
        assert_eq!(kind(b"cram"), Some(Kind::Video1));
        assert_eq!(kind(b"WHAM"), Some(Kind::Video1));
        assert_eq!(kind(&[1, 0, 0, 0]), Some(Kind::Rle));
        assert_eq!(kind(&[2, 0, 0, 0]), Some(Kind::Rle));
        assert_eq!(kind(b"mrle"), Some(Kind::Rle));
        assert_eq!(kind(&[0, 0, 0, 0]), None);
        assert_eq!(kind(b"CVID"), None);
    }

    #[test]
    fn palette_is_the_tail_of_the_extradata() {
        let extra: Vec<u8> = (0..70).collect();
        assert_eq!(palette_bytes(4, &extra), &extra[6..70]);
        assert_eq!(palette_bytes(8, &extra), &extra[..]);
        assert!(palette_bytes(16, &extra).is_empty());
        let mut marked = extra.clone();
        marked.extend_from_slice(b"BottomUp\0");
        assert_eq!(palette_bytes(4, &marked), &extra[6..70]);
    }

    #[test]
    fn bad_headers_are_unsupported() {
        assert!(MsVideoDecoder::new(Kind::Rle, 16, 16, &bitmap_info_header(16, &[])).is_err());
        assert!(MsVideoDecoder::new(Kind::Video1, 2, 2, &bitmap_info_header(16, &[])).is_err());
        assert!(MsVideoDecoder::new(Kind::Video1, 16, 16, &[]).is_ok());
    }
}
