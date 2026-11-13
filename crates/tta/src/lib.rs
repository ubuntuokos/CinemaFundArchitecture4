// Ported from libtta 2.3 (libtta-c-2.3: libtta.c, filter.h; Copyright (c) 1999-2015 Aleksander
// Djuric), LGPL-3.0 (`COPYING.LGPL-3.0`); see docs/licensing.md §5. CFA4 changes
// GPL-3.0-or-later.

//! Pure-Rust TTA (True Audio, TTA1) decoder (codec backlog CB-9.1).
//!
//! A TTA1 stream is a 22-byte header (`TTA1`, format, channels, bits, sample rate, length, CRC),
//! a seek table of frame sizes and frames of `256 × rate / 245` samples (about 1.045 s). Each
//! frame codes every channel with adaptive Rice codes, an adaptive hybrid filter and a fixed
//! first-order predictor; channels are decorrelated, and the frame ends with its CRC-32.
//!
//! - [`Info::parse`] reads the header (after an optional ID3v2 tag).
//! - [`Decoder::decode_frame`] decodes one frame into planar samples.
//! - [`decode_file`] decodes a whole `.tta` file.
//!
//! Supported: 8-, 16- and 24-bit audio, 1 to 6 channels (8 like ffmpeg), plain and
//! password-protected (format 2) streams. A frame that fails its CRC decodes to silence of its
//! length, as in libtta.
//!
//! The decoding algorithm is ported from libtta 2.3 (LGPL-3.0, Copyright (c) 1999-2015 Aleksander
//! Djuric; see `COPYING.LGPL-3.0` and `docs/licensing.md` §5).

/// Errors from header parsing and decoding. A damaged frame that still parses is not an error
/// (it decodes to silence).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("TTA data is truncated")]
    Truncated,
    #[error("invalid TTA data: {0}")]
    Invalid(&'static str),
    #[error("unsupported TTA stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Length of the TTA1 header (without an ID3v2 tag).
pub const HEADER_LEN: usize = 22;
/// The most channels accepted (ffmpeg's limit; libtta stops at 6).
pub const MAX_CHANNELS: u16 = 8;
/// Cap on decoded values (samples × channels) for [`decode_file`]: 2³⁰, 4 GiB of `i32`.
pub const MAX_FILE_VALUES: u64 = 1 << 30;

/// Stream format codes of the header.
pub const FORMAT_SIMPLE: u16 = 1;
pub const FORMAT_ENCRYPTED: u16 = 2;

/// The stream parameters of a TTA1 header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// [`FORMAT_SIMPLE`] or [`FORMAT_ENCRYPTED`].
    pub format: u16,
    pub channels: u16,
    /// Bits per sample: 8, 16 or 24.
    pub bits: u16,
    pub sample_rate: u32,
    /// Samples per channel of the whole stream.
    pub samples: u32,
}

/// CRC-32 (IEEE, reflected) as TTA uses it.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn le16(b: &[u8], i: usize) -> u16 {
    match b.get(i..i + 2) {
        Some(&[a, b]) => u16::from_le_bytes([a, b]),
        _ => 0,
    }
}

fn le32(b: &[u8], i: usize) -> u32 {
    match b.get(i..i + 4) {
        Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => 0,
    }
}

/// The length of an ID3v2 tag at the start of `data` (0 without one).
fn id3v2_len(data: &[u8]) -> usize {
    match data.get(..10) {
        Some(h) if h.starts_with(b"ID3") => {
            let footer = if h[5] & 0x10 != 0 { 10 } else { 0 };
            let size = h[6..10].iter().fold(0usize, |s, &b| (s << 7) | usize::from(b & 0x7F));
            10 + size + footer
        }
        _ => 0,
    }
}

