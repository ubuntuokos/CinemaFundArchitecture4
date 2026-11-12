//! Our pure-Rust Dirac / VC-2 decoder (`cfa4-dirac`: SMPTE ST 2042-1 VC-2 and the Dirac core
//! syntax with motion compensation): QuickTime / MP4 and AVI `drac`, Matroska `V_DIRAC`.

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
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par, pts: cfa4_time::Tick::ZERO })
}

/// Pictures held for reordering at most: Dirac codes pictures out of display order, and the
/// smallest held picture number goes out once more are waiting (FFmpeg's `MAX_DELAY`).
const MAX_DELAY: usize = 4;

#[derive(Default)]
pub struct DiracDecoder {
    dec: cfa4_dirac::Decoder,
    /// Decoded pictures (picture number, frame) waiting for their turn in display order.
    held: Vec<(u32, VideoFrame)>,
    /// The pts of the samples of the held pictures. Pictures go out with the smallest, so the
    /// times come out in order whether the container gives presentation times (MP4, Matroska)
    /// or decode-order times (AVI, raw streams).
    times: Vec<i64>,
    /// The picture number shown next; `None` before the first picture.
    next: Option<u32>,
    /// An intra picture was decoded since the last reset (inter pictures before it are dropped).
    started: bool,
}

impl DiracDecoder {
    fn hold(&mut self, number: u32, pts: i64, frame: VideoFrame) {
        self.next.get_or_insert(number);
        self.held.push((number, frame));
        self.times.push(pts);
    }

    /// The held picture to show now: the one numbered `next`, or the earliest when `all` or
    /// more than `MAX_DELAY` are waiting.
    fn pop(&mut self, all: bool) -> Option<DecodedFrame> {
        let next = self.next?;
        // display order relative to `next`, with picture numbers wrapping at 2^32
        let order = |n: u32| n.wrapping_sub(next) as i32;
        let i = match self.held.iter().position(|h| order(h.0) <= 0) {
            Some(i) => i,
            None if all || self.held.len() > MAX_DELAY => (0..self.held.len()).min_by_key(|&i| self.held.get(i).map_or(i32::MAX, |h| order(h.0)))?,
            None => return None,
        };
        let (number, frame) = self.held.remove(i);
        if order(number) >= 0 {
            self.next = Some(number.wrapping_add(1));
        }
        let t = (0..self.times.len()).min_by_key(|&k| self.times.get(k).copied().unwrap_or(i64::MAX))?;
        let pts = self.times.remove(t);
        Some(DecodedFrame { pts, frame, draft: false })
    }
}

impl VideoDecoder for DiracDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if !self.started && cfa4_dirac::has_inter_picture(sample) && !cfa4_dirac::is_random_access(sample) {
            // after a seek: nothing to predict from until an intra picture
            return Ok(Vec::new());
        }
        let pictures = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        let Some(seq) = self.dec.sequence() else { return Ok(Vec::new()) };
        let video = seq.video.clone();
        if !cfa4_dirac::is_core_syntax(sample) {
            // VC-2: display order already
            let mut out: Vec<DecodedFrame> = std::iter::from_fn(|| self.pop(true)).collect();
            for p in pictures {
                out.push(DecodedFrame { pts, frame: convert(p, &video)?, draft: false });
            }
            return Ok(out);
        }
        for p in pictures {
            self.started = true;
            let number = p.picture_number;
            self.hold(number, pts, convert(p, &video)?);
        }
        Ok(std::iter::from_fn(|| self.pop(false)).collect())
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        std::iter::from_fn(|| self.pop(true)).collect()
    }
    fn reset(&mut self) {
        self.dec.reset();
        self.held.clear();
        self.times.clear();
        self.next = None;
        self.started = false;
    }
    fn name(&self) -> &str {
        "CFA4 Dirac / VC-2"
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(cfa4_dirac::is_random_access(sample))
    }
    fn is_disposable(&self, sample: &[u8]) -> bool {
        cfa4_dirac::is_non_reference(sample)
    }
}

pub fn dirac_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if e.format != DRAC {
        return None;
    }
    Some(Ok(Box::new(DiracDecoder::default())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(n: u8) -> VideoFrame {
        VideoFrame::rgba8(1, 1, vec![n, 0, 0, 255])
    }

    /// Feed (picture number, sample pts) in decode order; the (picture number, pts) shown after
    /// each.
    fn run(d: &mut DiracDecoder, order: &[(u32, i64)]) -> Vec<Vec<(u32, i64)>> {
        let shown = |f: DecodedFrame| match &f.frame.data {
            PixelData::Rgba8(p) => (u32::from(p.first().copied().unwrap_or(0)), f.pts),
            _ => (u32::MAX, f.pts),
        };
        let mut out: Vec<Vec<(u32, i64)>> = order
            .iter()
            .map(|&(n, pts)| {
                d.hold(n, pts, frame(n as u8));
                std::iter::from_fn(|| d.pop(false)).map(shown).collect()
            })
            .collect();
        out.push(d.flush().into_iter().map(shown).collect());
        out
    }

    #[test]
    fn pictures_come_out_in_display_order_with_their_pts() {
        // I0 P2 B1 P4 B3 with presentation times: each picture keeps the pts of its own sample
        let mut d = DiracDecoder::default();
        let got = run(&mut d, &[(0, 0), (2, 20), (1, 10), (4, 40), (3, 30)]);
        assert_eq!(got, vec![vec![(0, 0)], vec![], vec![(1, 10), (2, 20)], vec![], vec![(3, 30), (4, 40)], vec![]]);
        // the same with decode-order times (AVI): the times stay in order
        let mut d = DiracDecoder::default();
        let got = run(&mut d, &[(0, 0), (2, 10), (1, 20), (4, 30), (3, 40)]);
        assert_eq!(got, vec![vec![(0, 0)], vec![], vec![(1, 10), (2, 20)], vec![], vec![(3, 30), (4, 40)], vec![]]);
        // a gap in the numbers (a lost picture) costs at most MAX_DELAY pictures of delay
        let mut d = DiracDecoder::default();
        let got = run(&mut d, &[(10, 0), (12, 2), (13, 3), (14, 4), (15, 5), (16, 6), (17, 7)]);
        let flat: Vec<u32> = got.iter().flatten().map(|s| s.0).collect();
        assert_eq!(flat, [10, 12, 13, 14, 15, 16, 17]);
        assert_eq!(got[5], vec![(12, 2), (13, 3), (14, 4), (15, 5), (16, 6)]);
        // numbers wrap at 2^32
        let mut d = DiracDecoder::default();
        let got = run(&mut d, &[(u32::MAX, 0), (1, 2), (0, 1)]);
        let flat: Vec<u32> = got.iter().flatten().map(|s| s.0).collect();
        assert_eq!(flat, [255, 0, 1]); // the test frames carry the number's low byte
        // reset forgets held pictures
        let mut d = DiracDecoder::default();
        d.hold(5, 0, frame(5));
        d.hold(7, 0, frame(7));
        d.reset();
        assert!(d.flush().is_empty());
    }
}
