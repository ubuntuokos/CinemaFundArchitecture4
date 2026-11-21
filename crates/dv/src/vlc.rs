// SPDX-License-Identifier: LGPL-2.1-or-later
// The DV AC variable-length code table below is ported from libdv 1.0.0 (libdv/vlc.c,
// Copyright (C) Charles 'Buck' Krasic, Erik Walthinsen - April 2000; "libdv is free software;
// you can redistribute it and/or modify it under the terms of the GNU Lesser Public License as
// published by the Free Software Foundation; either version 2.1, or (at your option) any later
// version"). It is the run/amplitude table of IEC 61834-2 / SMPTE 314M. Re-expressed for CFA4
// as a code list and a 16-bit lookup table; see crates/dv/README.md and docs/licensing.md §5.

//! AC coefficient VLCs: `(run, amplitude)` pairs, a sign bit after every non-zero amplitude,
//! and the end-of-block code `0110`.

use std::sync::OnceLock;

/// One decoded code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Code {
    /// `run` zero coefficients, then one coefficient of value `amp` (which may be 0).
    Coef { run: u8, amp: i16 },
    /// End of block.
    Eob,
}

/// Lookup entry: `len == 0` means no valid code starts with these bits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub len: u8,
    pub code: Code,
}

const INVALID: Entry = Entry { len: 0, code: Code::Eob };

/// Codes without their sign bit: (bits, run, amplitude). A non-zero amplitude is followed by
/// one sign bit (1 = negative); amplitude 0 and EOB have none.
const CODES: &[(&str, u8, u8)] = &[
    ("00", 0, 1),
    ("010", 0, 2),
    ("0111", 1, 1),
    ("1000", 0, 3),
    ("1001", 0, 4),
    ("10100", 2, 1),
    ("10101", 1, 2),
    ("10110", 0, 5),
    ("10111", 0, 6),
    ("110000", 3, 1),
    ("110001", 4, 1),
    ("110010", 0, 7),
    ("110011", 0, 8),
    ("1101000", 5, 1),
    ("1101001", 6, 1),
    ("1101010", 2, 2),
    ("1101011", 1, 3),
    ("1101100", 1, 4),
    ("1101101", 0, 9),
    ("1101110", 0, 10),
    ("1101111", 0, 11),
    ("11100000", 7, 1),
    ("11100001", 8, 1),
    ("11100010", 9, 1),
    ("11100011", 10, 1),
    ("11100100", 3, 2),
    ("11100101", 4, 2),
    ("11100110", 2, 3),
    ("11100111", 1, 5),
    ("11101000", 1, 6),
    ("11101001", 1, 7),
    ("11101010", 0, 12),
    ("11101011", 0, 13),
    ("11101100", 0, 14),
    ("11101101", 0, 15),
    ("11101110", 0, 16),
    ("11101111", 0, 17),
    ("111100000", 11, 1),
    ("111100001", 12, 1),
    ("111100010", 13, 1),
    ("111100011", 14, 1),
    ("111100100", 5, 2),
    ("111100101", 6, 2),
    ("111100110", 3, 3),
    ("111100111", 4, 3),
    ("111101000", 2, 4),
    ("111101001", 2, 5),
    ("111101010", 1, 8),
    ("111101011", 0, 18),
    ("111101100", 0, 19),
    ("111101101", 0, 20),
    ("111101110", 0, 21),
    ("111101111", 0, 22),
    ("1111100000", 5, 3),
    ("1111100001", 3, 4),
    ("1111100010", 3, 5),
    ("1111100011", 2, 6),
    ("1111100100", 1, 9),
    ("1111100101", 1, 10),
    ("1111100110", 1, 11),
    ("11111001110", 0, 0),
    ("11111001111", 1, 0),
    ("11111010000", 6, 3),
    ("11111010001", 4, 4),
    ("11111010010", 3, 6),
    ("11111010011", 1, 12),
    ("11111010100", 1, 13),
    ("11111010101", 1, 14),
    ("111110101100", 2, 0),
    ("111110101101", 3, 0),
    ("111110101110", 4, 0),
    ("111110101111", 5, 0),
    ("111110110000", 7, 2),
    ("111110110001", 8, 2),
    ("111110110010", 9, 2),
    ("111110110011", 10, 2),
    ("111110110100", 7, 3),
    ("111110110101", 8, 3),
    ("111110110110", 4, 5),
    ("111110110111", 3, 7),
    ("111110111000", 2, 7),
    ("111110111001", 2, 8),
    ("111110111010", 2, 9),
    ("111110111011", 2, 10),
    ("111110111100", 2, 11),
    ("111110111101", 1, 15),
    ("111110111110", 1, 16),
    ("111110111111", 1, 17),
];

