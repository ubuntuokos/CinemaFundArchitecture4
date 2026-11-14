//! Our pure-Rust MagicYUV decoder (`cfa4-magicyuv`): AVI and Matroska (VfW FourCC) `MAGY`.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of a MagicYUV stream (every frame carries its own header).
pub const MAGY: FourCc = FourCc(*b"MAGY");

/// Whether `fourcc` is one of MagicYUV's: `MAGY`, or the per-format `M8RG` / `M8RA` / `M8G0` /
/// `M8Y0` / `M8Y2` / `M8Y4` / `M8YA` (8-bit) and `M0..` / `M2..` / `M4..` (10- / 12- / 14-bit).
pub fn is_magicyuv(fourcc: &[u8; 4]) -> bool {
    let mut f = *fourcc;
    f.make_ascii_uppercase();
    &f == b"MAGY"
        || matches!(
            &f,
            b"M8RG"
                | b"M8RA"
                | b"M8G0"
                | b"M8Y0"
                | b"M8Y2"
                | b"M8Y4"
                | b"M8YA"
                | b"M0RA"
                | b"M0RG"
                | b"M0G0"
                | b"M0Y0"
                | b"M0Y2"
                | b"M0Y4"
                | b"M2RA"
                | b"M2RG"
                | b"M4RA"
                | b"M4RG"
        )
}

/// A sample entry for a MagicYUV stream of `w` x `h`.
pub fn sample_entry(w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(MAGY, CodecConfig::Unknown { fourcc: MAGY, raw: Vec::new() }, w, h)
}

#[derive(Default)]
pub struct MagicYuvDecoder {
    dec: cfa4_magicyuv::Decoder,
}

/// Convert a decoded picture to a frame.
pub(crate) fn convert(p: cfa4_magicyuv::Picture) -> Result<VideoFrame> {
    use cfa4_magicyuv::Format;
    let (w, h, bits, format) = (p.width, p.height, p.bits, p.format);
    let n = (w as usize).saturating_mul(h as usize);
    let mut color = crate::stream_color::resolve(w, h, &[]);
    match p.matrix {
        Some(cfa4_magicyuv::Matrix::Bt601) => color.matrix = cfa4_color::Matrix::Bt601,
        Some(cfa4_magicyuv::Matrix::Bt709) => color.matrix = cfa4_color::Matrix::Bt709,
        None => {}
    }
    if p.full_range {
        color.range = cfa4_color::Range::Full;
    }
    let mut planes = p.planes.into_iter();
    let mut next = || planes.next().ok_or_else(|| CodecError::Decode("MagicYUV picture without planes".into()));
    let to8 = |v: Vec<u16>| Arc::new(v.into_iter().map(|s| s as u8).collect::<Vec<u8>>());
    let frame = |data, color| VideoFrame { width: w, height: h, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() };
    match format {
        Format::Rgb | Format::Rgba => {
            let (r, g, b) = (next()?, next()?, next()?);
            let a = planes.next();
            if bits == 8 {
                let mut px = Vec::with_capacity(n.saturating_mul(4));
                for i in 0..n {
                    let at = |p: &Vec<u16>| p.get(i).copied().unwrap_or(0) as u8;
                    px.extend_from_slice(&[at(&r), at(&g), at(&b), a.as_ref().map_or(255, at)]);
                }
                Ok(VideoFrame::rgba8(w, h, px))
            } else {
                let data = PixelData::Rgb16 { planes: [Arc::new(r), Arc::new(g), Arc::new(b)], bits, alpha: a.map(Arc::new) };
                Ok(frame(data, color))
            }
        }
        _ => {
            let y = next()?;
            let (u, v) = if format == Format::Gray {
                let neutral = vec![1u16 << (bits - 1); n];
                (neutral.clone(), neutral)
            } else {
                (next()?, next()?)
            };
            let a = planes.next();
            let chroma = match format {
                Format::Yuv420 => Chroma::C420,
                Format::Yuv422 => Chroma::C422,
                _ => Chroma::C444,
            };
            let data = if bits == 8 {
                PixelData::Yuv8 { planes: [to8(y), to8(u), to8(v)], chroma, alpha: a.map(to8) }
            } else {
                PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma, bits, alpha: a.map(Arc::new) }
            };
            Ok(frame(data, color))
        }
    }
}

impl VideoDecoder for MagicYuvDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: convert(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 MagicYUV"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

pub fn magicyuv_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    is_magicyuv(&e.format.0).then(|| Ok(Box::new(MagicYuvDecoder::default()) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_garbage() {
        assert!(is_magicyuv(b"MAGY") && is_magicyuv(b"magy") && is_magicyuv(b"M8Y0") && is_magicyuv(b"M2RG") && !is_magicyuv(b"LAGS") && !is_magicyuv(b"M8XX"));
        let mut d = magicyuv_factory(&sample_entry(4, 2)).unwrap().unwrap();
        assert!(d.decode(&[0; 40], 0).is_err());
    }
}
