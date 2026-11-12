// Ported from libmpcdec (musepack_src_r475) libmpcdec/mpc_demux.c and streaminfo.c,
// BSD-3-Clause, Copyright (c) 2005-2009 The Musepack Development Team (`LICENSE.libmpcdec`).
// CFA4 changes GPL-3.0-or-later.

//! Pure-Rust Musepack decoder: stream versions 7 (`MP+`) and 8 (`MPCK`), ported from the
//! reference library libmpcdec in its floating-point build, so the output is libmpcdec's.
//!
//! [`decode_file`] decodes a whole `.mpc` file to planar `f32` samples in −1…1.

mod bits;
mod decoder;
mod huffman;
mod synth;

use bits::Reader;
use decoder::{Decoder, Params};

pub use decoder::FRAME_LENGTH;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not a Musepack stream")]
    NotMusepack,
    #[error("unsupported Musepack stream version {0}")]
    Version(u8),
    #[error("damaged Musepack header: {0}")]
    Header(&'static str),
    #[error("Musepack stream too long ({0} samples)")]
    TooLong(u64),
}

/// The stream parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    /// 7 or 8.
    pub stream_version: u8,
    pub sample_rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// Samples per channel the header announces, after the encoder's leading silence
    /// (0 when an SV8 stream does not say).
    pub samples: u64,
    /// Samples of leading silence the decoder drops (SV8).
    pub beg_silence: u64,
    /// The number of coded bands (SV7: the highest band index plus one).
    pub bands: u8,
    pub mid_side: bool,
    /// Byte offset of the stream header (after an ID3v2 tag).
    pub header_position: usize,
}

/// The decoded output is capped at this many samples per channel (about 6 hours at 48 kHz).
pub const MAX_SAMPLES: u64 = 1 << 30;

const SAMPLE_RATES: [u32; 8] = [44100, 48000, 37800, 32000, 0, 0, 0, 0];

/// `mpc_demux_skip_id3v2`: the size of a leading ID3v2 tag.
fn id3v2_size(data: &[u8]) -> Result<usize, Error> {
    if data.get(..3) != Some(b"ID3") {
        return Ok(0);
    }
    let h = data.get(..10).ok_or(Error::Header("short ID3v2 tag"))?;
    if h[5] & 0x0F != 0 || (h[6] | h[7] | h[8] | h[9]) & 0x80 != 0 {
        return Err(Error::Header("ID3v2 tag"));
    }
    let size = (usize::from(h[6]) << 21) | (usize::from(h[7]) << 14) | (usize::from(h[8]) << 7) | usize::from(h[9]);
    Ok(size + 10 + if h[5] & 0x10 != 0 { 10 } else { 0 })
}

/// The SV7 stream from the header on as libmpcdec reads it: little-endian 32-bit words, each
/// read from its most significant bit.
fn sv7_words(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(4) * 4);
    for c in data.chunks(4) {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        w.reverse();
        out.extend_from_slice(&w);
    }
    out
}

/// One SV8 block: its key, the payload range and the end of the block.
struct Block {
    key: [u8; 2],
    payload: std::ops::Range<usize>,
}

/// `mpc_bits_get_block` at byte `pos`, or `None` past the end.
fn block_at(data: &[u8], pos: usize) -> Option<Block> {
    let key = [*data.get(pos)?, *data.get(pos + 1)?];
    let mut size = 0u64;
    let mut header = 2usize;
    loop {
        let b = *data.get(pos + header)?;
        size = (size << 7) | u64::from(b & 0x7F);
        header += 1;
        if b & 0x80 == 0 || header > 11 {
            break;
        }
    }
    // the size includes the header (a size below it is taken as the payload size, as libmpcdec)
    let payload = if size >= header as u64 { size - header as u64 } else { size };
    let start = pos + header;
    let end = usize::try_from(payload).ok().and_then(|p| start.checked_add(p))?.min(data.len());
    Some(Block { key, payload: start..end })
}

