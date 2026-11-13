// SPDX-License-Identifier: GPL-3.0-or-later
//! CFA4: the interface the rest of CFA4 uses (not part of the ported crate).
//!
//! A carrier is the decoded PCM of a DTS-HD Master Audio, FLAC, WAV or LPCM track, as planar
//! 24-bit integers in the WAV / SMPTE channel order used across CFA4 (L R C LFE back-L back-R
//! side-L side-R; 5.1 is L R C LFE Ls Rs). [`detect`] says whether it carries an Auro-Codec side
//! channel and which layout it restores; [`unfold`] restores that layout. The output order is
//! [`output_order`]: the bed in the same WAV order, then the heights, then the top.
//!
//! Only a bit-exact carrier can be decoded: a lossy decode (DTS core, AC-3, AAC) has lost the
//! low bits. [`pcm24_from_f32`] turns CFA4's `f32` samples back into the integers and refuses a
//! channel that is not exact 24-bit PCM, which also keeps the detector off lossy material.

use crate::decode::StreamId;
use crate::detect::{Detection, Detector};
use crate::layout::Layout;
use crate::unfold::{MAX_PUSH, Unfolder};

/// Full scale of a 24-bit sample.
const FULL_SCALE: f32 = 8_388_608.0;

/// Most carrier channels a decode accepts (7.1).
pub const MAX_CARRIERS: usize = 8;

/// How far into a carrier [`detect`] looks before giving up: 10 s at 48 kHz. Blocks repeat every
/// few milliseconds, so a carrier latches within its first second.
pub const DETECT_FRAMES: usize = 480_000;

/// The stream each channel of a carrier of `channels` channels plays as, in WAV order. `None`
/// for a channel count no Auro carrier uses.
pub fn wav_carrier_ids(channels: usize) -> Option<&'static [StreamId]> {
    static MONO: [StreamId; 1] = [StreamId(2)];
    static STEREO: [StreamId; 2] = [StreamId(0), StreamId(1)];
    static QUAD: [StreamId; 4] = [StreamId(0), StreamId(1), StreamId(4), StreamId(5)];
    static FIVE: [StreamId; 5] = [StreamId(0), StreamId(1), StreamId(2), StreamId(4), StreamId(5)];
    static FIVE_ONE: [StreamId; 6] = [StreamId(0), StreamId(1), StreamId(2), StreamId(3), StreamId(4), StreamId(5)];
    static SEVEN_ONE: [StreamId; 8] = [StreamId(0), StreamId(1), StreamId(2), StreamId(3), StreamId(7), StreamId(8), StreamId(4), StreamId(5)];
    Some(match channels {
        1 => &MONO,
        2 => &STEREO,
        4 => &QUAD,
        5 => &FIVE,
        6 => &FIVE_ONE,
        8 => &SEVEN_ONE,
        _ => return None,
    })
}

/// Where a stream sits in CFA4's output order: the bed in WAV order, then the heights, then the
/// top.
fn rank(id: StreamId) -> u8 {
    match id.0 {
        0 => 0,   // L
        1 => 1,   // R
        2 => 2,   // C
        3 => 3,   // LFE
        7 => 4,   // Lb
        8 => 5,   // Rb
        4 => 6,   // Ls
        5 => 7,   // Rs
        6 => 8,   // Cs
        9 => 9,   // HL
        10 => 10, // HR
        11 => 11, // HC
        13 => 12, // HLs
        14 => 13, // HRs
        12 => 14, // T
        other => other.saturating_add(16),
    }
}

/// The streams `layout` restores, in CFA4's output order. `None` for a layout the tables do not
/// know.
pub fn output_order(layout: Layout) -> Option<Vec<StreamId>> {
    let mut ids: Vec<StreamId> = layout.streams()?.as_slice().iter().map(|&i| StreamId(i)).collect();
    ids.sort_by_key(|&id| rank(id));
    Some(ids)
}

