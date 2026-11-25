// Ported from FFmpeg n7.1.1 libavcodec/flicvideo.c (Copyright (C) 2003, 2004 The FFmpeg project,
// Mike Melanson) and libavformat/flic.c (Copyright (c) 2008 Sascha Sommer, Mike Melanson),
// LGPL-2.1-or-later, see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **Autodesk Animator FLI / FLC** animations (`.fli`, `.flc`): the 8-bit
//! paletted pictures of the 0xAF11 (FLI), 0xAF12 (FLC), 0xAF44 (DTA) and Magic Carpet files. The
//! output is the same as ffmpeg's `flic` decoder, byte for byte.
//!
//! [`Header::parse`] reads the 128-byte file header, a frame is one chunk with a `0xF1FA` or
//! `0xF5FA` preamble ([`CHUNK_MAGIC_1`], [`CHUNK_MAGIC_2`]), and [`Decoder::decode`] applies it to
//! the picture the previous frames left (every frame after the first is a delta).
//! Depths other than 8 bits (the 1-, 15-, 16- and 24-bit variants of FLX) are
//! [`Error::Unsupported`].

#[cfg(any(test, feature = "synth"))]
pub mod synth;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid FLIC data: {0}")]
    Invalid(&'static str),
    #[error("unsupported FLIC colour depth {0}")]
    Depth(u32),
    #[error("unsupported FLIC picture size {0}x{1}")]
    Size(u32, u32),
}

pub type Result<T> = std::result::Result<T, Error>;

pub const HEADER_SIZE: usize = 128;
/// The preamble of a chunk: size (`u32`) and type (`u16`).
pub const PREAMBLE_SIZE: usize = 6;
pub const FILE_MAGIC_FLI: u16 = 0xAF11;
pub const FILE_MAGIC_FLC: u16 = 0xAF12;
pub const FILE_MAGIC_DTA: u16 = 0xAF44;
pub const CHUNK_MAGIC_1: u16 = 0xF1FA;
pub const CHUNK_MAGIC_2: u16 = 0xF5FA;
/// The audio chunk of "X-COM: Terror from the Deep" files.
pub const CHUNK_TFTD_AUDIO: u16 = 0xAAAA;
/// The speed of Magic Carpet files, in 1/70 s.
const MC_SPEED: u32 = 5;
const DEFAULT_SPEED: u32 = 5;
const TFTD_SAMPLE_RATE: u32 = 22050;
/// The largest picture decoded, in pixels.
const MAX_PIXELS: u64 = 1 << 26;

const FLI_256_COLOR: u16 = 4;
const FLI_DELTA: u16 = 7;
const FLI_COLOR: u16 = 11;
const FLI_LC: u16 = 12;
const FLI_BLACK: u16 = 13;
const FLI_BRUN: u16 = 15;
const FLI_COPY: u16 = 16;
const FLI_MINI: u16 = 18;

fn le16(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 2) {
        Some(s) => u32::from(s[0]) | u32::from(s[1]) << 8,
        None => 0,
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    le16(b, at) | le16(b, at + 2) << 16
}

/// Whether `head` looks like a FLIC file, as ffmpeg's probe decides, and the first chunk is a
/// frame (or the audio of a Terror from the Deep file).
pub fn probe(head: &[u8]) -> bool {
    if head.len() < HEADER_SIZE {
        return false;
    }
    let magic = le16(head, 4) as u16;
    if magic != FILE_MAGIC_FLI && magic != FILE_MAGIC_FLC && magic != FILE_MAGIC_DTA {
        return false;
    }
    let carpet = le16(head, 0x10) as u16 == CHUNK_MAGIC_1;
    if !carpet && le32(head, 0x10) > 2000 {
        return false;
    }
    if le16(head, 0x08) > 4096 || le16(head, 0x0A) > 4096 {
        return false;
    }
    // the first chunk
    let first = if carpet { 12 } else { HEADER_SIZE };
    let kind = le16(head, first + 4) as u16;
    head.len() >= first + PREAMBLE_SIZE && matches!(kind, CHUNK_MAGIC_1 | CHUNK_MAGIC_2 | CHUNK_TFTD_AUDIO)
}

