//! The pictures and sound that travel through the frame ring (CFA4-DECKLINK-READY-001 V2.3, V2.4).
//!
//! A [`Frame`] is one picture as bytes plus its [`PixelFormat`]; the ring stores it unchanged and
//! [`Frame::to_video_frame`] turns it into the engine's `cfa4_frame::VideoFrame`.
//!
//! Layouts (little-endian, rows from the top):
//!
//! | format | code | bytes per frame (`w × h`) | layout |
//! |---|---|---|---|
//! | `Bgra8` | 1 | `stride × h`, `stride ≥ 4w` | packed B, G, R, A |
//! | `Rgba8` | 2 | `stride × h`, `stride ≥ 4w` | packed R, G, B, A (straight alpha) |
//! | `Yuv422P10` | 3 | `4 w h` (`w` even) | three planes of `u16`: Y (`w × h`), Cb and Cr (`w/2 × h`), 10 significant bits, limited range |
//! | `Rgb12P` | 4 | `6 w h` | three planes of `u16`: R, G, B (`w × h`), 12 significant bits, full range |
//! | `Uyvy8` | 5 | `stride × h`, `stride ≥ 2w` (`w` even) | packed Cb, Y0, Cr, Y1, limited range |
//!
//! The planar formats are stored tightly packed, so their `stride` is the byte length of one luma
//! (or R) row: `2 w`.

use cfa4_color::ColorInfo;
use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_time::Tick;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Pixel layout of a frame in the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixelFormat {
    /// 8-bit B, G, R, A per pixel, rows `stride` bytes apart.
    Bgra8,
    /// 8-bit R, G, B, A (straight alpha, sRGB) per pixel, rows `stride` bytes apart.
    Rgba8,
    /// Planar 4:2:2, 10 significant bits in `u16` (the layout of `cfa4_frame::PixelData::Yuv16`).
    Yuv422P10,
    /// Planar R, G, B, 12 significant bits in `u16`.
    Rgb12P,
    /// Packed 8-bit 4:2:2, Cb Y0 Cr Y1.
    Uyvy8,
}

impl PixelFormat {
    /// Code stored in the frame ring.
    pub fn code(self) -> u32 {
        match self {
            PixelFormat::Bgra8 => 1,
            PixelFormat::Rgba8 => 2,
            PixelFormat::Yuv422P10 => 3,
            PixelFormat::Rgb12P => 4,
            PixelFormat::Uyvy8 => 5,
        }
    }

    /// Format from a ring code (`None` for a code this version does not know).
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(PixelFormat::Bgra8),
            2 => Some(PixelFormat::Rgba8),
            3 => Some(PixelFormat::Yuv422P10),
            4 => Some(PixelFormat::Rgb12P),
            5 => Some(PixelFormat::Uyvy8),
            _ => None,
        }
    }

    /// Whether the planes are stored tightly packed one after the other.
    pub fn is_planar(self) -> bool {
        matches!(self, PixelFormat::Yuv422P10 | PixelFormat::Rgb12P)
    }

    /// Bytes of one pixel of the luma (or first) row: the packed pixel size, or the sample size
    /// of a plane.
    pub fn bytes_per_pixel(self) -> u32 {
        match self {
            PixelFormat::Bgra8 | PixelFormat::Rgba8 => 4,
            PixelFormat::Yuv422P10 | PixelFormat::Rgb12P | PixelFormat::Uyvy8 => 2,
        }
    }

    /// Whether the width must be even (4:2:2 subsampling).
    pub fn needs_even_width(self) -> bool {
        matches!(self, PixelFormat::Yuv422P10 | PixelFormat::Uyvy8)
    }

    /// Bytes of one tightly packed frame of `width × height`.
    pub fn bytes_per_frame(self, width: u32, height: u32) -> u64 {
        let px = u64::from(width) * u64::from(height);
        match self {
            PixelFormat::Bgra8 | PixelFormat::Rgba8 => px * 4,
            PixelFormat::Yuv422P10 => px * 4,
            PixelFormat::Rgb12P => px * 6,
            PixelFormat::Uyvy8 => px * 2,
        }
    }

    /// The stored length of a frame with this `stride`, or `None` when `stride` or `width` do not
    /// fit the format (a stride below the row size, a planar stride other than the luma row, an
    /// odd width of a 4:2:2 format).
    pub fn frame_len(self, width: u32, height: u32, stride: u32) -> Option<u64> {
        if self.needs_even_width() && !width.is_multiple_of(2) {
            return None;
        }
        let row = u64::from(width) * u64::from(self.bytes_per_pixel());
        if u64::from(stride) < row || (self.is_planar() && u64::from(stride) != row) {
            return None;
        }
        Some(if self.is_planar() { self.bytes_per_frame(width, height) } else { u64::from(stride) * u64::from(height) })
    }
}

/// One picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per row in `data` (the luma row for a planar format).
    pub stride: u32,
    /// Pixel layout.
    pub format: PixelFormat,
    /// Presentation time from the producer's clock, in nanoseconds.
    pub pts_ns: u64,
    /// The pixels, `PixelFormat::frame_len` bytes.
    pub data: Vec<u8>,
}