impl Info {
    /// Reads the header at the start of `data` (after an optional ID3v2 tag); returns the
    /// parameters and the offset of the seek table.
    pub fn parse(data: &[u8]) -> Result<(Info, usize)> {
        let at = id3v2_len(data);
        let h = data.get(at..at.checked_add(HEADER_LEN).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
        if !h.starts_with(b"TTA1") {
            return Err(Error::Invalid("no TTA1 header"));
        }
        if crc32(&h[..18]) != le32(h, 18) {
            return Err(Error::Invalid("header CRC"));
        }
        let info = Info { format: le16(h, 4), channels: le16(h, 6), bits: le16(h, 8), sample_rate: le32(h, 10), samples: le32(h, 14) };
        info.check()?;
        Ok((info, at + HEADER_LEN))
    }

    /// The parameters this decoder handles.
    pub fn check(&self) -> Result<()> {
        if self.format != FORMAT_SIMPLE && self.format != FORMAT_ENCRYPTED {
            return Err(Error::Unsupported("format"));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(Error::Unsupported("channel count"));
        }
        if !matches!(self.bits, 8 | 16 | 24) {
            return Err(Error::Unsupported("bits per sample"));
        }
        if self.sample_rate == 0 || self.sample_rate > 1_000_000 {
            return Err(Error::Invalid("sample rate"));
        }
        Ok(())
    }

    /// Samples per channel of a full frame (`256 × rate / 245`).
    pub fn frame_samples(&self) -> u32 {
        (256 * u64::from(self.sample_rate) / 245) as u32
    }

    /// Number of frames.
    pub fn frames(&self) -> u32 {
        let f = self.frame_samples().max(1);
        self.samples.div_ceil(f)
    }

    /// Samples per channel of frame `index`.
    pub fn frame_len(&self, index: u32) -> u32 {
        let f = self.frame_samples().max(1);
        let last = self.samples % f;
        if index + 1 == self.frames() && last != 0 { last } else { f }
    }

    /// A sample of this stream as a float in [-1, 1).
    pub fn to_f32(&self, v: i32) -> f32 {
        v as f32 / (1u32 << (self.bits.clamp(8, 24) - 1)) as f32
    }
}

/// The 8 key bytes of a password (CRC-64/ECMA-182 of its bytes, as libtta's
/// `compute_key_digits`).
pub fn password_key(password: &[u8]) -> [i8; 8] {
    const POLY: u64 = 0x42F0_E1EB_A9EA_3693;
    let mut crc = u64::MAX;
    for &b in password {
        crc ^= u64::from(b) << 56;
        for _ in 0..8 {
            crc = if crc >> 63 == 1 { (crc << 1) ^ POLY } else { crc << 1 };
        }
    }
    (!crc).to_le_bytes().map(|b| b as i8)
}

/// The adaptive hybrid filter of one channel (`TTA_fltst`).
#[derive(Clone, Copy)]
struct Filter {
    shift: u32,
    round: i32,
    error: i32,
    qm: [i32; 8],
    dx: [i32; 8],
    dl: [i32; 8],
}

impl Filter {
    fn new(key: &[i8; 8], shift: u32) -> Self {
        Self { shift, round: 1 << (shift - 1), error: 0, qm: key.map(i32::from), dx: [0; 8], dl: [0; 8] }
    }

    #[inline]
    fn decode(&mut self, v: i32) -> i32 {
        let (a, b, m) = (&mut self.dl, &mut self.qm, &mut self.dx);
        if self.error < 0 {
            for (b, m) in b.iter_mut().zip(m.iter()) {
                *b = b.wrapping_sub(*m);
            }
        } else if self.error > 0 {
            for (b, m) in b.iter_mut().zip(m.iter()) {
                *b = b.wrapping_add(*m);
            }
        }
        let sum = a.iter().zip(b.iter()).fold(self.round, |s, (a, b)| s.wrapping_add(a.wrapping_mul(*b)));
        m.copy_within(1..5, 0);
        a.copy_within(1..5, 0);
        m[4] = (a[4] >> 30) | 1;
        m[5] = ((a[5] >> 30) | 2) & !1;
        m[6] = ((a[6] >> 30) | 2) & !1;
        m[7] = ((a[7] >> 30) | 4) & !3;
        self.error = v;
        let v = v.wrapping_add(sum >> self.shift);
        a[4] = a[5].wrapping_neg();
        a[5] = a[6].wrapping_neg();
        a[6] = v.wrapping_sub(a[7]);
        a[7] = v;
        a[5] = a[5].wrapping_add(a[6]);
        a[4] = a[4].wrapping_add(a[5]);
        v
    }
}

/// `1 << k` for the Rice parameters, saturating at bit 31 as libtta's `bit_shift` table.
fn bit_shift(k: u32) -> Option<u32> {
    if k > 39 { None } else { Some(1u32 << k.min(31)) }
}

/// `shift_16[k]` of libtta: `bit_shift[k + 4]`.
fn shift_16(k: u32) -> Option<u32> {
    bit_shift(k + 4)
}

/// The adaptive Rice parameters of one channel (`TTA_adapt`).
#[derive(Clone, Copy)]
struct Rice {
    k0: u32,
    k1: u32,
    sum0: u32,
    sum1: u32,
}

impl Rice {
    fn new() -> Self {
        Self { k0: 10, k1: 10, sum0: 1 << 14, sum1: 1 << 14 }
    }
}

/// An LSB-first bit reader over one frame.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    cache: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, cache: 0, count: 0 }
    }

    #[inline]
    fn fill(&mut self) -> Result<()> {
        let b = *self.data.get(self.pos).ok_or(Error::Truncated)?;
        self.pos += 1;
        self.cache |= u64::from(b) << self.count;
        self.count += 8;
        Ok(())
    }

    /// The count of 1 bits before the next 0 bit (which is consumed).
    #[inline]
    fn unary(&mut self) -> Result<u32> {
        let mut n = 0u32;
        loop {
            if self.count == 0 {
                self.fill()?;
            }
            let ones = (!self.cache).trailing_zeros().min(self.count);
            n = n.saturating_add(ones);
            self.cache >>= ones;
            self.count -= ones;
            if self.count > 0 {
                self.cache >>= 1;
                self.count -= 1;
                return Ok(n);
            }
        }
    }

    #[inline]
    fn bits(&mut self, k: u32) -> Result<u32> {
        while self.count < k {
            self.fill()?;
        }
        let v = (self.cache & ((1u64 << k) - 1)) as u32;
        self.cache >>= k;
        self.count -= k;
        Ok(v)
    }

    /// Bytes started so far (the frame CRC follows at the next whole byte).
    fn bytes_used(&self) -> usize {
        self.pos
    }
}

