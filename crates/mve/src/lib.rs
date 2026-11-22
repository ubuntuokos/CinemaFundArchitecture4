// Ported from FFmpeg n7.1.1 libavcodec/interplayvideo.c (Interplay MVE Video Decoder by Mike
// Melanson, Copyright (C) 2003 The FFmpeg project), the Interplay part of libavcodec/dpcm.c
// (Copyright (c) 2003 The FFmpeg project) and libavformat/ipmovie.c (Interplay MVE File
// Demuxer, Copyright (c) 2003 The FFmpeg project), LGPL-2.1-or-later; see docs/licensing.md §5.
// CFA4 changes GPL-3.0-or-later.

//! Pure-Rust support for **Interplay MVE** (`.mve`, the cinematics of Interplay's games): the
//! video decoder (8-bit palette and 16-bit RGB 5:5:5, every block opcode, the three frame
//! formats), the sound decoders (16-bit and 8-bit PCM, Interplay DPCM) and the file's chunk and
//! opcode layout, as ffmpeg's `interplayvideo` / `interplay_dpcm` decoders and `ipmovie`
//! demuxer, with the same output.
//!
//! - [`parse`] walks the chunks and finds the video and audio packets of a file.
//! - [`Decoder`] decodes video packets; it is `Clone`, so a caller can keep a state to restart
//!   from.
//! - [`decode_audio`] decodes one audio packet to 16-bit samples.

use std::sync::Arc;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not an Interplay MVE file")]
    NotMve,
    #[error("invalid Interplay MVE stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported Interplay MVE stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Largest picture (luma samples) accepted.
pub const MAX_PIXELS: u64 = 1 << 26;

/// `"Interplay MVE File\x1A\0\x1A\0"`: ffmpeg compares the terminating zero too.
const SIGNATURE: &[u8; 22] = b"Interplay MVE File\x1A\0\x1A\0";

/// Whether `head` holds the file signature (ffmpeg looks for it anywhere in what it probes).
pub fn probe(head: &[u8]) -> bool {
    head.windows(SIGNATURE.len()).any(|w| w == SIGNATURE)
}

/// A byte range of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn of(self, data: &[u8]) -> &[u8] {
        data.get(self.start..self.start.saturating_add(self.len)).unwrap_or(&[])
    }
}

/// One video packet: what a frame's opcodes put together (the data of one video data opcode,
/// the decoding map and the skip map), whether the picture is shown, and a new palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoPacket {
    /// The video data opcode: 0x06, 0x10 or 0x11.
    pub format: u8,
    pub send_buffer: bool,
    pub video: Span,
    pub decode_map: Span,
    pub skip_map: Span,
    /// All 256 entries (`0xAARRGGBB`) when a packet carries a new palette.
    pub palette: Option<Arc<[u32; 256]>>,
}

/// The sound a file declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioParams {
    pub channels: u32,
    pub sample_rate: u32,
    pub bits: u32,
    /// Interplay DPCM (a version 1 opcode with bit 2 of the flags); else 8- or 16-bit PCM.
    pub dpcm: bool,
}

/// What the chunks of a file say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Movie {
    pub width: u32,
    pub height: u32,
    /// 16-bit RGB 5:5:5 pictures instead of 8-bit palette ones.
    pub bpp16: bool,
    /// Microseconds per video packet (0 when the file has no timer).
    pub frame_us: u64,
    pub video: Vec<VideoPacket>,
    pub audio: Option<AudioParams>,
    /// One span per audio packet: the data after the 6-byte header for PCM, with it for DPCM.
    pub audio_packets: Vec<Span>,
    /// The chunks stopped making sense before the end of the file.
    pub damaged: bool,
}

const CHUNK_INIT_VIDEO: u16 = 2;
const CHUNK_SHUTDOWN: u16 = 4;
const CHUNK_END: u16 = 5;

fn le16(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?])))
}

fn le32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*d.get(at)?, *d.get(at + 1)?, *d.get(at + 2)?, *d.get(at + 3)?]))
}

/// The packets a chunk's opcodes leave pending, as ffmpeg's demuxer holds them.
#[derive(Default)]
struct Pending {
    audio: Option<Span>,
    format: u8,
    send_buffer: bool,
    video: Span,
    decode_map: Span,
    skip_map: Span,
}

