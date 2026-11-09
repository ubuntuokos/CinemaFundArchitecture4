// SPDX-License-Identifier: LGPL-2.1-or-later
// The macroblock placement below is ported from libdv 1.0.0 (libdv/place.c, Copyright (C)
// Charles 'Buck' Krasic, Erik Walthinsen - April 2000; LGPL-2.1-or-later, "either version 2.1,
// or (at your option) any later version"). It is the shuffling of IEC 61834-2 / SMPTE 314M.
// Re-expressed for CFA4; see crates/dv/README.md and docs/licensing.md §5.

//! Where each compressed macroblock goes in the picture.
//!
//! The picture is cut into superblocks: 5 columns by (DIF sequences) rows, 27 macroblocks
//! each. Video segment `k` of DIF sequence `i` holds five macroblocks, macroblock `m` from the
//! superblock in row `(i + [2, 6, 8, 0, 4][m]) mod rows`, column `[2, 1, 3, 0, 4][m]`, at
//! position `k` of that superblock's zigzag through its macroblocks.

use crate::dif::ChromaFormat;

const SB_ROW: [usize; 5] = [2, 6, 8, 0, 4];
const SB_COL: [usize; 5] = [2, 1, 3, 0, 4];

/// Macroblock shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// 4:1:1, 32x8: four luma blocks side by side, one 8x8 block per chroma plane.
    Wide411,
    /// 4:1:1 at the right picture edge, 16x16: luma blocks 2x2, each chroma block holds a 4x16
    /// area (left half: upper 8 lines, right half: lower 8 lines).
    Right411,
    /// 4:2:0, 16x16: luma blocks 2x2, one 8x8 block per chroma plane.
    Square420,
}

/// Top-left luma pixel and shape of macroblock `m` (0-4) of video segment `k` (0-26) of DIF
/// sequence `seq`.
pub(crate) fn place(chroma: ChromaFormat, sequences: usize, seq: usize, k: usize, m: usize) -> (usize, usize, Shape) {
    let m = m % 5;
    let row = (seq + SB_ROW[m]) % sequences.max(1);
    let col = SB_COL[m];
    match chroma {
        ChromaFormat::Yuv420 | ChromaFormat::Yuv422 => {
            let (q, r) = (k / 3, k % 3);
            let mb_row = if q % 2 == 0 { r } else { 2 - r };
            let mb_col = q + col * 9;
            (mb_col * 16, (mb_row + row * 3) * 16, Shape::Square420)
        }
        ChromaFormat::Yuv411 => {
            const COL_OFFSET: [usize; 5] = [0, 4, 9, 13, 18];
            let n = if col % 2 == 1 { k + 3 } else { k };
            let (q, r) = (n / 6, n % 6);
            let mb_row = if q % 2 == 0 { r } else { 5 - r };
            let col_blocks = (q + COL_OFFSET[col]) * 4;
            if col_blocks < 22 * 4 {
                (col_blocks * 8, (mb_row + row * 6) * 8, Shape::Wide411)
            } else {
                (col_blocks * 8, (mb_row * 2 + row * 6) * 8, Shape::Right411)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every macroblock of a frame lands on its own spot and the frame is fully covered.
    fn coverage(chroma: ChromaFormat, sequences: usize, w: usize, h: usize) {
        let mut hit = vec![0u8; w * h];
        for seq in 0..sequences {
            for k in 0..27 {
                for m in 0..5 {
                    let (x, y, s) = place(chroma, sequences, seq, k, m);
                    let (mw, mh) = match s {
                        Shape::Wide411 => (32, 8),
                        _ => (16, 16),
                    };
                    for yy in y..y + mh {
                        for xx in x..x + mw {
                            hit[yy * w + xx] += 1;
                        }
                    }
                }
            }
        }
        assert!(hit.iter().all(|&c| c == 1));
    }

    #[test]
    fn covers_525_411() {
        coverage(ChromaFormat::Yuv411, 10, 720, 480);
    }

    #[test]
    fn covers_625_411() {
        coverage(ChromaFormat::Yuv411, 12, 720, 576);
    }

    #[test]
    fn covers_625_420() {
        coverage(ChromaFormat::Yuv420, 12, 720, 576);
    }
}
