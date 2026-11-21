// Ported from FFmpeg n7.1.1 libavcodec/roqvideo.c, roqvideo.h, roqvideodec.c (id RoQ video
// decoder by Dr. Tim Ferguson), the RoQ part of libavcodec/dpcm.c (Copyright (c) 2003 The FFmpeg
// project, Mike Melanson) and libavformat/idroqdec.c (by Mike Melanson), LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust support for **id RoQ** (`.roq`, the cinematics of id Software's and Raven's games):
//! the video decoder, the DPCM audio decoder and the file's chunk layout, as ffmpeg's
//! `roqvideo` / `roq_dpcm` decoders and `roq` demuxer, with the same output.
//!
//! - [`parse`] finds the video and audio packets of a file.
//! - [`Decoder`] decodes video packets (full range YUV 4:4:4); it is `Clone`, so a caller can
//!   keep a state to restart from.
//! - [`decode_audio`] decodes one audio chunk to 16-bit samples.

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not an id RoQ file")]
    NotRoq,
    #[error("invalid id RoQ stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported id RoQ stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Audio sample rate of every RoQ file.
pub const AUDIO_RATE: u32 = 22050;
/// Largest picture (luma samples) accepted.
pub const MAX_PIXELS: u64 = 1 << 26;

const INFO: u16 = 0x1001;
const QUAD_CODEBOOK: u16 = 0x1002;
const QUAD_VQ: u16 = 0x1011;
const SOUND_MONO: u16 = 0x1020;
const SOUND_STEREO: u16 = 0x1021;
const PREAMBLE: usize = 8;

/// Whether `head` starts like a RoQ file.
pub fn probe(head: &[u8]) -> bool {
    head.len() >= 6 && head[0] == 0x84 && head[1] == 0x10 && head[2..6] == [0xFF; 4]
}

/// A byte range of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn of(self, data: &[u8]) -> &[u8] {
        data.get(self.start..self.start.saturating_add(self.len)).unwrap_or(&[])
    }
}

/// What the chunks of a file say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    pub width: u32,
    pub height: u32,
    /// Frames per second (0 when the header does not say).
    pub fps: u32,
    /// Audio channels (1 or 2), from the kind of the first sound chunk.
    pub audio_channels: Option<u32>,
    /// One packet per video frame: a quad-codebook chunk and the vector-quantised chunk after
    /// it, or the latter alone.
    pub video: Vec<Span>,
    /// One packet per sound chunk, with its 8-byte preamble.
    pub audio: Vec<Span>,
    /// The chunks stopped making sense before the end of the file.
    pub damaged: bool,
}

fn le16(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?])))
}

fn le32(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from(u32::from_le_bytes([*d.get(at)?, *d.get(at + 1)?, *d.get(at + 2)?, *d.get(at + 3)?])))
}