/// Walk the chunks of a file, as ffmpeg's demuxer does.
pub fn parse(data: &[u8]) -> Result<Movie> {
    let sig = data.windows(SIGNATURE.len()).position(|w| w == SIGNATURE).ok_or(Error::NotMve)?;
    let mut m = Movie::default();
    let mut pos = sig + SIGNATURE.len() + 4;
    let mut palette = [0xFF00_0000u32; 256];
    let mut has_palette = false;
    let mut pend = Pending::default();
    let mut first = true;
    'chunks: while pos + 4 <= data.len() {
        let (Some(csize), Some(ctype)) = (le16(data, pos), le16(data, pos + 2)) else { break };
        pos += 4;
        let ctype = ctype as u16;
        if first {
            if ctype != CHUNK_INIT_VIDEO {
                return Err(Error::Invalid("the first chunk is not a video initialisation"));
            }
            first = false;
        }
        if ctype > CHUNK_END {
            m.damaged = true;
            break;
        }
        let mut left = csize as i64;
        while left > 0 {
            let (Some(osize), Some(&otype), Some(&over)) = (le16(data, pos), data.get(pos + 2), data.get(pos + 3)) else {
                m.damaged = true;
                break 'chunks;
            };
            let osize = osize as usize;
            pos += 4;
            left -= 4 + osize as i64;
            if left < 0 {
                m.damaged = true;
                break 'chunks;
            }
            let body = Span { start: pos, len: osize };
            // opcodes that read their bytes need them all
            let rd = |min: usize, max: usize| -> Option<&[u8]> { (osize >= min && osize <= max).then(|| data.get(pos..pos + osize)).flatten() };
            let bad = match otype {
                0x00 | 0x01 | 0x04 | 0x09 | 0x0A | 0x0B | 0x0D | 0x12..=0x15 => false,
                0x02 => match (over == 0).then(|| rd(6, 6)).flatten() {
                    Some(s) => {
                        m.frame_us = u64::from(le32(s, 0).unwrap_or(0)) * u64::from(le16(s, 4).unwrap_or(0));
                        false
                    }
                    None => true,
                },
                0x03 => match (over <= 1).then(|| rd(6, 10)).flatten() {
                    Some(s) => {
                        if m.audio.is_none() {
                            let flags = le16(s, 2).unwrap_or(0);
                            let bits = (((flags >> 1) & 1) + 1) * 8;
                            m.audio =
                                Some(AudioParams { channels: (flags & 1) + 1, sample_rate: le16(s, 4).unwrap_or(0), bits, dpcm: over == 1 && flags & 4 != 0 });
                        }
                        false
                    }
                    None => true,
                },
                0x05 => match (over <= 2 && !(over == 2 && osize < 8)).then(|| rd(4, 8)).flatten() {
                    Some(s) => {
                        let (w, h) = (le16(s, 0).unwrap_or(0) * 8, le16(s, 2).unwrap_or(0) * 8);
                        let bpp16 = over >= 2 && le16(s, 6).unwrap_or(0) != 0;
                        if m.width == 0 && m.height == 0 {
                            (m.width, m.height, m.bpp16) = (w, h, bpp16);
                            false
                        } else {
                            // a picture of another size: not supported
                            w != m.width || h != m.height || bpp16 != m.bpp16
                        }
                    }
                    None => true,
                },
                0x07 => {
                    pend.send_buffer = true;
                    false
                }
                0x08 => {
                    pend.audio = Some(body);
                    false
                }
                0x0C => match rd(4, 0x304) {
                    Some(s) => {
                        let first_color = le16(s, 0).unwrap_or(0) as i64;
                        let last_color = first_color + le16(s, 2).unwrap_or(0) as i64 - 1;
                        if first_color > 0xFF || last_color > 0xFF || (last_color - first_color + 1) * 3 + 4 > osize as i64 {
                            true
                        } else {
                            let mut j = 4;
                            for i in first_color..=last_color {
                                // a 6-bit VGA palette: each component is shifted up to 8 bits
                                let (r, g, b) = ((u32::from(s[j]) * 4) & 0xFF, (u32::from(s[j + 1]) * 4) & 0xFF, (u32::from(s[j + 2]) * 4) & 0xFF);
                                j += 3;
                                let v = 0xFF00_0000 | r << 16 | g << 8 | b;
                                palette[i as usize] = v | (v >> 6 & 0x03_0303);
                            }
                            has_palette = true;
                            false
                        }
                    }
                    None => true,
                },
                0x0E => {
                    pend.skip_map = body;
                    false
                }
                0x0F => {
                    pend.decode_map = body;
                    false
                }
                0x06 | 0x10 | 0x11 => {
                    pend.format = otype;
                    pend.video = body;
                    false
                }
                _ => true,
            };
            if bad {
                m.damaged = true;
                break 'chunks;
            }
            pos = (pos + osize).min(data.len());
        }
        if ctype == CHUNK_END || ctype == CHUNK_SHUTDOWN {
            break;
        }
        // the packets the chunk leaves: sound first, then the picture
        if let (Some(a), Some(p)) = (pend.audio, m.audio)
            && p.channels > 0
            && p.bits > 0
        {
            // PCM packets skip the 6-byte header; the DPCM decoder skips it itself
            if p.dpcm {
                m.audio_packets.push(a);
            } else if a.len >= 6 {
                m.audio_packets.push(Span { start: a.start + 6, len: a.len - 6 });
            }
            pend.audio = None;
        }
        if pend.format != 0 {
            m.video.push(VideoPacket {
                format: pend.format,
                send_buffer: pend.send_buffer,
                video: pend.video,
                decode_map: pend.decode_map,
                skip_map: pend.skip_map,
                palette: has_palette.then(|| Arc::new(palette)),
            });
            has_palette = false;
            pend = Pending { audio: pend.audio, ..Pending::default() };
        }
    }
    if m.video.is_empty() && m.audio_packets.is_empty() {
        return Err(Error::Invalid("no packets"));
    }
    if !m.video.is_empty() {
        if m.width == 0 || m.height == 0 {
            return Err(Error::Invalid("no picture size"));
        }
        if u64::from(m.width) * u64::from(m.height) > MAX_PIXELS {
            return Err(Error::Unsupported("picture too large"));
        }
    }
    Ok(m)
}