/// One coded value (`tta_decoder_get_value`).
#[inline]
fn value(bits: &mut Bits<'_>, rice: &mut Rice) -> Result<i32> {
    let unary = bits.unary()?;
    let (level, k, mut v) = if unary != 0 { (true, rice.k1, unary - 1) } else { (false, rice.k0, 0) };
    if k > 0 {
        if k > 31 {
            return Err(Error::Invalid("Rice parameter"));
        }
        v = v.wrapping_shl(k).wrapping_add(bits.bits(k)?);
    }
    let bad = Error::Invalid("Rice parameter");
    if level {
        rice.sum1 = rice.sum1.wrapping_add(v.wrapping_sub(rice.sum1 >> 4));
        if rice.k1 > 0 && rice.sum1 < shift_16(rice.k1).ok_or(bad.clone())? {
            rice.k1 -= 1;
        } else if rice.sum1 > shift_16(rice.k1 + 1).ok_or(bad.clone())? {
            rice.k1 += 1;
        }
        v = v.wrapping_add(bit_shift(rice.k0).ok_or(bad.clone())?);
    }
    rice.sum0 = rice.sum0.wrapping_add(v.wrapping_sub(rice.sum0 >> 4));
    if rice.k0 > 0 && rice.sum0 < shift_16(rice.k0).ok_or(bad.clone())? {
        rice.k0 -= 1;
    } else if rice.sum0 > shift_16(rice.k0 + 1).ok_or(bad)? {
        rice.k0 += 1;
    }
    let x = v as i32;
    Ok(if x & 1 == 1 { x.wrapping_add(1) >> 1 } else { x.wrapping_neg() >> 1 })
}

/// A decoder of the frames of one stream.
#[derive(Debug, Clone)]
pub struct Decoder {
    info: Info,
    key: [i8; 8],
}