impl Frame {
    /// Whether `data` has the length the format, size and stride call for.
    pub fn is_consistent(&self) -> bool {
        self.format.frame_len(self.width, self.height, self.stride) == Some(self.data.len() as u64)
    }

    /// The picture as the engine's frame, or `None` when the frame is not consistent. BGRA and
    /// RGBA become straight sRGB `Rgba8`; the YUV formats become `Yuv16` / `Yuv8` 4:2:2 tagged
    /// Rec. 709 limited range; `Rgb12P` becomes `Rgb16` with 12 bits.
    pub fn to_video_frame(&self) -> Option<VideoFrame> {
        if !self.is_consistent() {
            return None;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = self.stride as usize;
        let data = match self.format {
            PixelFormat::Bgra8 | PixelFormat::Rgba8 => {
                let mut out = Vec::with_capacity(w * h * 4);
                for row in self.data.chunks_exact(stride).take(h) {
                    for px in row.get(..w * 4)?.chunks_exact(4) {
                        if let [a, b, c, d] = *px {
                            if self.format == PixelFormat::Bgra8 {
                                out.extend_from_slice(&[c, b, a, d]);
                            } else {
                                out.extend_from_slice(&[a, b, c, d]);
                            }
                        }
                    }
                }
                PixelData::Rgba8(Arc::new(out))
            }
            PixelFormat::Yuv422P10 => {
                let words: Vec<u16> = self.data.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
                let (y, rest) = words.split_at(w * h);
                let (cb, cr) = rest.split_at(w / 2 * h);
                PixelData::Yuv16 { planes: [Arc::new(y.to_vec()), Arc::new(cb.to_vec()), Arc::new(cr.to_vec())], chroma: Chroma::C422, bits: 10, alpha: None }
            }
            PixelFormat::Rgb12P => {
                let words: Vec<u16> = self.data.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
                let (r, rest) = words.split_at(w * h);
                let (g, b) = rest.split_at(w * h);
                PixelData::Rgb16 { planes: [Arc::new(r.to_vec()), Arc::new(g.to_vec()), Arc::new(b.to_vec())], bits: 12, alpha: None }
            }
            PixelFormat::Uyvy8 => {
                let (mut y, mut cb, mut cr) = (Vec::with_capacity(w * h), Vec::with_capacity(w / 2 * h), Vec::with_capacity(w / 2 * h));
                for row in self.data.chunks_exact(stride).take(h) {
                    for q in row.get(..w * 2)?.chunks_exact(4) {
                        if let [u, y0, v, y1] = *q {
                            cb.push(u);
                            y.push(y0);
                            cr.push(v);
                            y.push(y1);
                        }
                    }
                }
                PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(cb), Arc::new(cr)], chroma: Chroma::C422, alpha: None }
            }
        };
        let color = if matches!(self.format, PixelFormat::Rgb12P | PixelFormat::Bgra8 | PixelFormat::Rgba8) { ColorInfo::SRGB_FULL } else { ColorInfo::REC709 };
        Some(VideoFrame { width: self.width, height: self.height, data, color, par: (1, 1), pts: Tick::ZERO, layout: Default::default() })
    }
}

/// Sample layout of the audio block of a ring slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// Signed 32-bit integer PCM, little-endian, the channels of one sample frame interleaved
    /// (the integer PCM of the parent plan's `CapturedFrame`).
    S32,
}

impl AudioFormat {
    /// Code stored in the ring.
    pub fn code(self) -> u16 {
        match self {
            AudioFormat::S32 => 1,
        }
    }

    /// Format from a ring code.
    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(AudioFormat::S32),
            _ => None,
        }
    }

    /// Bytes of one sample of one channel.
    pub fn bytes_per_sample(self) -> u32 {
        4
    }
}

/// The audio that belongs to one picture: `data` holds whole sample frames of `channels`
/// interleaved channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audio {
    /// Channels per sample frame (1..=[`MAX_AUDIO_CHANNELS`]).
    pub channels: u16,
    /// Sample layout.
    pub format: AudioFormat,
    /// Interleaved samples.
    pub data: Vec<u8>,
}

/// Most audio channels in a ring slot.
pub const MAX_AUDIO_CHANNELS: u16 = 64;

impl Audio {
    /// Whether the channel count is in range and `data` is a whole number of sample frames.
    pub fn is_consistent(&self) -> bool {
        let frame = u64::from(self.channels) * u64::from(self.format.bytes_per_sample());
        (1..=MAX_AUDIO_CHANNELS).contains(&self.channels) && !self.data.is_empty() && (self.data.len() as u64).is_multiple_of(frame)
    }

    /// Sample frames in the block.
    pub fn sample_frames(&self) -> usize {
        self.data.len() / (self.channels.max(1) as usize * self.format.bytes_per_sample() as usize)
    }
}

