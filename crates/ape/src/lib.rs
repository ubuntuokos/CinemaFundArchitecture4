//! Pure-Rust Monkey's Audio (APE) decoder (codec backlog CB-5.2).
//!
//! Monkey's Audio is a lossless audio codec. A file is a descriptor (`MAC `), a header, a seek
//! table of frame offsets, the original WAV header and the frames. Each frame starts with a
//! CRC-32 of its decoded PCM and codes every channel's residuals with an adaptive range coder;
//! two prediction stages (an optional cascade of sign-LMS "NN" filters, then a fixed
//! two-stage predictor with cross-channel input for stereo) restore the samples.
//!
//! - [`File::parse`] reads the descriptor, header and seek table.
//! - [`File::decode_frame`] decodes one frame into planar samples; [`decode_file`] decodes a
//!   whole file.
//!
//! Supported: files of format version 3.99 and later (every Monkey's Audio release since 2004),
//! all five compression levels, 8-, 16-, 24- and 32-bit integer and 32-bit float audio, 1 to 32
//! channels, including the 24-bit files written by Monkey's Audio 5.01–8.51 ("interim" mode).
//! Older format versions are reported as unsupported. A frame whose CRC or bitstream fails
//! decodes to silence of its length, as in the reference decoder.
//!
//! The decoding algorithm is ported from the Monkey's Audio SDK 13.46 (BSD-3-Clause, Copyright
//! 2000-2026 Matthew T. Ashland; see `LICENSE.MonkeysAudio` and `docs/licensing.md` §5).

mod predictor;
mod range;

use predictor::{Predictor, level_filters};
use range::{Damaged, Reader, State};

/// Errors from header parsing and file decoding. A damaged frame is not an error (it decodes to
/// silence).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("APE data is truncated")]
    Truncated,
    #[error("invalid APE data: {0}")]
    Invalid(&'static str),
    #[error("unsupported APE stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The oldest format version this decoder reads (3.99).
pub const MIN_VERSION: u16 = 3990;
/// The most channels a file may have (`APE_MAXIMUM_CHANNELS`).
pub const MAX_CHANNELS: u16 = 32;
/// Cap on decoded values (blocks × channels) for [`decode_file`]: 2³⁰, 4 GiB of `i32`.
pub const MAX_FILE_VALUES: usize = 1 << 30;
/// The most values reserved ahead of decoding (a hostile header cannot force a large
/// allocation before any audio decodes).
const PREALLOC: usize = 1 << 24;
/// How far the reference reader scans for the descriptor.
const MAX_SCAN: usize = 1 << 20;
/// The largest original WAV header or footer the reference reader accepts.
const MAX_WAV_HEADER: u32 = 8 << 20;

/// Format flags (`APE_FORMAT_FLAG_*`).
pub mod flags {
    pub const CREATE_WAV_HEADER: u16 = 1 << 5;
    pub const AIFF: u16 = 1 << 6;
    pub const W64: u16 = 1 << 7;
    pub const SND: u16 = 1 << 8;
    pub const BIG_ENDIAN: u16 = 1 << 9;
    pub const CAF: u16 = 1 << 10;
    pub const SIGNED_8_BIT: u16 = 1 << 11;
    pub const FLOATING_POINT: u16 = 1 << 12;
}

const SPECIAL_MONO_SILENCE: u32 = 1;
const SPECIAL_LEFT_SILENCE: u32 = 1;
const SPECIAL_RIGHT_SILENCE: u32 = 2;
const SPECIAL_PSEUDO_STEREO: u32 = 4;

/// The stream format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub sample_rate: u32,
    pub channels: u16,
    /// 8, 16, 24 or 32.
    pub bits: u16,
    /// 32-bit IEEE float samples (carried as their bit patterns).
    pub float: bool,
    /// Samples per channel.
    pub total_blocks: u64,
}

impl Info {
    /// One decoded value as a float in [-1, 1): integers are scaled by their bit depth, floats are
    /// taken from their bit pattern.
    pub fn to_f32(&self, v: i32) -> f32 {
        if self.float {
            f32::from_bits(v as u32)
        } else {
            let scale = (1u64 << self.bits.clamp(1, 32).saturating_sub(1)) as f32;
            v as f32 / scale
        }
    }
}