/// The file header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// 0xAF11, 0xAF12 or 0xAF44.
    pub magic: u16,
    pub width: u32,
    pub height: u32,
    /// Bits per pixel (0 in the file means 8).
    pub depth: u32,
    /// Offset of the first chunk: 128, or 12 in Magic Carpet files.
    pub first_chunk: usize,
    /// A Magic Carpet file: 6-bit palettes in every colour chunk.
    pub carpet: bool,
    /// Frames per second as numerator and denominator.
    pub frame_rate: (u32, u32),
    /// Block size of the 22050 Hz 8-bit mono audio chunks of a Terror from the Deep file. The audio
    /// is not decoded; its chunks are not frames.
    pub tftd_audio_block: Option<u32>,
    /// The 128-byte header with the first chunk's preamble, for the decoder.
    raw: Vec<u8>,
}

impl Header {
    /// Reads the header at the start of a file (at least the first 134 bytes of it).
    pub fn parse(data: &[u8]) -> Result<Header> {
        if data.len() < HEADER_SIZE + PREAMBLE_SIZE {
            return Err(Error::Invalid("short header"));
        }
        let magic = le16(data, 4) as u16;
        let speed = match le32(data, 0x10) {
            0 => DEFAULT_SPEED,
            s => s,
        };
        let (mut width, mut height) = (le16(data, 8), le16(data, 10));
        if width == 0 || height == 0 {
            // FFmpeg's workaround for a sample with no size
            width = 640;
            height = 480;
        }
        let carpet = le16(data, 0x10) as u16 == CHUNK_MAGIC_1;
        let tftd = le16(data, HEADER_SIZE + 4) as u16 == CHUNK_TFTD_AUDIO;
        let mut tftd_audio_block = None;
        let frame_rate = if tftd {
            let block = le32(data, HEADER_SIZE).max(1);
            tftd_audio_block = Some(block);
            (TFTD_SAMPLE_RATE, block)
        } else if carpet {
            (70, MC_SPEED)
        } else if magic == FILE_MAGIC_FLI {
            (70, speed)
        } else if magic == FILE_MAGIC_FLC || magic == FILE_MAGIC_DTA {
            (1000, speed)
        } else {
            return Err(Error::Invalid("unknown file magic"));
        };
        let mut depth = if carpet { 8 } else { le16(data, 12) };
        if depth == 0 {
            depth = 8;
        }
        if magic == FILE_MAGIC_FLC && depth == 16 {
            depth = 15;
        }
        Ok(Header {
            magic,
            width,
            height,
            depth,
            first_chunk: if carpet { 12 } else { HEADER_SIZE },
            carpet,
            frame_rate,
            tftd_audio_block,
            raw: data.get(..HEADER_SIZE).unwrap_or_default().to_vec(),
        })
    }

    /// The header bytes as read.
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
}

/// Reads like ffmpeg's `GetByteContext`: past the end it returns 0 and stays at the end.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn left(&self) -> usize {
        self.buf.len() - self.pos
    }
    fn byte(&mut self) -> u8 {
        match self.buf.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                b
            }
            None => 0,
        }
    }
    fn u16(&mut self) -> u32 {
        if self.left() < 2 {
            self.pos = self.buf.len();
            return 0;
        }
        let v = le16(self.buf, self.pos);
        self.pos += 2;
        v
    }
    fn u32(&mut self) -> u32 {
        if self.left() < 4 {
            self.pos = self.buf.len();
            return 0;
        }
        let v = le32(self.buf, self.pos);
        self.pos += 4;
        v
    }
    fn skip(&mut self, n: usize) {
        self.pos += n.min(self.left());
    }
}

/// A decoded picture: palette indices and the palette.
#[derive(Clone)]
pub struct Decoder {
    width: usize,
    height: usize,
    /// The row pitch of `pixels`: the width rounded up to 16, so a run past the end of a row
    /// lands in padding, as in ffmpeg's frame buffers.
    stride: usize,
    carpet: bool,
    pixels: Vec<u8>,
    palette: [u32; 256],
}

