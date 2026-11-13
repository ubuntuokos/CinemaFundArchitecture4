//! Our pure-Rust VC-2 decoder (SMPTE ST 2042-1, the intra subset of Dirac; `cfa4-dirac`):
//! QuickTime / MP4 and AVI `drac`, Matroska `V_DIRAC`.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::stream_color::ColorCodes;
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of a Dirac / VC-2 stream.
pub const DRAC: FourCc = FourCc(*b"drac");

/// A sample entry for a Dirac / VC-2 stream of `w` x `h`.
pub fn sample_entry(w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(DRAC, CodecConfig::Unknown { fourcc: DRAC, raw: Vec::new() }, w, h)
}

/// The H.273 colour code points of a sequence's video parameters (§11.4.9, §11.4.10).
fn color_codes(v: &cfa4_dirac::VideoParameters) -> ColorCodes {
    // Tables 11.7, 11.8 and 11.9 in H.273 terms; 2 is "unspecified"
    let primaries = match v.color_primaries {
        0 => 1,
        1 => 6,
        2 => 5,
        3 => 10,
        4 => 9,
        _ => 2,
    };
    let matrix = match v.color_matrix {
        0 => 1,
        1 => 6,
        2 => 8,
        3 => 0,
        4 => 9,
        _ => 2,
    };
    let transfer = match v.transfer_function {
        0 => 1,
        1 => 11,
        2 => 8,
        3 => 17,
        4 => 16,
        5 => 18,
        _ => 2,
    };
    ColorCodes { primaries, transfer, matrix, full_range: Some(v.luma_offset == 0) }
}

/// A decoded picture as a frame.
pub(crate) fn convert(p: cfa4_dirac::Picture, v: &cfa4_dirac::VideoParameters) -> Result<VideoFrame> {
    let chroma = match p.chroma {
        cfa4_dirac::ChromaFormat::C444 => Chroma::C444,
        cfa4_dirac::ChromaFormat::C422 => Chroma::C422,
        cfa4_dirac::ChromaFormat::C420 => Chroma::C420,
    };
    let bits = p.luma_depth.max(p.color_diff_depth).clamp(8, 16);
    let [y, c1, c2] = p.planes;
    // Scale a component of `depth` bits to the frame's depth.
    let scale = |plane: Vec<u16>, depth: u32| -> Vec<u16> {
        let up = bits.saturating_sub(depth);
        if up == 0 { plane } else { plane.into_iter().map(|s| s << up).collect() }
    };
    let planes = [scale(y, p.luma_depth), scale(c1, p.color_diff_depth), scale(c2, p.color_diff_depth)];
    let data = if bits == 8 {
        let to8 = |v: Vec<u16>| Arc::new(v.into_iter().map(|s| s.min(255) as u8).collect::<Vec<u8>>());
        let [y, cb, cr] = planes;
        PixelData::Yuv8 { planes: [to8(y), to8(cb), to8(cr)], chroma, alpha: None }
    } else {
        let [y, cb, cr] = planes;
        PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(cb), Arc::new(cr)], chroma, bits, alpha: None }
    };
    let par = (u32::try_from(v.pixel_aspect_ratio.0).unwrap_or(1), u32::try_from(v.pixel_aspect_ratio.1).unwrap_or(1));
    let par = if par.0 == 0 || par.1 == 0 { (1, 1) } else { par };
    let color = crate::stream_color::resolve(p.width, p.height, &[color_codes(v)]);
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par, pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

#[derive(Default)]
pub struct DiracDecoder {
    dec: cfa4_dirac::Decoder,
}

impl VideoDecoder for DiracDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pictures = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        let Some(seq) = self.dec.sequence() else { return Ok(Vec::new()) };
        let video = seq.video.clone();
        pictures.into_iter().map(|p| Ok(DecodedFrame { pts, frame: convert(p, &video)?, draft: false })).collect()
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec.reset();
    }
    fn name(&self) -> &str {
        "CFA4 VC-2"
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        // VC-2 pictures are all intra
        Some(cfa4_dirac::is_vc2(sample))
    }
}

pub fn dirac_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if e.format != DRAC {
        return None;
    }
    Some(Ok(Box::new(DiracDecoder::default())))
}