/// A parsed `.ape` file.
#[derive(Debug, Clone)]
pub struct File<'a> {
    bytes: &'a [u8],
    pub info: Info,
    /// Format version × 1000 (3990 for every current encoder).
    pub version: u16,
    /// 1000 (fast) … 5000 (insane).
    pub compression: u16,
    pub format_flags: u16,
    pub blocks_per_frame: u32,
    final_frame_blocks: u32,
    /// Absolute file offset of every frame.
    frame_offsets: Vec<u64>,
    /// Where the last frame ends.
    data_end: u64,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    let s = b.get(at..at + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `FindDescriptor`: skip an ID3v2 tag and scan for `MAC ` / `MACF`.
fn find_descriptor(bytes: &[u8]) -> Option<usize> {
    let mut start = 0usize;
    if bytes.starts_with(b"ID3") && bytes.len() >= 10 {
        let size = (usize::from(bytes[6] & 127) << 21) | (usize::from(bytes[7] & 127) << 14) | (usize::from(bytes[8] & 127) << 7) | usize::from(bytes[9] & 127);
        let footer = bytes[5] & 16 != 0;
        start = size + if footer { 20 } else { 10 };
        if !footer {
            while bytes.get(start) == Some(&0) {
                start += 1;
            }
        }
    }
    let end = start.saturating_add(MAX_SCAN + 4).min(bytes.len());
    let window = bytes.get(start..end)?;
    window.windows(4).position(|w| w == b"MAC " || w == b"MACF").map(|p| start + p)
}

impl<'a> File<'a> {
    /// Read the descriptor, header and seek table of a whole `.ape` file.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let base = find_descriptor(bytes).ok_or(Error::Invalid("no Monkey's Audio descriptor"))?;
        let version = u16_at(bytes, base + 4)?;
        if version < MIN_VERSION {
            return Err(Error::Unsupported("format version before 3.99"));
        }
        // APE_DESCRIPTOR
        let descriptor_bytes = u32_at(bytes, base + 8)? as usize;
        let header_bytes = u32_at(bytes, base + 12)? as usize;
        let seek_table_bytes = u32_at(bytes, base + 16)? as usize;
        let wav_header_bytes = u32_at(bytes, base + 20)?;
        let terminating_bytes = u32_at(bytes, base + 32)?;
        // APE_HEADER
        let h = base.checked_add(descriptor_bytes).ok_or(Error::Truncated)?;
        let compression = u16_at(bytes, h)?;
        let format_flags = u16_at(bytes, h + 2)?;
        let blocks_per_frame = u32_at(bytes, h + 4)?;
        let final_frame_blocks = u32_at(bytes, h + 8)?;
        let total_frames = u32_at(bytes, h + 12)?;
        let bits = u16_at(bytes, h + 16)?;
        let channels = u16_at(bytes, h + 18)?;
        let sample_rate = u32_at(bytes, h + 20)?;

        if channels == 0 || channels > MAX_CHANNELS {
            return Err(Error::Invalid("channel count"));
        }
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(Error::Unsupported("bit depth"));
        }
        if usize::from(bits / 8) * usize::from(channels) > 256 {
            return Err(Error::Invalid("block align"));
        }
        if sample_rate == 0 {
            return Err(Error::Invalid("sample rate"));
        }
        let max_blocks = if compression >= 5000 { 10_000_000 } else { 1_000_000 };
        if blocks_per_frame == 0 || blocks_per_frame > max_blocks || final_frame_blocks > blocks_per_frame {
            return Err(Error::Invalid("frame size"));
        }
        if level_filters(compression).is_none() {
            return Err(Error::Unsupported("compression level"));
        }
        let float = format_flags & flags::FLOATING_POINT != 0;
        if float && bits != 32 {
            return Err(Error::Invalid("float data must be 32-bit"));
        }
        if format_flags & flags::CREATE_WAV_HEADER == 0 && wav_header_bytes > MAX_WAV_HEADER {
            return Err(Error::Invalid("WAV header size"));
        }

        // The seek table: 32-bit offsets (relative to the descriptor) that may wrap past 4 GiB.
        let seek_at = h.checked_add(header_bytes).ok_or(Error::Truncated)?;
        let elements = seek_table_bytes / 4;
        if elements > bytes.len() / 4 || (total_frames as usize) > elements {
            return Err(Error::Invalid("seek table"));
        }
        let mut frame_offsets = Vec::with_capacity(total_frames as usize);
        let (mut add, mut previous) = (0u64, 0u32);
        for i in 0..total_frames as usize {
            let current = u32_at(bytes, seek_at + 4 * i)?;
            if current < previous {
                add += 1 << 32;
            }
            frame_offsets.push(base as u64 + add + u64::from(current));
            previous = current;
        }
        if frame_offsets.windows(2).any(|w| w[1] < w[0]) {
            return Err(Error::Invalid("seek table order"));
        }
        let data_end = (bytes.len() as u64).saturating_sub(u64::from(terminating_bytes));

        let total_blocks = if total_frames == 0 { 0 } else { u64::from(total_frames - 1) * u64::from(blocks_per_frame) + u64::from(final_frame_blocks) };
        Ok(Self {
            bytes,
            info: Info { sample_rate, channels, bits, float, total_blocks },
            version,
            compression,
            format_flags,
            blocks_per_frame,
            final_frame_blocks,
            frame_offsets,
            data_end,
        })
    }

    /// The number of frames.
    pub fn frames(&self) -> usize {
        self.frame_offsets.len()
    }

    /// The file offset at which frame `index` starts (0 past the last frame).
    pub fn frame_offset(&self, index: usize) -> u64 {
        self.frame_offsets.get(index).copied().unwrap_or(0)
    }

    /// Samples per channel in frame `index`.
    pub fn frame_blocks(&self, index: usize) -> u32 {
        if index + 1 == self.frames() { self.final_frame_blocks } else { self.blocks_per_frame }
    }

    /// The bytes the reference decoder hands its worker for frame `index`: from the 4-byte
    /// boundary at or before the frame to 4 bytes past its end (zero-filled past the file), and
    /// the bit at which the frame starts.
    fn frame_input(&self, index: usize) -> Option<(Vec<u8>, u64)> {
        let start = *self.frame_offsets.get(index)?;
        let end = self.frame_offsets.get(index + 1).copied().unwrap_or(self.data_end).max(start);
        let first = *self.frame_offsets.first()?;
        let remainder = (start - first) % 4;
        let from = usize::try_from(start - remainder).ok()?;
        let len = usize::try_from(end - start + remainder + 4).ok()?;
        let available = self.bytes.len().saturating_sub(from);
        if available + 4 < len {
            return None; // the reference rejects a frame read more than 4 bytes short
        }
        let mut buf = vec![0u8; len];
        let copy = len.min(available);
        buf.get_mut(..copy)?.copy_from_slice(self.bytes.get(from..from + copy)?);
        Some((buf, remainder * 8))
    }

    /// A decoder for this file's frames.
    pub fn decoder(&self) -> Decoder {
        Decoder::new(self)
    }
}

