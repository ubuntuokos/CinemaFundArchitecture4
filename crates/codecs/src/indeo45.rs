//! Our pure-Rust Indeo 4 and 5 decoders (`cfa4-indeo45`) in AVI and Matroska (VfW FourCC `IV41`,
//! `IV50`).

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_indeo45::Codec;
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The codec of a FourCC.
fn codec_of(fourcc: &[u8; 4]) -> Option<Codec> {
    if fourcc.eq_ignore_ascii_case(b"IV41") {
        Some(Codec::Indeo4)
    } else if fourcc.eq_ignore_ascii_case(b"IV50") {
        Some(Codec::Indeo5)
    } else {
        None
    }
}

/// Whether `fourcc` is Indeo 4's or 5's.
pub fn is_indeo45(fourcc: &[u8; 4]) -> bool {
    codec_of(fourcc).is_some()
}

/// The label of the stream's codec.
pub fn label(fourcc: &[u8; 4]) -> &'static str {
    match codec_of(fourcc) {
        Some(Codec::Indeo5) => "Indeo 5",
        _ => "Indeo 4",
    }
}

/// A sample entry for an Indeo 4 or 5 stream of `w` x `h`.
pub fn sample_entry(fourcc: &[u8; 4], w: u16, h: u16) -> SampleEntry {
    let f = FourCc(fourcc.to_ascii_uppercase().try_into().unwrap_or(*b"IV50"));
    SampleEntry::video(f, CodecConfig::Unknown { fourcc: f, raw: Vec::new() }, w, h)
}

/// 4:1:0 planes as a 4:2:0 frame (each chroma sample repeated to the half-resolution grid).
fn to_frame(p: cfa4_indeo45::Picture) -> Result<VideoFrame> {
    let (w, h) = (p.width as usize, p.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (sw, sh) = p.plane_size(1);
    let mut it = p.planes.into_iter();
    let (y, u, v) = match (it.next(), it.next(), it.next()) {
        (Some(y), Some(u), Some(v)) if sw > 0 && sh > 0 => (y, u, v),
        _ => return Err(CodecError::Decode("Indeo picture without planes".into())),
    };
    let up = |c: &[u8]| -> Vec<u8> {
        (0..ch).flat_map(|j| (0..cw).map(move |i| (i, j))).map(|(i, j)| c.get((j / 2).min(sh - 1) * sw + (i / 2).min(sw - 1)).copied().unwrap_or(128)).collect()
    };
    let color = crate::stream_color::resolve(p.width, p.height, &[]);
    let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(up(&u)), Arc::new(up(&v))], chroma: Chroma::C420, alpha: None };
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

pub struct Indeo45Decoder {
    dec: cfa4_indeo45::Decoder,
    name: &'static str,
    /// The last picture shown, repeated for frames the stream drops.
    last: Option<VideoFrame>,
}

impl VideoDecoder for Indeo45Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let frame = match self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))? {
            Some(pic) => {
                let f = to_frame(pic)?;
                self.last = Some(f.clone());
                f
            }
            // a null frame repeats the picture before it
            None => self.last.clone().ok_or_else(|| CodecError::Decode("Indeo: no picture yet".into()))?,
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
        self.name
    }
    // key frames and delta frames
    fn intra_only(&self) -> bool {
        false
    }
}

pub fn indeo45_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    let codec = codec_of(&e.format.0)?;
    let name = if codec == Codec::Indeo5 { "CFA4 Indeo 5" } else { "CFA4 Indeo 4" };
    Some(Ok(Box::new(Indeo45Decoder { dec: cfa4_indeo45::Decoder::new(codec), name, last: None })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_short_frame() {
        assert!(is_indeo45(b"IV41") && is_indeo45(b"iv50") && !is_indeo45(b"IV31"));
        assert_eq!((label(b"IV41"), label(b"IV50")), ("Indeo 4", "Indeo 5"));
        for f in [b"IV41", b"IV50"] {
            let mut d = indeo45_factory(&sample_entry(f, 16, 16)).unwrap().unwrap();
            assert!(d.decode(&[0; 10], 0).is_err());
        }
    }
}