fn varint(r: &mut Reader) -> u64 {
    let mut v = 0u64;
    for _ in 0..10 {
        let b = r.read(8);
        v = (v << 7) | u64::from(b & 0x7F);
        if b & 0x80 == 0 {
            break;
        }
    }
    v
}

fn check(info: &Info) -> Result<(), Error> {
    if info.bands == 0 || info.bands > 32 || info.channels == 0 || info.channels > 2 || info.sample_rate == 0 {
        return Err(Error::Header("stream parameters"));
    }
    Ok(())
}

/// Where the audio starts.
enum Start {
    /// SV7: the bit position in the word-swapped stream and its frame count.
    Sv7 { words: Vec<u8>, bit: usize, frames: u32, true_gapless: bool },
    /// SV8: the byte position of the first audio block.
    Sv8 { pos: usize, block_pwr: u32 },
}

fn parse(data: &[u8]) -> Result<(Info, Start), Error> {
    let hp = id3v2_size(data)?;
    let magic = data.get(hp..hp + 4).ok_or(Error::NotMusepack)?;
    if &magic[..3] == b"MP+" {
        let version = magic[3] & 15;
        if version != 7 {
            return Err(Error::Version(version));
        }
        let words = sv7_words(data.get(hp + 4..).unwrap_or(&[]));
        if words.len() < 24 {
            return Err(Error::Header("short SV7 header"));
        }
        let mut r = Reader::new(&words);
        let frames = r.read(32);
        r.read(1); // intensity stereo
        let ms = r.read(1) != 0;
        let max_band = r.read(6);
        r.read(4); // profile
        r.read(2); // link
        let sample_rate = SAMPLE_RATES[r.read(2) as usize & 7];
        r.skip(16 * 5); // ReplayGain
        let true_gapless = r.read(1) != 0;
        let mut last = u64::from(r.read(11));
        r.read(1); // fast seeking
        r.read(19);
        r.read(8); // encoder version
        if last == 0 {
            last = FRAME_LENGTH as u64;
        } else if last > FRAME_LENGTH as u64 {
            return Err(Error::Header("SV7 last frame length"));
        }
        let mut samples = u64::from(frames) * FRAME_LENGTH as u64;
        samples = if true_gapless { samples.saturating_sub(FRAME_LENGTH as u64 - last) } else { samples.saturating_sub(decoder::SYNTH_DELAY) };
        if max_band >= 32 {
            return Err(Error::Header("stream parameters"));
        }
        let info = Info { stream_version: 7, sample_rate, channels: 2, samples, beg_silence: 0, bands: max_band as u8 + 1, mid_side: ms, header_position: hp };
        if max_band == 0 {
            return Err(Error::Header("stream parameters"));
        }
        check(&info)?;
        let bit = r.pos();
        return Ok((info, Start::Sv7 { words, bit, frames, true_gapless }));
    }
    if magic != b"MPCK" {
        return Err(Error::NotMusepack);
    }
    let mut pos = hp + 4;
    let mut header: Option<(Info, u32)> = None;
    while let Some(b) = block_at(data, pos) {
        if !b.key.iter().all(u8::is_ascii_uppercase) {
            return Err(Error::Header("SV8 block key"));
        }
        if &b.key == b"AP" {
            let (info, block_pwr) = header.ok_or(Error::Header("no SV8 stream header"))?;
            return Ok((info, Start::Sv8 { pos, block_pwr }));
        }
        if &b.key == b"SH" {
            let p = data.get(b.payload.clone()).unwrap_or(&[]);
            let mut r = Reader::new(p);
            let crc = r.read(32);
            if p.len() < 4 || crc != bits::crc32(&p[4..]) {
                return Err(Error::Header("SV8 stream header CRC"));
            }
            let version = r.read(8) as u8;
            if version != 8 {
                return Err(Error::Version(version));
            }
            let samples = varint(&mut r);
            let beg_silence = varint(&mut r);
            let sample_rate = SAMPLE_RATES[r.read(3) as usize & 7];
            let bands = r.read(5) as u8 + 1;
            let channels = r.read(4) as u16 + 1;
            let ms = r.read(1) != 0;
            let block_pwr = r.read(3) * 2;
            let info = Info {
                stream_version: 8,
                sample_rate,
                channels,
                samples: samples.saturating_sub(beg_silence),
                beg_silence,
                bands,
                mid_side: ms,
                header_position: hp,
            };
            // libmpcdec takes `max_band` (here `bands`) below 32
            if bands >= 32 {
                return Err(Error::Header("stream parameters"));
            }
            check(&info)?;
            header = Some((info, block_pwr));
        }
        if &b.key == b"SE" {
            break;
        }
        pos = b.payload.end.max(pos + 3);
    }
    Err(Error::Header("no SV8 audio"))
}