/// The per-channel predictors: `int` arithmetic below 32 bits, `int64` at 32 bits.
#[derive(Debug, Clone)]
enum Predictors {
    Narrow(Vec<Predictor<i32>>),
    Wide(Vec<Predictor<i64>>),
}

impl Predictors {
    fn new(filters: &[(usize, u32)], bits: u16, channels: u16) -> Self {
        let n = usize::from(channels);
        if bits < 32 {
            Self::Narrow((0..n).map(|_| Predictor::new(filters, u32::from(bits))).collect())
        } else {
            Self::Wide((0..n).map(|_| Predictor::new(filters, u32::from(bits))).collect())
        }
    }
    fn flush(&mut self) {
        match self {
            Self::Narrow(p) => p.iter_mut().for_each(Predictor::flush),
            Self::Wide(p) => p.iter_mut().for_each(Predictor::flush),
        }
    }
    fn set_interim(&mut self) {
        if let Self::Narrow(p) = self {
            p.iter_mut().for_each(Predictor::set_interim);
        }
    }
    fn decompress(&mut self, channel: usize, a: i64, b: i64) -> Res32 {
        match self {
            Self::Narrow(p) => p.get_mut(channel).map(|p| p.decompress(a, b)),
            Self::Wide(p) => p.get_mut(channel).map(|p| p.decompress(a, b)),
        }
        .ok_or(Damaged)
    }
}

