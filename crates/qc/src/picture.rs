//! Picture analysis (plan §4.4): black pictures, luma clipped to black or white, and a contact
//! sheet of evenly spaced frames for the human review.
//!
//! Like the audio analysis it runs on the pictures the integrity decode returns, so the file is
//! read once. Only the luma plane is measured: 8-bit to 16-bit Y'CbCr directly, RGB through its
//! Rec. 709 luma.

use std::panic::{AssertUnwindSafe, catch_unwind};

use cfa4_color::Range;
use cfa4_frame::{PixelData, VideoFrame};

use crate::inspect::{Findings, Found};

/// A luma sample at most this far above black (as a fraction of the black–white range) is black
/// (as in `ffmpeg blackdetect`, `pix_th = 0.10`).
pub const BLACK_PIXEL: f64 = 0.10;
/// A picture is black when this share of its luma samples is black (`pic_th = 0.98`).
pub const BLACK_PICTURE: f64 = 0.98;
/// Frames on the contact sheet.
pub const SHEET_FRAMES: usize = 12;
pub const SHEET_COLUMNS: usize = 4;
/// Largest thumbnail side, in pixels.
pub const THUMB: usize = 160;
const GAP: usize = 4;
const BACKGROUND: [u8; 4] = [24, 24, 24, 255];

/// A run of black pictures, in presentation order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackRun {
    pub first: u64,
    pub frames: u64,
    pub start_ms: Option<f64>,
    pub end_ms: Option<f64>,
}

/// The contact sheet: a PNG and the frames on it, in reading order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContactSheet {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub frames: Vec<(u64, Option<f64>)>,
}

/// What the picture analysis found.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PictureFacts {
    pub analysed: u64,
    pub luma_samples: u64,
    /// Luma samples below the black limit of [`limits`].
    pub below_black: u64,
    /// Luma samples above the white limit of [`limits`].
    pub above_white: u64,
    /// Worst picture for each, `(frame, percent, ms)`.
    pub worst_black: Option<(u64, f64, Option<f64>)>,
    pub worst_white: Option<(u64, f64, Option<f64>)>,
    pub black_frames: u64,
    /// Runs of black pictures (at most [`crate::inspect::MAX_KEPT`]; `black_runs_total` counts all).
    pub black_runs: Vec<BlackRun>,
    pub black_runs_total: u64,
    pub full_range: bool,
    pub bits: u32,
    pub sheet: Option<ContactSheet>,
    /// Why there is no contact sheet.
    pub sheet_error: Option<String>,
}

struct Luma {
    n: u64,
    below: u64,
    above: u64,
    black: u64,
}

/// Count the luma samples of a plane. `lo`, `hi`: nominal black and white (they set the black
/// threshold); `under`, `over`: the limits beyond which a sample counts as clipped.
fn count<T: Copy + Into<u32>>(plane: &[T], n: usize, (lo, hi): (u32, u32), (under, over): (f64, f64)) -> Luma {
    let thr = f64::from(lo) + BLACK_PIXEL * f64::from(hi.saturating_sub(lo));
    let mut l = Luma { n: 0, below: 0, above: 0, black: 0 };
    for v in plane.iter().take(n).map(|v| f64::from((*v).into())) {
        l.n += 1;
        if v < under {
            l.below += 1;
        }
        if v > over {
            l.above += 1;
        }
        if v <= thr {
            l.black += 1;
        }
    }
    l
}

/// Nominal black and white and the clipping limits for `bits` and the range. Limited range: the
/// EBU R 103 limits, −1 % and 103 % of the black–white range (8-bit: below 13.8, above 241.6),
/// so lossy-coding ripple around legal black and white is not counted. Full range: the first and
/// last code.
pub fn limits(bits: u32, full: bool) -> ((u32, u32), (f64, f64)) {
    let bits = bits.clamp(8, 16);
    let max = (1u32 << bits) - 1;
    if full {
        return ((0, max), (0.5, f64::from(max) - 0.5));
    }
    let scale = f64::from(1u32 << (bits - 8));
    let (lo, hi) = (16.0 * scale, 235.0 * scale);
    ((lo as u32, hi as u32), (lo - 0.01 * (hi - lo), hi + 0.03 * (hi - lo)))
}