/// Reads the stream header of a `.mpc` file.
pub fn probe(data: &[u8]) -> Result<Info, Error> {
    parse(data).map(|(info, _)| info)
}

/// Decodes a whole `.mpc` file into one plane of `f32` samples (−1…1) per channel. A damaged or
/// truncated stream decodes as far as it can.
pub fn decode_file(data: &[u8]) -> Result<(Info, Vec<Vec<f32>>), Error> {
    let (info, start) = parse(data)?;
    let total = info.samples.saturating_add(info.beg_silence);
    if total > MAX_SAMPLES.saturating_add(info.beg_silence) {
        return Err(Error::TooLong(info.samples));
    }
    let channels = usize::from(info.channels);
    let cap = usize::try_from(info.samples.min(MAX_SAMPLES)).unwrap_or(0);
    let mut planes = vec![Vec::with_capacity(cap); channels];
    match start {
        Start::Sv7 { words, mut bit, frames, true_gapless } => {
            let mut d = Decoder::new(Params {
                sv8: false,
                max_band: usize::from(info.bands) - 1,
                ms: info.mid_side,
                channels,
                samples: info.samples,
                beg_silence: 0,
                true_gapless,
            });
            for _ in 0..frames {
                let mut r = Reader::at(&words, bit);
                let size = r.read(20) as usize;
                let frame_start = r.pos();
                if r.past_end() || frame_start.saturating_add(size) > words.len() * 8 {
                    break;
                }
                if !d.decode_frame(&mut r, false, &mut planes) {
                    break;
                }
                bit = frame_start + size;
            }
        }
        Start::Sv8 { mut pos, block_pwr } => {
            let mut d = Decoder::new(Params {
                sv8: true,
                max_band: usize::from(info.bands),
                ms: info.mid_side,
                channels,
                samples: total,
                beg_silence: info.beg_silence,
                true_gapless: true,
            });
            let frames_per_block = 1usize << block_pwr.min(14);
            'blocks: while let Some(b) = block_at(data, pos) {
                if !b.key.iter().all(u8::is_ascii_uppercase) || &b.key == b"SE" {
                    break;
                }
                if &b.key == b"AP" {
                    let payload = data.get(b.payload.clone()).unwrap_or(&[]);
                    let mut r = Reader::new(payload);
                    for i in 0..frames_per_block {
                        // a frame takes at least one bit: a short last block ends here
                        if i > 0 && r.pos() >= payload.len() * 8 {
                            break;
                        }
                        if r.past_end() || !d.decode_frame(&mut r, i == 0, &mut planes) {
                            break 'blocks;
                        }
                    }
                }
                pos = b.payload.end.max(pos + 3);
                if planes.first().map_or(0, Vec::len) as u64 >= MAX_SAMPLES {
                    break;
                }
            }
        }
    }
    for p in &mut planes {
        p.truncate(MAX_SAMPLES as usize);
    }
    Ok((info, planes))
}

#[cfg(test)]
mod tests;
