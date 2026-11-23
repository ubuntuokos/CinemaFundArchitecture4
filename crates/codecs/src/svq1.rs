//! Our pure-Rust Sorenson Video 1 decoder (`cfa4-svq1`) in AVI and Matroska (VfW FourCC `SVQ1`)
//! and in QuickTime / MP4 sample entries (`SVQ1`).

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of an SVQ1 stream.
pub const SVQ1: FourCc = FourCc(*b"SVQ1");

/// Whether `fourcc` is SVQ1's.
pub fn is_svq1(fourcc: &[u8; 4]) -> bool {
    fourcc.eq_ignore_ascii_case(b"SVQ1")
}

/// A sample entry for an SVQ1 stream of `w` x `h`.
pub fn sample_entry(w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(SVQ1, CodecConfig::Unknown { fourcc: SVQ1, raw: Vec::new() }, w, h)
}

/// 4:1:0 planes as a 4:2:0 frame (each chroma sample repeated to the half-resolution grid).
fn to_frame(p: cfa4_svq1::Picture) -> Result<VideoFrame> {
    let (w, h) = (p.width as usize, p.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (sw, sh) = p.plane_size(1);
    let mut it = p.planes.into_iter();
    let (y, u, v) = match (it.next(), it.next(), it.next()) {
        (Some(y), Some(u), Some(v)) if sw > 0 && sh > 0 => (y, u, v),
        _ => return Err(CodecError::Decode("SVQ1 picture without planes".into())),
    };
    let up = |c: &[u8]| -> Vec<u8> {
        (0..ch).flat_map(|j| (0..cw).map(move |i| (i, j))).map(|(i, j)| c.get((j / 2).min(sh - 1) * sw + (i / 2).min(sw - 1)).copied().unwrap_or(128)).collect()
    };
    let color = crate::stream_color::resolve(p.width, p.height, &[]);
    let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(up(&u)), Arc::new(up(&v))], chroma: Chroma::C420, alpha: None };
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

pub struct Svq1Decoder {
    dec: cfa4_svq1::Decoder,
}

impl VideoDecoder for Svq1Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(vec![DecodedFrame { pts, frame: to_frame(pic)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec.reset();
    }
    fn name(&self) -> &str {
        "CFA4 SVQ1"
    }
    // AVI marks every sample as a sync sample: only a key frame can start a run
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        cfa4_svq1::frame_type(sample).map(|t| t == 0)
    }
    // a delta frame no other picture refers to
    fn is_disposable(&self, sample: &[u8]) -> bool {
        cfa4_svq1::frame_type(sample) == Some(2)
    }
    // key frames and delta frames
    fn intra_only(&self) -> bool {
        false
    }
}

pub fn svq1_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !is_svq1(&e.format.0) {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let extradata = matches!(&e.codec, CodecConfig::Unknown { raw, .. } if !raw.is_empty());
    Some(Ok(Box::new(Svq1Decoder { dec: cfa4_svq1::Decoder::new(w, h, extradata) })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_short_frame() {
        assert!(is_svq1(b"SVQ1") && is_svq1(b"svq1") && !is_svq1(b"SVQ3"));
        let mut d = svq1_factory(&sample_entry(16, 16)).unwrap().unwrap();
        assert!(d.decode(&[0; 10], 0).is_err());
    }
}