impl Decoder {
    pub fn new(h: &Header) -> Result<Decoder> {
        if h.depth != 8 {
            return Err(Error::Depth(h.depth));
        }
        if u64::from(h.width) * u64::from(h.height) > MAX_PIXELS {
            return Err(Error::Size(h.width, h.height));
        }
        let (width, height) = (h.width as usize, h.height as usize);
        let stride = (width + 15) & !15;
        Ok(Decoder { width, height, stride, carpet: h.carpet, pixels: vec![0; stride * height], palette: [0; 256] })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The palette indices of the current picture, `width * height`.
    pub fn indices(&self) -> Vec<u8> {
        self.pixels.chunks(self.stride.max(1)).flat_map(|r| r.iter().take(self.width).copied()).collect()
    }

    /// The palette as `0xAARRGGBB`.
    pub fn palette(&self) -> &[u32; 256] {
        &self.palette
    }

    /// The current picture as opaque RGBA, `width * height * 4` bytes.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 4);
        for i in self.indices() {
            let e = self.palette.get(usize::from(i)).copied().unwrap_or(0);
            out.extend_from_slice(&[(e >> 16) as u8, (e >> 8) as u8, e as u8, 255]);
        }
        out
    }

    fn check(&self, ptr: isize, n: isize) -> Result<()> {
        if ptr + n > (self.stride * self.height) as isize {
            return Err(Error::Invalid("pixel pointer out of range"));
        }
        Ok(())
    }

    fn put(&mut self, ptr: isize, v: u8) {
        if let Ok(i) = usize::try_from(ptr)
            && let Some(p) = self.pixels.get_mut(i)
        {
            *p = v;
        }
    }

