//! Our pure-Rust FFV1 decoder (RFC 9043; `cfa4-ffv1`): AVI / Matroska `FFV1` (VfW FourCC) and
//! Matroska `V_FFV1`.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of an FFV1 stream; its `CodecConfig::Unknown` `raw` is the
/// configuration record (version 3) or empty (versions 0 and 1).
pub const FFV1: FourCc = FourCc(*b"FFV1");

/// A sample entry for an FFV1 stream of `w` x `h` with configuration record `record`.
pub fn sample_entry(record: &[u8], w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(FFV1, CodecConfig::Unknown { fourcc: FFV1, raw: record.to_vec() }, w, h)
}

pub struct Ffv1Decoder {
    dec: cfa4_ffv1::Decoder,
}

impl Ffv1Decoder {
    pub fn new(width: u32, height: u32, record: &[u8]) -> Result<Self> {
        let dec = cfa4_ffv1::Decoder::new(width, height, record).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(Self { dec })
    }
}

/// Chroma plane `p` (`cw` x `ch`) repeated to `w` x `h` (subsamplings the frame type can't hold).
fn upsample(p: &[u16], cw: usize, ch: usize, hs: u32, vs: u32, w: usize, h: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(w.saturating_mul(h));
    for y in 0..h {
        let row = (y >> vs).min(ch.saturating_sub(1)) * cw;
        for x in 0..w {
            out.push(p.get(row + (x >> hs).min(cw.saturating_sub(1))).copied().unwrap_or(0));
        }
    }
    out
}

/// Convert a decoded picture to a frame.
pub(crate) fn convert(p: cfa4_ffv1::Picture) -> Result<VideoFrame> {
    let (w, h) = (p.width as usize, p.height as usize);
    let bits = p.bits.clamp(8, 16);
    let par = p.sar.filter(|&(n, d)| n > 0 && d > 0).unwrap_or((1, 1));
    let mut planes = p.planes.into_iter();
    let mut next = || planes.next().ok_or_else(|| CodecError::Decode("FFV1 picture without planes".into()));
    let (chroma, yuv) = match p.format {
        cfa4_ffv1::Format::Rgb => {
            let (r, g, b) = (next()?, next()?, next()?);
            let down = bits - 8;
            let to8 = |v: u16| if down == 0 { v.min(255) as u8 } else { ((u32::from(v) + (1 << (down - 1))) >> down).min(255) as u8 };
            let mut px = Vec::with_capacity(w.saturating_mul(h).saturating_mul(4));
            for i in 0..w.saturating_mul(h) {
                let a = p.alpha.as_ref().and_then(|a| a.get(i)).map_or(255, |&v| to8(v));
                px.extend_from_slice(&[to8(r.get(i).copied().unwrap_or(0)), to8(g.get(i).copied().unwrap_or(0)), to8(b.get(i).copied().unwrap_or(0)), a]);
            }
            let mut f = VideoFrame::rgba8(p.width, p.height, px);
            f.par = par;
            return Ok(f);
        }
        cfa4_ffv1::Format::Gray => {
            let y = next()?;
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            let mid = vec![1u16 << (bits - 1); cw.saturating_mul(ch)];
            (Chroma::C420, [y, mid.clone(), mid])
        }
        cfa4_ffv1::Format::Yuv { log2_h, log2_v } => {
            let (y, cb, cr) = (next()?, next()?, next()?);
            match (log2_h, log2_v) {
                (1, 1) => (Chroma::C420, [y, cb, cr]),
                (1, 0) => (Chroma::C422, [y, cb, cr]),
                (0, 0) => (Chroma::C444, [y, cb, cr]),
                (hs, vs) => {
                    let (cw, ch) = (w.div_ceil(1 << hs), h.div_ceil(1 << vs));
                    (Chroma::C444, [y, upsample(&cb, cw, ch, hs, vs, w, h), upsample(&cr, cw, ch, hs, vs, w, h)])
                }
            }
        }
    };
    // FFV1 signals no matrix or range: the size-based default, studio swing
    let color = crate::stream_color::resolve(p.width, p.height, &[]);
    let data = if bits == 8 {
        let to8 = |v: Vec<u16>| Arc::new(v.into_iter().map(|s| s.min(255) as u8).collect::<Vec<u8>>());
        let [y, cb, cr] = yuv;
        PixelData::Yuv8 { planes: [to8(y), to8(cb), to8(cr)], chroma, alpha: p.alpha.map(to8) }
    } else {
        let [y, cb, cr] = yuv;
        PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(cb), Arc::new(cr)], chroma, bits, alpha: p.alpha.map(Arc::new) }
    };
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par, pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

impl VideoDecoder for Ffv1Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: convert(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec.reset();
    }
    fn name(&self) -> &str {
        "CFA4 FFV1"
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(cfa4_ffv1::is_keyframe(sample))
    }
}

pub fn ffv1_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if e.format != FFV1 {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let record = match &e.codec {
        CodecConfig::Unknown { raw, .. } => raw.as_slice(),
        _ => &[],
    };
    Some(Ffv1Decoder::new(w, h, record).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_subsampling_is_upsampled_to_444() {
        let p = cfa4_ffv1::Picture {
            width: 5,
            height: 3,
            bits: 8,
            format: cfa4_ffv1::Format::Yuv { log2_h: 2, log2_v: 2 },
            planes: vec![vec![16; 15], vec![1, 2], vec![3, 4]],
            alpha: None,
            key: true,
            sar: None,
            picture_structure: 0,
        };
        let f = convert(p).unwrap();
        let PixelData::Yuv8 { planes, chroma, .. } = &f.data else { panic!("8-bit YUV") };
        assert_eq!(*chroma, Chroma::C444);
        assert_eq!(planes[1].as_slice(), &[1, 1, 1, 1, 2, 1, 1, 1, 1, 2, 1, 1, 1, 1, 2]);
    }

    #[test]
    fn missing_planes_are_an_error() {
        let p = cfa4_ffv1::Picture {
            width: 2,
            height: 2,
            bits: 8,
            format: cfa4_ffv1::Format::Rgb,
            planes: vec![],
            alpha: None,
            key: true,
            sar: None,
            picture_structure: 0,
        };
        assert!(convert(p).is_err());
    }
}