/// Find the packets of a file, as ffmpeg's demuxer does.
pub fn parse(data: &[u8]) -> Result<Stream> {
    if !probe(data) || data.len() < PREAMBLE {
        return Err(Error::NotRoq);
    }
    let mut s = Stream { fps: le16(data, 6).unwrap_or(0), ..Stream::default() };
    let mut have_video = false;
    let mut pos = PREAMBLE;
    while pos < data.len() {
        let (Some(kind), Some(size)) = (le16(data, pos), le32(data, pos + 2)) else {
            s.damaged = true;
            break;
        };
        let at = pos + PREAMBLE;
        if at > data.len() || size > i32::MAX as u64 {
            s.damaged = true;
            break;
        }
        pos = at;
        // limited to what the file holds
        let size = (size as usize).min(data.len() - pos);
        match kind as u16 {
            INFO => {
                if !have_video {
                    let (Some(w), Some(h)) = (le16(data, pos), le16(data, pos + 2)) else {
                        s.damaged = true;
                        break;
                    };
                    if data.len() - pos < PREAMBLE {
                        s.damaged = true;
                        break;
                    }
                    s.width = w;
                    s.height = h;
                    have_video = true;
                }
                // ffmpeg reads (or skips) the 8 bytes after the preamble, whatever the size says
                pos = (pos + PREAMBLE).min(data.len());
            }
            QUAD_CODEBOOK => {
                if !have_video {
                    s.damaged = true;
                    break;
                }
                let start = pos - PREAMBLE;
                let next = pos + size;
                // the packet holds this codebook and the next chunk
                let Some(vq) = le32(data, next + 2).filter(|_| data.len() >= next + PREAMBLE) else {
                    s.damaged = true;
                    break;
                };
                let len = vq + 2 * PREAMBLE as u64 + size as u64;
                if len > i32::MAX as u64 || start as u64 + len > data.len() as u64 {
                    s.damaged = true;
                    break;
                }
                s.video.push(Span { start, len: len as usize });
                pos = start + len as usize;
            }
            SOUND_MONO | SOUND_STEREO | QUAD_VQ => {
                if kind as u16 == QUAD_VQ {
                    if !have_video {
                        s.damaged = true;
                        break;
                    }
                } else if s.audio_channels.is_none() {
                    s.audio_channels = Some(if kind as u16 == SOUND_STEREO { 2 } else { 1 });
                }
                let span = Span { start: pos - PREAMBLE, len: size + PREAMBLE };
                if kind as u16 == QUAD_VQ { s.video.push(span) } else { s.audio.push(span) }
                pos += size;
            }
            _ => {
                s.damaged = true;
                break;
            }
        }
    }
    if s.video.is_empty() && s.audio.is_empty() {
        return Err(Error::Invalid("no packets"));
    }
    if !s.video.is_empty() && (s.width == 0 || s.height == 0 || s.width % 16 != 0 || s.height % 16 != 0) {
        return Err(Error::Unsupported("picture size not a multiple of 16"));
    }
    if u64::from(s.width) * u64::from(s.height) > MAX_PIXELS {
        return Err(Error::Unsupported("picture too large"));
    }
    Ok(s)
}

/// Samples per channel an audio packet decodes to.
pub fn audio_samples(packet_len: usize, channels: u32) -> usize {
    let stereo = channels > 1;
    let len = if stereo && packet_len & 1 == 1 { packet_len - 1 } else { packet_len };
    len.saturating_sub(PREAMBLE).div_ceil(channels.max(1) as usize)
}

/// Decode one sound chunk (with its preamble) to interleaved 16-bit samples: the chunk's
/// argument is the first predictor (mono: 16 bits; stereo: the high bytes of the two channels),
/// each data byte is a signed square step.
pub fn decode_audio(packet: &[u8], channels: u32) -> Result<Vec<i16>> {
    let stereo = channels > 1;
    if channels == 0 || channels > 2 {
        return Err(Error::Invalid("channel count"));
    }
    let len = if stereo && packet.len() & 1 == 1 { packet.len() - 1 } else { packet.len() };
    if len <= PREAMBLE {
        return Err(Error::Invalid("sound chunk too small"));
    }
    let mut pred = [0i32; 2];
    if stereo {
        pred[1] = i32::from((packet[6] as i16) << 8);
        pred[0] = i32::from((packet[7] as i16) << 8);
    } else {
        pred[0] = i32::from(i16::from_le_bytes([packet[6], packet[7]]));
    }
    let mut ch = 0usize;
    let mut out = Vec::with_capacity(len - PREAMBLE);
    for &b in &packet[PREAMBLE..len] {
        let i = i32::from(b & 0x7F);
        let sq = (i * i) as i16 as i32;
        pred[ch] = (pred[ch] + if b & 0x80 != 0 { -sq } else { sq }).clamp(-32768, 32767);
        out.push(pred[ch] as i16);
        if stereo {
            ch ^= 1;
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Default)]
struct Cell {
    y: [u8; 4],
    u: u8,
    v: u8,
}

/// A decoded picture: three full-size planes, full range Y'CbCr 4:4:4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub planes: [Vec<u8>; 3],
}

/// Bytes with ffmpeg's bounds rules: a read that does not fit gives 0 and moves to the end.
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl Cur<'_> {
    fn u8(&mut self) -> u8 {
        match self.d.get(self.p) {
            Some(&b) => {
                self.p += 1;
                b
            }
            None => 0,
        }
    }
    fn le16(&mut self) -> u32 {
        if self.left() < 2 {
            self.p = self.d.len();
            return 0;
        }
        let v = u32::from(self.d[self.p]) | u32::from(self.d[self.p + 1]) << 8;
        self.p += 2;
        v
    }
    fn le32(&mut self) -> u64 {
        if self.left() < 4 {
            self.p = self.d.len();
            return 0;
        }
        let v = u64::from(u32::from_le_bytes([self.d[self.p], self.d[self.p + 1], self.d[self.p + 2], self.d[self.p + 3]]));
        self.p += 4;
        v
    }
    fn left(&self) -> usize {
        self.d.len() - self.p.min(self.d.len())
    }
}

