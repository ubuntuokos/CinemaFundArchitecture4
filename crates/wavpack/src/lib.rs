//! Pure-Rust WavPack 4 / 5 decoder (codec backlog CB-5.1).
//!
//! WavPack is a lossless (and optionally hybrid lossy) audio codec. A stream is a series of
//! self-contained blocks, each a 32-byte header (`wvpk`) followed by metadata sub-blocks, one of
//! which carries the entropy-coded audio. Multichannel audio is split into mono and stereo blocks
//! that repeat in sequence (`INITIAL_BLOCK` … `FINAL_BLOCK`).
//!
//! - [`Header::parse`] reads a block header; [`Info::probe`] the stream format from the first
//!   audio block.
//! - [`Decoder::decode_frame`] decodes the blocks of one sequence into planar samples.
//! - [`decode_file`] decodes a whole `.wv` file; [`matroska_blocks`] rebuilds the blocks of a
//!   Matroska `A_WAVPACK4` frame.
//!
//! Supported: lossless and hybrid (lossy, without the `.wvc` correction file) streams of 8- to
//! 32-bit integer and 32-bit float audio, 1 to 4096 channels, stream versions 0x402–0x410. DSD
//! streams and pre-4.0 files are reported as unsupported. A block that fails its checks (CRC,
//! magnitude limit, malformed metadata) decodes to silence of its length, as in the reference
//! decoder.
//!
//! The decoding algorithm is ported from the WavPack library 5.9.0 (BSD-3-Clause, Copyright (c)
//! 1998 - 2025 David Bryant; see `LICENSE.WavPack` and `docs/licensing.md` §5); the block format
//! follows "WavPack 4 & 5 Binary File / Block Format" (D. Bryant, 2020).

mod unpack;
mod words;

/// Errors from header parsing and file decoding. Damaged audio inside a valid block is not an
/// error (it decodes to silence).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("WavPack data is truncated")]
    Truncated,
    #[error("invalid WavPack data: {0}")]
    Invalid(&'static str),
    #[error("unsupported WavPack stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Length of a block header.
pub const HEADER_LEN: usize = 32;
/// The largest block the reference reader accepts (`ckSize` below 16 MiB).
pub const MAX_BLOCK_BYTES: usize = 16 << 20;
/// The largest `block_samples` the reference reader accepts.
pub const MAX_BLOCK_SAMPLES: u32 = 0x30000 - 1;
/// Cap on decoded values (frames × channels) for [`decode_file`]: 2³⁰, 4 GiB of `i32`.
pub const MAX_FILE_VALUES: usize = 1 << 30;

const MIN_STREAM_VERS: u16 = 0x402;
const MAX_STREAM_VERS: u16 = 0x410;

const SAMPLE_RATES: [u32; 15] = [6000, 8000, 9600, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000, 192_000];

/// Header flag bits (spec §2.0).
pub mod flags {
    pub const BYTES_STORED: u32 = 3;
    pub const MONO_FLAG: u32 = 4;
    pub const HYBRID_FLAG: u32 = 8;
    pub const JOINT_STEREO: u32 = 0x10;
    pub const CROSS_DECORR: u32 = 0x20;
    pub const HYBRID_SHAPE: u32 = 0x40;
    pub const FLOAT_DATA: u32 = 0x80;
    pub const INT32_DATA: u32 = 0x100;
    pub const HYBRID_BITRATE: u32 = 0x200;
    pub const HYBRID_BALANCE: u32 = 0x400;
    pub const INITIAL_BLOCK: u32 = 0x800;
    pub const FINAL_BLOCK: u32 = 0x1000;
    pub const SHIFT_LSB: u32 = 13;
    pub const SHIFT_MASK: u32 = 0x1f << SHIFT_LSB;
    pub const MAG_LSB: u32 = 18;
    pub const MAG_MASK: u32 = 0x1f << MAG_LSB;
    pub const SRATE_LSB: u32 = 23;
    pub const SRATE_MASK: u32 = 0xf << SRATE_LSB;
    pub const NEW_SHAPING: u32 = 0x2000_0000;
    pub const FALSE_STEREO: u32 = 0x4000_0000;
    pub const DSD_FLAG: u32 = 0x8000_0000;
    pub const MONO_DATA: u32 = MONO_FLAG | FALSE_STEREO;
}

/// A block header (spec §2.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// The whole block's length in bytes (`ckSize` + 8).
    pub size: usize,
    pub version: u16,
    pub block_index: u64,
    /// Total samples of the stream, when this is the first block and the length is known.
    pub total_samples: Option<u64>,
    pub block_samples: u32,
    pub flags: u32,
    pub crc: u32,
}

