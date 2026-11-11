//! Our pure-Rust Huffyuv decoder (`cfa4-huffyuv`): AVI and Matroska (VfW FourCC) `HFYU`.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of a Huffyuv stream; its `CodecConfig::Unknown` `raw` is the whole
/// `BITMAPINFOHEADER` (the bit count at byte 14 and the extradata after byte 40 set the stream up).
pub const HFYU: FourCc = FourCc(*b"HFYU");

/// A sample entry for a Huffyuv stream of `w` x `h` with `BITMAPINFOHEADER` `bih`.
pub fn sample_entry(bih: &[u8], w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(HFYU, CodecConfig::Unknown { fourcc: HFYU, raw: bih.to_vec() }, w, h)
}

/// A `BITMAPINFOHEADER` carrying only what the decoder reads: the bit count and the extradata.
pub fn bitmap_info_header(bit_count: u16, extra: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 40];
    if let Some(s) = b.get_mut(14..16) {
        s.copy_from_slice(&bit_count.to_le_bytes());
    }
    b.extend_from_slice(extra);
    b
}

pub struct HuffyuvDecoder {
    dec: cfa4_huffyuv::Decoder,
}

impl HuffyuvDecoder {
    pub fn new(width: u32, height: u32, bih: &[u8]) -> Result<Self> {
        let bit_count = bih.get(14..16).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let extra = bih.get(40..).unwrap_or_default();
        let dec = cfa4_huffyuv::Decoder::new(width, height, bit_count, extra).map_err(|e| CodecError::Unsupported(format!("Huffyuv: {e}")))?;
        Ok(Self { dec })
    }
}

/// Convert a decoded picture to a frame.
pub(crate) fn convert(p: cfa4_huffyuv::Picture) -> Result<VideoFrame> {
    let n = (p.width as usize).saturating_mul(p.height as usize);
    let mut planes = p.planes.into_iter();
    let mut next = || planes.next().ok_or_else(|| CodecError::Decode("Huffyuv picture without planes".into()));
    match p.format {
        cfa4_huffyuv::Format::Yuv422 => {
            let (y, u, v) = (next()?, next()?, next()?);
            // Huffyuv signals no matrix or range: the size-based default, studio swing
            let color = crate::stream_color::resolve(p.width, p.height, &[]);
            let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C422, alpha: None };
            Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO })
        }
        cfa4_huffyuv::Format::Rgb | cfa4_huffyuv::Format::Rgba => {
            let (r, g, b) = (next()?, next()?, next()?);
            let a = planes.next();
            let mut px = Vec::with_capacity(n.saturating_mul(4));
            for i in 0..n {
                let at = |p: &Vec<u8>| p.get(i).copied().unwrap_or(0);
                px.extend_from_slice(&[at(&r), at(&g), at(&b), a.as_ref().map_or(255, at)]);
            }
            Ok(VideoFrame::rgba8(p.width, p.height, px))
        }
    }
}

impl VideoDecoder for HuffyuvDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: convert(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 Huffyuv"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

pub fn huffyuv_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if e.format != HFYU {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let bih = match &e.codec {
        CodecConfig::Unknown { raw, .. } => raw.as_slice(),
        _ => &[],
    };
    Some(HuffyuvDecoder::new(w, h, bih).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_without_tables_is_unsupported() {
        let bih = bitmap_info_header(16, &[]);
        assert!(matches!(HuffyuvDecoder::new(64, 32, &bih), Err(CodecError::Unsupported(_))));
        assert!(HuffyuvDecoder::new(64, 32, &[]).is_err());
    }

    #[test]
    fn missing_planes_are_an_error() {
        let p = cfa4_huffyuv::Picture { width: 2, height: 2, format: cfa4_huffyuv::Format::Rgb, planes: vec![] };
        assert!(convert(p).is_err());
    }
}