type Res32 = std::result::Result<i32, Damaged>;

/// Decodes the frames of one file (frames are independent; the decoder only remembers whether
/// the file needed "interim" arithmetic).
#[derive(Debug, Clone)]
pub struct Decoder {
    info: Info,
    version: u16,
    predictors: Predictors,
    interim: bool,
}

impl Decoder {
    fn new(file: &File<'_>) -> Self {
        let filters = level_filters(file.compression).unwrap_or(&[]);
        Self { info: file.info, version: file.version, predictors: Predictors::new(filters, file.info.bits, file.info.channels), interim: false }
    }

    /// Decode frame `index` of `file` into planar samples (integers sign-extended from their
    /// bit depth, 8-bit as signed, floats as `f32` bits). A damaged frame decodes to silence.
    pub fn decode_frame(&mut self, file: &File<'_>, index: usize) -> Vec<Vec<i32>> {
        let blocks = file.frame_blocks(index) as usize;
        let channels = usize::from(self.info.channels);
        let decoded = file.frame_input(index).and_then(|(data, start)| {
            match self.try_frame(&data, start, blocks) {
                Some(planes) => Some(planes),
                // Monkey's Audio 5.01–8.51 wrote 24-bit frames with 64-bit sums: retry once.
                None if !self.interim && self.info.bits == 24 => {
                    self.interim = true;
                    self.predictors.set_interim();
                    self.try_frame(&data, start, blocks)
                }
                None => None,
            }
        });
        decoded.unwrap_or_else(|| {
            let silence = if self.info.bits == 8 { -1 } else { 0 };
            vec![vec![silence; blocks]; channels]
        })
    }

    /// One frame; `None` when it fails its bitstream, range or CRC checks.
    fn try_frame(&mut self, data: &[u8], start_bit: u64, blocks: usize) -> Option<Vec<Vec<i32>>> {
        let mut r = Reader::new(data, start_bit);
        let mut stored_crc = r.bits(32).ok()?;
        let mut special = 0;
        if self.version > 3820 {
            if stored_crc & 0x8000_0000 != 0 {
                special = r.bits(32).ok()?;
            }
            stored_crc &= 0x7FFF_FFFF;
        }
        self.predictors.flush();
        let channels = usize::from(self.info.channels);
        let mut states = vec![State::new(); channels];
        r.start_range().ok()?;

        let bytes_per_sample = usize::from(self.info.bits / 8);
        let mut pcm = Vec::with_capacity(blocks.checked_mul(channels)?.checked_mul(bytes_per_sample)?.min(PREALLOC));
        let mut values = vec![0i32; channels];
        let mut last_x = 0i32;
        for _ in 0..blocks {
            self.block(&mut r, &mut states, &mut values, &mut last_x, special).ok()?;
            unprepare(&values, self.info.bits, &mut pcm).ok()?;
        }
        if crc32(&pcm) >> 1 != stored_crc {
            return None;
        }
        Some(planar(&pcm, self.info.bits, channels, self.info.float))
    }

