// SPDX-License-Identifier: LGPL-2.1-or-later
// The audio shuffle tables, the samples-per-frame minimums, the AAUX source pack fields and the
// 12-bit to 16-bit expansion below are ported from libdv 1.0.0 (libdv/audio.c, libdv/dv_types.h;
// Copyright (C) Charles 'Buck' Krasic, Erik Walthinsen - April 2000; LGPL-2.1-or-later, "either
// version 2.1, or (at your option) any later version"). They are the audio tables of
// IEC 61834-2 / SMPTE 314M. Re-expressed for CFA4; see crates/dv/README.md and
// docs/licensing.md §5.

//! DV audio: PCM in the audio DIF blocks.
//!
//! Every DIF sequence has 9 audio blocks: bytes 3-7 hold an AAUX pack, bytes 8-79 the samples
//! (36 16-bit samples, or 24 pairs of 12-bit samples). The samples of a frame are shuffled over
//! the sequences and blocks; the first half of the sequences carries channels 1-2, the second
//! half channels 3-4 (12-bit four-channel mode) or nothing more. The AAUX source pack (`0x50`) of
//! the first audio blocks says the sampling rate, the quantisation and how many samples the frame
//! holds.

use crate::dif::{BLOCK_SIZE, SEQUENCE_SIZE};

/// Sample position of the first sample of audio block `[sequence][block]`, 525/60.
const UNSHUFFLE_60: [[u16; 9]; 5] = [
    [0, 15, 30, 10, 25, 40, 5, 20, 35],
    [3, 18, 33, 13, 28, 43, 8, 23, 38],
    [6, 21, 36, 1, 16, 31, 11, 26, 41],
    [9, 24, 39, 4, 19, 34, 14, 29, 44],
    [12, 27, 42, 7, 22, 37, 2, 17, 32],
];

/// The same for 625/50.
const UNSHUFFLE_50: [[u16; 9]; 6] = [
    [0, 18, 36, 13, 31, 49, 8, 26, 44],
    [3, 21, 39, 16, 34, 52, 11, 29, 47],
    [6, 24, 42, 1, 19, 37, 14, 32, 50],
    [9, 27, 45, 4, 22, 40, 17, 35, 53],
    [12, 30, 48, 7, 25, 43, 2, 20, 38],
    [15, 33, 51, 10, 28, 46, 5, 23, 41],
];

/// Fewest samples per frame, by system (525, 625) and rate (48, 44.1, 32 kHz); the AAUX source
/// pack adds 0-63 to it.
const MIN_SAMPLES: [[u16; 3]; 2] = [[1580, 1452, 1053], [1896, 1742, 1264]];

const RATES: [u32; 3] = [48_000, 44_100, 32_000];

/// The value that marks a damaged sample.
const ERROR_16: i16 = i16::MIN;
const ERROR_12: i32 = 0x800;

/// Byte offset of the AAUX source pack in a frame: first sequence, audio block 3 (after the 6
/// header / subcode / VAUX blocks, every audio block is followed by 15 video blocks).
const fn aaux_offset(sequence: usize, audio_block: usize) -> usize {
    sequence * SEQUENCE_SIZE + (6 + audio_block * 16) * BLOCK_SIZE + 3
}

/// The audio of one frame, as its AAUX source pack describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    /// 16 (linear) or 12 (non-linear, expanded to 16 on decode).
    pub bits: u8,
    /// 2, or 4 for 12-bit audio with a second channel pair.
    pub channels: usize,
    /// Samples per channel in this frame.
    pub samples: usize,
    /// Pre-emphasis is on (it is not undone).
    pub emphasis: bool,
    /// 625/50 (true) or 525/60.
    pub fifty: bool,
}

/// Decoded audio of one frame: one `Vec` per channel, `format.samples` long.
#[derive(Clone, Debug)]
pub struct Audio {
    pub format: AudioFormat,
    pub channels: Vec<Vec<i16>>,
    /// Damaged samples (replaced by the sample before them).
    pub errors: u32,
}

/// Fields of an AAUX source pack (`50 pc1 pc2 pc3 pc4`).
struct SourcePack {
    af_size: u16,
    audio_mode: u8,
    fifty: bool,
    smp: usize,
    qu: u8,
    emphasis: bool,
}

fn source_pack(frame: &[u8], at: usize) -> Option<SourcePack> {
    let p = frame.get(at..at + 5)?;
    if p[0] != 0x50 {
        return None;
    }
    Some(SourcePack {
        af_size: u16::from(p[1] & 0x3f),
        audio_mode: p[2] & 0x0f,
        fifty: p[3] & 0x20 != 0,
        smp: usize::from((p[4] >> 3) & 7),
        qu: p[4] & 7,
        emphasis: p[4] & 0x80 == 0,
    })
}

/// Byte offset in a frame of the AAUX source pack that says the frame's sample count.
pub const SOURCE_PACK_OFFSET: usize = aaux_offset(0, 3);

/// Samples per channel of a frame, from the 5 bytes at [`SOURCE_PACK_OFFSET`]: lets a reader
/// lay out a file's audio without decoding it.
pub fn source_pack_samples(pack: &[u8]) -> Option<usize> {
    let sp = source_pack(pack, 0)?;
    Some(usize::from(MIN_SAMPLES[usize::from(sp.fifty)].get(sp.smp)? + sp.af_size))
}