const INTERPLAY_DELTA: [i16; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 47, 51, 56, 61, 66, 72, 79, 86, 94, 102, 112, 122, 133, 145, 158, 173, 189, 206, 225, 245, 267, 292, 318, 348, 379, 414, 452, 493, 538, 587,
    640, 699, 763, 832, 908, 991, 1081, 1180, 1288, 1405, 1534, 1673, 1826, 1993, 2175, 2373, 2590, 2826, 3084, 3365, 3672, 4008, 4373, 4772, 5208, 5683, 6202,
    6767, 7385, 8059, 8794, 9597, 10472, 11428, 12471, 13609, 14851, 16206, 17685, 19298, 21060, 22981, 25078, 27367, 29864, 32589, -29973, -26728, -23186,
    -19322, -15105, -10503, -5481, -1, 1, 1, 5481, 10503, 15105, 19322, 23186, 26728, 29973, -32589, -29864, -27367, -25078, -22981, -21060, -19298, -17685,
    -16206, -14851, -13609, -12471, -11428, -10472, -9597, -8794, -8059, -7385, -6767, -6202, -5683, -5208, -4772, -4373, -4008, -3672, -3365, -3084, -2826,
    -2590, -2373, -2175, -1993, -1826, -1673, -1534, -1405, -1288, -1180, -1081, -991, -908, -832, -763, -699, -640, -587, -538, -493, -452, -414, -379, -348,
    -318, -292, -267, -245, -225, -206, -189, -173, -158, -145, -133, -122, -112, -102, -94, -86, -79, -72, -66, -61, -56, -51, -47, -43, -42, -41, -40, -39,
    -38, -37, -36, -35, -34, -33, -32, -31, -30, -29, -28, -27, -26, -25, -24, -23, -22, -21, -20, -19, -18, -17, -16, -15, -14, -13, -12, -11, -10, -9, -8,
    -7, -6, -5, -4, -3, -2, -1,
];

/// Samples (all channels) an audio packet decodes to.
pub fn audio_values(p: &AudioParams, packet_len: usize) -> usize {
    let ch = p.channels.max(1) as usize;
    if p.dpcm {
        let len = if ch > 1 && packet_len & 1 == 1 { packet_len - 1 } else { packet_len };
        len.saturating_sub(6 + ch)
    } else {
        let per = ch * (p.bits as usize / 8).max(1);
        packet_len / per * ch
    }
}

/// Decode one audio packet (the span [`parse`] found) to interleaved 16-bit samples.
pub fn decode_audio(p: &AudioParams, packet: &[u8]) -> Result<Vec<i16>> {
    let ch = p.channels as usize;
    if ch == 0 || ch > 2 {
        return Err(Error::Invalid("channel count"));
    }
    if !p.dpcm {
        return Ok(if p.bits == 16 {
            packet.chunks_exact(2 * ch).flat_map(|f| f.chunks_exact(2)).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
        } else {
            packet.chunks_exact(ch).flatten().map(|&b| (i16::from(b) - 128) << 8).collect()
        });
    }
    let stereo = ch == 2;
    let len = if stereo && packet.len() & 1 == 1 { packet.len() - 1 } else { packet.len() };
    let out = len as i64 - 6 - ch as i64;
    if out <= 0 {
        return Err(Error::Invalid("audio packet too small"));
    }
    let mut pred = [0i32; 2];
    let mut pos = 6;
    let mut samples = Vec::with_capacity(out as usize);
    for c in 0..ch {
        pred[c] = i32::from(i16::from_le_bytes([packet[pos], packet[pos + 1]]));
        pos += 2;
        samples.push(pred[c] as i16);
    }
    let mut c = 0usize;
    while (samples.len() as i64) < out {
        // past the end of the data a byte reads as 0, as in ffmpeg's bytestream reader
        let b = packet.get(pos).copied().unwrap_or(0);
        pos += 1;
        pred[c] = (pred[c] + i32::from(INTERPLAY_DELTA[usize::from(b)])).clamp(-32768, 32767);
        samples.push(pred[c] as i16);
        if stereo {
            c ^= 1;
        }
    }
    Ok(samples)
}