    fn block(&mut self, r: &mut Reader<'_>, states: &mut [State], values: &mut [i32], last_x: &mut i32, special: u32) -> std::result::Result<(), Damaged> {
        let channels = values.len();
        if channels > 2 {
            for (ch, (value, state)) in values.iter_mut().zip(states.iter_mut()).enumerate() {
                let v = r.value(state)?;
                *value = self.predictors.decompress(ch, v, 0)?;
            }
        } else if let ([x_out, y_out], [s0, s1]) = (&mut *values, &mut *states) {
            if special & SPECIAL_LEFT_SILENCE != 0 && special & SPECIAL_RIGHT_SILENCE != 0 {
                *x_out = 0;
                *y_out = 0;
            } else if special & SPECIAL_PSEUDO_STEREO != 0 {
                let v = r.value(s0)?;
                *x_out = self.predictors.decompress(0, v, 0)?;
                *y_out = 0;
            } else {
                let ny = r.value(s1)?;
                let nx = r.value(s0)?;
                let y = self.predictors.decompress(1, ny, i64::from(*last_x))?;
                let x = self.predictors.decompress(0, nx, i64::from(y))?;
                *last_x = x;
                *x_out = x;
                *y_out = y;
            }
        } else {
            let (Some(first), Some(s0)) = (values.first_mut(), states.first_mut()) else { return Err(Damaged) };
            if special & SPECIAL_MONO_SILENCE != 0 {
                *first = 0;
            } else {
                let v = r.value(s0)?;
                *first = self.predictors.decompress(0, v, 0)?;
            }
        }
        Ok(())
    }
}

/// `CPrepare::Unprepare`: one block of predictor outputs to the PCM bytes the reference writes
/// (stereo pairs are stored as mid/side: `first = X - Y/2`, `second = first + Y`).
fn unprepare(v: &[i32], bits: u16, out: &mut Vec<u8>) -> std::result::Result<(), Damaged> {
    let channels = v.len();
    let get = |i: usize| v.get(i).copied().unwrap_or(0);
    let pair = |a: i32, b: i32| {
        let first = a.wrapping_sub(b / 2);
        (first, first.wrapping_add(b))
    };
    let s16 = |out: &mut Vec<u8>, x: i32, check: bool| -> std::result::Result<(), Damaged> {
        if check && !(-32768..=32767).contains(&x) {
            return Err(Damaged);
        }
        out.extend_from_slice(&(x as i16).to_le_bytes());
        Ok(())
    };
    // Mono and stereo store a negative value as (x + 2^23) | 2^23, the same bytes for any
    // in-range sample.
    let s24 = |out: &mut Vec<u8>, x: i32| {
        let u = if channels <= 2 && x < 0 { (x.wrapping_add(0x80_0000) as u32) | 0x80_0000 } else { x as u32 };
        out.extend_from_slice(&u.to_le_bytes()[..3]);
    };
    match bits {
        32 => {
            if channels == 2 {
                let (a, b) = pair(get(0), get(1));
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&b.to_le_bytes());
            } else {
                v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
            }
        }
        24 => {
            // The pair layout of 4, 6, 7 and 8+ channels; any other count is stored as is.
            let pairs: &[usize] = match channels {
                2 | 4 => &[0, 2],
                n if n >= 8 => &[0, 4, 6],
                n if n >= 6 => &[0, 4],
                _ => &[],
            };
            write_channels(v, pairs, channels, |x| s24(out, x));
        }
        16 => {
            let pairs: &[usize] = match channels {
                2 | 4 => &[0, 2],
                n if n >= 8 => &[0, 4, 6],
                n if n >= 6 => &[0, 4],
                _ => &[],
            };
            // Paired channels (and the centre/LFE pair of 6+) must fit 16 bits.
            let mut result = Ok(());
            // centre and LFE of 6+ channels are checked too; the rest are truncated
            let checked = |i: usize| channels >= 6 && (i == 2 || i == 3);
            let mut i = 0;
            while i < channels {
                if pairs.contains(&i) && i + 1 < channels {
                    let (a, b) = pair(get(i), get(i + 1));
                    result = result.and(s16(out, a, true)).and(s16(out, b, true));
                    i += 2;
                } else {
                    result = result.and(s16(out, get(i), checked(i)));
                    i += 1;
                }
            }
            result?;
        }
        _ => {
            // 8-bit: unsigned bytes
            if channels == 2 {
                let first = get(0).wrapping_sub(get(1) / 2).wrapping_add(128) as u8;
                out.push(first);
                out.push(first.wrapping_add(get(1) as u8));
            } else {
                v.iter().for_each(|&x| out.push(x.wrapping_add(128) as u8));
            }
        }
    }
    Ok(())
}

