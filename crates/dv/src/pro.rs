// SPDX-License-Identifier: LGPL-2.1-or-later
// Ported from FFmpeg n7.1.1 (https://ffmpeg.org/, tag n7.1.1):
//   libavcodec/dv.c          dv_calc_mb_coordinates(), ff_dv_init_dynamic_tables()
//   libavcodec/dv_internal.h dv_work_pool_size(), dv_calculate_mb_xy()
//   libavcodec/dvdec.c       DV100 weights and quantiser steps (dv_init_weight_tables()) and the
//                            DVCPRO 50 / HD block placement of dv_decode_video_segment()
// DV decoder Copyright (c) 2002 Fabrice Bellard, Copyright (c) 2004 Roman Shaposhnik;
// 50 Mbps (DVCPRO50) support Copyright (c) 2006 Daniel Maas <dmaas@maasdigital.com>;
// 100 Mbps (DVCPRO HD) support: initial code by Daniel Maas (funded by BBC R&D), final code by
// Roman Shaposhnik. FFmpeg is LGPL-2.1-or-later ("either version 2.1 of the License, or (at
// your option) any later version"). Re-expressed in Rust for CFA4 as an owner-approved exception
// to the FFmpeg-only-in-tests rule (2026-11-11); see crates/dv/README.md and docs/licensing.md §5.

//! DVCPRO 50 (SMPTE 314M, 4:2:2 in two channels) and DVCPRO HD (SMPTE 370M, 1080i in four
//! channels, 720p in two): macroblock placement, the DV100 quantiser and the frame decoder.
//!
//! The bit stream of a video segment is the same as at 25 Mbit/s (`segment.rs`); DVCPRO HD has
//! eight blocks per macroblock (Y0-Y3, Cr0, Cr1, Cb0, Cb1) in smaller areas, always uses the 8x8
//! DCT, and its DCT mode bit (of Y0) marks a field-coded macroblock whose blocks hold alternate
//! lines.

use crate::dif::{BLOCK_SIZE, Profile, SEQUENCE_SIZE};
use crate::quant::SCAN_88;
use crate::segment::{self, AREAS_HD, AREAS_SD};
use crate::{CONCEAL, Picture, idct, quant, vlc};

const OFF: [usize; 5] = [2, 6, 8, 0, 4];
const SHUF1: [usize; 5] = [36, 18, 54, 0, 72];
const SHUF2: [usize; 5] = [24, 12, 36, 0, 48];
const SHUF3: [usize; 5] = [18, 9, 27, 0, 36];
const L_START: [usize; 10] = [0, 4, 9, 13, 18, 22, 27, 31, 36, 40];
const SERPENT1: [usize; 27] = [0, 1, 2, 2, 1, 0, 0, 1, 2, 2, 1, 0, 0, 1, 2, 2, 1, 0, 0, 1, 2, 2, 1, 0, 0, 1, 2];
/// 1080i60 (1280 wide): where the macroblocks right of column 80 go, by their row.
const REMAP: [[usize; 2]; 64] = [
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0], // dummy
    [0, 0],
    [0, 1],
    [0, 2],
    [0, 3],
    [10, 0],
    [10, 1],
    [10, 2],
    [10, 3],
    [20, 0],
    [20, 1],
    [20, 2],
    [20, 3],
    [30, 0],
    [30, 1],
    [30, 2],
    [30, 3],
    [40, 0],
    [40, 1],
    [40, 2],
    [40, 3],
    [50, 0],
    [50, 1],
    [50, 2],
    [50, 3],
    [60, 0],
    [60, 1],
    [60, 2],
    [60, 3],
    [70, 0],
    [70, 1],
    [70, 2],
    [70, 3],
    [0, 64],
    [0, 65],
    [0, 66],
    [10, 64],
    [10, 65],
    [10, 66],
    [20, 64],
    [20, 65],
    [20, 66],
    [30, 64],
    [30, 65],
    [30, 66],
    [40, 64],
    [40, 65],
    [40, 66],
    [50, 64],
    [50, 65],
    [50, 66],
    [60, 64],
    [60, 65],
    [60, 66],
    [70, 64],
    [70, 65],
    [70, 66],
    [0, 67],
    [20, 67],
    [40, 67],
    [60, 67],
];

