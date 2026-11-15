// Ported from FFmpeg n7.1.1 libavcodec/shorten.c (Copyright (c) 2005 Jeff Muizelaar) and the
// Shorten Rice codes of libavcodec/golomb.h, LGPL-2.1-or-later; see docs/licensing.md §5.
// CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **Shorten** (`.shn`), Tony Robinson's lossless audio format: versions
//! 0–3, the fixed and quantised LPC predictors, running means, bit shifts and block-size
//! changes, 8-bit unsigned and 16-bit signed PCM with a WAVE or AIFF header kept verbatim in the
//! stream. The output is the same as ffmpeg's `shorten` decoder, sample for sample.
//!
//! [`decode_file`] decodes a whole file to one plane per channel.

use std::fmt;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not a Shorten stream (no 'ajkg' magic)")]
    NotShorten,
    #[error("invalid Shorten stream: {0}")]
    Invalid(&'static str),
    #[error("unsupported Shorten stream: {0}")]
    Unsupported(&'static str),
    #[error("no audio in the Shorten stream")]
    Empty,
}

/// The sample format of the decoded audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8-bit (0…255, silence 128).
    U8,
    /// Signed 16-bit.
    S16,
}

/// The stream parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// Shorten format version (0–3).
    pub version: u8,
    pub channels: u16,
    /// From the WAVE or AIFF header.
    pub sample_rate: u32,
    /// From the WAVE or AIFF header: 8 or 16.
    pub bits_per_sample: u16,
    pub format: SampleFormat,
}

impl Info {
    /// A decoded sample as `f32` in −1…1.
    pub fn to_f32(&self, v: i32) -> f32 {
        match self.format {
            SampleFormat::U8 => (v - 128) as f32 / 128.0,
            SampleFormat::S16 => v as f32 / 32768.0,
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SampleFormat::U8 => "u8",
            SampleFormat::S16 => "s16",
        })
    }
}

/// The decoded output is capped at this many samples per channel (about 6 hours at 48 kHz).
pub const MAX_SAMPLES: usize = 1 << 30;

const MAX_CHANNELS: u32 = 8;
const MAX_BLOCKSIZE: u32 = 65535;
const OUT_BUFFER_SIZE: u32 = 16384;
const ULONGSIZE: u32 = 2;
const DEFAULT_BLOCK_SIZE: u32 = 256;
const TYPESIZE: u32 = 4;
const CHANSIZE: u32 = 0;
const LPCQSIZE: u32 = 2;
const ENERGYSIZE: u32 = 3;
const BITSHIFTSIZE: u32 = 2;
const TYPE_U8: u32 = 2;
const TYPE_S16HL: u32 = 3;
const TYPE_S16LH: u32 = 5;
const NWRAP: usize = 3;
const NSKIPSIZE: u32 = 1;
const LPCQUANT: u32 = 5;
const V2LPCQOFFSET: i32 = 1 << LPCQUANT;
const FNSIZE: u32 = 2;
const FN_QUIT: u32 = 4;
const FN_BLOCKSIZE: u32 = 5;
const FN_BITSHIFT: u32 = 6;
const FN_QLPC: u32 = 7;
const FN_ZERO: u32 = 8;
const FN_VERBATIM: u32 = 9;
const VERBATIM_CKSIZE_SIZE: u32 = 5;
const VERBATIM_BYTE_SIZE: u32 = 8;
const CANONICAL_HEADER_SIZE: u32 = 44;
const FIXED_COEFFS: [[i32; 3]; 4] = [[0, 0, 0], [1, 0, 0], [2, -1, 0], [3, -3, 1]];

/// An MSB-first bit reader; bits past the end read as 0, as ffmpeg's padded buffer does.
struct Bits<'a> {
    data: &'a [u8],
    pos: u64,
}

