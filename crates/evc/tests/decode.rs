// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoder tests on synthetic EVC Baseline streams (see `synth`).

mod synth;

use cfa4_evc::{Chroma, Decoder, Frame, split_nal_units};
use synth::{Content, PicCfg, Rng, SLICE_B, SLICE_I, SLICE_P, SeqCfg};

fn pic(slice_type: u32, idr: bool, tid: u32, num_refp: [u32; 2], content: Content) -> PicCfg {
    PicCfg { slice_type, idr, tid, num_refp, qp: 30, qp_u_offset: 0, qp_v_offset: 0, deblock: true, content }
}

fn build(seq: &SeqCfg, pics: &[PicCfg], seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut nals = vec![synth::sps(seq), synth::pps(seq)];
    for p in pics {
        nals.push(synth::slice(seq, p, &mut rng));
    }
    synth::stream(&nals)
}

fn decode(data: &[u8]) -> Result<Vec<Frame>, cfa4_evc::Error> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for nal in split_nal_units(data) {
        if let Some(f) = d.decode_nal(nal)? {
            out.push(f);
        }
    }
    out.extend(d.flush());
    Ok(out)
}

/// Decodes and ignores errors: hostile input must never panic.
fn decode_lossy(data: &[u8]) {
    let mut d = Decoder::new();
    for nal in split_nal_units(data) {
        let _ = d.decode_nal(nal);
    }
    let _ = d.flush();
}

/// IDR followed by low-delay P pictures (every picture a reference).
fn low_delay(seq: &SeqCfg, n: u32) -> Vec<PicCfg> {
    let mut v = vec![pic(SLICE_I, true, 0, [0, 0], Content::Random)];
    for k in 1..n {
        v.push(pic(SLICE_P, false, 0, [k.min(seq.max_num_ref_pics), 0], Content::Random));
    }
    v
}

/// IDR followed by sub-GOPs of two B pictures (temporal ids 0 and 1).
fn random_access(groups: u32) -> Vec<PicCfg> {
    let mut v = vec![pic(SLICE_I, true, 0, [0, 0], Content::Random)];
    for _ in 0..groups {
        v.push(pic(SLICE_B, false, 0, [1, 1], Content::Random));
        v.push(pic(SLICE_B, false, 1, [1, 1], Content::Random));
    }
    v
}

#[test]
fn flat_intra_picture_is_mid_grey() {
    for (bd, chroma) in [(8, true), (10, true), (12, false)] {
        let seq = SeqCfg { width: 136, height: 72, bit_depth: bd, chroma, ..SeqCfg::default() };
        let data = build(&seq, &[pic(SLICE_I, true, 0, [0, 0], Content::FlatIntra)], 1);
        let frames = decode(&data).expect("decodes");
        assert_eq!(frames.len(), 1);
        let f = &frames[0];
        assert_eq!((f.width, f.height, f.bit_depth), (136, 72, bd));
        assert_eq!(f.chroma, if chroma { Chroma::Yuv420 } else { Chroma::Mono });
        assert_eq!(f.planes.len(), if chroma { 3 } else { 1 });
        for (i, p) in f.planes.iter().enumerate() {
            let (w, h) = f.plane_size(i);
            assert_eq!(p.len(), w * h);
            assert!(p.iter().all(|&v| v == 1 << (bd - 1)), "plane {i} at {bd} bits is not flat");
        }
    }
}

#[test]
fn collocated_skip_copies_the_reference() {
    for deblock in [false, true] {
        let seq = SeqCfg { width: 104, height: 88, ..SeqCfg::default() };
        let mut p = pic(SLICE_P, false, 0, [1, 0], Content::SkipCollocated);
        p.deblock = deblock;
        let data = build(&seq, &[pic(SLICE_I, true, 0, [0, 0], Content::Random), p], 7);
        let frames = decode(&data).expect("decodes");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].poc, 0);
        assert_eq!(frames[1].poc, 1);
        assert_eq!(frames[0].planes, frames[1].planes, "deblock {deblock}");
        // The random intra picture is not trivially flat.
        assert!(frames[0].planes[0].iter().any(|&v| v != frames[0].planes[0][0]));
    }
}