/// DV100 quantiser step by QNO (0 and 1 both mean no quantisation).
const DV100_QSTEP: [u32; 16] = [1, 1, 2, 3, 4, 5, 6, 7, 8, 16, 18, 20, 22, 24, 28, 52];

/// DV100 weights in zigzag order (the "inverse" weights are the spec weights).
const W1080_Y: [u32; 64] = [
    128, 16, 16, 17, 17, 17, 18, 18, 18, 18, 18, 18, 19, 18, 18, 19, 19, 19, 19, 19, 19, 42, 38, 40, 40, 40, 38, 42, 44, 43, 41, 41, 41, 41, 43, 44, 45, 45,
    42, 42, 42, 45, 45, 48, 46, 43, 43, 46, 48, 49, 48, 44, 48, 49, 101, 98, 98, 101, 104, 109, 104, 116, 116, 123,
];
const W1080_C: [u32; 64] = [
    128, 16, 16, 17, 17, 17, 25, 25, 25, 25, 26, 25, 26, 25, 26, 26, 26, 27, 27, 26, 26, 42, 38, 40, 40, 40, 38, 42, 44, 43, 41, 41, 41, 41, 43, 44, 91, 91,
    84, 84, 84, 91, 91, 96, 93, 86, 86, 93, 96, 197, 191, 177, 191, 197, 203, 197, 197, 203, 209, 219, 209, 232, 232, 246,
];
const W720_Y: [u32; 64] = [
    128, 16, 16, 17, 17, 17, 18, 18, 18, 18, 18, 18, 19, 18, 18, 19, 19, 19, 19, 19, 19, 42, 38, 40, 40, 40, 38, 42, 44, 43, 41, 41, 41, 41, 43, 44, 68, 68,
    63, 63, 63, 68, 68, 96, 92, 86, 86, 92, 96, 98, 96, 88, 96, 98, 202, 196, 196, 202, 208, 218, 208, 232, 232, 246,
];
const W720_C: [u32; 64] = [
    128, 24, 24, 26, 26, 26, 36, 36, 36, 36, 36, 36, 38, 36, 36, 38, 38, 38, 38, 38, 38, 84, 76, 80, 80, 80, 76, 84, 88, 86, 82, 82, 82, 82, 86, 88, 182, 182,
    168, 168, 168, 182, 182, 192, 186, 192, 172, 186, 192, 394, 382, 354, 382, 394, 406, 394, 394, 406, 418, 438, 418, 464, 464, 492,
];

/// Last macroblock row of 1080 lines (8-line units): its macroblocks are 32x8.
const LAST_ROW_1080: usize = 134;

