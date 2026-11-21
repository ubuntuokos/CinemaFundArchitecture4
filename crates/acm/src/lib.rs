// Ported from FFmpeg n7.1.1 libavcodec/interplayacm.c (Copyright (c) 2004-2008 Marko Kreen,
// (c) 2008 Adam Gashlin, (c) 2015 Paul B Mahol; ISC licence, see COPYING.ISC-ACM) and
// libavformat/acm.c (Copyright (c) 2015 Paul B Mahol, LGPL-2.1-or-later); see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **Interplay ACM** audio (`.acm`, the audio of Interplay's games), as
//! ffmpeg's `interplayacm` decoder: the output is the same samples, sample for sample.
//!
//! An ACM file is a 14-byte header (`97 28 03 01`, the number of samples in all channels, the
//! channel count, the sample rate, and the block shape) followed by one bit stream cut into
//! blocks of `rows × 2^level` samples. Each block is a column-by-column quantised matrix that
//! is inverse-transformed ("juggled") in place.
//!
//! - [`parse_header`] reads the header, [`Decoder`] decodes a byte stream block by block.
//! - [`decode_file`] decodes a whole `.acm` file to interleaved 16-bit samples.

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not an Interplay ACM stream")]
    NotAcm,
    #[error("invalid Interplay ACM stream: {0}")]
    Invalid(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Length of the file header.
pub const HEADER_BYTES: usize = 14;
/// Largest block (samples in all channels) accepted: 2²⁴.
pub const MAX_BLOCK: usize = 1 << 24;
/// Cap on decoded samples (all channels) for [`decode_file`]: 2²⁹ (1 GiB of `i16`).
pub const MAX_FILE_SAMPLES: u64 = 1 << 29;

const MAP_1BIT: [i32; 2] = [-1, 1];
const MAP_2BIT_NEAR: [i32; 4] = [-2, -1, 1, 2];
const MAP_2BIT_FAR: [i32; 4] = [-3, -2, 2, 3];
const MAP_3BIT: [i32; 8] = [-4, -3, -2, -1, 1, 2, 3, 4];

/// What a header says about the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub channels: u32,
    pub sample_rate: u32,
    /// Samples per channel (the header's count over the channels); 0 when the file does not say.
    pub frames: u64,
    pub level: u32,
    pub rows: u32,
}

impl Header {
    /// Samples (all channels) in one block.
    pub fn block_len(&self) -> usize {
        (self.rows as usize) << self.level
    }
}

/// Whether `head` starts like an ACM file.
pub fn probe(head: &[u8]) -> bool {
    head.starts_with(&[0x97, 0x28, 0x03, 0x01])
}

/// Read the 14-byte header.
pub fn parse_header(data: &[u8]) -> Result<Header> {
    let h = data.get(..HEADER_BYTES).ok_or(Error::NotAcm)?;
    if !probe(h) {
        return Err(Error::NotAcm);
    }
    let le16 = |i: usize| u32::from(u16::from_le_bytes([h[i], h[i + 1]]));
    let total = u64::from(u32::from_le_bytes([h[4], h[5], h[6], h[7]]));
    let channels = le16(8);
    let sample_rate = le16(10);
    if channels == 0 || sample_rate == 0 {
        return Err(Error::Invalid("no channels or sample rate"));
    }
    let shape = le16(12);
    let (level, rows) = (shape & 0xF, shape >> 4);
    if rows == 0 {
        return Err(Error::Invalid("empty block"));
    }
    let header = Header { channels, sample_rate, frames: total / u64::from(channels), level, rows };
    if header.block_len() > MAX_BLOCK {
        return Err(Error::Invalid("block too large"));
    }
    Ok(header)
}

/// Little-endian bit reader; reading past the end gives zero bits (ffmpeg reads zero padding).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let p = self.pos + i as usize;
            let byte = self.data.get(p >> 3).copied().unwrap_or(0);
            v |= u32::from((byte >> (p & 7)) & 1) << i;
        }
        self.pos += n as usize;
        v
    }
    fn bit(&mut self) -> usize {
        self.get(1) as usize
    }
}

