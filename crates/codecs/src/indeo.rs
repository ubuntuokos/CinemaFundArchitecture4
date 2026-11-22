//! Our pure-Rust Indeo decoders (`cfa4-indeo`): Indeo 2 (`RT21`) in AVI and Matroska (VfW FourCC).

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of an Indeo 2 stream.
pub const RT21: FourCc = FourCc(*b"RT21");

/// Whether `fourcc` is Indeo 2's.
pub fn is_indeo2(fourcc: &[u8; 4]) -> bool {
    fourcc.eq_ignore_ascii_case(b"RT21")
}

/// A sample entry for an Indeo 2 stream of `w` x `h`.
pub fn sample_entry_indeo2(w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(RT21, CodecConfig::Unknown { fourcc: RT21, raw: Vec::new() }, w, h)
}

/// 4:1:0 planes as a 4:2:0 frame (each chroma sample repeated to the half-resolution grid).
fn to_frame(p: cfa4_indeo::Picture) -> Result<VideoFrame> {
    let (w, h) = (p.width as usize, p.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (sw, sh) = p.plane_size(1);
    let mut it = p.planes.into_iter();
    let (y, u, v) = match (it.next(), it.next(), it.next()) {
        (Some(y), Some(u), Some(v)) => (y, u, v),
        _ => return Err(CodecError::Decode("Indeo picture without planes".into())),
    };
    let up = |c: &[u8]| -> Vec<u8> {
        (0..ch).flat_map(|j| (0..cw).map(move |i| (i, j))).map(|(i, j)| c.get((j / 2).min(sh - 1) * sw + (i / 2).min(sw - 1)).copied().unwrap_or(128)).collect()
    };
    let color = crate::stream_color::resolve(p.width, p.height, &[]);
    let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(up(&u)), Arc::new(up(&v))], chroma: Chroma::C420, alpha: None };
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

pub struct Indeo2Decoder {
    dec: cfa4_indeo::ir2::Decoder,
    size: (u32, u32),
}

impl VideoDecoder for Indeo2Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: to_frame(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        if let Ok(d) = cfa4_indeo::ir2::Decoder::new(self.size.0, self.size.1) {
            self.dec = d;
        }
    }
    fn name(&self) -> &str {
        "CFA4 Indeo 2"
    }
    // AVI marks every sample as a sync sample: only an intra frame can start a run
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        let mut fresh = cfa4_indeo::ir2::Decoder::new(self.size.0, self.size.1).ok()?;
        Some(fresh.decode(sample).is_ok_and(|p| p.key))
    }
    // intra frames and delta frames
    fn intra_only(&self) -> bool {
        false
    }
}

pub fn indeo_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    if is_indeo2(&e.format.0) {
        return Some(
            cfa4_indeo::ir2::Decoder::new(w, h)
                .map(|dec| Box::new(Indeo2Decoder { dec, size: (w, h) }) as Box<dyn VideoDecoder>)
                .map_err(|e| CodecError::Unsupported(format!("Indeo 2: {e}"))),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_short_frame() {
        assert!(is_indeo2(b"RT21") && is_indeo2(b"rt21") && !is_indeo2(b"IV31"));
        let mut d = indeo_factory(&sample_entry_indeo2(16, 16)).unwrap().unwrap();
        assert!(d.decode(&[0; 10], 0).is_err());
        assert!(indeo_factory(&sample_entry_indeo2(0, 16)).unwrap().is_err());
    }
}