/// Bytes with ffmpeg's bounds rules: a read that does not fit gives 0 and moves to the end.
#[derive(Clone, Copy)]
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl Cur<'_> {
    fn left(&self) -> usize {
        self.d.len() - self.p.min(self.d.len())
    }
    fn u8(&mut self) -> u8 {
        self.le(1) as u8
    }
    fn le16(&mut self) -> u32 {
        self.le(2) as u32
    }
    fn le32(&mut self) -> u32 {
        self.le(4) as u32
    }
    fn le64(&mut self) -> u64 {
        self.le(8)
    }
    fn le(&mut self, n: usize) -> u64 {
        if self.left() < n {
            self.p = self.d.len();
            return 0;
        }
        let v = self.d[self.p..self.p + n].iter().rev().fold(0u64, |a, &b| a << 8 | u64::from(b));
        self.p += n;
        v
    }
    fn skip(&mut self, n: usize) {
        self.p += n.min(self.left());
    }
    /// Copy up to `dst.len()` bytes (as many as there are): `dst` keeps what is not overwritten.
    fn get_buffer(&mut self, dst: &mut [u16]) {
        let n = dst.len().min(self.left());
        for (o, &b) in dst.iter_mut().zip(&self.d[self.p..self.p + n]) {
            *o = u16::from(b);
        }
        self.p += n;
    }
}

/// One decoded picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bpp16: bool,
    /// Palette indices, or RGB 5:5:5 values (`0RRRRRGGGGGBBBBB`).
    pub pixels: Vec<u16>,
    pub palette: Box<[u32; 256]>,
}

impl Frame {
    /// The picture as RGBA, 8 bits per component: palette entries as they are, 5-bit
    /// components widened by repeating their top bits.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &v in &self.pixels {
            if self.bpp16 {
                let c = |s: u32| {
                    let x = u32::from(v >> s) & 31;
                    ((x << 3) | (x >> 2)) as u8
                };
                out.extend_from_slice(&[c(10), c(5), c(0), 255]);
            } else {
                let p = self.palette[usize::from(v & 0xFF)];
                out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
            }
        }
        out
    }
}

/// The video decoder: the palette and the pictures it keeps between packets.
#[derive(Clone)]
pub struct Decoder {
    w: usize,
    h: usize,
    bpp16: bool,
    pal: Box<[u32; 256]>,
    last: Option<Vec<u16>>,
    second_last: Option<Vec<u16>>,
    /// Format 0x10 decodes into these two and copies to the output.
    cur_decode: Option<Vec<u16>>,
    prev_decode: Option<Vec<u16>>,
    /// Pictures shown so far (ffmpeg's `frame_num`).
    shown: u64,
}

/// A packet to decode: the pieces [`VideoPacket`] spans.
pub struct Input<'a> {
    pub format: u8,
    pub send_buffer: bool,
    pub video: &'a [u8],
    pub decode_map: &'a [u8],
    pub skip_map: &'a [u8],
    pub palette: Option<&'a [u32; 256]>,
}

/// What a block opcode needs besides the picture being made.
struct Refs<'a> {
    last: Option<&'a [u16]>,
    second_last: Option<&'a [u16]>,
}

/// State of one packet's decode.
struct Run<'a> {
    w: usize,
    h: usize,
    bpp16: bool,
    stream: Cur<'a>,
    mv: Cur<'a>,
}

/// Where a block copy reads from.
#[derive(Clone, Copy)]
enum Src<'a> {
    Other(Option<&'a [u16]>),
    Same,
}

