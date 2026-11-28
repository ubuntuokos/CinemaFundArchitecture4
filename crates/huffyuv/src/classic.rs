//! The built-in "classic" Huffman tables of Huffyuv 1.x (files without tables in the header).
//!
//! Ported from FFmpeg n9.0.2 `libavcodec/huffyuvdec.c` (LGPL-2.1-or-later, `COPYING.LGPL-2.1`; the
//! owner's general FFmpeg-port approval of 2026-11-14 and the decision of 2026-11-23): the code
//! lengths are stored run-length coded (`SHIFT_*`), the codes themselves are listed (`ADD_*`)
//! because they do not follow the code assignment of the 2.x tables.

use crate::error::Result;
use crate::huffman::Table;
use crate::read_lengths;

pub(crate) const SHIFT_LUMA: [u8; 42] = [
    34, 36, 35, 69, 135, 232, 9, 16, 10, 24, 11, 23, 12, 16, 13, 10, 14, 8, 15, 8, 16, 8, 17, 20, 16, 10, 207, 206, 205, 236, 11, 8, 10, 21, 9, 23, 8, 8, 199,
    70, 69, 68,
];

pub(crate) const SHIFT_CHROMA: [u8; 59] = [
    66, 36, 37, 38, 39, 40, 41, 75, 76, 77, 110, 239, 144, 81, 82, 83, 84, 85, 118, 183, 56, 57, 88, 89, 56, 89, 154, 57, 58, 57, 26, 141, 57, 56, 58, 57, 58,
    57, 184, 119, 214, 245, 116, 83, 82, 49, 80, 79, 78, 77, 44, 75, 41, 40, 39, 38, 37, 36, 34,
];

pub(crate) const ADD_LUMA: [u16; 256] = [
    3, 9, 5, 12, 10, 35, 32, 29, 27, 50, 48, 45, 44, 41, 39, 37, 73, 70, 68, 65, 64, 61, 58, 56, 53, 50, 49, 46, 44, 41, 38, 36, 68, 65, 63, 61, 58, 55, 53,
    51, 48, 46, 45, 43, 41, 39, 38, 36, 35, 33, 32, 30, 29, 27, 26, 25, 48, 47, 46, 44, 43, 41, 40, 39, 37, 36, 35, 34, 32, 31, 30, 28, 27, 26, 24, 23, 22, 20,
    19, 37, 35, 34, 33, 31, 30, 29, 27, 26, 24, 23, 21, 20, 18, 17, 15, 29, 27, 26, 24, 22, 21, 19, 17, 16, 14, 26, 25, 23, 21, 19, 18, 16, 15, 27, 25, 23, 21,
    19, 17, 16, 14, 26, 25, 23, 21, 18, 17, 14, 12, 17, 19, 13, 4, 9, 2, 11, 1, 7, 8, 0, 16, 3, 14, 6, 12, 10, 5, 15, 18, 11, 10, 13, 15, 16, 19, 20, 22, 24,
    27, 15, 18, 20, 22, 24, 26, 14, 17, 20, 22, 24, 27, 15, 18, 20, 23, 25, 28, 16, 19, 22, 25, 28, 32, 36, 21, 25, 29, 33, 38, 42, 45, 49, 28, 31, 34, 37, 40,
    42, 44, 47, 49, 50, 52, 54, 56, 57, 59, 60, 62, 64, 66, 67, 69, 35, 37, 39, 40, 42, 43, 45, 47, 48, 51, 52, 54, 55, 57, 59, 60, 62, 63, 66, 67, 69, 71, 72,
    38, 40, 42, 43, 46, 47, 49, 51, 26, 28, 30, 31, 33, 34, 18, 19, 11, 13, 7, 8,
];

pub(crate) const ADD_CHROMA: [u16; 256] = [
    3, 1, 2, 2, 2, 2, 3, 3, 7, 5, 7, 5, 8, 6, 11, 9, 7, 13, 11, 10, 9, 8, 7, 5, 9, 7, 6, 4, 7, 5, 8, 7, 11, 8, 13, 11, 19, 15, 22, 23, 20, 33, 32, 28, 27, 29,
    51, 77, 43, 45, 76, 81, 46, 82, 75, 55, 56, 144, 58, 80, 60, 74, 147, 63, 143, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
    84, 85, 86, 87, 88, 89, 90, 91, 27, 30, 21, 22, 17, 14, 5, 6, 100, 54, 47, 50, 51, 53, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 4, 117, 118, 92,
    94, 121, 122, 3, 124, 103, 2, 1, 0, 129, 130, 131, 120, 119, 126, 125, 136, 137, 138, 139, 140, 141, 142, 134, 135, 132, 133, 104, 64, 101, 62, 57, 102,
    95, 93, 59, 61, 28, 97, 96, 52, 49, 48, 29, 32, 25, 24, 46, 23, 98, 45, 44, 43, 20, 42, 41, 19, 18, 99, 40, 15, 39, 38, 16, 13, 12, 11, 37, 10, 9, 8, 36,
    7, 128, 127, 105, 123, 116, 35, 34, 33, 145, 31, 79, 42, 146, 78, 26, 83, 48, 49, 50, 44, 47, 26, 31, 30, 18, 17, 19, 21, 24, 25, 13, 14, 16, 17, 18, 20,
    21, 12, 14, 15, 9, 10, 6, 9, 6, 5, 8, 6, 12, 8, 10, 7, 9, 6, 4, 6, 2, 2, 3, 3, 3, 3, 2,
];

/// The three tables (Y, U, V or B, G, R) of a classic stream: luma and two chroma tables, or the
/// luma table for all three planes of an RGB stream.
pub(crate) fn tables(rgb: bool) -> Result<[Table; 3]> {
    let build = |shift: &[u8], add: &[u16; 256]| -> Result<Table> {
        let lens = read_lengths(shift, &mut 0)?;
        let mut codes = [0u32; 256];
        for (c, &a) in codes.iter_mut().zip(add) {
            *c = u32::from(a);
        }
        Table::with_codes(&lens, &codes)
    };
    let luma = build(&SHIFT_LUMA, &ADD_LUMA)?;
    if rgb {
        return Ok([luma.clone(), luma.clone(), luma]);
    }
    let chroma = build(&SHIFT_CHROMA, &ADD_CHROMA)?;
    Ok([luma, chroma.clone(), chroma])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::huffman::BitReader;

    /// Every symbol of a classic table decodes from its own code.
    #[test]
    fn every_code_decodes() {
        for (shift, add) in [(&SHIFT_LUMA[..], &ADD_LUMA), (&SHIFT_CHROMA[..], &ADD_CHROMA)] {
            let lens = read_lengths(shift, &mut 0).unwrap();
            let mut codes = [0u32; 256];
            for (c, &a) in codes.iter_mut().zip(add) {
                *c = u32::from(a);
            }
            let t = Table::with_codes(&lens, &codes).unwrap();
            for s in 0..256usize {
                let l = u32::from(lens[s]);
                if l == 0 {
                    continue;
                }
                // the code, left aligned in a 32-bit word, stored little endian
                let word = (codes[s] << (32 - l)).to_le_bytes();
                let data = [word[0], word[1], word[2], word[3], 0, 0, 0, 0];
                let mut br = BitReader::new(&data);
                assert_eq!(t.decode(&mut br), s as u8, "symbol {s}");
                assert!(!br.overrun());
            }
        }
    }
}
