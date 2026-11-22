// Ported from FFmpeg n7.1.1: libavformat/thp.c (Copyright (c) 2007 Marco Gerards, (c) 2009 David
// Conrad), libavcodec/adpcm.c (the THP part, Copyright (c) 2001-2003 The FFmpeg project) and
// libavcodec/mjpegdec.c, libavcodec/simple_idct_template.c; all LGPL-2.1-or-later
// (`COPYING.LGPL-2.1`); see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust support for Nintendo **THP** movies (`.thp`, codec backlog CB-28.1).
//!
//! A THP file is a header, a component list (one video and at most one audio) and a chain of
//! frames; each frame holds one JPEG picture and the ADPCM sound of that frame's time.
//!
//! - [`probe`] and [`parse`] read the header and find the packets, as ffmpeg's `thp` demuxer does.
//! - [`jpeg::decode`] decodes a picture (baseline JPEG without byte stuffing and without restart
//!   markers, exactly like ffmpeg's `thp` decoder), bit for bit.
//! - [`decode_audio`] decodes the sound of one frame (THP ADPCM, 4 bits, 14 samples per 8 bytes).

mod idct;
pub mod jpeg;

pub use jpeg::{Chroma, Picture};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("THP data is truncated")]
    Truncated,
    #[error("invalid THP data: {0}")]
    Invalid(&'static str),
    #[error("not a THP file")]
    NotThp,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A byte range of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn of(self, data: &[u8]) -> &[u8] {
        &data[self.start..self.start + self.len]
    }
}

/// The sound a file declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    pub channels: u32,
    pub sample_rate: u32,
    /// Samples per channel the header announces.
    pub duration: u32,
}

/// What the header and the frame chain say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Movie {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    /// Frame rate as ffmpeg derives it from the header's float (`av_d2q`).
    pub fps_num: i64,
    pub fps_den: i64,
    /// Frames the header announces.
    pub frame_count: u32,
    pub video: Vec<Span>,
    pub audio: Option<AudioInfo>,
    /// The sound packet of each frame that has one.
    pub audio_packets: Vec<Span>,
    /// The frame chain stopped making sense before the announced end.
    pub damaged: bool,
}

/// Most frames kept for one file.
const MAX_FRAMES: usize = 1 << 22;

const MAGIC: &[u8; 4] = b"THP\0";

fn be32(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(p..p.checked_add(4)?)?.try_into().ok()?))
}

pub fn probe(head: &[u8]) -> bool {
    head.len() >= 20 && &head[..4] == MAGIC
}

/// ffmpeg's `av_reduce` (positive `max`).
fn reduce(num: i64, den: i64, max: i64) -> (i64, i64) {
    let gcd = {
        let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a as i64
    };
    let sign = (num < 0) != (den < 0);
    let (mut num, mut den) = (num.abs(), den.abs());
    if gcd != 0 {
        num /= gcd;
        den /= gcd;
    }
    let mut a0 = (0i64, 1i64);
    let mut a1 = (1i64, 0i64);
    if num <= max && den <= max {
        a1 = (num, den);
        den = 0;
    }
    while den != 0 {
        let mut x = num / den;
        let next_den = num - den * x;
        let a2 = (x * a1.0 + a0.0, x * a1.1 + a0.1);
        if a2.0 > max || a2.1 > max {
            if a1.0 != 0 {
                x = (max - a0.0) / a1.0;
            }
            if a1.1 != 0 {
                x = x.min((max - a0.1) / a1.1);
            }
            if den * (2 * x * a1.1 + a0.1) > num * a1.1 {
                a1 = (x * a1.0 + a0.0, x * a1.1 + a0.1);
            }
            break;
        }
        a0 = a1;
        a1 = a2;
        num = den;
        den = next_den;
    }
    (if sign { -a1.0 } else { a1.0 }, a1.1)
}

/// ffmpeg's `av_d2q(d, INT_MAX)`.
fn d2q(d: f64) -> (i64, i64) {
    const MAX: i64 = i32::MAX as i64;
    if d.is_nan() {
        return (0, 0);
    }
    if d.abs() > (MAX + 3) as f64 {
        return (if d < 0.0 { -1 } else { 1 }, 0);
    }
    let exponent = if d == 0.0 { 0 } else { (d.abs().log2().floor() as i32).max(0) };
    let den = 1i64 << (61 - exponent);
    reduce((d * den as f64 + 0.5).floor() as i64, den, MAX)
}