impl Run<'_> {
    /// ffmpeg's `copy_from`: the 8×8 block at pixel `pos` of `dst` comes from `(dx, dy)` away in
    /// `src`; a vector off the left or right edge wraps to the next or previous row.
    fn copy_from(&self, dst: &mut [u16], pos: usize, src: Src<'_>, dx: i32, dy: i32) -> std::result::Result<(), ()> {
        let w = self.w as i32;
        let (x, y) = ((pos % self.w) as i32, (pos / self.w) as i32);
        let tx = dx + x;
        let wrap = i32::from(tx >= w) - i32::from(tx < 0);
        let (ddx, ddy) = (tx - wrap * w, dy + y + wrap);
        let off = i64::from(ddy) * i64::from(w) + i64::from(ddx);
        let upper = (self.h as i64 - 8) * i64::from(w) + (w as i64 - 8);
        if off < 0 || off > upper {
            return Err(());
        }
        let off = off as usize;
        match src {
            Src::Other(None) => return Err(()),
            Src::Other(Some(s)) => {
                for r in 0..8 {
                    let (d, o) = (pos + r * self.w, off + r * self.w);
                    dst[d..d + 8].copy_from_slice(&s[o..o + 8]);
                }
            }
            Src::Same => {
                for r in 0..8 {
                    let (d, o) = (pos + r * self.w, off + r * self.w);
                    dst.copy_within(o..o + 8, d);
                }
            }
        }
        Ok(())
    }

    fn rd(&mut self) -> u16 {
        if self.bpp16 { self.stream.le16() as u16 } else { u16::from(self.stream.u8()) }
    }

    /// Whether a colour pair selects the first coding: `P[i] <= P[i + 1]`, or in 16 bits the
    /// top bit of `P[i]` clear.
    fn first(&self, p: &[u16], i: usize) -> bool {
        if self.bpp16 { p[i] & 0x8000 == 0 } else { p[i] <= p[i + 1] }
    }

    /// The motion byte of opcodes 2 to 4: from the motion stream in 16 bits.
    fn motion_byte(&mut self) -> u8 {
        if self.bpp16 { self.mv.u8() } else { self.stream.u8() }
    }

    fn block(&mut self, op: u8, frame: &mut [u16], pos: usize, r: &Refs<'_>) -> std::result::Result<(), ()> {
        let stride = self.w as isize;
        let mut q = pos as isize;
        let line_inc = stride - 8;
        let bpp16 = self.bpp16;
        let put = |frame: &mut [u16], at: isize, v: u16| {
            if let Some(p) = usize::try_from(at).ok().and_then(|i| frame.get_mut(i)) {
                *p = v;
            }
        };
        match op {
            0x0 => return self.copy_from(frame, pos, Src::Other(r.last), 0, 0),
            0x1 => return self.copy_from(frame, pos, Src::Other(r.second_last), 0, 0),
            0x2 => {
                let b = i32::from(self.motion_byte());
                let (x, y) = if b < 56 { (8 + b % 7, b / 7) } else { (-14 + (b - 56) % 29, 8 + (b - 56) / 29) };
                return self.copy_from(frame, pos, Src::Other(r.second_last), x, y);
            }
            0x3 => {
                let b = i32::from(self.motion_byte());
                let (x, y) = if b < 56 { (-(8 + b % 7), -(b / 7)) } else { (-(-14 + (b - 56) % 29), -(8 + (b - 56) / 29)) };
                return self.copy_from(frame, pos, Src::Same, x, y);
            }
            0x4 => {
                let b = i32::from(self.motion_byte());
                return self.copy_from(frame, pos, Src::Other(r.last), -8 + (b & 0xF), -8 + ((b >> 4) & 0xF));
            }
            0x5 => {
                let x = i32::from(self.stream.u8() as i8);
                let y = i32::from(self.stream.u8() as i8);
                return self.copy_from(frame, pos, Src::Other(r.last), x, y);
            }
            0x6 if !bpp16 => {} // a mystery opcode: ffmpeg reports success and leaves the block
            0x6 => {
                let x = i32::from(self.stream.u8() as i8);
                let y = i32::from(self.stream.u8() as i8);
                return self.copy_from(frame, pos, Src::Other(r.second_last), x, y);
            }
            0x7 => {
                if !bpp16 && self.stream.left() < 4 {
                    return Err(());
                }
                let p = [self.rd(), self.rd()];
                if self.first(&p, 0) {
                    for _ in 0..8 {
                        let mut flags = u32::from(self.stream.u8()) | 0x100;
                        while flags != 1 {
                            put(frame, q, p[(flags & 1) as usize]);
                            q += 1;
                            flags >>= 1;
                        }
                        q += line_inc;
                    }
                } else {
                    let mut flags = self.stream.le16();
                    for _ in (0..8).step_by(2) {
                        for x in (0..8).step_by(2) {
                            let v = p[(flags & 1) as usize];
                            for o in [0, 1, stride, stride + 1] {
                                put(frame, q + x + o, v);
                            }
                            flags >>= 1;
                        }
                        q += stride * 2;
                    }
                }
            }
            0x8 => {
                if !bpp16 && self.stream.left() < 12 {
                    return Err(());
                }
                let mut p = [self.rd(), self.rd(), 0, 0];
                let mut flags = 0u32;
                if self.first(&p, 0) {
                    for y in 0..16 {
                        if y & 3 == 0 {
                            if y != 0 {
                                p[0] = self.rd();
                                p[1] = self.rd();
                            }
                            flags = self.stream.le16();
                        }
                        for _ in 0..4 {
                            put(frame, q, p[(flags & 1) as usize]);
                            q += 1;
                            flags >>= 1;
                        }
                        q += stride - 4;
                        if y == 7 {
                            q -= 8 * stride - 4;
                        }
                    }
                } else {
                    flags = self.stream.le32();
                    p[2] = self.rd();
                    p[3] = self.rd();
                    if self.first(&p, 2) {
                        // vertical split: the left and right halves are 2-colour coded
                        for y in 0..16 {
                            for _ in 0..4 {
                                put(frame, q, p[(flags & 1) as usize]);
                                q += 1;
                                flags >>= 1;
                            }
                            q += stride - 4;
                            if y == 7 {
                                q -= 8 * stride - 4;
                                p[0] = p[2];
                                p[1] = p[3];
                                flags = self.stream.le32();
                            }
                        }
                    } else {
                        // horizontal split
                        for y in 0..8 {
                            if y == 4 {
                                p[0] = p[2];
                                p[1] = p[3];
                                flags = self.stream.le32();
                            }
                            for _ in 0..8 {
                                put(frame, q, p[(flags & 1) as usize]);
                                q += 1;
                                flags >>= 1;
                            }
                            q += line_inc;
                        }
                    }
                }
            }
            0x9 => {
                if !bpp16 && self.stream.left() < 8 {
                    return Err(());
                }
                let p = [self.rd(), self.rd(), self.rd(), self.rd()];
                if self.first(&p, 0) {
                    if self.first(&p, 2) {
                        // 1 of 4 colours for each pixel
                        for _ in 0..8 {
                            let mut flags = self.stream.le16();
                            for _ in 0..8 {
                                put(frame, q, p[(flags & 3) as usize]);
                                q += 1;
                                flags >>= 2;
                            }
                            q += line_inc;
                        }
                    } else {
                        // for each 2×2 block
                        let mut flags = self.stream.le32();
                        for _ in (0..8).step_by(2) {
                            for x in (0..8).step_by(2) {
                                let v = p[(flags & 3) as usize];
                                for o in [0, 1, stride, stride + 1] {
                                    put(frame, q + x + o, v);
                                }
                                flags >>= 2;
                            }
                            q += stride * 2;
                        }
                    }
                } else {
                    let mut flags = self.stream.le64();
                    if self.first(&p, 2) {
                        // for each 2×1 block
                        for _ in 0..8 {
                            for x in (0..8).step_by(2) {
                                let v = p[(flags & 3) as usize];
                                put(frame, q + x, v);
                                put(frame, q + x + 1, v);
                                flags >>= 2;
                            }
                            q += stride;
                        }
                    } else {
                        // for each 1×2 block
                        for _ in (0..8).step_by(2) {
                            for x in 0..8 {
                                let v = p[(flags & 3) as usize];
                                put(frame, q + x, v);
                                put(frame, q + x + stride, v);
                                flags >>= 2;
                            }
                            q += stride * 2;
                        }
                    }
                }
            }
            0xA => {
                if !bpp16 && self.stream.left() < 16 {
                    return Err(());
                }
                let mut p = [0u16; 8];
                for v in &mut p[..4] {
                    *v = self.rd();
                }
                if self.first(&p, 0) {
                    // 4 colours for each quadrant
                    let mut flags = 0u32;
                    for y in 0..16 {
                        if y & 3 == 0 {
                            if y != 0 {
                                for v in &mut p[..4] {
                                    *v = self.rd();
                                }
                            }
                            flags = self.stream.le32();
                        }
                        for _ in 0..4 {
                            put(frame, q, p[(flags & 3) as usize]);
                            q += 1;
                            flags >>= 2;
                        }
                        q += stride - 4;
                        if y == 7 {
                            q -= 8 * stride - 4;
                        }
                    }
                } else {
                    let mut flags = self.stream.le64();
                    for v in &mut p[4..] {
                        *v = self.rd();
                    }
                    let vert = self.first(&p, 4);
                    for y in 0..16 {
                        for _ in 0..4 {
                            put(frame, q, p[(flags & 3) as usize]);
                            q += 1;
                            flags >>= 2;
                        }
                        if vert {
                            q += stride - 4;
                            if y == 7 {
                                q -= 8 * stride - 4;
                            }
                        } else if y & 1 == 1 {
                            q += line_inc;
                        }
                        if y == 7 {
                            p.copy_within(4..8, 0);
                            flags = self.stream.le64();
                        }
                    }
                }
            }
            0xB => {
                for _ in 0..8 {
                    let at = usize::try_from(q).unwrap_or(0);
                    if bpp16 {
                        for x in 0..8 {
                            let v = self.stream.le16() as u16;
                            put(frame, q + x, v);
                        }
                    } else if let Some(row) = frame.get_mut(at..at + 8) {
                        self.stream.get_buffer(row);
                    }
                    q += stride;
                }
            }
            0xC => {
                for _ in (0..8).step_by(2) {
                    for x in (0..8).step_by(2) {
                        let v = self.rd();
                        for o in [0, 1, stride, stride + 1] {
                            put(frame, q + x + o, v);
                        }
                    }
                    q += stride * 2;
                }
            }
            0xD => {
                if !bpp16 && self.stream.left() < 4 {
                    return Err(());
                }
                let mut p = [0u16; 2];
                for y in 0..8 {
                    if y & 3 == 0 {
                        p = [self.rd(), self.rd()];
                    }
                    for x in 0..8 {
                        put(frame, q + x, p[(x >> 2) as usize]);
                    }
                    q += stride;
                }
            }
            0xE => {
                let v = self.rd();
                for _ in 0..8 {
                    for x in 0..8 {
                        put(frame, q + x, v);
                    }
                    q += stride;
                }
            }
            // 0xF: a dithered block in 8 bits, "copy from the second last picture" in 16
            _ if bpp16 => return self.copy_from(frame, pos, Src::Other(r.second_last), 0, 0),
            _ => {
                let s = [u16::from(self.stream.u8()), u16::from(self.stream.u8())];
                for y in 0..8usize {
                    for _ in (0..8).step_by(2) {
                        put(frame, q, s[y & 1]);
                        put(frame, q + 1, s[1 - (y & 1)]);
                        q += 2;
                    }
                    q += line_inc;
                }
            }
        }
        Ok(())
    }

    /// Format 0x11: a 4-bit opcode per block from the decoding map (low nibble first).
    fn format11(&mut self, frame: &mut [u16], dmap: &[u8], r: &Refs<'_>) {
        self.stream.skip(14);
        if self.bpp16 {
            self.mv = self.stream;
            let n = self.stream.le16() as usize;
            self.mv.skip(n);
        }
        let mut nib = 0usize;
        for y in (0..self.h).step_by(8) {
            for x in (0..self.w).step_by(8) {
                let Some(&byte) = dmap.get(nib / 2) else { return };
                let op = if nib.is_multiple_of(2) { byte & 0xF } else { byte >> 4 };
                nib += 1;
                if self.block(op, frame, x + y * self.w, r).is_err() {
                    return;
                }
            }
        }
    }

    /// Format 0x06: the decoding map is at the top of the data; two passes.
    fn format06(&mut self, frame: &mut [u16], dmap: &[u8], r: &Refs<'_>, shown: u64) {
        let mut dm = Cur { d: dmap, p: 0 };
        for pass in 0..2 {
            dm.p = 0;
            for y in (0..self.h).step_by(8) {
                for x in (0..self.w).step_by(8) {
                    let opcode = dm.le16() as u16 as i16;
                    let pos = x + y * self.w;
                    if pass == 0 {
                        if opcode == 0 {
                            self.raw_block(frame, pos);
                        } else if shown > 2 {
                            let _ = self.copy_from(frame, pos, Src::Other(r.second_last), 0, 0);
                        }
                    } else if opcode < 0 {
                        let off = i32::from(opcode as u16) - 0xC000;
                        let _ = self.copy_from(frame, pos, Src::Other(r.last), off % self.w as i32, off / self.w as i32);
                    } else if opcode > 0 {
                        let off = i32::from(opcode as u16) - 0x4000;
                        let _ = self.copy_from(frame, pos, Src::Same, off % self.w as i32, off / self.w as i32);
                    }
                }
            }
        }
    }

    fn raw_block(&mut self, frame: &mut [u16], pos: usize) {
        for line in 0..8 {
            let at = pos + line * self.w;
            if let Some(row) = frame.get_mut(at..at + 8) {
                self.stream.get_buffer(row);
            }
        }
    }
}

