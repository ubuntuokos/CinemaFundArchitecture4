// The Theora decoding is ported from FFmpeg n7.1.1 (libavcodec/vp3.c, vp3dsp.c, vp3data.h),
// LGPL-2.1-or-later (`COPYING.LGPL-2.1`), an owner-approved exception (owner decision
// 2026-11-14, general FFmpeg-port permission); the format follows the open Theora specification
// (Xiph.Org, BSD-style). See docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust **Theora** (VP3 family) video decoder: the three header packets, then data packets.
//! A damaged packet is an error, never a panic.

mod bits;
mod dsp;
mod frame;
mod header;
mod vlc;

pub mod synth;

use frame::{Frame, FrameDecoder, Geometry};
pub use header::Info;

/// Errors of a header or a packet.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("truncated packet")]
    Truncated,
    #[error("invalid Theora data: {0}")]
    Invalid(&'static str),
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Chroma subsampling of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    C420,
    C422,
    C444,
}

/// A decoded picture: the visible region, top row first, 8 bits per sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub chroma: Chroma,
    /// Y, Cb and Cr; chroma planes are `width >> shift` rounded up by `height >> shift` rounded up.
    pub planes: [Vec<u8>; 3],
}

/// A Theora stream decoder.
pub struct Decoder {
    info: Info,
    setup: header::Setup,
    frames: FrameDecoder,
    last: Option<Frame>,
    golden: Option<Frame>,
}

impl Decoder {
    /// A decoder from the identification, comment and setup header packets.
    pub fn new(identification: &[u8], comment: &[u8], setup: &[u8]) -> Result<Self> {
        if comment.first() != Some(&0x81) || comment.get(1..7) != Some(&b"theora"[..]) {
            return Err(Error::Invalid("not a Theora comment header"));
        }
        let info = header::parse_info(identification)?;
        let setup = header::parse_setup(setup)?;
        let frames = FrameDecoder::new(&info)?;
        Ok(Self { info, setup, frames, last: None, golden: None })
    }

    /// The identification header.
    pub fn info(&self) -> &Info {
        &self.info
    }

    /// Forgets the reference frames (after a seek): inter packets fail until a key packet.
    pub fn reset(&mut self) {
        self.last = None;
        self.golden = None;
    }

    /// Whether `packet` is a data packet of a key frame.
    pub fn is_key(packet: &[u8]) -> bool {
        packet.first().is_some_and(|b| b & 0xC0 == 0)
    }

    /// Decodes a data packet. An empty packet repeats the previous picture.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Picture> {
        if packet.is_empty() {
            let last = self.last.as_ref().ok_or(Error::Invalid("repeated frame without a previous one"))?;
            return Ok(self.picture(last));
        }
        if packet.first().is_some_and(|b| b & 0x80 != 0) {
            return Err(Error::Invalid("header packet among the data"));
        }
        let (cur, key) = self.frames.decode(&self.setup, packet, self.last.as_ref(), self.golden.as_ref())?;
        let pic = self.picture(&cur);
        if key {
            self.golden = Some(cur.clone());
        }
        self.last = Some(cur);
        Ok(pic)
    }

    /// The visible region of `f`, flipped to top-down order.
    fn picture(&self, f: &Frame) -> Picture {
        let i = &self.info;
        let g: &Geometry = self.frames.geometry();
        let (cx, cy) = match i.chroma {
            Chroma::C420 => (1, 1),
            Chroma::C422 => (1, 0),
            Chroma::C444 => (0, 0),
        };
        let mut planes: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (p, out) in planes.iter_mut().enumerate() {
            let (sx, sy) = if p == 0 { (0, 0) } else { (cx, cy) };
            let (stride, _) = g.plane_size(p);
            let x0 = (i.pic_x >> sx) as usize;
            let y0 = (i.pic_y >> sy) as usize;
            let w = (i.pic_width as usize).div_ceil(1 << sx);
            let h = (i.pic_height as usize).div_ceil(1 << sy);
            let src = f.planes.get(p).map_or(&[][..], |v| v.as_slice());
            out.reserve(w * h);
            for r in 0..h {
                let row = y0 + h - 1 - r;
                let start = row * stride + x0;
                match src.get(start..start + w) {
                    Some(s) => out.extend_from_slice(s),
                    None => out.extend(std::iter::repeat_n(0, w)),
                }
            }
        }
        Picture { width: i.pic_width, height: i.pic_height, chroma: i.chroma, planes }
    }
}
