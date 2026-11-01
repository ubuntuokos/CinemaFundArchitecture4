//! DV video decoder: consumer DV / DVCAM (IEC 61834-2, 25 Mbit/s) and DVCPRO 25 (SMPTE 314M).
//!
//! - 525/60 4:1:1 (720x480) and 625/50 4:2:0 (720x576) DV, DVCPRO 25 4:1:1 at both line
//!   counts; 8-8 and 2-4-8 (interlaced) DCT; 4:3 and 16:9.
//! - Input: one DIF frame (from a raw `.dv` / `.dif` file, an AVI, MOV or MXF sample). Output:
//!   planar 8-bit Y, Cb, Cr at the coded size (no setup / range change; DV is bottom field
//!   first).
//! - DVCPRO 50 and DVCPRO HD are recognised by [`probe`] but not decoded yet.
//!
//! ```no_run
//! # let data: Vec<u8> = vec![];
//! let mut dec = cfa4_dv::Decoder::new();
//! for range in cfa4_dv::frames(&data)? {
//!     let pic = dec.decode(&data[range])?;
//!     println!("{}x{} {:?}", pic.width, pic.height, pic.profile.chroma);
//! }
//! # Ok::<(), cfa4_dv::Error>(())
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod dif;
mod idct;
mod layout;
mod quant;
mod segment;
mod vlc;

use std::ops::Range;

pub use dif::{Aspect, ChromaFormat, Format, Profile, System, probe};

use dif::{BLOCK_SIZE, SEQUENCE_SIZE};
use layout::Shape;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a DV frame (no DIF header block)")]
    NotDv,
    #[error("truncated DV frame: {got} of {need} bytes")]
    Truncated { need: usize, got: usize },
    #[error("unsupported DV format: {0:?} (decoding of DVCPRO 50 and DVCPRO HD is not implemented yet)")]
    Unsupported(Format),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A decoded frame.
#[derive(Clone, Debug)]
pub struct Picture {
    pub profile: Profile,
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
    /// Macroblocks replaced by mid-grey because their DIF block was missing or damaged.
    pub concealed: u32,
    /// Coding errors (invalid codes, blocks without end of block) found in the frame.
    pub errors: u32,
}

impl Picture {
    pub fn chroma_width(&self) -> u32 {
        self.width >> self.profile.chroma.shifts().0
    }
    pub fn chroma_height(&self) -> u32 {
        self.height >> self.profile.chroma.shifts().1
    }

    /// 4:1:1 chroma upsampled horizontally to 4:2:2 (`width / 2` × `height` per plane), as
    /// `(cb, cr)`; `None` for other chroma formats. CFA4 has no 4:1:1 frame type (owner
    /// decision, 2026-11-01), so 4:1:1 pictures enter the engine as 4:2:2. Every other 4:2:2
    /// sample is a 4:1:1 sample (both are co-sited with the even luma samples); the ones in
    /// between are the rounded mean of their neighbours, the last one repeats its left neighbour.
    pub fn chroma_422(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        if self.profile.chroma != ChromaFormat::Yuv411 {
            return None;
        }
        let (src_w, h) = (self.chroma_width() as usize, self.chroma_height() as usize);
        let up = |plane: &[u8]| -> Vec<u8> {
            let mut out = Vec::with_capacity(src_w * 2 * h);
            for row in plane.chunks_exact(src_w.max(1)).take(h) {
                for (i, &a) in row.iter().enumerate() {
                    let b = row.get(i + 1).copied().unwrap_or(a);
                    out.push(a);
                    out.push((u16::from(a) + u16::from(b)).div_ceil(2) as u8);
                }
            }
            out
        };
        Some((up(&self.cb), up(&self.cr)))
    }
}