/// Converts a BGRA frame to tightly packed RGBA (the format of the engine's `frame.render`).
pub fn bgra_to_rgba(frame: &Frame) -> Vec<u8> {
    let row = frame.width as usize * 4;
    let mut out = Vec::with_capacity(row.saturating_mul(frame.height as usize));
    for r in 0..frame.height as usize {
        let start = r.saturating_mul(frame.stride as usize);
        let Some(src) = frame.data.get(start..start.saturating_add(row)) else { break };
        for px in src.chunks_exact(4) {
            if let [b, g, r, a] = *px {
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_per_format() {
        assert_eq!(PixelFormat::Yuv422P10.bytes_per_frame(4, 2), 32);
        assert_eq!(PixelFormat::Rgb12P.bytes_per_frame(4, 2), 48);
        assert_eq!(PixelFormat::Uyvy8.bytes_per_frame(4, 2), 16);
        assert_eq!(PixelFormat::Bgra8.frame_len(4, 2, 20), Some(40));
        assert_eq!(PixelFormat::Bgra8.frame_len(4, 2, 12), None, "stride below the row");
        assert_eq!(PixelFormat::Yuv422P10.frame_len(4, 2, 8), Some(32));
        assert_eq!(PixelFormat::Yuv422P10.frame_len(4, 2, 12), None, "planar strides are the luma row");
        assert_eq!(PixelFormat::Yuv422P10.frame_len(5, 2, 10), None, "4:2:2 needs an even width");
        for f in [PixelFormat::Bgra8, PixelFormat::Rgba8, PixelFormat::Yuv422P10, PixelFormat::Rgb12P, PixelFormat::Uyvy8] {
            assert_eq!(PixelFormat::from_code(f.code()), Some(f));
        }
        assert_eq!(PixelFormat::from_code(0), None);
        assert_eq!(PixelFormat::from_code(99), None);
    }

    #[test]
    fn yuv422p10_matches_the_engines_yuv16() {
        let (w, h) = (4u32, 2u32);
        let words: Vec<u16> = (0..16).map(|v| v * 60).collect();
        let data: Vec<u8> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
        let f = Frame { width: w, height: h, stride: w * 2, format: PixelFormat::Yuv422P10, pts_ns: 0, data };
        let v = f.to_video_frame().expect("converts");
        match v.data {
            PixelData::Yuv16 { planes, chroma, bits, alpha } => {
                assert_eq!((chroma, bits, alpha.is_none()), (Chroma::C422, 10, true));
                assert_eq!(planes[0].as_slice(), &words[0..8]);
                assert_eq!(planes[1].as_slice(), &words[8..12]);
                assert_eq!(planes[2].as_slice(), &words[12..16]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rgb12p_uyvy_and_bgra_convert() {
        let words: Vec<u16> = (0..12).map(|v| v * 300).collect();
        let data: Vec<u8> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
        let f = Frame { width: 2, height: 2, stride: 4, format: PixelFormat::Rgb12P, pts_ns: 0, data };
        assert!(matches!(f.to_video_frame().expect("rgb").data, PixelData::Rgb16 { bits: 12, .. }));
        let f = Frame { width: 2, height: 1, stride: 4, format: PixelFormat::Uyvy8, pts_ns: 0, data: vec![10, 20, 30, 40] };
        match f.to_video_frame().expect("uyvy").data {
            PixelData::Yuv8 { planes, .. } => {
                assert_eq!((planes[0].as_slice(), planes[1].as_slice(), planes[2].as_slice()), (&[20, 40][..], &[10][..], &[30][..]))
            }
            other => panic!("unexpected {other:?}"),
        }
        let f = Frame { width: 1, height: 1, stride: 4, format: PixelFormat::Bgra8, pts_ns: 0, data: vec![1, 2, 3, 4] };
        match f.to_video_frame().expect("bgra").data {
            PixelData::Rgba8(d) => assert_eq!(d.as_slice(), &[3, 2, 1, 4]),
            other => panic!("unexpected {other:?}"),
        }
        let bad = Frame { width: 1, height: 1, stride: 4, format: PixelFormat::Bgra8, pts_ns: 0, data: vec![1] };
        assert!(bad.to_video_frame().is_none());
    }

    #[test]
    fn audio_blocks_are_checked() {
        let ok = Audio { channels: 2, format: AudioFormat::S32, data: vec![0; 16] };
        assert!(ok.is_consistent());
        assert_eq!(ok.sample_frames(), 2);
        assert!(!Audio { channels: 2, format: AudioFormat::S32, data: vec![0; 12] }.is_consistent());
        assert!(!Audio { channels: 0, format: AudioFormat::S32, data: vec![0; 4] }.is_consistent());
        assert!(!Audio { channels: 65, format: AudioFormat::S32, data: vec![0; 260] }.is_consistent());
        assert_eq!(AudioFormat::from_code(AudioFormat::S32.code()), Some(AudioFormat::S32));
        assert_eq!(AudioFormat::from_code(9), None);
    }

    #[test]
    fn bgra_becomes_rgba() {
        let f = Frame { width: 2, height: 1, stride: 12, format: PixelFormat::Bgra8, pts_ns: 0, data: vec![1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0] };
        assert_eq!(bgra_to_rgba(&f), vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