impl Decoder {
    pub fn new(width: u32, height: u32, bpp16: bool) -> Result<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(8) || !height.is_multiple_of(8) {
            return Err(Error::Unsupported("picture size not a multiple of 8"));
        }
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Unsupported("picture too large"));
        }
        Ok(Self {
            w: width as usize,
            h: height as usize,
            bpp16,
            pal: Box::new([0xFF00_0000; 256]),
            last: None,
            second_last: None,
            cur_decode: None,
            prev_decode: None,
            shown: 0,
        })
    }

    /// Bytes of the state a clone keeps (up to four pictures).
    pub fn state_bytes(&self) -> usize {
        self.w * self.h * 2 * 4 + 1024
    }

    /// Decode one video packet. `Ok(Some)` is a picture to show (the packet's `send_buffer`);
    /// the decoder's state moves on either way. An error is a packet ffmpeg refuses before it
    /// changes anything.
    pub fn decode(&mut self, p: &Input<'_>) -> Result<Option<Frame>> {
        let (w, h) = (self.w, self.h);
        let n = w * h;
        if self.cur_decode.is_none() {
            self.cur_decode = Some(vec![0; n]);
            self.prev_decode = Some(vec![0; n]);
        }
        // the data the opcodes read, by frame format
        let mut stream = p.video;
        let mut dmap = p.decode_map;
        match p.format {
            0x06 => {
                if !p.decode_map.is_empty() || !p.skip_map.is_empty() || self.bpp16 {
                    return Err(Error::Invalid("maps or 16 bits in format 0x06"));
                }
                // the decoding map is at the top of the pixel data, after 14 bytes of opcodes
                let map = (w / 8) * (h / 8) * 2;
                if (p.video.len() as i64) - (map as i64 + 14) <= 0 {
                    return Err(Error::Invalid("format 0x06 data too short"));
                }
                dmap = &p.video[14..14 + map];
                stream = &p.video[14 + map..];
            }
            0x10 => {
                if p.decode_map.is_empty() || p.skip_map.is_empty() || self.bpp16 {
                    return Err(Error::Invalid("format 0x10 needs both maps and 8 bits"));
                }
            }
            0x11 => {
                if p.decode_map.is_empty() || !p.skip_map.is_empty() {
                    return Err(Error::Invalid("format 0x11 needs a decoding map and no skip map"));
                }
            }
            _ => return Err(Error::Unsupported("frame format")),
        }
        if !self.bpp16
            && let Some(pal) = p.palette
        {
            *self.pal = *pal;
        }
        // ffmpeg's new picture starts as whatever its buffer held; this one as the last picture
        let mut frame = self.last.clone().unwrap_or_else(|| vec![0; n]);
        let mut run = Run { w, h, bpp16: self.bpp16, stream: Cur { d: stream, p: 0 }, mv: Cur { d: &[], p: 0 } };
        let refs_last = self.last.take();
        let refs_second = self.second_last.take();
        let refs = Refs { last: refs_last.as_deref(), second_last: refs_second.as_deref() };
        match p.format {
            0x06 => run.format06(&mut frame, dmap, &refs, self.shown),
            0x11 => run.format11(&mut frame, dmap, &refs),
            _ => {
                let mut cur = self.cur_decode.take().unwrap_or_default();
                let prev = self.prev_decode.take().unwrap_or_default();
                let done = run.format10(&mut frame, &mut cur, &prev, p.decode_map, p.skip_map, &refs, self.shown);
                if done {
                    self.cur_decode = Some(prev);
                    self.prev_decode = Some(cur);
                } else {
                    self.cur_decode = Some(cur);
                    self.prev_decode = Some(prev);
                }
            }
        }
        let out = p.send_buffer.then(|| Frame { width: w as u32, height: h as u32, bpp16: self.bpp16, pixels: frame.clone(), palette: self.pal.clone() });
        if out.is_some() {
            self.shown += 1;
        }
        // shuffle: the picture before the last one is dropped
        self.second_last = refs_last;
        self.last = Some(frame);
        Ok(out)
    }
}