fn luma(f: &VideoFrame) -> (Luma, bool, u32) {
    let n = (f.width as usize).saturating_mul(f.height as usize);
    let full = f.color.range == Range::Full;
    match &f.data {
        PixelData::Yuv8 { planes, .. } => {
            let (nominal, clip) = limits(8, full);
            (count(planes[0].as_slice(), n, nominal, clip), full, 8)
        }
        PixelData::Yuv16 { planes, bits, .. } => {
            let bits = (*bits).clamp(8, 16);
            let (nominal, clip) = limits(bits, full);
            (count(planes[0].as_slice(), n, nominal, clip), full, bits)
        }
        PixelData::Rgba8(_) | PixelData::RgbaF32(_) => {
            let (nominal, clip) = limits(8, true);
            (count(&f.luma8(), n, nominal, clip), true, 8)
        }
    }
}

/// A thumbnail: RGBA, at most [`THUMB`] on its longer side (nearest neighbour).
fn thumbnail(f: &VideoFrame) -> Option<(Vec<u8>, usize, usize)> {
    let (w, h) = (f.width as usize, f.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = catch_unwind(AssertUnwindSafe(|| f.to_rgba8())).ok()?;
    if rgba.len() < w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let (tw, th) = if w >= h { (THUMB.min(w), (h * THUMB.min(w) / w).max(1)) } else { ((w * THUMB.min(h) / h).max(1), THUMB.min(h)) };
    let mut out = vec![0u8; tw * th * 4];
    for y in 0..th {
        let sy = y * h / th;
        for x in 0..tw {
            let sx = x * w / tw;
            let (s, d) = ((sy * w + sx) * 4, (y * tw + x) * 4);
            if let (Some(src), Some(dst)) = (rgba.get(s..s + 4), out.get_mut(d..d + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    Some((out, tw, th))
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Lay the thumbnails out in a grid and encode it.
fn compose(thumbs: &[(u64, Option<f64>, Vec<u8>, usize, usize)]) -> Result<ContactSheet, String> {
    let cols = SHEET_COLUMNS.min(thumbs.len()).max(1);
    let rows = thumbs.len().div_ceil(cols);
    let cw = thumbs.iter().map(|t| t.3).max().unwrap_or(1);
    let ch = thumbs.iter().map(|t| t.4).max().unwrap_or(1);
    let w = cols * cw + (cols + 1) * GAP;
    let h = rows * ch + (rows + 1) * GAP;
    let mut px: Vec<u8> = BACKGROUND.iter().copied().cycle().take(w * h * 4).collect();
    for (k, (_, _, rgba, tw, th)) in thumbs.iter().enumerate() {
        let (cx, cy) = (GAP + (k % cols) * (cw + GAP), GAP + (k / cols) * (ch + GAP));
        // centre the thumbnail in its cell
        let (ox, oy) = (cx + (cw - tw) / 2, cy + (ch - th) / 2);
        for y in 0..*th {
            let (s, d) = (y * tw * 4, ((oy + y) * w + ox) * 4);
            if let (Some(src), Some(dst)) = (rgba.get(s..s + tw * 4), px.get_mut(d..d + tw * 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    let (wu, hu) = (u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?);
    Ok(ContactSheet { png: encode_png(&px, wu, hu)?, width: wu, height: hu, columns: cols as u32, frames: thumbs.iter().map(|t| (t.0, t.1)).collect() })
}

/// Streaming analysis of the decoded pictures, in presentation order.
pub(crate) struct PictureAnalyzer {
    facts: PictureFacts,
    /// Frames for the contact sheet (ascending presentation indices).
    pick: Vec<u64>,
    thumbs: Vec<(u64, Option<f64>, Vec<u8>, usize, usize)>,
    run: Option<BlackRun>,
    last_ms: Option<f64>,
}

impl PictureAnalyzer {
    /// `expected`: how many pictures the stream should have (to spread the contact sheet).
    pub(crate) fn new(expected: usize) -> Self {
        let n = expected as u64;
        let k = SHEET_FRAMES as u64;
        let pick = if n <= k { (0..n).collect() } else { (0..k).map(|i| i * (n - 1) / (k - 1)).collect() };
        PictureAnalyzer { facts: PictureFacts::default(), pick, thumbs: Vec::new(), run: None, last_ms: None }
    }

    /// Analyse picture `index` (presentation order), shown at `ms`.
    pub(crate) fn take(&mut self, f: &VideoFrame, index: u64, ms: Option<f64>) {
        let (l, full, bits) = luma(f);
        let fa = &mut self.facts;
        fa.analysed = fa.analysed.saturating_add(1);
        fa.full_range = full;
        fa.bits = bits;
        fa.luma_samples = fa.luma_samples.saturating_add(l.n);
        fa.below_black = fa.below_black.saturating_add(l.below);
        fa.above_white = fa.above_white.saturating_add(l.above);
        if l.n > 0 {
            let pb = l.below as f64 * 100.0 / l.n as f64;
            let pw = l.above as f64 * 100.0 / l.n as f64;
            if pb > 0.0 && fa.worst_black.is_none_or(|w| pb > w.1) {
                fa.worst_black = Some((index, pb, ms));
            }
            if pw > 0.0 && fa.worst_white.is_none_or(|w| pw > w.1) {
                fa.worst_white = Some((index, pw, ms));
            }
        }
        let black = l.n > 0 && l.black as f64 >= BLACK_PICTURE * l.n as f64;
        if black {
            fa.black_frames = fa.black_frames.saturating_add(1);
            let r = self.run.get_or_insert(BlackRun { first: index, frames: 0, start_ms: ms, end_ms: None });
            r.frames = r.frames.saturating_add(1);
        } else {
            self.close_run(ms);
        }
        self.last_ms = ms;
        if self.pick.binary_search(&index).is_ok()
            && let Some((rgba, w, h)) = thumbnail(f)
        {
            self.thumbs.push((index, ms, rgba, w, h));
        }
    }

    fn close_run(&mut self, end_ms: Option<f64>) {
        if let Some(mut r) = self.run.take() {
            r.end_ms = end_ms;
            self.facts.black_runs_total = self.facts.black_runs_total.saturating_add(1);
            if self.facts.black_runs.len() < crate::inspect::MAX_KEPT {
                self.facts.black_runs.push(r);
            }
        }
    }

    /// `end_ms`: when the last picture ends (the stream's presented end).
    pub(crate) fn finish(mut self, end_ms: Option<f64>) -> PictureFacts {
        self.close_run(end_ms.or(self.last_ms));
        if self.thumbs.is_empty() {
            self.facts.sheet_error = Some("no picture could be turned into a thumbnail".into());
        } else {
            match compose(&self.thumbs) {
                Ok(s) => self.facts.sheet = Some(s),
                Err(e) => self.facts.sheet_error = Some(format!("the contact sheet could not be encoded: {e}")),
            }
        }
        self.facts
    }
}

/// Black runs as findings (for messages): where they start and how long they last.
pub fn black_findings(runs: &[BlackRun]) -> Findings {
    let mut f = Findings::default();
    for r in runs {
        f.push(Found { index: r.first, ms: r.start_ms, end_ms: r.end_ms, message: format!("{} black picture(s)", r.frames) });
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_color::ColorInfo;
    use std::sync::Arc;

    fn frame(y: u8, w: u32, h: u32) -> VideoFrame {
        let n = (w * h) as usize;
        VideoFrame {
            width: w,
            height: h,
            data: PixelData::Yuv8 {
                planes: [Arc::new(vec![y; n]), Arc::new(vec![128; n / 4]), Arc::new(vec![128; n / 4])],
                chroma: cfa4_frame::Chroma::C420,
                alpha: None,
            },
            color: ColorInfo::default(),
            par: (1, 1),
            pts: cfa4_time::Tick(0),
        }
    }

    #[test]
    fn black_runs_clipping_and_sheet() {
        let mut a = PictureAnalyzer::new(20);
        for i in 0..20u64 {
            let y = match i {
                5..=7 => 16,
                10 => 250,
                _ => 120,
            };
            a.take(&frame(y, 32, 18), i, Some(i as f64 * 40.0));
        }
        let f = a.finish(Some(800.0));
        assert_eq!(f.analysed, 20);
        assert_eq!(f.black_frames, 3);
        assert_eq!(f.black_runs, vec![BlackRun { first: 5, frames: 3, start_ms: Some(200.0), end_ms: Some(320.0) }]);
        assert_eq!(f.above_white, 32 * 18);
        assert_eq!(f.worst_white.map(|w| (w.0, w.1)), Some((10, 100.0)));
        assert_eq!(f.below_black, 0, "legal black is not below black");
        let s = f.sheet.unwrap();
        assert_eq!(s.frames.len(), SHEET_FRAMES);
        assert_eq!((s.frames[0].0, s.frames[11].0), (0, 19));
        assert_eq!(&s.png[1..4], b"PNG");
        assert_eq!(s.width as usize, 4 * 32 + 5 * GAP);
    }

    #[test]
    fn short_planes_do_not_crash() {
        let mut f = frame(0, 64, 64);
        f.data = PixelData::Yuv8 { planes: [Arc::new(vec![0; 10]), Arc::new(vec![]), Arc::new(vec![])], chroma: cfa4_frame::Chroma::C420, alpha: None };
        let mut a = PictureAnalyzer::new(1);
        a.take(&f, 0, None);
        let r = a.finish(None);
        assert_eq!(r.luma_samples, 10);
        assert!(r.sheet.is_none() && r.sheet_error.is_some());
    }
}