impl Decoder {
    /// A decoder for `info`; a password-protected stream needs its `password`.
    pub fn new(info: Info, password: Option<&[u8]>) -> Result<Self> {
        info.check()?;
        let key = match (info.format, password) {
            (FORMAT_ENCRYPTED, None) => return Err(Error::Unsupported("password-protected stream without a password")),
            (FORMAT_ENCRYPTED, Some(p)) => password_key(p),
            _ => [0; 8],
        };
        Ok(Self { info, key })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// Decodes a frame of `samples` samples per channel from the start of `data`; returns the
    /// planar samples and the bytes used (the frame and its CRC). A frame whose CRC does not
    /// match decodes to silence.
    pub fn decode_frame(&self, data: &[u8], samples: u32) -> Result<(Vec<Vec<i32>>, usize)> {
        self.decode(data, samples, false)
    }

    /// Decodes a frame that is exactly `data` (a Matroska packet: the frame and its CRC), whose
    /// length is a full frame or, for the last one, as many samples as its bytes hold (the zero
    /// padding of its last byte can add a few silent samples at the very end of a stream).
    pub fn decode_packet(&self, data: &[u8]) -> Result<Vec<Vec<i32>>> {
        Ok(self.decode(data, self.info.frame_samples(), true)?.0)
    }

    fn decode(&self, data: &[u8], samples: u32, packet: bool) -> Result<(Vec<Vec<i32>>, usize)> {
        // a packet's CRC is its last 4 bytes; a frame's follows its last whole byte
        let crc_at = if packet { Some(data.len().checked_sub(4).ok_or(Error::Truncated)?) } else { None };
        let n = usize::from(self.info.channels);
        let shift = match self.info.bits {
            16 => 9,
            _ => 10,
        };
        let pred = if self.info.bits == 8 { 4 } else { 5 };
        let len = samples as usize;
        let mut planes: Vec<Vec<i32>> = (0..n).map(|_| Vec::with_capacity(len)).collect();
        let mut filters = vec![Filter::new(&self.key, shift); n];
        let mut rices = vec![Rice::new(); n];
        let mut prev = vec![0i32; n];
        let mut cache = vec![0i32; n];
        let mut bits = Bits::new(crc_at.and_then(|c| data.get(..c)).unwrap_or(data));
        let sign = 32 - u32::from(self.info.bits);
        'samples: for _ in 0..len {
            for ch in 0..n {
                let (Some(f), Some(r), Some(p), Some(c)) = (filters.get_mut(ch), rices.get_mut(ch), prev.get_mut(ch), cache.get_mut(ch)) else {
                    return Err(Error::Invalid("channel"));
                };
                let v = match value(&mut bits, r) {
                    Ok(v) => f.decode(v),
                    // a short last packet ends where its bytes run out (a partial sample is
                    // padding)
                    Err(Error::Truncated) if crc_at.is_some() => break 'samples,
                    Err(e) => return Err(e),
                };
                // fixed first-order prediction, x · (2^k − 1) / 2^k (k is 4 for 8-bit audio)
                let v = v.wrapping_add((((i64::from(*p) << pred) - i64::from(*p)) >> pred) as i32);
                *p = v;
                *c = v;
            }
            if n > 1 {
                // channel decorrelation: the last channel carries the mid, the others differences
                if let (Some(&a), Some(&b)) = (cache.get(n - 2), cache.get(n - 1))
                    && let Some(l) = cache.get_mut(n - 1)
                {
                    *l = b.wrapping_add(a / 2);
                }
                for i in (0..n - 1).rev() {
                    if let (Some(&next), Some(c)) = (cache.get(i + 1), cache.get(i)) {
                        let v = next.wrapping_sub(*c);
                        if let Some(c) = cache.get_mut(i) {
                            *c = v;
                        }
                    }
                }
            }
            for (plane, &v) in planes.iter_mut().zip(&cache) {
                plane.push(v.wrapping_shl(sign) >> sign);
            }
        }
        let used = crc_at.unwrap_or(bits.bytes_used());
        let crc = data.get(used..used + 4).ok_or(Error::Truncated)?;
        if crc32(data.get(..used).unwrap_or(&[])) != u32::from_le_bytes([crc[0], crc[1], crc[2], crc[3]]) {
            for p in &mut planes {
                p.iter_mut().for_each(|v| *v = 0);
            }
        }
        Ok((planes, used + 4))
    }
}

/// Decodes a whole `.tta` file (`password` for a protected one): the stream parameters and the
/// planar samples. Frames are found through the seek table when its CRC holds (a damaged frame
/// then decodes to silence), else one after the other.
pub fn decode_file(data: &[u8], password: Option<&[u8]>) -> Result<(Info, Vec<Vec<i32>>)> {
    let (info, table_at) = Info::parse(data)?;
    let total = u64::from(info.samples) * u64::from(info.channels);
    if total > MAX_FILE_VALUES {
        return Err(Error::Unsupported("stream too long"));
    }
    let dec = Decoder::new(info, password)?;
    let frames = info.frames() as usize;
    let table_len = frames.checked_mul(4).and_then(|v| v.checked_add(4)).ok_or(Error::Invalid("frame count"))?;
    let table = data.get(table_at..table_at + table_len).ok_or(Error::Truncated)?;
    let sizes: Option<Vec<usize>> =
        (crc32(&table[..table_len - 4]) == le32(table, table_len - 4)).then(|| (0..frames).map(|i| le32(table, 4 * i) as usize).collect());
    let mut planes: Vec<Vec<i32>> = (0..info.channels).map(|_| Vec::with_capacity(info.samples as usize)).collect();
    let mut at = table_at + table_len;
    for i in 0..frames {
        let len = info.frame_len(i as u32);
        let rest = data.get(at..).unwrap_or(&[]);
        let (frame, used) = match &sizes {
            Some(s) => {
                let size = s.get(i).copied().unwrap_or(0);
                let slice = rest.get(..size).unwrap_or(rest);
                let frame = match dec.decode_frame(slice, len) {
                    Ok((f, _)) => f,
                    Err(_) => (0..info.channels).map(|_| vec![0; len as usize]).collect(),
                };
                (frame, size)
            }
            None => dec.decode_frame(rest, len)?,
        };
        for (p, f) in planes.iter_mut().zip(frame) {
            p.extend(f);
        }
        at = at.saturating_add(used);
    }
    Ok((info, planes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_key() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // CRC-64/XZ-style init and final xor with the ECMA-182 polynomial, unreflected (CRC-64/WE)
        let k = password_key(b"123456789");
        let v = u64::from_le_bytes(k.map(|b| b as u8));
        assert_eq!(v, 0x62EC_59E3_F1A4_F00A);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Info::parse(b"TTA1").is_err());
        assert!(Info::parse(&[0; 64]).is_err());
        assert!(decode_file(b"ID3\x03\0\0\x7f\x7f\x7f\x7f", None).is_err());
    }
}