/// The audio format of a frame, from the AAUX source pack of its first DIF sequence. `None`
/// when the pack is missing or names an unsupported format (20-bit, an unknown rate).
pub fn audio_format(frame: &[u8]) -> Option<AudioFormat> {
    let sp = source_pack(frame, aaux_offset(0, 3))?;
    let sample_rate = *RATES.get(sp.smp)?;
    let bits = match sp.qu {
        0 => 16,
        1 => 12,
        _ => return None,
    };
    let samples = usize::from(MIN_SAMPLES[usize::from(sp.fifty)][sp.smp] + sp.af_size);
    // 12-bit: a second channel pair when the second half's source pack carries audio
    let channels = if bits == 12 {
        let half = if sp.fifty { 6 } else { 5 };
        let block = if half % 2 == 0 { 3 } else { 0 };
        match source_pack(frame, aaux_offset(half, block)) {
            Some(second) if second.audio_mode != 0xf => 4,
            _ => 2,
        }
    } else {
        2
    };
    Some(AudioFormat { sample_rate, bits, channels, samples, emphasis: sp.emphasis, fifty: sp.fifty })
}

/// Expand a 12-bit non-linear sample to 16 bits (IEC 61834-2).
fn expand_12(sample: i32) -> i16 {
    let shift = (sample & 0xf00) >> 8;
    let v = match shift {
        0x2..=0x7 => {
            let s = shift - 1;
            (sample - 256 * s) << s
        }
        0x8..=0xd => {
            let s = 0xe - shift;
            ((sample + 256 * s + 1) << s) - 1
        }
        _ => sample,
    };
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Decode the audio of one DIF frame. `None` when the frame carries no (supported) audio.
pub fn decode_audio(frame: &[u8]) -> Option<Audio> {
    let format = audio_format(frame)?;
    let (half, stride) = if format.fifty { (6usize, 54usize) } else { (5, 45) };
    let max = stride * if format.bits == 16 { 36 } else { 24 };
    let mut channels = vec![vec![ERROR_16; max]; format.channels];
    for seq in 0..2 * half {
        let (pair, ds) = if seq < half { (0, seq) } else { (1, seq - half) };
        for block in 0..9 {
            let base = if format.fifty { UNSHUFFLE_50.get(ds).and_then(|r| r.get(block)) } else { UNSHUFFLE_60.get(ds).and_then(|r| r.get(block)) };
            let Some(&base) = base else { continue };
            let start = aaux_offset(seq, block) - 3;
            let Some(b) = frame.get(start..start + BLOCK_SIZE) else { continue };
            if b[0] >> 5 != crate::dif::sct::AUDIO {
                continue;
            }
            let base = usize::from(base);
            if format.bits == 16 {
                let Some(ch) = channels.get_mut(pair) else { continue };
                for (k, s) in b[8..].chunks_exact(2).enumerate() {
                    if let Some(d) = ch.get_mut(base + k * stride) {
                        *d = i16::from_be_bytes([s[0], s[1]]);
                    }
                }
            } else {
                if pair * 2 + 1 >= channels.len() {
                    continue;
                }
                for (k, s) in b[8..].chunks_exact(3).enumerate() {
                    let y = (i32::from(s[0]) << 4) | i32::from(s[2] >> 4);
                    let z = (i32::from(s[1]) << 4) | i32::from(s[2] & 0xf);
                    for (c, v) in [(pair * 2, y), (pair * 2 + 1, z)] {
                        let v = if v > 2048 { v - 4096 } else { v };
                        if let Some(d) = channels.get_mut(c).and_then(|ch| ch.get_mut(base + k * stride)) {
                            *d = if v == ERROR_12 { ERROR_16 } else { expand_12(v) };
                        }
                    }
                }
            }
        }
    }
    // damaged (or missing) samples repeat the sample before them
    let mut errors = 0u32;
    for ch in &mut channels {
        ch.truncate(format.samples.min(max));
        let mut last = 0i16;
        for s in ch.iter_mut() {
            if *s == ERROR_16 {
                errors = errors.saturating_add(1);
                *s = last;
            } else {
                last = *s;
            }
        }
    }
    Some(Audio { format, channels, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unshuffle_tables_cover_every_position_once() {
        for (rows, stride) in [(&UNSHUFFLE_60[..], 45usize), (&UNSHUFFLE_50[..], 54)] {
            let mut seen = vec![false; stride];
            for r in rows {
                for &v in r {
                    assert!(!seen[usize::from(v)]);
                    seen[usize::from(v)] = true;
                }
            }
            assert!(seen.iter().all(|&s| s));
        }
    }

    #[test]
    fn twelve_bit_expansion_is_monotonic_and_symmetric() {
        assert_eq!(expand_12(0), 0);
        assert_eq!(expand_12(0x1ff), 0x1ff);
        assert_eq!(expand_12(0x7ff), 32704);
        let mut prev = expand_12(-2047);
        for v in -2046..=2047 {
            let e = expand_12(v);
            assert!(e > prev, "{v}");
            prev = e;
        }
        assert_eq!(expand_12(-1), -1);
    }

    #[test]
    fn no_audio_without_a_source_pack() {
        assert!(decode_audio(&[0u8; 1000]).is_none());
        assert!(audio_format(&[]).is_none());
    }
}