/// Byte ranges of the whole frames in a raw DIF stream (`.dv`, `.dif`). The frame size comes
/// from the first frame; a trailing partial frame is left out.
pub fn frames(data: &[u8]) -> Result<Vec<Range<usize>>> {
    let p = probe(data).ok_or(Error::NotDv)?;
    let size = p.frame_size();
    if size == 0 {
        return Err(Error::Unsupported(p.format));
    }
    Ok((0..data.len() / size).map(|i| i * size..(i + 1) * size).collect())
}

/// DV frame decoder. Frames are independent (intra only), so one decoder can decode frames in
/// any order.
#[derive(Debug)]
pub struct Decoder {
    threads: bool,
    errors: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Mid-grey for concealed macroblocks.
const CONCEAL: u8 = 128;

impl Decoder {
    pub fn new() -> Self {
        Self { threads: cfg!(feature = "threads"), errors: 0 }
    }

    /// Decode the video segments in parallel (feature `threads`; ignored without it).
    pub fn set_threads(&mut self, on: bool) {
        self.threads = on && cfg!(feature = "threads");
    }

    /// Total coding errors and concealed macroblocks seen so far.
    pub fn errors(&self) -> u64 {
        self.errors
    }

    /// Decode one DIF frame.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Picture> {
        let profile = probe(frame).ok_or(Error::NotDv)?;
        if !profile.is_supported() {
            return Err(Error::Unsupported(profile.format));
        }
        let need = profile.frame_size();
        if frame.len() < need {
            return Err(Error::Truncated { need, got: frame.len() });
        }
        let pic = decode_frame(frame, profile, self.threads);
        self.errors = self.errors.saturating_add(u64::from(pic.errors));
        Ok(pic)
    }
}

/// The pixels of one decoded macroblock.
struct MbPixels {
    x: usize,
    y: usize,
    shape: Shape,
    /// Y0-Y3, Cr, Cb (8x8 each, row-major).
    blocks: [[u8; 64]; 6],
}

fn decode_segment(frame: &[u8], profile: &Profile, seq: usize, k: usize) -> (Vec<MbPixels>, u32, u32) {
    let table = vlc::table();
    // DIF block of the segment's first macroblock: skip header, subcode, VAUX (6 blocks) and
    // one audio block per group of three segments
    let first = seq * SEQUENCE_SIZE + (6 + (k / 3) * 16 + 1 + (k % 3) * 5) * BLOCK_SIZE;
    let data = frame.get(first..first + 5 * BLOCK_SIZE).unwrap_or(&[]);
    let seg = segment::decode(data, seq, table);
    let mut concealed = 0;
    let mut out = Vec::with_capacity(5);
    for (m, mb) in seg.mbs.iter().enumerate() {
        let (x, y, shape) = layout::place(profile.chroma, profile.sequences, seq, k, m);
        let blocks = if mb.concealed {
            concealed += 1;
            [[CONCEAL; 64]; 6]
        } else {
            std::array::from_fn(|b| {
                let blk = &mb.blocks[b];
                let c = quant::dequantise(blk.dc, &blk.amps, blk.mode248, blk.class, mb.qno);
                if blk.mode248 { idct::idct_248(&c) } else { idct::idct_88(&c) }
            })
        };
        out.push(MbPixels { x, y, shape, blocks });
    }
    (out, seg.errors, concealed)
}

fn put(plane: &mut [u8], stride: usize, x: usize, y: usize, w: usize, h: usize, src: &[u8; 64], src_x: usize) {
    for r in 0..h {
        let (Some(dst), Some(s)) = (plane.get_mut((y + r) * stride + x..(y + r) * stride + x + w), src.get(r * 8 + src_x..r * 8 + src_x + w)) else {
            continue;
        };
        dst.copy_from_slice(s);
    }
}

