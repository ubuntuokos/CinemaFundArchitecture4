// Ported from FFmpeg n9.0.2 libavcodec/adxdec.c, libavcodec/adx.c (Copyright (c) 2001,2003 BERO,
// Copyright (c) 2011 Justin Ruggles) and libavformat/adxdec.c, LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **CRI ADX**, the 4-bit ADPCM audio format of SEGA and CRIWARE games
//! (`.adx`): encoding 3 (fixed-block ADPCM with a cutoff-derived 2nd-order predictor), 1 to 6
//! channels. The output is the same as ffmpeg's `adpcm_adx` decoder, sample for sample.
//!
//! [`decode_file`] decodes a whole `.adx` file to one plane of signed 16-bit samples per
//! channel. [`decode_blocks`] decodes the raw block data behind an already parsed [`Header`].

/// Bytes of one channel's block.
pub const BLOCK_SIZE: usize = 18;
/// Samples in one channel's block.
pub const BLOCK_SAMPLES: usize = 32;
/// The most channels the decoder accepts (ffmpeg's `MAX_CHANNELS`).
pub const MAX_CHANNELS: usize = 6;
/// The most samples (all channels together) one decode may produce: 1 GiB of `i16`.
pub const MAX_SAMPLES: usize = 1 << 29;
/// Fixed-point bits of the predictor coefficients.
const COEFF_BITS: u32 = 12;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not an ADX file (no 0x8000 signature)")]
    NotAdx,
    #[error("invalid ADX header: {0}")]
    Invalid(&'static str),
    #[error("unsupported ADX format: {0}")]
    Unsupported(&'static str),
    #[error("no audio in the ADX file")]
    Empty,
    #[error("the decoded audio would exceed {MAX_SAMPLES} samples")]
    TooLarge,
}

/// The stream parameters of the ADX header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub channels: u8,
    pub sample_rate: u32,
    /// The low-pass cutoff frequency in Hz that the predictor is derived from.
    pub cutoff: u16,
    /// ADX header version (byte 0x12; 3, 4 or 5), informational.
    pub version: u8,
    /// Offset of the first block (the header size).
    pub data_offset: usize,
    coeff: [i32; 2],
}

impl Header {
    /// A decoded sample as `f32` in −1…1.
    pub fn to_f32(v: i16) -> f32 {
        f32::from(v) / 32768.0
    }
}

/// The two quantised predictor coefficients for a cutoff (ffmpeg's `ff_adx_calculate_coeffs`).
fn calculate_coeffs(cutoff: u32, sample_rate: u32, bits: u32) -> [i32; 2] {
    let a = std::f64::consts::SQRT_2 - (2.0 * std::f64::consts::PI * f64::from(cutoff) / f64::from(sample_rate)).cos();
    let b = std::f64::consts::SQRT_2 - 1.0;
    let c = (a - ((a + b) * (a - b)).sqrt()) / b;
    // `lrintf`: the double is narrowed to float first, then rounded half to even.
    let q = |x: f64| (x as f32).round_ties_even() as i32;
    [q(c * 2.0 * f64::from(1u32 << bits)), q(-(c * c) * f64::from(1u32 << bits))]
}