impl Run<'_> {
    /// Format 0x10: two passes into `cur`, then the picture is assembled from `cur` and the
    /// last picture by the skip map. `false` when the skip map ran out early (ffmpeg returns
    /// from there, leaving the buffers as they are).
    #[allow(clippy::too_many_arguments)]
    fn format10(&mut self, frame: &mut [u16], cur: &mut [u16], prev: &[u16], dmap: &[u8], smap: &[u8], r: &Refs<'_>, shown: u64) -> bool {
        self.stream.skip(14);
        let (w, h) = (self.w, self.h);
        let mut dm = Cur { d: dmap, p: 0 };
        let mut sm = Cur { d: smap, p: 0 };
        for pass in 0..2 {
            dm.p = 0;
            sm.p = 0;
            let mut skip = sm.le16() as u16 as i16;
            for y in (0..h).step_by(8) {
                for x in (0..w).step_by(8) {
                    let pos = x + y * w;
                    while skip <= 0 {
                        if skip != i16::MIN && skip != 0 {
                            let opcode = dm.le16() as u16 as i16;
                            if pass == 0 {
                                if opcode == 0 {
                                    self.raw_block(cur, pos);
                                }
                            } else if opcode < 0 {
                                let off = i32::from(opcode as u16) - 0xC000;
                                let _ = self.copy_from(cur, pos, Src::Other(Some(prev)), off % w as i32, off / w as i32);
                            } else if opcode > 0 {
                                let off = i32::from(opcode as u16) - 0x4000;
                                let _ = self.copy_from(cur, pos, Src::Same, off % w as i32, off / w as i32);
                            }
                            break;
                        }
                        if sm.left() < 2 {
                            return false;
                        }
                        skip = sm.le16() as u16 as i16;
                    }
                    skip = skip.wrapping_mul(2);
                }
            }
        }
        sm.p = 0;
        let mut skip = sm.le16() as u16 as i16;
        for y in (0..h).step_by(8) {
            for x in (0..w).step_by(8) {
                let pos = x + y * w;
                let mut changed = false;
                while skip <= 0 {
                    if skip != i16::MIN && skip != 0 {
                        changed = true;
                        break;
                    }
                    if sm.left() < 2 {
                        return false;
                    }
                    skip = sm.le16() as u16 as i16;
                }
                if changed {
                    let _ = self.copy_from(frame, pos, Src::Other(Some(cur)), 0, 0);
                } else if shown > 0 {
                    let _ = self.copy_from(frame, pos, Src::Other(r.last), 0, 0);
                }
                skip = skip.wrapping_mul(2);
            }
        }
        true
    }
}