/// Exact 24-bit integers from samples in -1.0 ..= 1.0 (a 24-bit sample divided by 2^23).
/// `None` when a sample is not one, i.e. the channel is not bit-exact 24-bit PCM.
pub fn pcm24_from_f32(samples: &[f32]) -> Option<Vec<i32>> {
    samples
        .iter()
        .map(|&v| {
            let s = v * FULL_SCALE;
            (s.is_finite() && s.fract() == 0.0 && (-FULL_SCALE..FULL_SCALE).contains(&s)).then_some(s as i32)
        })
        .collect()
}

/// 24-bit integers back to samples in -1.0 ..= 1.0.
pub fn f32_from_pcm24(samples: &[i32]) -> Vec<f32> {
    samples.iter().map(|&v| v as f32 / FULL_SCALE).collect()
}

/// Looks for an Auro-Codec side channel in the first `max_frames` frames of a carrier. `None`
/// when no layout latches (plain PCM, or a channel count no carrier uses).
pub fn detect(carrier: &[&[i32]], max_frames: usize) -> Option<Detection> {
    if carrier.is_empty() || carrier.len() > MAX_CARRIERS {
        return None;
    }
    let frames = carrier.iter().map(|c| c.len()).min().unwrap_or(0).min(max_frames);
    let mut det = Detector::new(carrier.len());
    let mut start = 0;
    while start < frames {
        let end = start.saturating_add(MAX_PUSH).min(frames);
        for (ch, samples) in carrier.iter().enumerate() {
            if let Some(d) = samples.get(start..end).and_then(|s| det.push(ch, s)) {
                return Some(d);
            }
        }
        start = end;
    }
    det.detection()
}

/// A decoded Auro-3D carrier.
#[derive(Debug, Clone, PartialEq)]
pub struct Unfolded {
    pub detection: Detection,
    /// The streams of [`Unfolded::channels`], in [`output_order`].
    pub speakers: Vec<StreamId>,
    /// Planar 24-bit samples, as many frames as the carrier.
    pub channels: Vec<Vec<i32>>,
    /// Blocks that validated but did not decode; their samples play the carrier as authored
    /// (bed) and silence (heights).
    pub decode_errors: u64,
}

/// Detects and decodes a whole carrier (planar, WAV order, all channels the same length).
/// `None` when it carries no Auro-Codec side channel, or one whose layout the tables do not
/// know.
pub fn unfold(carrier: &[&[i32]]) -> Option<Unfolded> {
    let detection = detect(carrier, DETECT_FRAMES)?;
    let (speakers, channels, decode_errors) = unfold_with(&detection, carrier)?;
    Some(Unfolded { detection, speakers, channels, decode_errors })
}