/// Write `channels` values, decoding the mid/side pairs that start at `pairs`.
fn write_channels(v: &[i32], pairs: &[usize], channels: usize, mut put: impl FnMut(i32)) {
    let get = |i: usize| v.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < channels {
        if pairs.contains(&i) && i + 1 < channels {
            let first = get(i).wrapping_sub(get(i + 1) / 2);
            put(first);
            put(first.wrapping_add(get(i + 1)));
            i += 2;
        } else {
            put(get(i));
            i += 1;
        }
    }
}

/// The PCM bytes of a frame as planar values.
fn planar(pcm: &[u8], bits: u16, channels: usize, float: bool) -> Vec<Vec<i32>> {
    let bytes = usize::from(bits / 8).max(1);
    let blocks = pcm.len() / (bytes * channels.max(1));
    let mut planes = vec![Vec::with_capacity(blocks); channels];
    for (i, s) in pcm.chunks_exact(bytes).enumerate() {
        let v = match bytes {
            1 => i32::from(s[0]) - 128,
            2 => i32::from(i16::from_le_bytes([s[0], s[1]])),
            3 => (i32::from_le_bytes([0, s[0], s[1], s[2]])) >> 8,
            _ => {
                let raw = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                if float { float_transform(raw) as i32 } else { raw as i32 }
            }
        };
        if let Some(p) = planes.get_mut(i % channels.max(1)) {
            p.push(v);
        }
    }
    planes
}

/// `CFloatTransform::Process`: the stored integer pattern to the IEEE float bits.
fn float_transform(v: u32) -> u32 {
    let mut out = v & 0xC3FF_FFFF;
    out |= !(v & 0x3C00_0000) ^ 0xC3FF_FFFF;
    if out & 0x8000_0000 != 0 {
        out = !out | 0x8000_0000;
    }
    out
}

/// CRC-32 (IEEE, reflected) of a frame's PCM, as `CRC_update` with the initial and final XOR.
fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                k += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    };
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Decode a whole `.ape` file into its format and planar samples.
pub fn decode_file(bytes: &[u8]) -> Result<(Info, Vec<Vec<i32>>)> {
    let file = File::parse(bytes)?;
    let channels = usize::from(file.info.channels);
    let total = usize::try_from(file.info.total_blocks).map_err(|_| Error::Invalid("length"))?;
    if total.saturating_mul(channels) > MAX_FILE_VALUES {
        return Err(Error::Unsupported("file longer than the decode cap"));
    }
    let mut planes: Vec<Vec<i32>> = vec![Vec::with_capacity(total.min(PREALLOC)); channels];
    let mut decoder = file.decoder();
    for index in 0..file.frames() {
        for (plane, frame) in planes.iter_mut().zip(decoder.decode_frame(&file, index)) {
            plane.extend(frame);
        }
    }
    Ok((file.info, planes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_is_ieee() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn float_transform_round_trips_simple_values() {
        // The transform is an involution on the exponent bits that keeps 0.0 at 0.
        for f in [0.0f32, 0.5, -0.25, 1.0e-3, -0.999] {
            let bits = f.to_bits();
            assert_eq!(float_transform(float_transform(bits)), bits, "{f}");
        }
    }

    #[test]
    fn stereo_pairs_are_mid_side() {
        let mut out = Vec::new();
        assert!(unprepare(&[10, 4], 16, &mut out).is_ok());
        assert_eq!(out, [8, 0, 12, 0]);
        out.clear();
        assert!(unprepare(&[40000, 0], 16, &mut out).is_err());
    }

    #[test]
    fn garbage_is_rejected_without_panicking() {
        assert!(File::parse(b"").is_err());
        assert!(File::parse(b"MAC \xa6\x0f").is_err());
        let mut fake = b"MAC ".to_vec();
        fake.extend_from_slice(&3990u16.to_le_bytes());
        fake.extend(std::iter::repeat_n(0xFFu8, 200));
        assert!(File::parse(&fake).is_err());
    }
}