/// The end-of-block code.
const EOB: &str = "0110";

fn fill(table: &mut [Entry], bits: u32, len: u32, code: Code) {
    // every 16-bit window whose first `len` bits are `bits`
    let shift = 16 - len;
    let first = (bits << shift) as usize;
    let count = 1usize << shift;
    if let Some(slots) = table.get_mut(first..first + count) {
        for s in slots {
            *s = Entry { len: len as u8, code };
        }
    }
}

fn parse(bits: &str) -> u32 {
    bits.bytes().fold(0, |acc, b| (acc << 1) | u32::from(b == b'1'))
}

fn build() -> Vec<Entry> {
    let mut t = vec![INVALID; 1 << 16];
    fill(&mut t, parse(EOB), EOB.len() as u32, Code::Eob);
    for &(bits, run, amp) in CODES {
        let (v, l) = (parse(bits), bits.len() as u32);
        if amp == 0 {
            fill(&mut t, v, l, Code::Coef { run, amp: 0 });
        } else {
            fill(&mut t, v << 1, l + 1, Code::Coef { run, amp: i16::from(amp) });
            fill(&mut t, (v << 1) | 1, l + 1, Code::Coef { run, amp: -i16::from(amp) });
        }
    }
    // 13 bits: 1111110 + 6-bit run (6..=61), amplitude 0
    for run in 6u32..=61 {
        fill(&mut t, (0b111_1110 << 6) | run, 13, Code::Coef { run: run as u8, amp: 0 });
    }
    // 15 bits + sign: 1111111 + 8-bit amplitude (23..=255), run 0
    for amp in 23u32..=255 {
        let v = (0b111_1111 << 8) | amp;
        fill(&mut t, v << 1, 16, Code::Coef { run: 0, amp: amp as i16 });
        fill(&mut t, (v << 1) | 1, 16, Code::Coef { run: 0, amp: -(amp as i16) });
    }
    t
}

/// The 16-bit lookup table (built once).
pub(crate) fn table() -> &'static [Entry] {
    static T: OnceLock<Vec<Entry>> = OnceLock::new();
    T.get_or_init(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_free_and_complete_enough() {
        let t = table();
        // EOB
        let e = t[0b0110 << 12];
        assert_eq!((e.len, e.code), (4, Code::Eob));
        // 00s
        assert_eq!(t[0].code, Code::Coef { run: 0, amp: 1 });
        assert_eq!(t[0b001 << 13].code, Code::Coef { run: 0, amp: -1 });
        // 13-bit run escape
        let e = t[((0b111_1110 << 6) | 6) << 3];
        assert_eq!((e.len, e.code), (13, Code::Coef { run: 6, amp: 0 }));
        // 15-bit amplitude escape, negative
        let e = t[(0b111_1111_1111_1111) << 1 | 1];
        assert_eq!((e.len, e.code), (16, Code::Coef { run: 0, amp: -255 }));
        // reserved: run escape with run < 6
        assert_eq!(t[((0b111_1110 << 6) | 1) << 3].len, 0);
        // no code was overwritten by a longer one: count filled slots per code
        let filled = t.iter().filter(|e| e.len > 0).count();
        let mut want = 1usize << 12; // EOB
        for &(b, _, a) in CODES {
            want += 1 << (16 - b.len());
            let _ = a;
        }
        want += 56 * (1 << 3) + 233 * 2;
        assert_eq!(filled, want);
    }
}