impl Bits<'_> {
    fn size(&self) -> u64 {
        self.data.len() as u64 * 8
    }

    fn left(&self) -> i64 {
        self.size() as i64 - self.pos as i64
    }

    fn bit(&mut self) -> u32 {
        let b = usize::try_from(self.pos / 8).ok().and_then(|i| self.data.get(i)).map_or(0, |&b| u32::from(b >> (7 - self.pos % 8)) & 1);
        self.pos += 1;
        b
    }

    fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n.min(32) {
            v = (v << 1) | self.bit();
        }
        v
    }

    /// `get_ur_golomb_jpegls(gb, k, INT_MAX, 0)`: a unary prefix of zeros, a 1, then `k` bits;
    /// `u32::MAX` (−1) when the prefix runs past the end.
    fn ur(&mut self, k: u32) -> u32 {
        let mut i = 0u32;
        loop {
            if self.pos >= self.size() {
                return u32::MAX;
            }
            if self.bit() == 1 {
                break;
            }
            i = i.wrapping_add(1);
        }
        let low = self.bits(k);
        low.wrapping_add(i.checked_shl(k).unwrap_or(0))
    }

    /// `get_sr_golomb_shorten`.
    fn sr(&mut self, k: u32) -> i32 {
        let u = self.ur(k + 1) as i32;
        (u >> 1) ^ -(u & 1)
    }
}

/// The decoder state (`ShortenContext`).
struct Shorten<'a> {
    gb: Bits<'a>,
    version: u32,
    channels: usize,
    blocksize: usize,
    nmean: usize,
    ftype: u32,
    nwrap: usize,
    lpcqoffset: i32,
    bitshift: u32,
    swap: bool,
    /// Per channel: `nwrap` history samples, then `blocksize` samples.
    decoded: Vec<Vec<i32>>,
    offset: Vec<Vec<i32>>,
    coeffs: Vec<i32>,
}

impl Shorten<'_> {
    /// `get_uint`.
    fn uint(&mut self, k: u32) -> Result<u32, Error> {
        let mut k = k;
        if self.version != 0 {
            k = self.gb.ur(ULONGSIZE);
            if k > 31 {
                return Err(Error::Invalid("bad uint size"));
            }
        }
        Ok(self.gb.ur(k))
    }
}

/// `av_log2`.
fn log2(v: u32) -> u32 {
    31 - (v | 1).leading_zeros()
}