/// Read the header and find the packets.
pub fn parse(data: &[u8]) -> Result<Movie> {
    if !probe(data) {
        return Err(Error::NotThp);
    }
    let t = |p: usize| be32(data, p).ok_or(Error::Truncated);
    let mut m = Movie { version: t(4)?, ..Movie::default() };
    let fps = f32::from_bits(t(16)?);
    let (num, den) = d2q(f64::from(fps));
    if den <= 0 || num <= 0 {
        return Err(Error::Invalid("frame rate"));
    }
    (m.fps_num, m.fps_den) = (num, den);
    m.frame_count = t(20)?;
    let first_size = t(24)? as usize;
    let comp_off = t(32)? as usize;
    let first_frame = t(40)? as usize;
    // components
    let count = t(comp_off)? as usize;
    if count > 16 {
        return Err(Error::Invalid("component count"));
    }
    let types = data.get(comp_off + 4..comp_off + 20).ok_or(Error::Truncated)?;
    let mut p = comp_off + 20;
    let mut have_video = false;
    for &ty in types.iter().take(count) {
        match ty {
            0 => {
                if have_video {
                    break;
                }
                have_video = true;
                m.width = t(p)?;
                m.height = t(p + 4)?;
                p += 8;
                if m.version == 0x11000 {
                    p += 4;
                }
            }
            1 => {
                if m.audio.is_some() {
                    break;
                }
                m.audio = Some(AudioInfo { channels: t(p)?, sample_rate: t(p + 4)?, duration: t(p + 8)? });
                p += 12;
            }
            _ => {}
        }
    }
    if !have_video {
        return Err(Error::Invalid("no video component"));
    }
    if m.width == 0 || m.height == 0 || u64::from(m.width) * u64::from(m.height) > jpeg::MAX_PIXELS {
        return Err(Error::Invalid("picture size"));
    }
    // frames
    let mut next = first_frame;
    let mut next_size = first_size;
    for _ in 0..(m.frame_count as usize).min(MAX_FRAMES) {
        let (Some(nsz), Some(size)) = (be32(data, next), be32(data, next + 8)) else {
            m.damaged = true;
            break;
        };
        let mut q = next + 12;
        let mut asize = 0usize;
        if m.audio.is_some() {
            let Some(a) = be32(data, q) else {
                m.damaged = true;
                break;
            };
            asize = a as usize;
            q += 4;
        }
        let size = size as usize;
        if q.checked_add(size).is_none_or(|e| e > data.len()) {
            m.damaged = true;
            break;
        }
        m.video.push(Span { start: q, len: size });
        q += size;
        if m.audio.is_some() {
            if q.checked_add(asize).is_none_or(|e| e > data.len()) {
                m.damaged = true;
                break;
            }
            m.audio_packets.push(Span { start: q, len: asize });
        }
        next += next_size.max(1);
        next_size = nsz as usize;
    }
    Ok(m)
}

/// Samples per channel an audio packet of `len` bytes holds when its header gives no count.
fn estimate(len: usize, ch: usize) -> usize {
    let Some(rest) = len.checked_sub(8 + 36 * ch) else { return 0 };
    let rest = rest / ch;
    rest / 8 * 14 + if rest % 8 > 1 { (rest % 8 - 1) * 2 } else { 0 }
}

/// The decoder's state between packets: the two previous samples of each channel.
#[derive(Clone, Debug, Default)]
pub struct AudioDecoder {
    history: Option<Vec<(i32, i32)>>,
}

impl AudioDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one frame's sound to interleaved 16-bit samples. The first packet sets the history
    /// of every channel; later packets continue from the samples decoded before.
    pub fn decode(&mut self, packet: &[u8], channels: u32) -> Result<Vec<i16>> {
        let ch = channels as usize;
        if ch == 0 || ch > 8 {
            return Err(Error::Invalid("channel count"));
        }
        let est = estimate(packet.len(), ch);
        let coded = be32(packet, 4).ok_or(Error::Truncated)? as i32;
        if est == 0 || coded <= 0 || coded as usize > est {
            return Err(Error::Invalid("sample count"));
        }
        let n = coded as usize;
        let s16 = |p: usize| i32::from(i16::from_be_bytes([packet[p], packet[p + 1]]));
        let mut table = vec![[0i32; 16]; ch];
        let mut p = 8;
        for t in table.iter_mut() {
            for v in t.iter_mut() {
                *v = s16(p);
                p += 2;
            }
        }
        if self.history.is_none() {
            let mut h = Vec::new();
            for _ in 0..ch {
                h.push((s16(p), s16(p + 2)));
                p += 4;
            }
            self.history = Some(h);
        } else {
            p += ch * 4;
        }
        let hist = self.history.as_mut().expect("set above");
        let mut out = vec![0i16; n * ch];
        let get = |p: usize| i32::from(packet.get(p).copied().unwrap_or(0));
        for c in 0..ch {
            let (mut s1, mut s2) = hist[c];
            for i in 0..n.div_ceil(14) {
                let mut byte = get(p);
                p += 1;
                let index = ((byte >> 4) & 7) as usize;
                let exp = (byte & 15) as u32;
                let (f1, f2) = (i64::from(table[c][index * 2]), i64::from(table[c][index * 2 + 1]));
                let mut k = 0;
                while k < 14 && i * 14 + k < n {
                    let nib = if k & 1 == 1 {
                        (byte << 28) >> 28
                    } else {
                        byte = get(p);
                        p += 1;
                        ((byte >> 4) << 28) >> 28
                    };
                    let v = ((i64::from(s1) * f1 + i64::from(s2) * f2) >> 11) as i32;
                    let v = v.wrapping_add(nib.wrapping_mul(1 << exp)).clamp(-32768, 32767);
                    out[(i * 14 + k) * ch + c] = v as i16;
                    s2 = s1;
                    s1 = v;
                    k += 1;
                }
            }
            hist[c] = (s1, s2);
        }
        Ok(out)
    }
}