#[inline]
fn le32(b: &[u8], i: usize) -> u32 {
    match b.get(i..i + 4) {
        Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => 0,
    }
}

impl Header {
    /// Parses and validates the header at the start of `b` with the reference reader's checks.
    pub fn parse(b: &[u8]) -> Result<Header> {
        let h = b.get(..HEADER_LEN).ok_or(Error::Truncated)?;
        if &h[..4] != b"wvpk" {
            return Err(Error::Invalid("no wvpk block id"));
        }
        let ck = le32(h, 4);
        let version = u16::from_le_bytes([h[8], h[9]]);
        let block_samples = le32(h, 20);
        if ck & 1 != 0 || ck < 24 || ck as usize + 8 > MAX_BLOCK_BYTES || block_samples > MAX_BLOCK_SAMPLES {
            return Err(Error::Invalid("block size"));
        }
        if version >> 8 != 4 {
            return Err(Error::Unsupported("not a WavPack 4 or 5 block"));
        }
        if !(MIN_STREAM_VERS..=MAX_STREAM_VERS).contains(&version) {
            return Err(Error::Unsupported("stream version"));
        }
        let total_lo = le32(h, 12);
        let total = (total_lo != u32::MAX).then(|| u64::from(total_lo) + (u64::from(h[11]) << 32) - u64::from(h[11]));
        Ok(Header {
            size: ck as usize + 8,
            version,
            block_index: u64::from(le32(h, 16)) | u64::from(h[10]) << 32,
            total_samples: total,
            block_samples,
            flags: le32(h, 24),
            crc: le32(h, 28),
        })
    }

    pub fn is_mono(&self) -> bool {
        self.flags & flags::MONO_FLAG != 0
    }
    /// Output channels of this block: 1, or 2 (also for false stereo).
    pub fn channels(&self) -> usize {
        if self.is_mono() { 1 } else { 2 }
    }
    pub fn is_initial(&self) -> bool {
        self.flags & flags::INITIAL_BLOCK != 0
    }
    pub fn is_final(&self) -> bool {
        self.flags & flags::FINAL_BLOCK != 0
    }
    pub fn bytes_per_sample(&self) -> u32 {
        (self.flags & flags::BYTES_STORED) + 1
    }
    pub fn is_float(&self) -> bool {
        self.flags & flags::FLOAT_DATA != 0
    }
    pub fn is_hybrid(&self) -> bool {
        self.flags & flags::HYBRID_FLAG != 0
    }
    pub fn is_dsd(&self) -> bool {
        self.flags & flags::DSD_FLAG != 0
    }
    /// The sample rate from the header's rate index (`None` for "custom", index 15).
    pub fn standard_rate(&self) -> Option<u32> {
        SAMPLE_RATES.get(((self.flags & flags::SRATE_MASK) >> flags::SRATE_LSB) as usize).copied()
    }
}

/// The stream format, from the first audio block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub sample_rate: u32,
    pub channels: usize,
    /// Microsoft speaker mask (0 when the stream does not say).
    pub channel_mask: u32,
    /// Bits of the stored sample container (8, 16, 24 or 32); 32 for float.
    pub bits: u32,
    pub float: bool,
    /// Hybrid (lossy without the correction file) blocks were seen.
    pub hybrid: bool,
    pub total_samples: Option<u64>,
}

