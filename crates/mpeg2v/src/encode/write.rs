//! Bit writing and the encoder's view of the Annex B code tables (built from the decoder's
//! tables, so both always agree).

use std::sync::OnceLock;

use crate::vlc::*;

/// Big-endian bit writer.
#[derive(Clone, Default)]
pub(super) struct BitWriter {
    pub out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    /// The low `bits` (0..=32) bits of `v`.
    #[inline(always)]
    pub fn put(&mut self, v: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
        self.acc = (self.acc << bits) | (v & mask) as u64;
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    #[inline(always)]
    pub fn code(&mut self, c: Code) {
        self.put(c.0, c.1 as u32);
    }

    /// Pad with zero bits to a byte boundary.
    pub fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }

    /// A byte-aligned start code.
    pub fn start(&mut self, code: u8) {
        self.align();
        self.out.extend_from_slice(&[0, 0, 1, code]);
    }

    pub fn bits(&self) -> u64 {
        self.out.len() as u64 * 8 + self.n as u64
    }
}

/// A code: value and length (0: no code).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct Code(pub u32, pub u8);

/// Largest level with a table code (B.14 / B.15 stop at 40).
pub(super) const MAX_TABLE_LEVEL: usize = 40;

pub(super) struct EncTables {
    /// macroblock_address_increment 1..=33 and the escape (index 34).
    pub mba: [Code; 35],
    /// macroblock_type by flag set (B.2, B.3).
    pub mbtype_i: [Code; 32],
    pub mbtype_p: [Code; 32],
    pub mbtype_b: [Code; 32],
    /// coded_block_pattern (B.9) by value.
    pub cbp: [Code; 64],
    /// motion_code -16..=16 at index code + 16.
    pub motion: [Code; 33],
    pub dc_luma: [Code; 12],
    pub dc_chroma: [Code; 12],
    /// DCT coefficient codes without the sign bit: table zero / one, by run and level.
    pub dct: [Vec<[Code; MAX_TABLE_LEVEL + 1]>; 2],
}

fn code(s: &str) -> Code {
    let (v, n) = parse_code(s);
    Code(v, n as u8)
}

fn fill<const N: usize>(table: &[(&str, i16)], offset: i16) -> [Code; N] {
    let mut out = [Code::default(); N];
    for (s, v) in table {
        if let Some(e) = usize::try_from(v + offset).ok().and_then(|i| out.get_mut(i)) {
            *e = code(s);
        }
    }
    out
}

pub(super) fn tables() -> &'static EncTables {
    static T: OnceLock<EncTables> = OnceLock::new();
    T.get_or_init(|| {
        let dct = [dct_table(DCT_ZERO, DCT_ZERO_LONG), dct_table(DCT_ONE, &[])].map(|t| {
            let mut runs = vec![[Code::default(); MAX_TABLE_LEVEL + 1]; 32];
            for (s, v) in t {
                if v >= 0 {
                    let (run, level) = ((v & 63) as usize, (v >> 6) as usize);
                    if let Some(e) = runs.get_mut(run).and_then(|r| r.get_mut(level)) {
                        *e = code(s);
                    }
                }
            }
            runs
        });
        EncTables {
            mba: fill(MBA, 0),
            mbtype_i: fill(MBTYPE_I, 0),
            mbtype_p: fill(MBTYPE_P, 0),
            mbtype_b: fill(MBTYPE_B, 0),
            cbp: fill(CBP, 0),
            motion: fill(MOTION, 16),
            dc_luma: fill(DC_LUMA, 0),
            dc_chroma: fill(DC_CHROMA, 0),
            dct,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_bits_msb_first_and_aligns() {
        let mut w = BitWriter::default();
        w.put(0b101, 3);
        w.put(0xABCDE, 20);
        assert_eq!(w.bits(), 23);
        w.align();
        assert_eq!(w.out, [0b1011_0101, 0b0111_1001, 0b1011_1100]);
        w.start(0xB3);
        assert_eq!(&w.out[3..], [0, 0, 1, 0xB3]);
        w.put(u32::MAX, 32);
        assert_eq!(&w.out[7..], [0xFF; 4]);
    }

    #[test]
    fn tables_cover_every_value() {
        let t = tables();
        assert!(t.mba[1..].iter().all(|c| c.1 > 0));
        assert!(t.cbp.iter().all(|c| c.1 > 0));
        assert!(t.motion.iter().all(|c| c.1 > 0));
        assert_eq!(t.mbtype_p[(MB_FWD | MB_PATTERN) as usize], Code(1, 1));
        assert_eq!(t.dct[0][0][1], Code(0b11, 2));
        assert_eq!(t.dct[1][0][15], Code(0b1111_1111, 8));
        // run 0 has codes for levels 1..=40 in both tables, run 1 for 1..=18
        for tab in &t.dct {
            assert!((1..=40).all(|l| tab[0][l].1 > 0));
            assert!((1..=18).all(|l| tab[1][l].1 > 0));
            assert!((1..=31).all(|r| tab[r][1].1 > 0));
        }
    }
}