/// Parse the ADX header at the start of `buf`.
pub fn parse_header(buf: &[u8]) -> Result<Header, Error> {
    if buf.len() < 24 {
        return Err(Error::Invalid("header shorter than 24 bytes"));
    }
    if buf[0] != 0x80 || buf[1] != 0 {
        return Err(Error::NotAdx);
    }
    let offset = usize::from(u16::from_be_bytes([buf[2], buf[3]])) + 4;
    // The copyright string "(c)CRI" ends the header; ffmpeg checks it when it is within the data.
    if buf.len() >= offset && offset >= 6 && &buf[offset - 6..offset] != b"(c)CRI" {
        return Err(Error::Invalid("missing (c)CRI marker"));
    }
    if offset < 24 {
        return Err(Error::Invalid("header size"));
    }
    if buf[4] != 3 || buf[5] != 18 || buf[6] != 4 {
        return Err(Error::Unsupported("encoding other than 3 with 18-byte blocks of 4-bit samples"));
    }
    // Version 4 with flags 8 or 9 marks an encrypted file: the samples would be noise.
    if buf[18] == 4 && matches!(buf[19], 8 | 9) {
        return Err(Error::Unsupported("encrypted ADX"));
    }
    let channels = buf[7];
    if channels == 0 || usize::from(channels) > MAX_CHANNELS {
        return Err(Error::Invalid("channel count"));
    }
    let sample_rate = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
    if sample_rate < 1 || sample_rate > i32::MAX as u32 / (u32::from(channels) * BLOCK_SIZE as u32 * 8) {
        return Err(Error::Invalid("sample rate"));
    }
    let cutoff = u16::from_be_bytes([buf[16], buf[17]]);
    Ok(Header { channels, sample_rate, cutoff, version: buf[18], data_offset: offset, coeff: calculate_coeffs(u32::from(cutoff), sample_rate, COEFF_BITS) })
}

/// Decode the block data (everything behind the header): one plane per channel. Decoding stops
/// at an end-of-stream block (scale with bit 15 set) and drops a trailing partial block group,
/// as ffmpeg does.
pub fn decode_blocks(header: &Header, data: &[u8]) -> Result<Vec<Vec<i16>>, Error> {
    let ch = usize::from(header.channels);
    // The fields are public: a header changed after `parse_header` must not reach the division.
    if ch == 0 || ch > MAX_CHANNELS {
        return Err(Error::Invalid("channel count"));
    }
    let groups = data.len() / (BLOCK_SIZE * ch);
    if groups * BLOCK_SAMPLES * ch > MAX_SAMPLES {
        return Err(Error::TooLarge);
    }
    let mut planes: Vec<Vec<i16>> = Vec::with_capacity(ch);
    for _ in 0..ch {
        let mut p = Vec::new();
        p.try_reserve_exact(groups * BLOCK_SAMPLES).map_err(|_| Error::TooLarge)?;
        planes.push(p);
    }
    let mut hist = [(0i32, 0i32); MAX_CHANNELS];
    'groups: for g in 0..groups {
        // A group is decoded only when none of its channel blocks is an end marker.
        let base = g * BLOCK_SIZE * ch;
        for c in 0..ch {
            if data[base + c * BLOCK_SIZE] & 0x80 != 0 {
                break 'groups;
            }
        }
        for c in 0..ch {
            let block = &data[base + c * BLOCK_SIZE..base + (c + 1) * BLOCK_SIZE];
            let scale = i32::from(u16::from_be_bytes([block[0], block[1]]));
            let (mut s1, mut s2) = hist[c];
            for i in 0..BLOCK_SAMPLES {
                let byte = block[2 + i / 2];
                let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0x0F };
                let d = i32::from(nibble ^ 8) - 8;
                let s0 = d * scale + ((header.coeff[0] * s1 + header.coeff[1] * s2) >> COEFF_BITS);
                s2 = s1;
                s1 = s0.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                planes[c].push(s1 as i16);
            }
            hist[c] = (s1, s2);
        }
    }
    Ok(planes)
}

/// Decode a whole `.adx` file.
pub fn decode_file(bytes: &[u8]) -> Result<(Header, Vec<Vec<i16>>), Error> {
    let header = parse_header(bytes)?;
    // ffmpeg's demuxer reads the header as extradata and fails when the file ends inside it.
    let data = bytes.get(header.data_offset..).ok_or(Error::Invalid("file ends inside the header"))?;
    let planes = decode_blocks(&header, data)?;
    if planes.first().is_none_or(Vec::is_empty) {
        return Err(Error::Empty);
    }
    Ok((header, planes))
}