fn decode_frame(frame: &[u8], profile: Profile, threads: bool) -> Picture {
    let (w, h) = (profile.width as usize, profile.height as usize);
    let (sx, sy) = profile.chroma.shifts();
    let (cw, ch) = (w >> sx, h >> sy);
    let jobs: Vec<(usize, usize)> = (0..profile.sequences).flat_map(|s| (0..27).map(move |k| (s, k))).collect();
    let run = |&(s, k): &(usize, usize)| decode_segment(frame, &profile, s, k);
    #[cfg(feature = "threads")]
    let results: Vec<_> = if threads {
        use rayon::prelude::*;
        jobs.par_iter().map(run).collect()
    } else {
        jobs.iter().map(run).collect()
    };
    #[cfg(not(feature = "threads"))]
    let results: Vec<_> = {
        let _ = threads;
        jobs.iter().map(run).collect()
    };
    let mut pic =
        Picture { profile, width: w as u32, height: h as u32, y: vec![0; w * h], cb: vec![0; cw * ch], cr: vec![0; cw * ch], concealed: 0, errors: 0 };
    for (mbs, errors, concealed) in results {
        pic.errors = pic.errors.saturating_add(errors);
        pic.concealed = pic.concealed.saturating_add(concealed);
        for mb in mbs {
            let [y0, y1, y2, y3, cr, cb] = &mb.blocks;
            let (x, y) = (mb.x, mb.y);
            match mb.shape {
                Shape::Wide411 => {
                    for (i, b) in [y0, y1, y2, y3].into_iter().enumerate() {
                        put(&mut pic.y, w, x + 8 * i, y, 8, 8, b, 0);
                    }
                    put(&mut pic.cr, cw, x / 4, y, 8, 8, cr, 0);
                    put(&mut pic.cb, cw, x / 4, y, 8, 8, cb, 0);
                }
                Shape::Right411 | Shape::Square420 => {
                    put(&mut pic.y, w, x, y, 8, 8, y0, 0);
                    put(&mut pic.y, w, x + 8, y, 8, 8, y1, 0);
                    put(&mut pic.y, w, x, y + 8, 8, 8, y2, 0);
                    put(&mut pic.y, w, x + 8, y + 8, 8, 8, y3, 0);
                    if mb.shape == Shape::Right411 {
                        for (plane, b) in [(&mut pic.cr, cr), (&mut pic.cb, cb)] {
                            put(plane, cw, x / 4, y, 4, 8, b, 0);
                            put(plane, cw, x / 4, y + 8, 4, 8, b, 4);
                        }
                    } else {
                        put(&mut pic.cr, cw, x / 2, y / 2, 8, 8, cr, 0);
                        put(&mut pic.cb, cw, x / 2, y / 2, 8, 8, cb, 0);
                    }
                }
            }
        }
    }
    pic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_dv() {
        let mut d = Decoder::new();
        assert!(matches!(d.decode(&[0u8; 100]), Err(Error::NotDv)));
        assert!(matches!(frames(b"RIFF...."), Err(Error::NotDv)));
    }
}

#[cfg(test)]
mod chroma_tests {
    use super::*;

    fn pic(chroma: ChromaFormat, cb: Vec<u8>, cr: Vec<u8>, w: u32, h: u32) -> Picture {
        let mut head = dif::tests_support::head(0, 1, 0, 0);
        head.resize(6 * 80, 0xff);
        let mut profile = probe(&head).unwrap();
        profile.chroma = chroma;
        Picture { profile, width: w, height: h, y: vec![0; (w * h) as usize], cb, cr, concealed: 0, errors: 0 }
    }

    #[test]
    fn upsample_411_to_422() {
        // 8x2 luma, 2x2 chroma
        let p = pic(ChromaFormat::Yuv411, vec![10, 20, 100, 101], vec![0, 255, 7, 7], 8, 2);
        let (cb, cr) = p.chroma_422().unwrap();
        assert_eq!(cb, vec![10, 15, 20, 20, 100, 101, 101, 101]);
        assert_eq!(cr, vec![0, 128, 255, 255, 7, 7, 7, 7]);
        let p = pic(ChromaFormat::Yuv420, vec![0; 4], vec![0; 4], 4, 4);
        assert!(p.chroma_422().is_none());
    }
}
