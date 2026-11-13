//! Our pure-Rust Ut Video decoder (`cfa4-utvideo`): AVI and Matroska (VfW FourCC) `ULY0`,
//! `ULH0`, `ULY2`, `ULH2`, `ULY4`, `ULH4`, `ULRG`, `ULRA`.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The FourCCs of the Ut Video formats (any other `UL..` / `UQ..` / `UM..` one is still routed
/// here, to get a clear "unsupported" error).
pub fn is_utvideo(fourcc: &[u8; 4]) -> bool {
    matches!(&fourcc[..2], b"UL" | b"UQ" | b"UM")
        && matches!(&fourcc[2..], b"Y0" | b"H0" | b"Y2" | b"H2" | b"Y4" | b"H4" | b"RG" | b"RA" | b"R0" | b"R2" | b"Y" | b"YA")
}

/// A sample entry for a Ut Video stream of `w` x `h`: the format is the FourCC, the
/// `CodecConfig::Unknown` `raw` the extradata (the `BITMAPINFOHEADER` bytes after the first 40).
pub fn sample_entry(fourcc: [u8; 4], extra: &[u8], w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(FourCc(fourcc), CodecConfig::Unknown { fourcc: FourCc(fourcc), raw: extra.to_vec() }, w, h)
}

pub struct UtVideoDecoder {
    dec: cfa4_utvideo::Decoder,
}

impl UtVideoDecoder {
    pub fn new(fourcc: [u8; 4], width: u32, height: u32, extra: &[u8]) -> Result<Self> {
        let dec = cfa4_utvideo::Decoder::new(fourcc, width, height, extra).map_err(|e| CodecError::Unsupported(format!("Ut Video: {e}")))?;
        Ok(Self { dec })
    }
}

/// Convert a decoded picture to a frame.
pub(crate) fn convert(p: cfa4_utvideo::Picture) -> Result<VideoFrame> {
    let n = (p.width as usize).saturating_mul(p.height as usize);
    let mut planes = p.planes.into_iter();
    let mut next = || planes.next().ok_or_else(|| CodecError::Decode("Ut Video picture without planes".into()));
    match p.format {
        cfa4_utvideo::Format::Yuv { log2_h, log2_v } => {
            let (y, u, v) = (next()?, next()?, next()?);
            let chroma = match (log2_h, log2_v) {
                (1, 1) => Chroma::C420,
                (1, 0) => Chroma::C422,
                _ => Chroma::C444,
            };
            // the FourCC names the matrix (ULY: BT.601, ULH: BT.709); studio swing
            let mut color = crate::stream_color::resolve(p.width, p.height, &[]);
            color.matrix = match p.matrix {
                cfa4_utvideo::Matrix::Bt601 => cfa4_color::Matrix::Bt601,
                cfa4_utvideo::Matrix::Bt709 => cfa4_color::Matrix::Bt709,
            };
            let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma, alpha: None };
            Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
        }
        cfa4_utvideo::Format::Rgb | cfa4_utvideo::Format::Rgba => {
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

impl VideoDecoder for UtVideoDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: convert(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 Ut Video"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

pub fn utvideo_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !is_utvideo(&e.format.0) {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let extra = match &e.codec {
        CodecConfig::Unknown { raw, .. } => raw.as_slice(),
        _ => &[],
    };
    Some(UtVideoDecoder::new(e.format.0, w, h, extra).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourccs() {
        for f in [b"ULY0", b"ULH2", b"ULRA", b"UQY2", b"UMRG"] {
            assert!(is_utvideo(f), "{f:?}");
        }
        for f in [b"HFYU", b"FFV1", b"ULXX"] {
            assert!(!is_utvideo(f), "{f:?}");
        }
    }

    #[test]
    fn unsupported_formats_say_so() {
        let extra: Vec<u8> = [0x0100_00F0u32, 0, 4, 1].iter().flat_map(|w| w.to_le_bytes()).collect();
        assert!(matches!(UtVideoDecoder::new(*b"UQY2", 64, 32, &extra), Err(CodecError::Unsupported(_))));
        assert!(UtVideoDecoder::new(*b"ULY0", 64, 32, &extra).is_ok());
    }
}