/// A column filler: reads one column of `rows` values into the block.
#[derive(Clone, Copy)]
enum Filler {
    Zero,
    Bad,
    Linear,
    K13,
    K12,
    T15,
    K24,
    K23,
    T27,
    K35,
    K34,
    K45,
    K44,
    T37,
}

const FILLERS: [Filler; 32] = {
    use Filler::{Bad, K12, K13, K23, K24, K34, K35, K44, K45, Linear, T15, T27, T37, Zero};
    [
        Zero, Bad, Bad, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear, K13, K12, T15, K24, K23,
        T27, K35, K34, Bad, K45, K44, Bad, T37, Bad, Bad,
    ]
};

/// An ACM decoder: bytes in, interleaved 16-bit samples out.
#[derive(Clone)]
pub struct Decoder {
    channels: usize,
    level: u32,
    rows: usize,
    cols: usize,
    block: Vec<i32>,
    wrap: Vec<i32>,
    /// ffmpeg's `ampbuf`: the table of amplitudes, indexed from the middle (so negative too);
    /// entries a block does not write keep the value an earlier block left.
    amp: Vec<i32>,
    /// Samples per channel still to output (`u64::MAX`: unlimited).
    max_samples: u64,
    /// Bits of the first byte of the pending data the last block already used.
    skip: u32,
    pending: Vec<u8>,
    failed: bool,
}

const MID: usize = 0x8000;

impl Decoder {
    pub fn new(h: &Header) -> Self {
        let cols = 1usize << h.level;
        Self {
            channels: h.channels as usize,
            level: h.level,
            rows: h.rows as usize,
            cols,
            block: vec![0; h.block_len()],
            wrap: vec![0; (2 * cols).saturating_sub(2)],
            amp: vec![0; 0x10000],
            max_samples: if h.frames == 0 { u64::MAX } else { h.frames },
            skip: 0,
            pending: Vec::new(),
            failed: false,
        }
    }

    /// Whether the stream is finished: its declared samples are all out, or it was damaged.
    pub fn is_done(&self) -> bool {
        self.failed || self.max_samples == 0
    }

    fn set(&mut self, row: usize, col: usize, idx: i32) {
        let v = self.amp[(MID as i64 + i64::from(idx)) as usize & 0xFFFF];
        let pos = (row << self.level) + col;
        if let Some(b) = self.block.get_mut(pos) {
            *b = v;
        }
    }

