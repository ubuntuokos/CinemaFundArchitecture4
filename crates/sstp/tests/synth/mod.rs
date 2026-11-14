//! Synthetic Simple Studio Profile streams for the oracle tests (also used by `cfa4-codecs`):
//! every macroblock is random bits that the strict decoder accepts.

#![allow(dead_code)]

use cfa4_sstp::Decoder;

pub struct W(pub Vec<bool>);

impl W {
    pub fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.0.push(v >> i & 1 != 0);
        }
    }

    pub fn align(&mut self) {
        while !self.0.len().is_multiple_of(8) {
            self.0.push(false);
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.0.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | u8::from(b) << (7 - i))).collect()
    }
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[derive(Clone, Copy)]
pub struct Cfg {
    pub rgb: bool,
    pub chroma_format: u32,
    pub width: u32,
    pub height: u32,
    pub alternate_scan: bool,
    pub dct_precision: u32,
    pub intra_dc_precision: u32,
    pub q_scale_type: bool,
    pub matrices: bool,
    pub mpeg_quant: bool,
    pub slice_every: usize,
}

/// VO, VOL and VOP headers, byte-aligned.
pub fn headers(c: &Cfg, rng: &mut Rng) -> W {
    let mut w = W(Vec::new());
    w.put(0x100, 32);
    w.put(0x120, 32);
    w.put(0, 1);
    w.put(15, 8); // core studio
    w.put(1, 4);
    w.put(0, 2);
    w.put(0, 4);
    w.put(1, 1);
    w.put(u32::from(c.rgb), 1);
    w.put(c.chroma_format, 2);
    w.put(10, 4);
    w.put(1, 1);
    w.put(c.width, 14);
    w.put(1, 1);
    w.put(c.height, 14);
    w.put(1, 1);
    w.put(1, 4); // square pixels
    w.put(4, 4);
    for n in [15, 1, 15, 1, 15, 1, 3, 11, 1, 15, 1] {
        w.put(if n == 1 { 1 } else { 0x55 & ((1 << n) - 1) }, n);
    }
    w.put(1, 1); // low_delay
    w.put(u32::from(c.mpeg_quant), 1);
    w.align();
    w.put(0x1B6, 32);
    for _ in 0..4 {
        w.put(0x1234, 16);
        w.put(1, 1);
    }
    w.put(0, 4);
    w.put(0, 10);
    w.put(3, 2);
    w.put(0, 2); // I
    w.put(1, 1);
    w.put(0b001, 3);
    w.put(1, 1); // reset DC
    w.put(u32::from(c.alternate_scan), 1);
    w.put(1, 1);
    w.put(c.dct_precision, 2);
    w.put(c.intra_dc_precision, 2);
    w.put(u32::from(c.q_scale_type), 1);
    w.align();
    if c.matrices {
        w.put(0x1B8, 32);
        w.put(3, 4);
        for k in 0..4 {
            w.put(1, 1);
            for _ in 0..64 {
                let v = if k == 0 || k == 2 { 1 + rng.next() % 64 } else { rng.next() % 256 };
                w.put(v as u32, 8);
            }
        }
        w.align();
    }
    w
}

pub fn slice_header(w: &mut W, mb: usize, mb_count: usize, rng: &mut Rng) {
    w.align();
    w.put(0x1B7, 32);
    w.put(mb as u32, usize::BITS - mb_count.leading_zeros());
    w.put(1 + (rng.next() % 31) as u32, 5);
    w.put(0, 1);
}

/// One access unit of `c` whose every macroblock the strict mode accepts, built macroblock by
/// macroblock from random bits.
pub fn picture(c: &Cfg, rng: &mut Rng) -> Vec<u8> {
    let mut w = headers(c, rng);
    let mbs = c.width.div_ceil(16) as usize * c.height.div_ceil(16) as usize;
    slice_header(&mut w, 0, mbs, rng);
    for k in 0..mbs {
        if k > 0 && k % c.slice_every == 0 {
            slice_header(&mut w, k, mbs, rng);
        }
        let mut found = false;
        for _ in 0..20_000 {
            let mut cand = W(w.0.clone());
            // a DPCM macroblock one time in four
            cand.put(u32::from(!rng.next().is_multiple_of(4)), 1);
            for _ in 0..1500 {
                cand.put(rng.next() as u32, 32);
            }
            let mut d = Decoder::new();
            d.set_strict(true);
            let _ = d.decode(&cand.bytes());
            if d.mbs_decoded() > k {
                cand.0.truncate(d.bits_used());
                w = cand;
                found = true;
                break;
            }
        }
        assert!(found, "no valid macroblock {k}");
    }
    w.align();
    w.put(0, 32);
    let p = w.bytes();
    let mut d = Decoder::new();
    d.set_strict(true);
    d.decode(&p).unwrap();
    p
}

/// 4:2:2 at 40 x 24, default parameters.
pub const BASE: Cfg = Cfg {
    rgb: false,
    chroma_format: 2,
    width: 40,
    height: 24,
    alternate_scan: false,
    dct_precision: 0,
    intra_dc_precision: 0,
    q_scale_type: false,
    matrices: false,
    mpeg_quant: false,
    slice_every: 100,
};