/// Decodes a carrier whose layout is already known, for instance a window of a longer carrier
/// that [`detect`] latched on before: (the streams in [`output_order`], their planar samples,
/// blocks that did not decode). Every block that lies wholly inside the window is decoded; a
/// block cut by either edge plays the carrier as authored (bed) and silence (heights), so a caller
/// that wants a range exact reads [`crate::block::MAX_BLOCK`] samples more on both sides and crops.
/// `None` when the carrier's channel count or the layout is unknown.
pub fn unfold_with(detection: &Detection, carrier: &[&[i32]]) -> Option<(Vec<StreamId>, Vec<Vec<i32>>, u64)> {
    let carrier_ids = wav_carrier_ids(carrier.len())?;
    let speakers = output_order(detection.original)?;
    if speakers.is_empty() {
        return None;
    }
    let width = speakers.len();
    let frames = carrier.iter().map(|c| c.len()).min().unwrap_or(0);
    let mut u = Unfolder::new(carrier_ids, &speakers);
    let mut channels: Vec<Vec<i32>> = vec![Vec::with_capacity(frames); width];
    let mut out = vec![0i32; width.saturating_mul(MAX_PUSH)];
    let mut drain = |u: &mut Unfolder, channels: &mut Vec<Vec<i32>>| {
        loop {
            let n = u.take(&speakers, &mut out);
            if n == 0 {
                break;
            }
            for frame in out.chunks_exact(width).take(n) {
                for (c, &v) in channels.iter_mut().zip(frame) {
                    c.push(v);
                }
            }
        }
    };
    let mut start = 0;
    while start < frames {
        let end = start.saturating_add(MAX_PUSH).min(frames);
        for (ch, samples) in carrier.iter().enumerate() {
            u.push(ch, samples.get(start..end).unwrap_or_default());
        }
        drain(&mut u, &mut channels);
        start = end;
    }
    u.finish();
    drain(&mut u, &mut channels);
    let errors = u.decode_errors();
    Some((speakers, channels, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    /// A synthetic 5.1 carrier announcing configuration 30 (9.1 = 5.1 + 4 heights), each channel
    /// folding only itself.
    fn carrier_51(blocks: usize) -> Vec<Vec<i32>> {
        let ids = wav_carrier_ids(6).unwrap();
        ids.iter().enumerate().map(|(c, id)| synth::channel(c as u32 + 1, blocks, 0.25, 30, id.0)).collect()
    }

    #[test]
    fn a_synthetic_carrier_unfolds_to_its_layout() {
        let planes = carrier_51(12);
        let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
        let u = unfold(&refs).unwrap();
        assert_eq!(u.detection.original.name(), Some("5.1_4H"));
        assert_eq!(u.decode_errors, 0);
        let names: Vec<&str> = u.speakers.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["L", "R", "C", "LFE", "Ls", "Rs", "HL", "HR", "HLs", "HRs"]);
        let frames = planes[0].len();
        assert!(u.channels.iter().all(|c| c.len() == frames));
        // The bed is the carrier with its four borrowed bits cleared, the heights are silent.
        for (c, plane) in planes.iter().enumerate() {
            assert!(plane.iter().zip(&u.channels[c]).all(|(&p, &o)| o == p & !0xF), "channel {c}");
        }
        assert!(u.channels[6..].iter().all(|c| c.iter().all(|&v| v == 0)));
    }

    #[test]
    fn plain_pcm_and_odd_shapes_give_none() {
        let noise: Vec<i32> = (0..20_000).map(|i| ((i * 7919) % 65_536) - 32_768).collect();
        assert!(unfold(&[&noise, &noise]).is_none());
        assert!(unfold(&[]).is_none());
        let nine: Vec<&[i32]> = (0..9).map(|_| noise.as_slice()).collect();
        assert!(unfold(&nine).is_none());
        // a carrier with a channel count no layout uses
        let planes = carrier_51(4);
        let refs: Vec<&[i32]> = planes.iter().take(3).map(Vec::as_slice).collect();
        assert!(unfold(&refs).is_none());
        // channels of different lengths decode the common part
        let mut planes = carrier_51(6);
        planes[2].truncate(3000);
        let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
        assert!(unfold(&refs).is_some_and(|u| u.channels.iter().all(|c| c.len() == 3000)));
    }

    #[test]
    fn f32_round_trip_is_exact_and_lossy_samples_are_refused() {
        let v = [0, 1, -1, 8_388_607, -8_388_608, 12_345];
        let f = f32_from_pcm24(&v);
        assert_eq!(pcm24_from_f32(&f).as_deref(), Some(&v[..]));
        assert!(pcm24_from_f32(&[0.1]).is_none());
        assert!(pcm24_from_f32(&[f32::NAN]).is_none());
        assert!(pcm24_from_f32(&[1.0]).is_none());
    }

    /// Mutation fuzz (AGENTS.md §0.8): damaged carriers never panic.
    #[test]
    fn damaged_carriers_never_panic() {
        let clean = carrier_51(8);
        let mut rng = 0x2545_f491_u32;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        for round in 0..300 {
            let mut planes = clean.clone();
            for _ in 0..(1 + next() % 64) {
                let c = next() as usize % planes.len();
                let len = planes[c].len();
                if len == 0 {
                    continue;
                }
                let i = next() as usize % len;
                match next() % 4 {
                    0 => planes[c][i] ^= 1 << (next() % 24),
                    1 => planes[c][i] = next() as i32,
                    2 => planes[c][i] |= 0xF,
                    _ => planes[c].truncate(i),
                }
            }
            let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
            let r = std::panic::catch_unwind(|| {
                let _ = unfold(&refs);
                if let Some(d) = detect(&carrier_51(4).iter().map(Vec::as_slice).collect::<Vec<_>>(), DETECT_FRAMES) {
                    let _ = unfold_with(&d, &refs);
                }
            });
            assert!(r.is_ok(), "round {round} panicked");
        }
    }

    /// Mutation fuzz of the stream layer, past the CRC: random payloads whose header passes, so
    /// the two- and three-stream folds, the codebook and the Rice reader see hostile input.
    #[test]
    fn garbage_payloads_never_panic_the_block_decoder() {
        use crate::block::{MAX_BLOCK, SyncHeader, bit_location};
        use crate::decode::decode_block;
        use crate::rice::Codebook;
        use crate::stream::Payload;
        use crate::unmix::GainTable;
        let mut rng = 0x9e37_79b9_u32;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        let mut payload = Box::new(Payload::new());
        let mut codebook = Box::new(Codebook::new());
        let mut residuals = vec![0i32; 2 * MAX_BLOCK];
        let mut outputs: [Box<[i32]>; 3] = std::array::from_fn(|_| vec![0; MAX_BLOCK].into_boxed_slice());
        let gains = GainTable::new();
        let mut decoded = 0;
        for _ in 0..3000 {
            let m = 3 + (next() % 12) as u8;
            let block_size = 17 + (next() as usize % (MAX_BLOCK - 16));
            // Half the blocks are garbage throughout; the other half have clear low bits (an
            // all-zero codebook and Rice stream, which decode) under random fields.
            let garbage = next() % 2 == 0;
            let low = !((1i32 << m) - 1);
            let mut samples: Vec<i32> = (0..block_size).map(|_| if garbage { next() as i32 >> 8 } else { (next() as i32 >> 8) & low }).collect();
            let mut pos = 0usize;
            let mut put = |samples: &mut [i32], bits: u32, value: u32| {
                for i in (0..bits).rev() {
                    let (idx, b) = bit_location(48 + pos, usize::from(m), true);
                    if let Some(s) = samples.get_mut(idx) {
                        *s = (*s & !(1 << b)) | ((((value >> i) & 1) as i32) << b);
                    }
                    pos += 1;
                }
            };
            // a stream-layer header that passes: 0x0100 + 0..0xA
            put(&mut samples, 16, 0x0100 | (next() % 0xB));
            // the fields word: adaptive, Rice parameter, codebook count code, no ADOL, width
            let w = (next() & 0x4000_0000) | (next() % 8) << 24 | (next() % 0x54) << 16 | (1 + next() % 31);
            put(&mut samples, 32, w);
            let ids = match next() % 3 {
                0 => 0x09FF_FFFF,
                1 => 0x0009_FFFF,
                _ => 0x0009_0CFF,
            };
            put(&mut samples, 32, ids);
            put(&mut samples, 32, 0);
            for _ in 0..5 {
                put(&mut samples, 32, next());
            }
            let header = SyncHeader { block_size: block_size as u16, lsb_bits: m };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                decode_block(&samples, header, &mut payload, &mut codebook, &mut residuals, &mut outputs, &gains).is_ok()
            }));
            match r {
                Ok(ok) => decoded += usize::from(ok),
                Err(_) => panic!("block of {block_size} samples, m {m} panicked"),
            }
            // a truncated block is an error, not a panic
            assert!(decode_block(&samples[..block_size / 2], header, &mut payload, &mut codebook, &mut residuals, &mut outputs, &gains).is_err());
        }
        assert!(decoded > 0, "no garbage block got past the stream layer");
    }
}