impl Info {
    /// The format of the stream whose first audio block is `block` (header included).
    pub fn probe(block: &[u8]) -> Result<Info> {
        let h = Header::parse(block)?;
        if h.is_dsd() {
            return Err(Error::Unsupported("DSD audio"));
        }
        let body = block.get(..h.size).ok_or(Error::Truncated)?;
        let mut channels = h.channels();
        let mut channel_mask = if channels == 1 { 4 } else { 3 };
        let mut rate = h.standard_rate();
        let subs = match unpack::metadata(body) {
            Ok(s) | Err(s) => s,
        };
        for (id, d) in subs {
            match id {
                unpack::ID_CHANNEL_INFO => {
                    let (n, mask) = channel_info(d)?;
                    channels = n;
                    channel_mask = mask;
                }
                unpack::ID_SAMPLE_RATE if d.len() == 3 || d.len() == 4 => {
                    let mut r = u32::from(d[0]) | u32::from(d[1]) << 8 | u32::from(d[2]) << 16;
                    if let Some(b) = d.get(3) {
                        r |= u32::from(b & 0x7f) << 24;
                    }
                    if rate.is_none() {
                        rate = Some(r);
                    }
                }
                _ => {}
            }
        }
        let sample_rate = rate.filter(|r| *r > 0).ok_or(Error::Invalid("no sample rate"))?;
        Ok(Info {
            sample_rate,
            channels,
            channel_mask,
            bits: if h.is_float() { 32 } else { h.bytes_per_sample() * 8 },
            float: h.is_float(),
            hybrid: h.is_hybrid(),
            total_samples: if h.block_index == 0 { h.total_samples } else { None },
        })
    }

    /// One decoded value as a float in [-1, 1) (floats as they are).
    pub fn to_f32(&self, v: i32) -> f32 {
        if self.float { f32::from_bits(v as u32) } else { (f64::from(v) / f64::from(1u32 << (self.bits.clamp(8, 32) - 1))) as f32 }
    }
}

/// The reference `read_channel_info`: (channel count, channel mask).
fn channel_info(d: &[u8]) -> Result<(usize, u32)> {
    if d.is_empty() || d.len() > 7 {
        return Err(Error::Invalid("channel info"));
    }
    let (channels, max_streams, mask) = if d.len() >= 6 {
        let channels = (usize::from(d[0]) | usize::from(d[2] & 0xf) << 8) + 1;
        let streams = (usize::from(d[1]) | usize::from(d[2] & 0xf0) << 4) + 1;
        if channels < streams {
            return Err(Error::Invalid("channel info"));
        }
        let mut mask = u32::from(d[3]) | u32::from(d[4]) << 8 | u32::from(d[5]) << 16;
        if let Some(b) = d.get(6) {
            mask |= u32::from(*b) << 24;
        }
        (channels, streams, mask)
    } else {
        let channels = usize::from(d[0]);
        let mask = d[1..].iter().enumerate().fold(0u32, |m, (i, b)| m | u32::from(*b) << (8 * i));
        (channels, channels.max(8), mask)
    };
    if channels == 0 || channels > max_streams * 2 || mask.count_ones() as usize > channels {
        return Err(Error::Invalid("channel info"));
    }
    Ok((channels, mask))
}

/// Decodes the block sequences of a stream with a known format.
#[derive(Debug, Clone)]
pub struct Decoder {
    info: Info,
}