#[test]
fn random_low_delay_streams_decode() {
    let cases = [
        SeqCfg { width: 64, height: 64, max_num_ref_pics: 2, ..SeqCfg::default() },
        SeqCfg { width: 200, height: 120, max_num_ref_pics: 4, cu_qp_delta: true, ..SeqCfg::default() },
        SeqCfg { width: 72, height: 48, chroma: false, bit_depth: 10, max_num_ref_pics: 3, cip: true, ..SeqCfg::default() },
        SeqCfg { width: 96, height: 80, bit_depth: 12, max_num_ref_pics: 2, crop: Some([2, 1, 0, 3]), ..SeqCfg::default() },
    ];
    for (n, seq) in cases.iter().enumerate() {
        for seed in 0..4u64 {
            let pics = low_delay(seq, 8);
            let data = build(seq, &pics, seed * 31 + n as u64);
            let frames = decode(&data).unwrap_or_else(|e| panic!("case {n} seed {seed}: {e}"));
            assert_eq!(frames.len(), 8, "case {n} seed {seed}");
            let pocs: Vec<i32> = frames.iter().map(|f| f.poc).collect();
            assert_eq!(pocs, (0..8).collect::<Vec<_>>());
            let crop = seq.crop.unwrap_or([0; 4]);
            let cs = if seq.chroma { 2 } else { 1 };
            assert_eq!(frames[0].width, seq.width - cs * (crop[0] + crop[1]));
            assert_eq!(frames[0].height, seq.height - cs * (crop[2] + crop[3]));
            let max = (1u16 << seq.bit_depth) - 1;
            for f in &frames {
                for p in &f.planes {
                    assert!(p.iter().all(|&v| v <= max));
                }
            }
            // Decoding is deterministic.
            assert_eq!(decode(&data).ok(), Some(frames.clone()));
        }
    }
}

#[test]
fn random_access_streams_decode_in_output_order() {
    for seed in 0..6u64 {
        let seq = SeqCfg { width: 128, height: 96, log2_sub_gop_length: 1, max_num_ref_pics: 1, cu_qp_delta: seed % 2 == 1, ..SeqCfg::default() };
        let data = build(&seq, &random_access(4), seed);
        let frames = decode(&data).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let pocs: Vec<i32> = frames.iter().map(|f| f.poc).collect();
        assert_eq!(pocs, (0..9).collect::<Vec<_>>(), "seed {seed}");
    }
}

#[test]
fn stream_info_and_probe() {
    let seq = SeqCfg { width: 176, height: 144, bit_depth: 10, ..SeqCfg::default() };
    let data = build(&seq, &[pic(SLICE_I, true, 0, [0, 0], Content::Random)], 3);
    let info = cfa4_evc::probe(&data).expect("probe finds the SPS");
    assert_eq!((info.width, info.height, info.bit_depth), (176, 144, 10));
    assert_eq!(info.chroma, Chroma::Yuv420);
    assert_eq!(info.profile_idc, 0);
}

#[test]
fn inter_picture_without_references_is_an_error() {
    let seq = SeqCfg::default();
    let data = build(&seq, &[pic(SLICE_P, false, 0, [1, 0], Content::Random)], 5);
    assert!(decode(&data).is_err());
}

#[test]
fn damaged_streams_do_not_panic() {
    let seqs = [
        SeqCfg { width: 96, height: 72, max_num_ref_pics: 2, cu_qp_delta: true, ..SeqCfg::default() },
        SeqCfg { width: 64, height: 64, log2_sub_gop_length: 1, max_num_ref_pics: 1, ..SeqCfg::default() },
    ];
    for (n, seq) in seqs.iter().enumerate() {
        let pics = if seq.log2_sub_gop_length == 0 { low_delay(seq, 5) } else { random_access(2) };
        let data = build(seq, &pics, 11 + n as u64);
        decode(&data).expect("the undamaged stream decodes");
        let mut rng = Rng::new(99 + n as u64);
        // Truncations.
        for cut in (0..data.len()).step_by(7) {
            decode_lossy(&data[..cut]);
        }
        // Bit flips and byte overwrites (length prefixes included).
        for _ in 0..400 {
            let mut d = data.clone();
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(d.len() as u32) as usize;
                if rng.pct(50) {
                    d[i] ^= 1 << rng.below(8);
                } else {
                    d[i] = rng.next() as u8;
                }
            }
            decode_lossy(&d);
        }
    }
    // Random bytes behind valid parameter sets.
    let seq = SeqCfg::default();
    let mut rng = Rng::new(1234);
    for _ in 0..200 {
        let len = 2 + rng.below(300) as usize;
        let mut slice = vec![0u8; len];
        for b in &mut slice {
            *b = rng.next() as u8;
        }
        slice[0] = 0x04; // IDR NAL header
        slice[1] = 0x00;
        decode_lossy(&synth::stream(&[synth::sps(&seq), synth::pps(&seq), slice]));
    }
}