    fn fill_column(&mut self, g: &mut Bits<'_>, kind: Filler, ind: u32, col: usize) -> Result<()> {
        let rows = self.rows;
        match kind {
            Filler::Zero => (0..rows).for_each(|i| self.set(i, col, 0)),
            Filler::Bad => return Err(Error::Invalid("bad column filler")),
            Filler::Linear => {
                let middle = 1i32 << (ind - 1);
                for i in 0..rows {
                    let b = g.get(ind) as i32;
                    self.set(i, col, b - middle);
                }
            }
            Filler::K13 => {
                let mut i = 0;
                while i < rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, 0);
                    } else if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else {
                        let b = g.bit();
                        self.set(i, col, MAP_1BIT[b]);
                    }
                    i += 1;
                }
            }
            Filler::K12 => {
                for i in 0..rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else {
                        let b = g.bit();
                        self.set(i, col, MAP_1BIT[b]);
                    }
                }
            }
            Filler::K24 => {
                let mut i = 0;
                while i < rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, 0);
                    } else if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else {
                        let b = g.get(2) as usize;
                        self.set(i, col, MAP_2BIT_NEAR[b]);
                    }
                    i += 1;
                }
            }
            Filler::K23 => {
                for i in 0..rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else {
                        let b = g.get(2) as usize;
                        self.set(i, col, MAP_2BIT_NEAR[b]);
                    }
                }
            }
            Filler::K35 => {
                let mut i = 0;
                while i < rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, 0);
                    } else if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else if g.bit() == 0 {
                        let b = g.bit();
                        self.set(i, col, MAP_1BIT[b]);
                    } else {
                        let b = g.get(2) as usize;
                        self.set(i, col, MAP_2BIT_FAR[b]);
                    }
                    i += 1;
                }
            }
            Filler::K34 => {
                for i in 0..rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else if g.bit() == 0 {
                        let b = g.bit();
                        self.set(i, col, MAP_1BIT[b]);
                    } else {
                        let b = g.get(2) as usize;
                        self.set(i, col, MAP_2BIT_FAR[b]);
                    }
                }
            }
            Filler::K45 => {
                let mut i = 0;
                while i < rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, 0);
                    } else if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else {
                        let b = g.get(3) as usize;
                        self.set(i, col, MAP_3BIT[b]);
                    }
                    i += 1;
                }
            }
            Filler::K44 => {
                for i in 0..rows {
                    if g.bit() == 0 {
                        self.set(i, col, 0);
                    } else {
                        let b = g.get(3) as usize;
                        self.set(i, col, MAP_3BIT[b]);
                    }
                }
            }
            Filler::T15 => self.triples(g, col, 5, 26, 3, 1)?,
            Filler::T27 => self.triples(g, col, 7, 124, 5, 2)?,
            Filler::T37 => {
                let mut i = 0;
                while i < rows {
                    let b = g.get(7) as usize;
                    if b > 120 {
                        return Err(Error::Invalid("pair code too large"));
                    }
                    self.set(i, col, (b % 11) as i32 - 5);
                    i += 1;
                    if i >= rows {
                        break;
                    }
                    self.set(i, col, (b / 11) as i32 - 5);
                    i += 1;
                }
            }
        }
        Ok(())
    }

    /// `t15` / `t27`: three values per `bits`-bit code in base `base`, each offset by `bias`.
    fn triples(&mut self, g: &mut Bits<'_>, col: usize, bits: u32, max: usize, base: usize, bias: i32) -> Result<()> {
        let mut i = 0;
        while i < self.rows {
            let b = g.get(bits) as usize;
            if b > max {
                return Err(Error::Invalid("triple code too large"));
            }
            for k in 0..3 {
                let n = ((b / base.pow(k)) % base) as i32 - bias;
                self.set(i, col, n);
                i += 1;
                if i >= self.rows {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn juggle_block(&mut self) {
        if self.level == 0 {
            return;
        }
        let step = if self.level > 9 { 1 } else { (2048usize >> self.level) - 2 };
        let mut todo = self.rows;
        let mut block_p = 0usize;
        loop {
            let mut wrap_p = 0usize;
            let mut sub_count = step.min(todo);
            let mut sub_len = self.cols / 2;
            sub_count *= 2;
            juggle(&mut self.wrap, wrap_p, &mut self.block, block_p, sub_len, sub_count);
            wrap_p += sub_len * 2;
            let mut p = block_p;
            for _ in 0..sub_count {
                self.block[p] = self.block[p].wrapping_add(1);
                p += sub_len;
            }
            while sub_len > 1 {
                sub_len /= 2;
                sub_count *= 2;
                juggle(&mut self.wrap, wrap_p, &mut self.block, block_p, sub_len, sub_count);
                wrap_p += sub_len * 2;
            }
            if todo <= step {
                break;
            }
            todo -= step;
            block_p += step << self.level;
        }
    }

    /// Decode one block from `data`; the bit position after it.
    fn decode_block(&mut self, data: &[u8]) -> Result<usize> {
        let mut g = Bits { data, pos: self.skip as usize };
        let pwr = g.get(4);
        let val = g.get(16) as i32;
        let count = 1usize << pwr;
        let mut x = 0i32;
        for i in 0..count {
            self.amp[MID + i] = x;
            x = x.wrapping_add(val);
        }
        let mut x = val.wrapping_neg();
        for i in 1..=count {
            self.amp[MID - i] = x;
            x = x.wrapping_sub(val);
        }
        for col in 0..self.cols {
            let ind = g.get(5);
            self.fill_column(&mut g, FILLERS[ind as usize], ind, col)?;
        }
        self.juggle_block();
        Ok(g.pos)
    }

    /// Decode one block from the start of the pending bytes. ffmpeg decodes exactly one block's
    /// worth of bytes at a time (the bits after them read as zero); at the end of the stream
    /// what is left is the block's data.
    fn run_block(&mut self, out: &mut Vec<i16>, at_end: bool) -> Result<()> {
        let limit = if at_end { self.pending.len() } else { self.block.len().min(self.pending.len()) };
        let pending = std::mem::take(&mut self.pending);
        let r = self.decode_block(&pending[..limit]);
        self.pending = pending;
        let bits = match r {
            Ok(b) => b,
            Err(e) => {
                self.failed = true;
                return Err(e);
            }
        };
        let used = bits / 8;
        if used > limit && !at_end {
            self.failed = true;
            return Err(Error::Invalid("block runs past its data"));
        }
        let n = (self.block.len() / self.channels).min(usize::try_from(self.max_samples).unwrap_or(usize::MAX));
        self.max_samples -= (n as u64).min(self.max_samples);
        out.extend(self.block.iter().take(n * self.channels).map(|&v| (v >> self.level) as i16));
        self.skip = (bits % 8) as u32;
        self.pending.drain(..used.min(self.pending.len()));
        Ok(())
    }

    /// Feed bytes; decoded samples (all channels, interleaved) are appended to `out`. After an
    /// error the stream is dead (ffmpeg's is too); [`Decoder::is_done`] says so.
    pub fn push(&mut self, data: &[u8], out: &mut Vec<i16>) -> Result<()> {
        if self.is_done() {
            return Ok(());
        }
        self.pending.extend_from_slice(data);
        while self.pending.len() >= self.block.len() && !self.is_done() {
            self.run_block(out, false)?;
        }
        Ok(())
    }

    /// End of the stream: decode what is left (zero padded), as ffmpeg's flush does.
    pub fn finish(&mut self, out: &mut Vec<i16>) -> Result<()> {
        while !self.pending.is_empty() && !self.is_done() {
            self.run_block(out, true)?;
        }
        Ok(())
    }
}

fn juggle(wrap: &mut [i32], wrap_p: usize, block: &mut [i32], block_p: usize, sub_len: usize, sub_count: usize) {
    let mut bp = block_p;
    let mut wp = wrap_p;
    for _ in 0..sub_len {
        let mut p = bp;
        let mut r0 = wrap[wp] as u32;
        let mut r1 = wrap[wp + 1] as u32;
        for _ in 0..sub_count / 2 {
            let r2 = block[p] as u32;
            block[p] = r1.wrapping_mul(2).wrapping_add(r0.wrapping_add(r2)) as i32;
            p += sub_len;
            let r3 = block[p] as u32;
            block[p] = r2.wrapping_mul(2).wrapping_sub(r1.wrapping_add(r3)) as i32;
            p += sub_len;
            r0 = r2;
            r1 = r3;
        }
        wrap[wp] = r0 as i32;
        wrap[wp + 1] = r1 as i32;
        wp += 2;
        bp += 1;
    }
}

/// Decode a whole `.acm` file: the header and the interleaved 16-bit samples (the declared
/// length; a stream that ends or breaks early gives the samples before that point).
pub fn decode_file(data: &[u8]) -> Result<(Header, Vec<i16>)> {
    let h = parse_header(data)?;
    let mut dec = Decoder::new(&h);
    let mut out = Vec::new();
    let body = &data[HEADER_BYTES..];
    let cap = h.frames.saturating_mul(u64::from(h.channels)).min(MAX_FILE_SAMPLES);
    // blocks are decoded as the bytes arrive, a block's worth at a time
    for chunk in body.chunks(h.block_len().max(1)) {
        if dec.push(chunk, &mut out).is_err() || out.len() as u64 >= cap && h.frames != 0 {
            break;
        }
    }
    if !dec.is_done() {
        let _ = dec.finish(&mut out);
    }
    if h.frames != 0 {
        out.truncate(cap as usize);
    }
    Ok((h, out))
}