    /// Applies one frame chunk (with its 16-byte frame header, as the demuxer packets it).
    pub fn decode(&mut self, buf: &[u8]) -> Result<()> {
        if buf.len() < 16 {
            return Err(Error::Invalid("short frame"));
        }
        let stride = self.stride as isize;
        let height = self.height as isize;
        let mut g = Reader { buf, pos: 0 };
        let mut frame_size = (g.u32() as usize).min(buf.len());
        g.skip(2);
        let mut num_chunks = g.u16();
        g.skip(8);
        if frame_size < 16 {
            return Err(Error::Invalid("short frame"));
        }
        frame_size -= 16;
        while frame_size >= 6 && num_chunks > 0 && g.left() >= 4 {
            let chunk_size = (g.u32() as usize).min(frame_size);
            let after = g.pos - 4 + chunk_size;
            let kind = g.u16() as u16;
            match kind {
                FLI_256_COLOR | FLI_COLOR => self.palette_chunk(&mut g, kind, after),
                FLI_DELTA => {
                    let mut y_ptr: isize = 0;
                    let mut lines = g.u16() as i32;
                    while lines > 0 {
                        if g.pos + 2 > after {
                            break;
                        }
                        self.check(y_ptr, 0)?;
                        let packets = g.u16() as u16 as i16 as i32;
                        if packets & 0xC000 == 0xC000 {
                            let n = -packets;
                            if n as isize > height {
                                return Err(Error::Invalid("line skip beyond the picture"));
                            }
                            y_ptr += n as isize * stride;
                        } else if packets & 0xC000 == 0x4000 {
                            // undefined opcode: ignored
                        } else if packets & 0xC000 == 0x8000 {
                            // the last pixel of the line (ffmpeg writes at the end of the padded
                            // row, so it drops the byte unless the width is a multiple of its row alignment)
                            let p = y_ptr + self.width as isize - 1;
                            self.check(p, 0)?;
                            self.put(p, (packets & 0xff) as u8);
                        } else {
                            lines -= 1;
                            let mut p = y_ptr;
                            self.check(p, 0)?;
                            for _ in 0..packets {
                                if g.pos + 2 > after {
                                    break;
                                }
                                p += isize::from(g.byte());
                                let run = isize::from(g.byte() as i8);
                                if run < 0 {
                                    let run = -run;
                                    let (a, b) = (g.byte(), g.byte());
                                    self.check(p, run * 2)?;
                                    for _ in 0..run {
                                        self.put(p, a);
                                        self.put(p + 1, b);
                                        p += 2;
                                    }
                                } else {
                                    self.check(p, run * 2)?;
                                    if g.pos + run as usize * 2 > after {
                                        break;
                                    }
                                    for _ in 0..run * 2 {
                                        let v = g.byte();
                                        self.put(p, v);
                                        p += 1;
                                    }
                                }
                            }
                            y_ptr += stride;
                        }
                    }
                }
                FLI_LC => {
                    let start = g.u16() as isize;
                    if start >= height {
                        return Err(Error::Invalid("starting line beyond the picture"));
                    }
                    let mut y_ptr = start * stride;
                    let mut lines = g.u16();
                    while lines > 0 {
                        let mut p = y_ptr;
                        self.check(p, 0)?;
                        if g.pos + 1 > after {
                            break;
                        }
                        let packets = g.byte();
                        for _ in 0..packets {
                            if g.pos + 1 > after {
                                break;
                            }
                            p += isize::from(g.byte());
                            let run = isize::from(g.byte() as i8);
                            if run > 0 {
                                self.check(p, run)?;
                                if g.pos + run as usize > after {
                                    break;
                                }
                                for _ in 0..run {
                                    let v = g.byte();
                                    self.put(p, v);
                                    p += 1;
                                }
                            } else if run < 0 {
                                let run = -run;
                                let v = g.byte();
                                self.check(p, run)?;
                                for _ in 0..run {
                                    self.put(p, v);
                                    p += 1;
                                }
                            }
                        }
                        y_ptr += stride;
                        lines -= 1;
                    }
                }
                FLI_BLACK => self.pixels.fill(0),
                FLI_BRUN => {
                    let mut y_ptr: isize = 0;
                    for _ in 0..height {
                        let mut p = y_ptr;
                        g.skip(1);
                        let mut countdown = self.width as isize;
                        while countdown > 0 {
                            if g.pos + 1 > after {
                                break;
                            }
                            let run = isize::from(g.byte() as i8);
                            if run == 0 {
                                return Err(Error::Invalid("zero byte run"));
                            }
                            if run > 0 {
                                let v = g.byte();
                                self.check(p, run)?;
                                for _ in 0..run {
                                    self.put(p, v);
                                    p += 1;
                                    countdown -= 1;
                                }
                            } else {
                                let run = -run;
                                self.check(p, run)?;
                                if g.pos + run as usize > after {
                                    break;
                                }
                                for _ in 0..run {
                                    let v = g.byte();
                                    self.put(p, v);
                                    p += 1;
                                    countdown -= 1;
                                }
                            }
                        }
                        y_ptr += stride;
                    }
                }
                FLI_COPY => {
                    let want = ((self.width + 3) & !3) * self.height;
                    if (chunk_size as u32).wrapping_sub(6) as usize != want {
                        g.skip((chunk_size as u32).wrapping_sub(6) as usize);
                    } else {
                        let mut y_ptr: isize = 0;
                        while self.check(y_ptr, stride).is_ok() {
                            for x in 0..self.width {
                                if g.left() == 0 {
                                    break;
                                }
                                let v = g.byte();
                                self.put(y_ptr + x as isize, v);
                            }
                            if self.width & 3 != 0 {
                                g.skip(4 - (self.width & 3));
                            }
                            y_ptr += stride;
                        }
                    }
                }
                FLI_MINI => {}
                _ => {}
            }
            if after >= g.pos {
                g.skip(after - g.pos);
            } else {
                break;
            }
            frame_size -= chunk_size;
            num_chunks -= 1;
        }
        Ok(())
    }

    fn palette_chunk(&mut self, g: &mut Reader<'_>, kind: u16, after: usize) {
        let shift = if kind == FLI_256_COLOR && !self.carpet { 0 } else { 2 };
        let packets = g.u16();
        let mut idx: usize = 0;
        for _ in 0..packets {
            idx += usize::from(g.byte());
            let mut changes = usize::from(g.byte());
            if changes == 0 {
                changes = 256;
            }
            if g.pos + changes * 3 > after {
                break;
            }
            for _ in 0..changes {
                if idx >= 256 {
                    idx = 0;
                }
                let r = u32::from(g.byte() << shift);
                let gr = u32::from(g.byte() << shift);
                let b = u32::from(g.byte() << shift);
                let mut e = 0xFFu32 << 24 | r << 16 | gr << 8 | b;
                if shift == 2 {
                    e |= e >> 6 & 0x30303;
                }
                if let Some(p) = self.palette.get_mut(idx) {
                    *p = e;
                }
                idx += 1;
            }
        }
    }
}