/// `decode_wave_header`: the sample rate and bits per sample.
fn wave_header(h: &[u8]) -> Result<(u32, u16), Error> {
    let le32 = |p: usize| h.get(p..p + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let le16 = |p: usize| h.get(p..p + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    if h.get(8..12) != Some(b"WAVE") {
        return Err(Error::Invalid("missing WAVE tag"));
    }
    let mut p = 12usize;
    while h.get(p..p + 4) != Some(b"fmt ") {
        let len = le32(p + 4) as i32;
        p = p.saturating_add(8).saturating_add(len.max(0) as usize);
        if len < 0 || h.len().saturating_sub(p) < 16 {
            return Err(Error::Invalid("no fmt chunk"));
        }
    }
    let len = le32(p + 4);
    if len < 16 {
        return Err(Error::Invalid("fmt chunk too short"));
    }
    if le16(p + 8) != 1 {
        return Err(Error::Unsupported("wave format"));
    }
    let rate = le32(p + 12);
    let bps = le16(p + 22);
    if bps != 16 && bps != 8 {
        return Err(Error::Unsupported("bits per sample"));
    }
    Ok((rate, bps))
}

/// `decode_aiff_header`: the sample rate, bits per sample and whether the samples are swapped
/// (AIFC).
fn aiff_header(h: &[u8]) -> Result<(u32, u16, bool), Error> {
    let be32 = |p: usize| h.get(p..p + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let tag = h.get(8..12);
    if tag != Some(b"AIFF") && tag != Some(b"AIFC") {
        return Err(Error::Invalid("missing AIFF tag"));
    }
    let swap = tag == Some(b"AIFC");
    let mut p = 12usize;
    while h.get(p..p + 4) != Some(b"COMM") {
        let len = be32(p + 4) as i32;
        let after = p.saturating_add(8);
        if len < 0 || (h.len().saturating_sub(after) as i64) < 18 + i64::from(len) + i64::from(len & 1) {
            return Err(Error::Invalid("no COMM chunk"));
        }
        p = after + len as usize + (len & 1) as usize;
    }
    let len = be32(p + 4);
    if len < 18 {
        return Err(Error::Invalid("COMM chunk too short"));
    }
    let c = p + 8;
    let bps = h.get(c + 6..c + 8).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
    if bps != 16 && bps != 8 {
        return Err(Error::Unsupported("bits per sample"));
    }
    let exp = i32::from(h.get(c + 8..c + 10).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))) - 16383 - 63;
    let val = h.get(c + 10..c + 18).map_or(0, |b| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
    if !(-63..=63).contains(&exp) {
        return Err(Error::Invalid("sample rate exponent"));
    }
    let rate = if exp >= 0 { val.wrapping_shl(exp as u32) } else { val.wrapping_add(1u64 << (-exp - 1)) >> -exp };
    Ok((rate as u32, bps, swap))
}

/// The stream parameters, without decoding the audio.
pub fn probe(data: &[u8]) -> Result<Info, Error> {
    Ok(open(data)?.0)
}

/// `read_header`.
fn open(data: &[u8]) -> Result<(Info, Shorten<'_>), Error> {
    let mut gb = Bits { data, pos: 0 };
    if gb.bits(32) != u32::from_be_bytes(*b"ajkg") {
        return Err(Error::NotShorten);
    }
    let version = gb.bits(8);
    let mut s = Shorten {
        gb,
        version,
        channels: 0,
        blocksize: DEFAULT_BLOCK_SIZE as usize,
        nmean: 0,
        ftype: 0,
        nwrap: NWRAP,
        lpcqoffset: 0,
        bitshift: 0,
        swap: false,
        decoded: Vec::new(),
        offset: Vec::new(),
        coeffs: Vec::new(),
    };
    s.ftype = s.uint(TYPESIZE)?;
    let channels = s.uint(CHANSIZE)?;
    if channels == 0 {
        return Err(Error::Invalid("no channels"));
    }
    if channels > MAX_CHANNELS {
        return Err(Error::Unsupported("more than 8 channels"));
    }
    s.channels = channels as usize;
    let mut maxnlpc = 0u32;
    // -1 in ffmpeg: no running mean before version 1
    let mut nmean: i64 = -1;
    if version > 0 {
        let blocksize = s.uint(log2(DEFAULT_BLOCK_SIZE))?;
        if blocksize == 0 || blocksize > MAX_BLOCKSIZE {
            return Err(Error::Invalid("block size"));
        }
        s.blocksize = blocksize as usize;
        maxnlpc = s.uint(LPCQSIZE)?;
        if maxnlpc > 1024 {
            return Err(Error::Invalid("LPC order"));
        }
        let m = s.uint(0)?;
        if m > 32768 {
            return Err(Error::Invalid("mean length"));
        }
        nmean = i64::from(m);
        let skip = s.uint(NSKIPSIZE)?;
        if i64::from(skip) > s.gb.left().max(0) / 8 {
            return Err(Error::Invalid("skip bytes"));
        }
        s.gb.pos += u64::from(skip) * 8;
    }
    s.nwrap = NWRAP.max(maxnlpc as usize);
    if version > 1 {
        s.lpcqoffset = V2LPCQOFFSET;
    }
    if s.gb.ur(FNSIZE) != FN_VERBATIM {
        return Err(Error::Invalid("no verbatim header"));
    }
    let header_size = s.gb.ur(VERBATIM_CKSIZE_SIZE);
    if !(CANONICAL_HEADER_SIZE..OUT_BUFFER_SIZE).contains(&header_size) {
        return Err(Error::Invalid("header size"));
    }
    let header: Vec<u8> = (0..header_size).map(|_| s.gb.ur(VERBATIM_BYTE_SIZE) as u8).collect();
    let (sample_rate, bits_per_sample) = match header.get(..4) {
        Some(b"RIFF") => wave_header(&header)?,
        Some(b"FORM") => {
            let (r, b, swap) = aiff_header(&header)?;
            s.swap = swap;
            (r, b)
        }
        _ => return Err(Error::Unsupported("bit packing")),
    };
    // allocate_buffers / init_offset; ffmpeg keeps nmean = -1 for version 0 (no running mean)
    s.nmean = usize::try_from(nmean).unwrap_or(0);
    let mean = match s.ftype {
        TYPE_U8 => 0x80,
        TYPE_S16HL | TYPE_S16LH => 0,
        _ => return Err(Error::Unsupported("audio type")),
    };
    s.offset = vec![vec![mean; s.nmean.max(1)]; s.channels];
    s.decoded = vec![vec![0; s.nwrap + s.blocksize]; s.channels];
    s.coeffs = vec![0; s.nwrap];
    let format = if s.ftype == TYPE_U8 { SampleFormat::U8 } else { SampleFormat::S16 };
    let info = Info { version: version as u8, channels: channels as u16, sample_rate, bits_per_sample, format };
    if nmean < 0 {
        s.nmean = usize::MAX; // marks "no running mean" (see `coffset`)
    }
    Ok((info, s))
}

impl Shorten<'_> {
    /// The offset for the next block of `ch` (`coffset`).
    fn coffset(&self, ch: usize) -> i32 {
        let off = &self.offset[ch];
        match self.nmean {
            0 => off[0],
            // version 0: ffmpeg's nmean is -1; the sum loop does not run and 0 / -1 is 0
            usize::MAX => 0,
            n => {
                let mut sum: i32 = if self.version < 2 { 0 } else { (n / 2) as i32 };
                for &v in off.iter().take(n) {
                    sum = sum.wrapping_add(v);
                }
                let c = sum / n as i32;
                if self.version >= 2 && self.bitshift != 0 { c >> (self.bitshift - 1) >> 1 } else { c }
            }
        }
    }

    /// `decode_subframe_lpc`.
    fn lpc(&mut self, cmd: u32, ch: usize, residual_size: u32, coffset: i32) -> Result<(), Error> {
        let nwrap = self.nwrap;
        let (order, qshift) = if cmd == FN_QLPC {
            let order = self.gb.ur(LPCQSIZE);
            if order as usize > nwrap {
                return Err(Error::Invalid("prediction order"));
            }
            for i in 0..order as usize {
                self.coeffs[i] = self.gb.sr(LPCQUANT);
            }
            (order as usize, LPCQUANT)
        } else {
            (cmd as usize, 0)
        };
        let fixed = FIXED_COEFFS.get(order).copied().unwrap_or([0; 3]);
        let coeffs: &[i32] = if cmd == FN_QLPC { &self.coeffs[..order] } else { &fixed[..order] };
        let dec = &mut self.decoded[ch];
        if cmd == FN_QLPC && coffset != 0 {
            for v in &mut dec[nwrap - order..nwrap] {
                *v = v.wrapping_sub(coffset);
            }
        }
        let init = if order == 0 {
            coffset
        } else if cmd == FN_QLPC {
            self.lpcqoffset
        } else {
            0
        };
        for i in 0..self.blocksize {
            let at = nwrap + i;
            let mut sum = init;
            for (j, &c) in coeffs.iter().enumerate() {
                sum = sum.wrapping_add(c.wrapping_mul(dec[at - j - 1]));
            }
            dec[at] = self.gb.sr(residual_size).wrapping_add(sum >> qshift);
        }
        if cmd == FN_QLPC && coffset != 0 {
            for v in &mut dec[nwrap..nwrap + self.blocksize] {
                *v = v.wrapping_add(coffset);
            }
        }
        Ok(())
    }
}

/// Decodes a whole Shorten file into one plane per channel: unsigned 8-bit or signed 16-bit
/// sample values, as [`Info::format`] says. A stream cut short or damaged after the header
/// decodes as far as it can.
pub fn decode_file(data: &[u8]) -> Result<(Info, Vec<Vec<i32>>), Error> {
    let (info, mut s) = open(data)?;
    let mut planes: Vec<Vec<i32>> = vec![Vec::new(); s.channels];
    let nwrap = s.nwrap;
    let mut cur = 0usize;
    'stream: loop {
        if s.gb.left() < i64::from(3 + FNSIZE) {
            break;
        }
        let cmd = s.gb.ur(FNSIZE);
        if cmd > FN_VERBATIM {
            break;
        }
        match cmd {
            FN_VERBATIM => {
                let len = s.gb.ur(VERBATIM_CKSIZE_SIZE) as i32;
                if len < 0 || i64::from(len) > s.gb.left() {
                    break;
                }
                for _ in 0..len {
                    s.gb.ur(VERBATIM_BYTE_SIZE);
                }
            }
            FN_BITSHIFT => {
                let b = s.gb.ur(BITSHIFTSIZE);
                if b > 32 {
                    break;
                }
                s.bitshift = b;
            }
            FN_BLOCKSIZE => {
                let Ok(b) = s.uint(log2(s.blocksize as u32)) else { break };
                if b as usize > s.blocksize || b == 0 || b > MAX_BLOCKSIZE {
                    break;
                }
                s.blocksize = b as usize;
            }
            FN_QUIT => break,
            _ => {
                let ch = cur;
                let mut residual_size = 0u32;
                if cmd != FN_ZERO {
                    let mut r = s.gb.ur(ENERGYSIZE) as i32;
                    if s.version == 0 {
                        r = r.wrapping_sub(1);
                    }
                    if r as u32 > 30 {
                        break;
                    }
                    residual_size = r as u32;
                }
                let coffset = s.coffset(ch);
                if cmd == FN_ZERO {
                    s.decoded[ch][nwrap..nwrap + s.blocksize].fill(0);
                } else if s.lpc(cmd, ch, residual_size, coffset).is_err() {
                    break;
                }
                let bs = s.blocksize;
                if s.nmean > 0 && s.nmean != usize::MAX {
                    let n = s.nmean;
                    let mut sum: i64 = if s.version < 2 { 0 } else { (bs / 2) as i64 };
                    for &v in &s.decoded[ch][nwrap..nwrap + bs] {
                        sum += i64::from(v);
                    }
                    let off = &mut s.offset[ch];
                    off.copy_within(1..n, 0);
                    let mean = sum / bs as i64;
                    off[n - 1] = if s.version < 2 {
                        mean as i32
                    } else if s.bitshift == 32 {
                        0
                    } else {
                        mean.wrapping_mul(1i64 << s.bitshift) as i32
                    };
                }
                let dec = &mut s.decoded[ch];
                dec.copy_within(bs..bs + nwrap, 0);
                // fix_bitshift on the block (after the history copy, as ffmpeg)
                let out = &mut dec[nwrap..nwrap + bs];
                if s.bitshift == 32 {
                    out.fill(0);
                } else if s.bitshift != 0 {
                    for v in out.iter_mut() {
                        *v = (*v as u32).wrapping_shl(s.bitshift) as i32;
                    }
                }
                cur += 1;
                if cur == s.channels {
                    cur = 0;
                    for (c, plane) in planes.iter_mut().enumerate() {
                        for &v in &s.decoded[c][nwrap..nwrap + bs] {
                            let v = match info.format {
                                SampleFormat::U8 => v.clamp(0, 255),
                                SampleFormat::S16 => {
                                    let v = v.clamp(-32768, 32767);
                                    if s.swap { i32::from((v as i16).swap_bytes()) } else { v }
                                }
                            };
                            plane.push(v);
                        }
                    }
                    if planes.first().map_or(0, Vec::len) >= MAX_SAMPLES {
                        break 'stream;
                    }
                }
            }
        }
    }
    for p in &mut planes {
        p.truncate(MAX_SAMPLES);
    }
    if planes.first().is_none_or(Vec::is_empty) {
        return Err(Error::Empty);
    }
    Ok((info, planes))
}
