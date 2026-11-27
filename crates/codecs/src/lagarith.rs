//! Our pure-Rust Lagarith decoder (`cfa4-lagarith`): AVI and Matroska (VfW FourCC) `LAGS`.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of a Lagarith stream; its `CodecConfig::Unknown` `raw` is the
/// `BITMAPINFOHEADER` bit count (2 bytes, little-endian), which sets the layout of solid frames.
pub const LAGS: FourCc = FourCc(*b"LAGS");

/// Whether `fourcc` is Lagarith's.
pub fn is_lagarith(fourcc: &[u8; 4]) -> bool {
    fourcc.eq_ignore_ascii_case(b"LAGS")
}

/// A sample entry for a Lagarith stream of `w` x `h` with `BITMAPINFOHEADER` bit count `bit_count`.
pub fn sample_entry(bit_count: u16, w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(LAGS, CodecConfig::Unknown { fourcc: LAGS, raw: bit_count.to_le_bytes().to_vec() }, w, h)
}

pub struct LagarithDecoder {
    dec: cfa4_lagarith::Decoder,
}

impl LagarithDecoder {
    pub fn new(width: u32, height: u32, bit_count: u16) -> Result<Self> {
        let dec = cfa4_lagarith::Decoder::new(width, height, bit_count).map_err(|e| CodecError::Unsupported(format!("Lagarith: {e}")))?;
        Ok(Self { dec })
    }
}

/// Convert a decoded picture to a frame.
pub(crate) fn convert(p: cfa4_lagarith::Picture) -> Result<VideoFrame> {
    let n = (p.width as usize).saturating_mul(p.height as usize);
    let format = p.format;
    let mut planes = p.planes.into_iter();
    let mut next = || planes.next().ok_or_else(|| CodecError::Decode("Lagarith picture without planes".into()));
    match format {
        cfa4_lagarith::Format::Yuv422 | cfa4_lagarith::Format::Yuv420 => {
            let (y, u, v) = (next()?, next()?, next()?);
            let chroma = if format == cfa4_lagarith::Format::Yuv420 { Chroma::C420 } else { Chroma::C422 };
            // Lagarith signals no matrix or range: the size-based default, studio swing
            let color = crate::stream_color::resolve(p.width, p.height, &[]);
            let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma, alpha: None };
            Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
        }
        cfa4_lagarith::Format::Rgb | cfa4_lagarith::Format::Rgba => {
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

impl VideoDecoder for LagarithDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: convert(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 Lagarith"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

pub fn lagarith_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !is_lagarith(&e.format.0) {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let bit_count = match &e.codec {
        CodecConfig::Unknown { raw, .. } => raw.get(..2).map_or(24, |b| u16::from_le_bytes([b[0], b[1]])),
        _ => 24,
    };
    Some(LagarithDecoder::new(w, h, bit_count).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_solid_frame() {
        assert!(is_lagarith(b"LAGS") && is_lagarith(b"lags") && !is_lagarith(b"HFYU"));
        let mut d = lagarith_factory(&sample_entry(24, 4, 2)).unwrap().unwrap();
        let f = d.decode(&[6, 10, 20, 30], 0).unwrap();
        let PixelData::Rgba8(px) = &f[0].frame.data else { panic!("RGBA frame") };
        assert_eq!(&px[..4], &[30, 20, 10, 255]);
        assert!(lagarith_factory(&sample_entry(24, 0, 2)).unwrap().is_err());
    }
}
