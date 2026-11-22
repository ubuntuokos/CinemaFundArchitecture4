//! Our pure-Rust Indeo 3 decoder (`cfa4-indeo3`) in AVI and Matroska (VfW FourCC `IV31`, `IV32`).

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of an Indeo 3 stream.
pub const IV32: FourCc = FourCc(*b"IV32");

/// Whether `fourcc` is Indeo 3's.
pub fn is_indeo3(fourcc: &[u8; 4]) -> bool {
    fourcc.eq_ignore_ascii_case(b"IV31") || fourcc.eq_ignore_ascii_case(b"IV32")
}

/// A sample entry for an Indeo 3 stream of `w` x `h`.
pub fn sample_entry(w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(IV32, CodecConfig::Unknown { fourcc: IV32, raw: Vec::new() }, w, h)
}

/// 4:1:0 planes as a 4:2:0 frame (each chroma sample repeated to the half-resolution grid).
fn to_frame(p: cfa4_indeo3::Picture) -> Result<VideoFrame> {
    let (w, h) = (p.width as usize, p.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (sw, sh) = p.plane_size(1);
    let mut it = p.planes.into_iter();
    let (y, u, v) = match (it.next(), it.next(), it.next()) {
        (Some(y), Some(u), Some(v)) if sw > 0 && sh > 0 => (y, u, v),
        _ => return Err(CodecError::Decode("Indeo 3 picture without planes".into())),
    };
    let up = |c: &[u8]| -> Vec<u8> {
        (0..ch).flat_map(|j| (0..cw).map(move |i| (i, j))).map(|(i, j)| c.get((j / 2).min(sh - 1) * sw + (i / 2).min(sw - 1)).copied().unwrap_or(128)).collect()
    };
    let color = crate::stream_color::resolve(p.width, p.height, &[]);
    let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(up(&u)), Arc::new(up(&v))], chroma: Chroma::C420, alpha: None };
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

pub struct Indeo3Decoder {
    dec: cfa4_indeo3::Decoder,
    /// The last picture shown, repeated for frames the stream drops.
    last: Option<VideoFrame>,
}

impl VideoDecoder for Indeo3Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let frame = match self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))? {
            Some(pic) => {
                let f = to_frame(pic)?;
                self.last = Some(f.clone());
                f
            }
            // a dropped (null) frame repeats the picture before it
            None => self.last.clone().ok_or_else(|| CodecError::Decode("Indeo 3: no picture yet".into()))?,
        };
        Ok(vec![DecodedFrame { pts, frame, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec.reset();
        self.last = None;
    }
    fn name(&self) -> &str {
        "CFA4 Indeo 3"
    }
    // key frames and delta frames
    fn intra_only(&self) -> bool {
        false
    }
}

pub fn indeo3_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if is_indeo3(&e.format.0) {
        return Some(Ok(Box::new(Indeo3Decoder { dec: cfa4_indeo3::Decoder::new(), last: None })));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_short_frame() {
        assert!(is_indeo3(b"IV31") && is_indeo3(b"iv32") && !is_indeo3(b"RT21"));
        let mut d = indeo3_factory(&sample_entry(16, 16)).unwrap().unwrap();
        assert!(d.decode(&[0; 10], 0).is_err());
    }
}
