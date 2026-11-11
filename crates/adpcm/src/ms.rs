//! Microsoft ADPCM (`WAVE_FORMAT_ADPCM`, 0x0002; Microsoft Multimedia Standards Update, 1994):
//! a second-order fixed predictor chosen per block from a coefficient set, and a 4-bit signed
//! code scaled by an adaptive delta.

use crate::Error;

/// The seven standard coefficient pairs (in units of 1/256).
pub const STANDARD_COEFFS: [(i32, i32); 7] = [(256, 0), (512, -256), (0, 0), (192, 64), (240, 0), (460, -208), (392, -232)];
/// Delta adaptation factors (in units of 1/256) per code.
const ADAPT: [i64; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230];

/// The coefficient set of an `ADPCMWAVEFORMAT` extension: `wSamplesPerBlock`, `wNumCoef`, then
/// the pairs. The standard set when the extension is missing or damaged.
pub fn coefficients(extra: &[u8]) -> Vec<(i32, i32)> {
    let u16_at = |at: usize| extra.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let i16_at = |at: usize| extra.get(at..at + 2).map(|b| i32::from(i16::from_le_bytes([b[0], b[1]])));
    let Some(n) = u16_at(2).map(usize::from).filter(|&n| n > 0) else { return STANDARD_COEFFS.to_vec() };
    let pairs: Option<Vec<(i32, i32)>> = (0..n).map(|k| Some((i16_at(4 + 4 * k)?, i16_at(6 + 4 * k)?))).collect();
    pairs.unwrap_or_else(|| STANDARD_COEFFS.to_vec())
}

/// Sample frames in a block of `len` bytes (a short final block holds fewer).
pub fn block_frames(len: usize, channels: usize) -> usize {
    let head = 7 * channels;
    if channels == 0 || len < head {
        return 0;
    }
    (len - head) * 2 / channels + 2
}

struct Channel {
    c1: i64,
    c2: i64,
    delta: i64,
    s1: i64,
    s2: i64,
}

/// Largest delta kept: far above anything a real encoder reaches, it bounds hostile blocks, at
/// the largest value whose adaptation (×768) fits 32 bits: fitted to ffmpeg's decoder, which
/// recovers from a runaway delta exactly as fast.
const MAX_DELTA: i64 = i32::MAX as i64 / 768;

impl Channel {
    fn decode(&mut self, code: u8) -> i16 {
        let signed = i64::from(code & 15) - if code & 8 != 0 { 16 } else { 0 };
        // a division (rounding toward zero), not a shift: they differ for negative predictions
        let predicted = (self.s1 * self.c1 + self.s2 * self.c2) / 256;
        let s = (predicted + signed * self.delta).clamp(-32768, 32767);
        self.s2 = self.s1;
        self.s1 = s;
        self.delta = ((ADAPT.get(usize::from(code & 15)).copied().unwrap_or(230) * self.delta) >> 8).clamp(16, MAX_DELTA);
        s as i16
    }
}

/// Decode one block: per channel the predictor index (u8), then per channel the delta, then the
/// newer and then the older history sample (i16 each); the older sample plays first. Codes follow,
/// high nibble first, channels interleaved code by code.
pub fn decode_block(block: &[u8], channels: usize, coeffs: &[(i32, i32)], out: &mut [Vec<i16>]) -> Result<(), Error> {
    let head = 7usize.checked_mul(channels).ok_or(Error::Invalid("channel count"))?;
    if channels == 0 || out.len() != channels {
        return Err(Error::Invalid("channel count"));
    }
    let hdr = block.get(..head).ok_or(Error::Truncated("Microsoft ADPCM block header"))?;
    let i16_at = |at: usize| hdr.get(at..at + 2).map_or(0, |b| i64::from(i16::from_le_bytes([b[0], b[1]])));
    let mut state = Vec::with_capacity(channels);
    for c in 0..channels {
        let idx = usize::from(hdr.get(c).copied().unwrap_or(0));
        let &(c1, c2) = coeffs.get(idx).ok_or(Error::Invalid("Microsoft ADPCM predictor index"))?;
        let delta = i16_at(channels + 2 * c);
        let s1 = i16_at(3 * channels + 2 * c);
        let s2 = i16_at(5 * channels + 2 * c);
        state.push(Channel { c1: i64::from(c1), c2: i64::from(c2), delta, s1, s2 });
    }
    for (o, s) in out.iter_mut().zip(&state) {
        o.push(s.s2 as i16);
        o.push(s.s1 as i16);
    }
    let mut ch = 0;
    for &b in block.get(head..).unwrap_or_default() {
        for code in [b >> 4, b & 15] {
            if let (Some(s), Some(o)) = (state.get_mut(ch), out.get_mut(ch)) {
                o.push(s.decode(code));
            }
            ch = (ch + 1) % channels;
        }
    }
    Ok(())
}
