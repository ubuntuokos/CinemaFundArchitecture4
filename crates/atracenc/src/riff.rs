//! AT3 files: ATRAC3 in a RIFF WAVE (format 0x0270, `WAVE_FORMAT_SONY_SCX`) with the 14 bytes of
//! extra data Sony's tools and FFmpeg read, and a `fact` chunk with the sample count and the
//! encoder delay (the samples a player skips at the start, as vgmstream reads them).
//!
//! The layout follows atracdenc 0.2.3 `src/at3.cpp` (LGPL-2.1-or-later) and FFmpeg's
//! `atrac3_decode_init`; CFA4 changes GPL-3.0-or-later.

use crate::atrac3::{self, Encoder, Mode};
use crate::{Error, Result};

/// Description of an AT3 stream.
#[derive(Clone, Copy, Debug)]
pub struct At3Header {
    pub channels: u16,
    pub block_align: u16,
    pub joint_stereo: bool,
    /// Samples per channel of the source.
    pub samples: u32,
    /// Samples per channel a decoder outputs before the first source sample.
    pub delay: u32,
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// A complete AT3 file of `data` (whole frames).
pub fn at3_file(h: &At3Header, data: &[u8]) -> Result<Vec<u8>> {
    let total = 12 + 8 + 32 + 8 + 8 + 8 + data.len() as u64;
    if total > u64::from(u32::MAX) {
        return Err(Error::TooLarge(total));
    }
    let mut fmt = Vec::with_capacity(32);
    fmt.extend_from_slice(&0x0270u16.to_le_bytes());
    fmt.extend_from_slice(&h.channels.to_le_bytes());
    fmt.extend_from_slice(&atrac3::SAMPLE_RATE.to_le_bytes());
    let byte_rate = u64::from(h.block_align) * u64::from(atrac3::SAMPLE_RATE) / atrac3::SAMPLES_PER_FRAME as u64;
    fmt.extend_from_slice(&(byte_rate as u32).to_le_bytes());
    fmt.extend_from_slice(&h.block_align.to_le_bytes());
    fmt.extend_from_slice(&0u16.to_le_bytes()); // bits per sample
    fmt.extend_from_slice(&14u16.to_le_bytes()); // cbSize
    fmt.extend_from_slice(&1u16.to_le_bytes());
    // 0x1000 in Sony's stereo files
    fmt.extend_from_slice(&(2048 * u32::from(h.channels)).to_le_bytes());
    let js = u16::from(h.joint_stereo);
    fmt.extend_from_slice(&js.to_le_bytes());
    fmt.extend_from_slice(&js.to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes()); // frame factor
    fmt.extend_from_slice(&0u16.to_le_bytes());
    let mut fact = Vec::with_capacity(8);
    fact.extend_from_slice(&h.samples.to_le_bytes());
    fact.extend_from_slice(&h.delay.to_le_bytes());
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    chunk(&mut out, b"fmt ", &fmt);
    chunk(&mut out, b"fact", &fact);
    chunk(&mut out, b"data", data);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    Ok(out)
}

/// The decoder output delay of this encoder (QMF tree and MDCT overlap), in samples.
pub const ATRAC3_DELAY: u32 = 1162;

/// Encodes interleaved 44.1 kHz `pcm` of `channels` (1 or 2) into an AT3 file at `mode`, with
/// enough trailing frames that the whole source decodes; `progress` gets the fraction done and
/// stops the encoding when it returns `false` (then the result is `None`).
pub fn encode_at3(pcm: &[f32], channels: u16, mode: Mode, progress: &mut dyn FnMut(f32) -> bool) -> Result<Option<Vec<u8>>> {
    let mut enc = Encoder::new(channels, mode)?;
    let c = usize::from(channels);
    let samples = pcm.len() / c;
    let frames = (samples + ATRAC3_DELAY as usize).div_ceil(atrac3::SAMPLES_PER_FRAME).max(1);
    let ba = enc.block_align();
    let size = frames as u64 * ba as u64;
    if size > u64::from(u32::MAX) - 1024 {
        return Err(Error::TooLarge(size));
    }
    let mut data = Vec::with_capacity(size as usize);
    let step = atrac3::SAMPLES_PER_FRAME * c;
    for f in 0..frames {
        let from = (f * step).min(pcm.len());
        let to = ((f + 1) * step).min(pcm.len());
        data.extend_from_slice(&enc.encode(&pcm[from..to]));
        if f % 64 == 0 && !progress(f as f32 / frames as f32) {
            return Ok(None);
        }
    }
    let h = At3Header {
        channels,
        block_align: ba as u16,
        joint_stereo: enc.mode().joint_stereo,
        samples: u32::try_from(samples).map_err(|_| Error::TooLarge(samples as u64))?,
        delay: ATRAC3_DELAY,
    };
    at3_file(&h, &data).map(Some)
}