/// The video decoder: the codebooks and the two frame buffers it keeps between packets.
#[derive(Clone)]
pub struct Decoder {
    width: usize,
    height: usize,
    cb2: [Cell; 256],
    cb4: [[u8; 4]; 256],
    current: Option<[Vec<u8>; 3]>,
    last: Option<[Vec<u8>; 3]>,
}

impl Decoder {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || width % 16 != 0 || height % 16 != 0 {
            return Err(Error::Unsupported("picture size not a multiple of 16"));
        }
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Unsupported("picture too large"));
        }
        Ok(Self { width: width as usize, height: height as usize, cb2: [Cell::default(); 256], cb4: [[0; 4]; 256], current: None, last: None })
    }

    /// Bytes of the state a clone keeps (the two pictures).
    pub fn state_bytes(&self) -> usize {
        self.width * self.height * 3 * (usize::from(self.current.is_some()) + usize::from(self.last.is_some()))
    }

    fn blank(&self) -> [Vec<u8>; 3] {
        let n = self.width * self.height;
        [vec![0; n], vec![0; n], vec![0; n]]
    }

    /// Decode one video packet (the data of the packet [`parse`] found); the picture it shows.
    pub fn decode(&mut self, packet: &[u8]) -> Frame {
        // as ffmpeg: the first picture decodes into an empty buffer, the second into a copy of
        // the first; from then on the two buffers swap, so a block a packet leaves alone keeps
        // the pixels of the picture two packets back
        let copy = self.current.is_none() && self.last.is_some();
        let mut cur = match self.current.take() {
            Some(c) => c,
            None if copy => self.last.clone().unwrap_or_else(|| self.blank()),
            None => self.blank(),
        };
        self.decode_into(&mut cur, packet);
        let frame = Frame { width: self.width as u32, height: self.height as u32, planes: cur.clone() };
        self.current = self.last.take();
        self.last = Some(cur);
        frame
    }

    fn decode_into(&mut self, cur: &mut [Vec<u8>; 3], packet: &[u8]) {
        let mut g = Cur { d: packet, p: 0 };
        let (mut chunk_size, mut chunk_arg) = (0u64, 0u32);
        while g.left() >= 8 {
            let chunk_id = g.le16();
            chunk_size = g.le32();
            chunk_arg = g.le16();
            if chunk_id == u32::from(QUAD_VQ) {
                break;
            }
            if chunk_id == u32::from(QUAD_CODEBOOK) {
                let mut nv1 = (chunk_arg >> 8) as usize;
                if nv1 == 0 {
                    nv1 = 256;
                }
                let mut nv2 = (chunk_arg & 0xFF) as usize;
                if nv2 == 0 && ((nv1 * 6) as u64) < chunk_size {
                    nv2 = 256;
                }
                for i in 0..nv1 {
                    let c = &mut self.cb2[i];
                    for k in 0..4 {
                        c.y[k] = g.u8();
                    }
                    c.u = g.u8();
                    c.v = g.u8();
                }
                for i in 0..nv2 {
                    for j in 0..4 {
                        self.cb4[i][j] = g.u8();
                    }
                }
            }
        }
        let chunk_start = g.p;
        if chunk_size > g.left() as u64 {
            chunk_size = g.left() as u64;
        }
        let end = chunk_start + chunk_size as usize;
        let (w, h) = (self.width, self.height);
        let bias = (((chunk_arg >> 8) as u8) as i8 as i32, (chunk_arg as u8) as i8 as i32);
        let (mut xpos, mut ypos) = (0usize, 0usize);
        let (mut vqflg, mut vqflg_pos) = (0u32, -1i32);
        while g.p < end {
            let mut yp = ypos;
            while yp < ypos + 16 {
                let mut xp = xpos;
                while xp < xpos + 16 {
                    if g.p >= end {
                        return;
                    }
                    let vqid = next_code(&mut g, &mut vqflg, &mut vqflg_pos);
                    match vqid {
                        0 => {}
                        1 => {
                            let byte = i32::from(g.u8());
                            self.motion(cur, xp, yp, 8 - (byte >> 4) - bias.0, 8 - (byte & 0xF) - bias.1, 8);
                        }
                        2 => {
                            let q = self.cb4[usize::from(g.u8())];
                            self.vector4(cur, xp, yp, q[0]);
                            self.vector4(cur, xp + 4, yp, q[1]);
                            self.vector4(cur, xp, yp + 4, q[2]);
                            self.vector4(cur, xp + 4, yp + 4, q[3]);
                        }
                        _ => {
                            for k in 0..4 {
                                let (mut x, mut y) = (xp, yp);
                                if k & 1 != 0 {
                                    x += 4;
                                }
                                if k & 2 != 0 {
                                    y += 4;
                                }
                                if g.p >= end {
                                    return;
                                }
                                match next_code(&mut g, &mut vqflg, &mut vqflg_pos) {
                                    0 => {}
                                    1 => {
                                        let byte = i32::from(g.u8());
                                        self.motion(cur, x, y, 8 - (byte >> 4) - bias.0, 8 - (byte & 0xF) - bias.1, 4);
                                    }
                                    2 => {
                                        let q = self.cb4[usize::from(g.u8())];
                                        self.vector2(cur, x, y, q[0]);
                                        self.vector2(cur, x + 2, y, q[1]);
                                        self.vector2(cur, x, y + 2, q[2]);
                                        self.vector2(cur, x + 2, y + 2, q[3]);
                                    }
                                    _ => {
                                        for (dx, dy) in [(0, 0), (2, 0), (0, 2), (2, 2)] {
                                            let i = g.u8();
                                            self.vector2(cur, x + dx, y + dy, i);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    xp += 8;
                }
                yp += 8;
            }
            xpos += 16;
            if xpos >= w {
                xpos -= w;
                ypos += 16;
            }
            if ypos >= h {
                break;
            }
        }
    }

    fn vector2(&self, cur: &mut [Vec<u8>; 3], x: usize, y: usize, idx: u8) {
        let c = self.cb2[usize::from(idx)];
        let w = self.width;
        let at = y * w + x;
        cur[0][at] = c.y[0];
        cur[0][at + 1] = c.y[1];
        cur[0][at + w] = c.y[2];
        cur[0][at + w + 1] = c.y[3];
        for (plane, v) in [(1, c.u), (2, c.v)] {
            for o in [0, 1, w, w + 1] {
                cur[plane][at + o] = v;
            }
        }
    }

    fn vector4(&self, cur: &mut [Vec<u8>; 3], x: usize, y: usize, idx: u8) {
        let c = self.cb2[usize::from(idx)];
        let w = self.width;
        for (k, &yy) in c.y.iter().enumerate() {
            let (bx, by) = ((k & 1) * 2, (k >> 1) * 2);
            for dy in 0..2 {
                for dx in 0..2 {
                    cur[0][(y + by + dy) * w + x + bx + dx] = yy;
                }
            }
        }
        for (plane, v) in [(1, c.u), (2, c.v)] {
            for dy in 0..4 {
                for dx in 0..4 {
                    cur[plane][(y + dy) * w + x + dx] = v;
                }
            }
        }
    }

    fn motion(&self, cur: &mut [Vec<u8>; 3], x: usize, y: usize, dx: i32, dy: i32, sz: usize) {
        let (mx, my) = (x as i32 + dx, y as i32 + dy);
        // out of the picture, or no picture to copy from: the block stays as it is
        if mx < 0 || mx > self.width as i32 - sz as i32 || my < 0 || my > self.height as i32 - sz as i32 {
            return;
        }
        let Some(last) = self.last.as_ref() else { return };
        let w = self.width;
        for p in 0..3 {
            for r in 0..sz {
                let src = (my as usize + r) * w + mx as usize;
                let dst = (y + r) * w + x;
                cur[p][dst..dst + sz].copy_from_slice(&last[p][src..src + sz]);
            }
        }
    }
}

/// The next 2-bit code: a 16-bit flag word holds eight, the first in its top bits.
fn next_code(g: &mut Cur<'_>, vqflg: &mut u32, pos: &mut i32) -> u32 {
    if *pos < 0 {
        *vqflg = g.le16();
        *pos = 7;
    }
    let id = (*vqflg >> (*pos * 2)) & 3;
    *pos -= 1;
    id
}
