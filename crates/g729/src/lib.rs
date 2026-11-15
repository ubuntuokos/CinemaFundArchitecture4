// Ported from bcg729 1.1.2 (https://github.com/BelledonneCommunications/bcg729), GPL-3.0-or-later,
// Copyright (c) 2011-2019 Belledonne Communications SARL.
// CFA4 changes GPL-3.0-or-later.

//! Pure-Rust ITU-T G.729 speech decoder with Annex B comfort noise, ported from bcg729's
//! fixed-point decoder, so the output is bcg729's sample for sample.
//!
//! [`decode_file`] reads a raw G.729 stream (10-byte frames, as ffmpeg's `g729` format writes
//! it) or an ITU-T test-vector bitstream (`.bit`, one 16-bit word per bit).

mod decoder;
mod math;
mod tables;

pub use decoder::{Decoder, FRAME_LENGTH, Frame};

/// The sample rate of G.729.
pub const SAMPLE_RATE: u32 = 8000;

/// The decoded output is capped at this many samples (about 37 hours at 8 kHz).
pub const MAX_SAMPLES: usize = 1 << 30;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("no G.729 frames")]
    Empty,
}

const BIT_SYNC: u16 = 0x6b21;
const BIT_SYNC_ERASED: u16 = 0x6b20;
const BIT_ONE: u16 = 0x0081;

/// True when `data` starts like an ITU-T serial bitstream (`.bit`).
fn is_itu_bitstream(data: &[u8]) -> bool {
    matches!(data.get(..2), Some([0x21 | 0x20, 0x6b]))
}

/// Decodes the frames of an ITU-T serial bitstream: each frame is a sync word, a bit count
/// (80 for speech, 15 or 16 for an Annex B SID frame, 0 for a frame not sent) and one word per bit.
fn decode_itu(data: &[u8], d: &mut Decoder, out: &mut Vec<i16>) {
    let words: Vec<u16> = data.chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect();
    let mut i = 0usize;
    while let (Some(&sync), Some(&len)) = (words.get(i), words.get(i + 1)) {
        let len = usize::from(len);
        let Some(bits) = words.get(i + 2..i + 2 + len) else { break };
        i += 2 + len;
        let mut packed = [0u8; 10];
        for (k, &b) in bits.iter().take(80).enumerate() {
            if b == BIT_ONE {
                packed[k / 8] |= 0x80 >> (k % 8);
            }
        }
        let frame = match (sync, len) {
            (BIT_SYNC, 80) => Frame::Speech(&packed),
            (BIT_SYNC, 15 | 16) => {
                let [a, b, ..] = packed;
                let sid = [a, b];
                out.extend_from_slice(&d.decode(Frame::Sid(&sid)));
                if out.len() >= MAX_SAMPLES {
                    return;
                }
                continue;
            }
            (BIT_SYNC | BIT_SYNC_ERASED, _) => Frame::Erased,
            _ => break,
        };
        out.extend_from_slice(&d.decode(frame));
        if out.len() >= MAX_SAMPLES {
            return;
        }
    }
}

/// Decodes a whole G.729 file into 8 kHz 16-bit samples.
pub fn decode_file(data: &[u8]) -> Result<Vec<i16>, Error> {
    let mut d = Decoder::new();
    let mut out = Vec::with_capacity((data.len() / 10).saturating_mul(FRAME_LENGTH).min(MAX_SAMPLES));
    if is_itu_bitstream(data) {
        decode_itu(data, &mut d, &mut out);
    } else {
        for f in data.chunks_exact(10) {
            let Ok(frame) = <&[u8; 10]>::try_from(f) else { break };
            out.extend_from_slice(&d.decode(Frame::Speech(frame)));
            if out.len() >= MAX_SAMPLES {
                break;
            }
        }
    }
    if out.is_empty() {
        return Err(Error::Empty);
    }
    out.truncate(MAX_SAMPLES);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_itu_framing_agree() {
        let mut raw = Vec::new();
        let mut itu = Vec::new();
        for n in 0..30u8 {
            let f: [u8; 10] = std::array::from_fn(|k| n.wrapping_mul(37).wrapping_add(k as u8 * 11));
            raw.extend_from_slice(&f);
            itu.extend_from_slice(&BIT_SYNC.to_le_bytes());
            itu.extend_from_slice(&80u16.to_le_bytes());
            for k in 0..80 {
                let bit = f[k / 8] >> (7 - k % 8) & 1;
                itu.extend_from_slice(&(if bit == 1 { BIT_ONE } else { 0x007f }).to_le_bytes());
            }
        }
        let a = decode_file(&raw).ok();
        assert!(a.as_ref().is_some_and(|a| a.len() == 30 * FRAME_LENGTH));
        assert_eq!(a, decode_file(&itu).ok());
    }

    #[test]
    fn damaged_input_never_panics() {
        assert_eq!(decode_file(&[]), Err(Error::Empty));
        assert_eq!(decode_file(&[1, 2, 3]), Err(Error::Empty));
        let mut x = 0x9e37_79b9u32;
        for len in [2usize, 9, 10, 11, 100, 1000, 4096] {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let _ = decode_file(&data);
            let mut itu = vec![0x21, 0x6b];
            itu.extend_from_slice(&data);
            let _ = decode_file(&itu);
        }
    }
}