impl Decoder {
    pub fn new(info: Info) -> Self {
        Self { info }
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// Decodes one block sequence (complete blocks, headers included, `INITIAL_BLOCK` first) into
    /// `channels` planar channels of the first block's `block_samples` values each. Integers are
    /// right-justified in their container (16-bit: ±32768); floats are raw `f32` bits.
    /// Channels no block covers, and blocks that fail their checks, are silent.
    pub fn decode_frame(&mut self, blocks: &[&[u8]]) -> Result<Vec<Vec<i32>>> {
        let first = Header::parse(blocks.first().ok_or(Error::Truncated)?)?;
        let n = first.block_samples as usize;
        let ch = self.info.channels;
        let mut out = vec![vec![0i32; n]; ch];
        let mut offset = 0usize;
        for b in blocks {
            if offset >= ch {
                break;
            }
            let h = Header::parse(b)?;
            if h.is_dsd() {
                return Err(Error::Unsupported("DSD audio"));
            }
            let body = b.get(..h.size).ok_or(Error::Truncated)?;
            let bch = h.channels();
            if h.block_samples as usize == n
                && let Some(samples) = unpack::unpack(&h, body)
            {
                for (i, frame) in samples.chunks_exact(bch).enumerate() {
                    for (c, v) in frame.iter().enumerate() {
                        if let Some(slot) = out.get_mut(offset + c).and_then(|o| o.get_mut(i)) {
                            *slot = *v;
                        }
                    }
                }
            }
            offset += bch;
            if h.is_final() {
                break;
            }
        }
        Ok(out)
    }
}

/// The complete blocks in `bytes` (a `.wv` file or stream), in order. Bytes between blocks
/// (tags, junk) are skipped by searching for the next valid header.
pub fn blocks(bytes: &[u8]) -> impl Iterator<Item = (Header, &[u8])> {
    let mut p = 0usize;
    std::iter::from_fn(move || {
        loop {
            let rest = bytes.get(p..)?;
            if rest.len() < HEADER_LEN {
                return None;
            }
            match Header::parse(rest) {
                Ok(h) if h.size <= rest.len() => {
                    p += h.size;
                    return rest.get(..h.size).map(|b| (h, b));
                }
                Ok(_) => return None,
                Err(_) => {
                    // next candidate 'w'
                    let skip = rest.iter().skip(1).position(|b| *b == b'w').map_or(rest.len(), |i| i + 1);
                    p += skip;
                }
            }
        }
    })
}

/// Decodes a whole `.wv` file: its format and planar samples (see [`Decoder::decode_frame`]).
pub fn decode_file(bytes: &[u8]) -> Result<(Info, Vec<Vec<i32>>)> {
    let mut info: Option<Info> = None;
    let mut out: Vec<Vec<i32>> = Vec::new();
    let mut seq: Vec<&[u8]> = Vec::new();
    let mut values = 0usize;
    let mut flush = |seq: &mut Vec<&[u8]>, info: &Info, out: &mut Vec<Vec<i32>>| -> Result<()> {
        if seq.is_empty() {
            return Ok(());
        }
        let planes = Decoder::new(*info).decode_frame(seq)?;
        seq.clear();
        let n = planes.first().map_or(0, Vec::len);
        values = values.saturating_add(n.saturating_mul(info.channels));
        if values > MAX_FILE_VALUES {
            return Err(Error::Invalid("decoded size above the cap"));
        }
        for (o, p) in out.iter_mut().zip(planes) {
            o.extend(p);
        }
        Ok(())
    };
    for (h, b) in blocks(bytes) {
        if h.block_samples == 0 {
            continue;
        }
        let i = match info {
            Some(i) => i,
            None => {
                let i = Info::probe(b)?;
                out = vec![Vec::new(); i.channels];
                info = Some(i);
                i
            }
        };
        if h.is_initial() {
            flush(&mut seq, &i, &mut out)?;
        }
        seq.push(b);
        if h.is_final() {
            flush(&mut seq, &i, &mut out)?;
        }
    }
    let info = info.ok_or(Error::Invalid("no WavPack audio blocks"))?;
    flush(&mut seq, &info, &mut out)?;
    if let Some(total) = info.total_samples.and_then(|t| usize::try_from(t).ok()) {
        for o in out.iter_mut() {
            o.truncate(total);
        }
    }
    Ok((info, out))
}

/// Rebuilds the complete blocks of one Matroska `A_WAVPACK4` frame. Matroska strips the parts of
/// each block header it carries elsewhere: a frame is `block_samples` (u32 LE) followed, per
/// block, by `flags`, `crc` and (when the frame holds more than one block) the block's byte
/// length, then its data. `version` is the stream version from `CodecPrivate`.
pub fn matroska_blocks(version: u16, frame: &[u8]) -> Result<Vec<Vec<u8>>> {
    let samples = frame.get(..4).map(|_| le32(frame, 0)).ok_or(Error::Truncated)?;
    let mut p = 4usize;
    let mut out = Vec::new();
    while p < frame.len() {
        let flags = frame.get(p..p + 8).map(|_| le32(frame, p)).ok_or(Error::Truncated)?;
        let crc = le32(frame, p + 4);
        p += 8;
        let multi = flags & (flags::INITIAL_BLOCK | flags::FINAL_BLOCK) != (flags::INITIAL_BLOCK | flags::FINAL_BLOCK);
        let len = if multi {
            let l = frame.get(p..p + 4).map(|_| le32(frame, p) as usize).ok_or(Error::Truncated)?;
            p += 4;
            l
        } else {
            frame.len() - p
        };
        let data = frame.get(p..p.saturating_add(len)).ok_or(Error::Truncated)?;
        p += len;
        let ck = u32::try_from(len + 24).map_err(|_| Error::Invalid("block size"))?;
        let mut b = Vec::with_capacity(HEADER_LEN + len);
        b.extend_from_slice(b"wvpk");
        b.extend_from_slice(&ck.to_le_bytes());
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&samples.to_le_bytes());
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&crc.to_le_bytes());
        b.extend_from_slice(data);
        Header::parse(&b)?;
        out.push(b);
        if !multi || flags & flags::FINAL_BLOCK != 0 {
            break;
        }
    }
    if out.is_empty() {
        return Err(Error::Truncated);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(ck: u32, version: u16, samples: u32, flags: u32) -> Vec<u8> {
        let mut b = b"wvpk".to_vec();
        b.extend(ck.to_le_bytes());
        b.extend(version.to_le_bytes());
        b.extend([0, 0]);
        b.extend(1000u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(samples.to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b
    }

    #[test]
    fn headers_are_validated() {
        let h = Header::parse(&header(24, 0x410, 10, 0x1800 | 9 << 23)).unwrap();
        assert_eq!((h.size, h.total_samples, h.standard_rate(), h.channels()), (32, Some(1000), Some(44100), 2));
        assert_eq!(Header::parse(&header(25, 0x410, 10, 0)), Err(Error::Invalid("block size")));
        assert_eq!(Header::parse(&header(24, 0x401, 10, 0)), Err(Error::Unsupported("stream version")));
        assert_eq!(Header::parse(&header(24, 0x410, 0x30000, 0)), Err(Error::Invalid("block size")));
        assert_eq!(Header::parse(&header(24, 0x410, 1, 0)[..31]), Err(Error::Truncated));
        assert!(Header::parse(&header(u32::MAX - 1, 0x410, 1, 0)).is_err());
    }

    #[test]
    fn hostile_files_and_frames_do_not_panic() {
        // an audio block with no bitstream decodes to silence of its length
        let mut b = header(24, 0x410, 100, 0x1800 | 4 | 10 << 23);
        let (info, out) = decode_file(&b).unwrap();
        assert_eq!((info.channels, info.sample_rate, out[0].len()), (1, 48000, 100));
        assert!(out[0].iter().all(|v| *v == 0));
        // junk around blocks, truncation at every length
        b.splice(0..0, *b"junkwvp");
        b.extend(b"tail");
        assert_eq!(decode_file(&b).unwrap().1[0].len(), 100);
        for n in 0..b.len() {
            let _ = decode_file(&b[..n]);
        }
        assert_eq!(decode_file(b"nothing"), Err(Error::Invalid("no WavPack audio blocks")));
        // custom rate without a rate sub-block; DSD
        assert!(decode_file(&header(24, 0x410, 1, 0x1800 | 15 << 23)).is_err());
        assert_eq!(decode_file(&header(24, 0x410, 1, 0x8000_1800)), Err(Error::Unsupported("DSD audio")));
        for frame in [&[][..], &[1, 0, 0, 0], &[1, 0, 0, 0, 0, 0x18, 0, 0, 1, 2, 3, 4], &[1, 0, 0, 0, 0, 0x08, 0, 0, 1, 2, 3, 4, 0xff, 0xff, 0, 0]] {
            let _ = matroska_blocks(0x410, frame);
        }
        assert!(channel_info(&[3, 0x0f]).is_err());
        assert_eq!(channel_info(&[6, 0x3f]), Ok((6, 0x3f)));
        assert_eq!(channel_info(&[5, 2, 0, 0x3f, 0, 0]), Ok((6, 0x3f)));
    }
}