/// Dequantise a DVCPRO HD block: `(level * (qstep << (class + 9)) * weight + 2^13) >> 14`, kept
/// as FFmpeg computes it (wrapping to 16 bits), into natural order.
fn dequantise_hd(dc: i32, amps: &[i16; 64], class: u8, qno: u8, chroma: bool, is720: bool) -> [f32; 64] {
    let w = match (is720, chroma) {
        (false, false) => &W1080_Y,
        (false, true) => &W1080_C,
        (true, false) => &W720_Y,
        (true, true) => &W720_C,
    };
    let step = i64::from(DV100_QSTEP[usize::from(qno & 15)]) << (u32::from(class & 3) + 9);
    let mut out = [0f32; 64];
    out[0] = (dc * 4) as f32;
    for (zz, &a) in amps.iter().enumerate().skip(1) {
        if a != 0 {
            let f = step * i64::from(w[zz]);
            let v = (i64::from(a) * f + (1 << 13)) >> 14;
            out[usize::from(SCAN_88[zz] & 63)] = f32::from(v as i16);
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// DVCPRO 50: 16x8 macroblocks (Y0, Y2 used; Cr, Cb 8x8).
    Sd422,
    /// 1080i50, 1440 wide.
    Hd1440,
    /// 1080i60, 1280 wide.
    Hd1280,
    /// 720p, 960 wide.
    Hd960,
}

fn kind(p: &Profile) -> Option<Kind> {
    match (p.width, p.stype) {
        (720, 4) => Some(Kind::Sd422),
        (1440, _) => Some(Kind::Hd1440),
        (1280, _) => Some(Kind::Hd1280),
        (960, _) => Some(Kind::Hd960),
        _ => None,
    }
}

/// Video segments that carry no picture: channels 1-3 of DIF sequence 11 in 1080i50 and DIF
/// sequences 10-11 in 720p50.
fn skipped(p: &Profile, chan: usize, seq: usize) -> bool {
    (p.width == 1440 && chan != 0 && seq == 11) || (p.is_720p() && seq > 9)
}

/// Position (in units of 8 pixels / 8 lines) of macroblock `m` of video segment `slot` of DIF
/// sequence `seq` of channel `chan`.
fn coords(kind: Kind, sequences: usize, chan: usize, seq: usize, slot: usize, m: usize) -> (usize, usize) {
    let m = m % 5;
    let (x, y) = match kind {
        Kind::Hd1440 => {
            let blk = (chan * 11 + seq) * 27 + slot;
            if chan == 0 && seq == 11 {
                let x = m * 27 + slot;
                if x < 90 { (x, 0) } else { ((x - 90) * 2, 67) }
            } else {
                let i = (4 * chan + blk + OFF[m]) % 11;
                let k = (blk / 11) % 27;
                (SHUF1[m] + (chan & 1) * 9 + k % 9, (i * 3 + k / 9) * 2 + (chan >> 1) + 1)
            }
        }
        Kind::Hd1280 => {
            let blk = (chan * 10 + seq) * 27 + slot;
            let i = (4 * chan + seq / 5 + 2 * blk + OFF[m]) % 10;
            let k = (blk / 5) % 27;
            let x = SHUF1[m] + (chan & 1) * 9 + k % 9;
            let y = (i * 3 + k / 9) * 2 + (chan >> 1) + 4;
            if x >= 80 {
                let r = REMAP.get(y).copied().unwrap_or([0, 0]);
                (r[0] + ((x - 80) << usize::from(y > 59)), r[1])
            } else {
                (x, y)
            }
        }
        Kind::Hd960 => {
            let blk = (chan * 10 + seq) * 27 + slot;
            let i = (4 * chan + seq / 5 + 2 * blk + OFF[m]) % 10;
            let k = (blk / 5) % 27 + (i & 1) * 3;
            (SHUF2[m] + k % 6 + 6 * (chan & 1), L_START[i] + k / 6 + 45 * (chan >> 1))
        }
        Kind::Sd422 => {
            let x = SHUF3[m] + slot / 3;
            let y = SERPENT1.get(slot).copied().unwrap_or(0) + ((((seq + OFF[m]) % sequences.max(1)) << 1) + chan) * 3;
            // 16x8 macroblocks: y is already in 8-line units
            return (x * 2, y);
        }
    };
    (x * 2, y * 2)
}

/// 720p: a 100 Mbit/s DIF frame carries two pictures; the first half (channels 0-1, signalled
/// in the first DIF block's ID) sits 72 macroblock rows lower in FFmpeg's numbering.
fn adjust_720(frame: &[u8], mb_y: usize) -> usize {
    if frame.get(1).is_some_and(|b| b & 0x0c == 0) { if mb_y > 17 { mb_y - 18 } else { mb_y + 72 } } else { mb_y }
}

/// One decoded macroblock: its position, field coding and pixel blocks.
struct Mb {
    mb_x: usize,
    mb_y: usize,
    field: bool,
    blocks: Vec<[u8; 64]>,
}

fn decode_segment(frame: &[u8], p: &Profile, kind: Kind, chan: usize, seq: usize, slot: usize) -> (Vec<Mb>, u32, u32) {
    let hd = kind != Kind::Sd422;
    let g = chan * p.sequences + seq;
    let first = g * SEQUENCE_SIZE + (6 + (slot / 3) * 16 + 1 + (slot % 3) * 5) * BLOCK_SIZE;
    let data = frame.get(first..first + 5 * BLOCK_SIZE).unwrap_or(&[]);
    let seg = segment::decode(data, seq, vlc::table(), if hd { AREAS_HD } else { AREAS_SD });
    let mut concealed = 0;
    let mut out = Vec::with_capacity(5);
    for (m, mb) in seg.mbs.iter().enumerate() {
        let (mb_x, mut mb_y) = coords(kind, p.sequences, chan, seq, slot, m);
        if kind == Kind::Hd960 {
            mb_y = adjust_720(frame, mb_y);
        }
        let n = if hd { 8 } else { 6 };
        if mb.concealed {
            concealed += 1;
            out.push(Mb { mb_x, mb_y, field: false, blocks: vec![[CONCEAL; 64]; n] });
            continue;
        }
        let field = hd && mb.blocks.first().is_some_and(|b| b.mode248);
        let blocks = (0..n)
            .map(|b| {
                let Some(blk) = mb.blocks.get(b) else { return [CONCEAL; 64] };
                if hd {
                    idct::idct_88(&dequantise_hd(blk.dc, &blk.amps, blk.class, mb.qno, b >= 4, kind == Kind::Hd960))
                } else {
                    let c = quant::dequantise(blk.dc, &blk.amps, blk.mode248, blk.class, mb.qno);
                    if blk.mode248 { idct::idct_248(&c) } else { idct::idct_88(&c) }
                }
            })
            .collect();
        out.push(Mb { mb_x, mb_y, field, blocks });
    }
    (out, seg.errors, concealed)
}

/// Copy `h` rows of `w` pixels from `src` (8 wide, from row `src_row`, column `src_x`) to
/// `plane` at (`x`, `y`), every `step` lines.
#[allow(clippy::too_many_arguments)]
fn put(plane: &mut [u8], stride: usize, x: usize, y: usize, step: usize, src: &[u8; 64], src_row: usize, src_x: usize, w: usize, h: usize) {
    for r in 0..h {
        let at = (y + r * step) * stride + x;
        let (Some(dst), Some(s)) = (plane.get_mut(at..at + w), src.get((src_row + r) * 8 + src_x..(src_row + r) * 8 + src_x + w)) else {
            continue;
        };
        dst.copy_from_slice(s);
    }
}

fn place(pic: &mut Picture, kind: Kind, mb: &Mb) {
    let (w, cw) = (pic.width as usize, (pic.width / 2) as usize);
    let (px, py, cx) = (mb.mb_x * 8, mb.mb_y * 8, mb.mb_x * 4);
    let b = |i: usize| mb.blocks.get(i).copied().unwrap_or([CONCEAL; 64]);
    if kind == Kind::Sd422 {
        put(&mut pic.y, w, px, py, 1, &b(0), 0, 0, 8, 8);
        put(&mut pic.y, w, px + 8, py, 1, &b(2), 0, 0, 8, 8);
        put(&mut pic.cr, cw, cx, py, 1, &b(4), 0, 0, 8, 8);
        put(&mut pic.cb, cw, cx, py, 1, &b(5), 0, 0, 8, 8);
        return;
    }
    if mb.mb_y == LAST_ROW_1080 {
        // 32x8 luma, 16x8 chroma
        if mb.field {
            for (i, (dx, dy)) in [(0, 0), (8, 0), (0, 1), (8, 1)].into_iter().enumerate() {
                put(&mut pic.y, w, px + dx, py + dy, 2, &b(i), 0, 0, 8, 4);
                put(&mut pic.y, w, px + dx + 16, py + dy, 2, &b(i), 4, 0, 8, 4);
            }
            for (plane, first) in [(&mut pic.cr, 4), (&mut pic.cb, 6)] {
                for (j, dy) in [0, 1].into_iter().enumerate() {
                    put(plane, cw, cx, py + dy, 2, &b(first + j), 0, 0, 8, 4);
                    put(plane, cw, cx + 8, py + dy, 2, &b(first + j), 4, 0, 8, 4);
                }
            }
        } else {
            for i in 0..4 {
                put(&mut pic.y, w, px + 8 * i, py, 1, &b(i), 0, 0, 8, 8);
            }
            for (plane, first) in [(&mut pic.cr, 4), (&mut pic.cb, 6)] {
                put(plane, cw, cx, py, 1, &b(first), 0, 0, 8, 8);
                put(plane, cw, cx + 8, py, 1, &b(first + 1), 0, 0, 8, 8);
            }
        }
        return;
    }
    // 16x16 luma, 8x16 chroma; a field macroblock interleaves its upper and lower blocks
    let (step, down) = if mb.field { (2, 1) } else { (1, 8) };
    for i in 0..4 {
        put(&mut pic.y, w, px + 8 * (i & 1), py + down * (i >> 1), step, &b(i), 0, 0, 8, 8);
    }
    for (plane, first) in [(&mut pic.cr, 4), (&mut pic.cb, 6)] {
        put(plane, cw, cx, py, step, &b(first), 0, 0, 8, 8);
        put(plane, cw, cx, py + down, step, &b(first + 1), 0, 0, 8, 8);
    }
}

/// Decode a DVCPRO 50 or DVCPRO HD frame (`frame` holds at least `p.frame_size()` bytes).
pub(crate) fn decode_frame(frame: &[u8], p: Profile, threads: bool) -> Picture {
    let (w, h) = (p.width as usize, p.height as usize);
    let cw = w / 2;
    let mut pic =
        Picture { profile: p, width: w as u32, height: h as u32, y: vec![0; w * h], cb: vec![0; cw * h], cr: vec![0; cw * h], concealed: 0, errors: 0 };
    let Some(kind) = kind(&p) else { return pic };
    let jobs: Vec<(usize, usize, usize)> = (0..p.channels)
        .flat_map(|c| (0..p.sequences).map(move |s| (c, s)))
        .filter(|&(c, s)| !skipped(&p, c, s))
        .flat_map(|(c, s)| (0..27).map(move |k| (c, s, k)))
        .collect();
    let run = |&(c, s, k): &(usize, usize, usize)| decode_segment(frame, &p, kind, c, s, k);
    #[cfg(feature = "threads")]
    let results: Vec<_> = if threads {
        use rayon::prelude::*;
        jobs.par_iter().map(run).collect()
    } else {
        jobs.iter().map(run).collect()
    };
    #[cfg(not(feature = "threads"))]
    let results: Vec<_> = {
        let _ = threads;
        jobs.iter().map(run).collect()
    };
    for (mbs, errors, concealed) in results {
        pic.errors = pic.errors.saturating_add(errors);
        pic.concealed = pic.concealed.saturating_add(concealed);
        for mb in &mbs {
            place(&mut pic, kind, mb);
        }
    }
    pic
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dif::{ChromaFormat, probe, tests_support::head};

    fn profile(dsf: u8, stype: u8) -> Profile {
        probe(&head(dsf, 1, stype, 0)).unwrap()
    }

    /// Every pixel of the picture is covered exactly once by the macroblocks of a frame.
    fn coverage(p: Profile, first_half: bool) {
        let kind = kind(&p).unwrap();
        let (w, h) = (p.width as usize, p.height as usize);
        let mut hit = vec![0u8; w * h];
        let head = [0u8, if first_half { 0 } else { 0x04 }];
        for c in 0..p.channels {
            for s in 0..p.sequences {
                if skipped(&p, c, s) {
                    continue;
                }
                for k in 0..27 {
                    for m in 0..5 {
                        let (mx, mut my) = coords(kind, p.sequences, c, s, k, m);
                        if kind == Kind::Hd960 {
                            my = adjust_720(&head, my);
                        }
                        let (mw, mh) = match kind {
                            Kind::Sd422 => (16, 8),
                            _ if my == LAST_ROW_1080 => (32, 8),
                            _ => (16, 16),
                        };
                        for yy in my * 8..my * 8 + mh {
                            for xx in mx * 8..mx * 8 + mw {
                                hit[yy * w + xx] += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(hit.iter().all(|&c| c == 1), "{}x{}", w, h);
    }

    #[test]
    fn covers_every_format() {
        coverage(profile(0, 4), true);
        coverage(profile(1, 4), true);
        coverage(profile(0, 0x14), true);
        coverage(profile(1, 0x14), true);
        coverage(profile(0, 0x18), true);
        coverage(profile(1, 0x18), true);
        assert_eq!(profile(1, 4).chroma, ChromaFormat::Yuv422);
    }

    #[test]
    fn hd_dequantiser() {
        let mut amps = [0i16; 64];
        amps[1] = 3;
        amps[63] = -2;
        let c = dequantise_hd(5, &amps, 1, 4, false, false);
        assert_eq!(c[0], 20.0);
        // 3 * (4 << 10) * 16 / 2^14 = 12; -2 * (4 << 10) * 123 / 2^14 = -61.5 -> -61
        assert_eq!((c[1], c[63]), (12.0, -61.0));
    }
}
