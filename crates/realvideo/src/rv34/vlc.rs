// Ported from FFmpeg n7.1.1 libavcodec/rv34.c (rv34_init_tables, rv34_gen_vlc_ext) and rv40.c
// (rv40_init_tables) (Copyright (c) 2007 Mike Melanson, Konstantin Shishkov), LGPL-2.1-or-later;
// see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! The RealVideo 3 / 4 variable-length codes.

use std::sync::OnceLock;

use super::tables::*;
use crate::Gb;

/// First-level lookup width.
const LUT_BITS: u32 = 9;

#[derive(Clone, Copy)]
enum E {
    Invalid,
    /// Symbol and total code length.
    Sym(u16, u8),
    /// Second-level table and its width.
    Sub(u32, u8),
}

/// A prefix code read through a two-level lookup table.
pub(super) struct Vlc {
    lut: Vec<E>,
    subs: Vec<Vec<E>>,
}

impl Vlc {
    /// A code from `(code, length, symbol)` triples (lengths 1..=24).
    fn new(codes: &[(u32, u8, u16)]) -> Self {
        let mut lut = vec![E::Invalid; 1 << LUT_BITS];
        // Widest code behind each long prefix.
        let mut widest = vec![0u8; 1 << LUT_BITS];
        for &(code, len, _) in codes {
            if u32::from(len) > LUT_BITS {
                let p = (code >> (u32::from(len) - LUT_BITS)) as usize;
                if let Some(w) = widest.get_mut(p) {
                    *w = (*w).max(len - LUT_BITS as u8);
                }
            }
        }
        let mut subs: Vec<Vec<E>> = Vec::new();
        for (p, &w) in widest.iter().enumerate() {
            if w > 0 {
                if let Some(e) = lut.get_mut(p) {
                    *e = E::Sub(subs.len() as u32, w);
                }
                subs.push(vec![E::Invalid; 1 << w]);
            }
        }
        for &(code, len, sym) in codes {
            let l = u32::from(len);
            if len == 0 || l > 24 {
                continue;
            }
            if l <= LUT_BITS {
                let start = (code << (LUT_BITS - l)) as usize;
                for e in lut.iter_mut().skip(start).take(1 << (LUT_BITS - l)) {
                    *e = E::Sym(sym, len);
                }
            } else {
                let p = (code >> (l - LUT_BITS)) as usize;
                if let Some(&E::Sub(i, w)) = lut.get(p) {
                    let rest = l - LUT_BITS;
                    let low = code & ((1 << rest) - 1);
                    let start = (low << (u32::from(w) - rest)) as usize;
                    if let Some(t) = subs.get_mut(i as usize) {
                        for e in t.iter_mut().skip(start).take(1 << (u32::from(w) - rest)) {
                            *e = E::Sym(sym, len);
                        }
                    }
                }
            }
        }
        Self { lut, subs }
    }

    /// `rv34_gen_vlc_ext`: canonical codes from lengths in table order; the symbol is the index
    /// or `syms[index]`.
    fn canonical(lens: &[u8], syms: Option<&[u8]>) -> Self {
        let mut counts = [0u32; 17];
        for &l in lens {
            if let Some(c) = counts.get_mut(usize::from(l)) {
                *c += 1;
            }
        }
        counts[0] = 0;
        let mut next = [0u32; 17];
        for i in 0..16 {
            next[i + 1] = (next[i] + counts[i]) << 1;
        }
        let mut codes = Vec::with_capacity(lens.len());
        for (i, &l) in lens.iter().enumerate() {
            let Some(n) = next.get_mut(usize::from(l)) else { continue };
            let code = *n;
            *n += 1;
            if l == 0 {
                continue;
            }
            let sym = match syms {
                Some(s) => u16::from(s.get(i).copied().unwrap_or(0)),
                None => i as u16,
            };
            codes.push((code, l, sym));
        }
        Self::new(&codes)
    }

    /// `ff_vlc_init_from_lengths`: codes assigned in table order.
    fn from_lengths(entries: impl Iterator<Item = (u16, u8)>) -> Self {
        let mut acc: u64 = 0;
        let mut codes = Vec::new();
        for (sym, len) in entries {
            if len == 0 || len > 24 {
                continue;
            }
            let code = (acc >> (32 - u32::from(len))) as u32;
            codes.push((code, len, sym));
            acc += 1u64 << (32 - u32::from(len));
        }
        Self::new(&codes)
    }

    pub(super) fn read(&self, gb: &mut Gb<'_>) -> Option<u16> {
        let v = gb.peek(LUT_BITS) as usize;
        match *self.lut.get(v)? {
            E::Invalid => None,
            E::Sym(s, l) => {
                gb.skip(u32::from(l));
                Some(s)
            }
            E::Sub(i, w) => {
                let v2 = (gb.peek(LUT_BITS + u32::from(w)) & ((1 << w) - 1)) as usize;
                match *self.subs.get(i as usize)?.get(v2)? {
                    E::Sym(s, l) => {
                        gb.skip(u32::from(l));
                        Some(s)
                    }
                    _ => None,
                }
            }
        }
    }
}

const CBPPAT_SIZE: usize = 1296;
const CBP_SIZE: usize = 16;
const FIRSTBLK_SIZE: usize = 864;
const OTHERBLK_SIZE: usize = 108;
const COEFF_SIZE: usize = 32;

/// One RV34 VLC set (`RV34VLC`).
pub(super) struct VlcSet {
    pub cbppattern: Vec<Vlc>,
    /// `cbp[table][ones]`.
    pub cbp: Vec<Vec<Vlc>>,
    pub first: Vec<Vlc>,
    pub second: Vec<Vlc>,
    pub third: Vec<Vlc>,
    pub coeff: Vlc,
}

fn part(t: &[u8], idx: usize, size: usize) -> &[u8] {
    t.get(idx * size..(idx + 1) * size).unwrap_or(&[])
}

pub(super) struct Tables {
    pub intra: Vec<VlcSet>,
    pub inter: Vec<VlcSet>,
    pub aic_top: Vlc,
    /// Indexed `B + C * 10`; every tenth is empty.
    pub aic_mode1: Vec<Option<Vlc>>,
    pub aic_mode2: Vec<Vlc>,
    pub ptype: Vec<Vlc>,
    pub btype: Vec<Vlc>,
}

fn pairs(t: &[u8], idx: usize, n: usize) -> impl Iterator<Item = (u16, u8)> + '_ {
    (0..n).filter_map(move |k| {
        let o = (idx * n + k) * 2;
        Some((u16::from(*t.get(o)?), *t.get(o + 1)?))
    })
}

pub(super) fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let intra = (0..5)
            .map(|i| VlcSet {
                cbppattern: (0..2).map(|j| Vlc::canonical(part(&INTRA_CBPPAT, i * 2 + j, CBPPAT_SIZE), None)).collect(),
                cbp: (0..2).map(|j| (0..4).map(|k| Vlc::canonical(part(&INTRA_CBP, i * 8 + j + k * 2, CBP_SIZE), Some(&CBP_CODE))).collect()).collect(),
                first: (0..4).map(|j| Vlc::canonical(part(&INTRA_FIRSTPAT, i * 4 + j, FIRSTBLK_SIZE), None)).collect(),
                second: (0..2).map(|j| Vlc::canonical(part(&INTRA_SECONDPAT, i * 2 + j, OTHERBLK_SIZE), None)).collect(),
                third: (0..2).map(|j| Vlc::canonical(part(&INTRA_THIRDPAT, i * 2 + j, OTHERBLK_SIZE), None)).collect(),
                coeff: Vlc::canonical(part(&INTRA_COEFF, i, COEFF_SIZE), None),
            })
            .collect();
        let inter = (0..7)
            .map(|i| VlcSet {
                cbppattern: vec![Vlc::canonical(part(&INTER_CBPPAT, i, CBPPAT_SIZE), None)],
                cbp: vec![(0..4).map(|j| Vlc::canonical(part(&INTER_CBP, i * 4 + j, CBP_SIZE), Some(&CBP_CODE))).collect()],
                first: (0..2).map(|j| Vlc::canonical(part(&INTER_FIRSTPAT, i * 2 + j, FIRSTBLK_SIZE), None)).collect(),
                second: (0..2).map(|j| Vlc::canonical(part(&INTER_SECONDPAT, i * 2 + j, OTHERBLK_SIZE), None)).collect(),
                third: (0..2).map(|j| Vlc::canonical(part(&INTER_THIRDPAT, i * 2 + j, OTHERBLK_SIZE), None)).collect(),
                coeff: Vlc::canonical(part(&INTER_COEFF, i, COEFF_SIZE), None),
            })
            .collect();
        let aic_mode2 = (0..20)
            .map(|i| {
                Vlc::from_lengths((0..81).filter_map(|j| {
                    let s = *AIC_MODE2_SYMS.get(i * 81 + j)?;
                    let l = *AIC_MODE2_BITS_TAB.get(i * 81 + j)?;
                    Some((u16::from(s), l))
                }))
            })
            .collect();
        Tables {
            intra,
            inter,
            aic_top: Vlc::from_lengths(pairs(&AIC_TOP_VLC_TAB, 0, 16)),
            aic_mode1: (0..90).map(|i| if i % 10 == 9 { None } else { Some(Vlc::from_lengths(pairs(&AIC_MODE1_TABS, i, 9))) }).collect(),
            aic_mode2,
            ptype: (0..7).map(|i| Vlc::from_lengths(pairs(&PTYPE_VLC_TABS, i, 8))).collect(),
            btype: (0..6).map(|i| Vlc::from_lengths(pairs(&BTYPE_VLC_TABS, i, 7))).collect(),
        }
    })
}

/// `get_interleaved_ue_golomb`.
pub(super) fn ue(gb: &mut Gb<'_>) -> u32 {
    let mut ret: u32 = 1;
    for _ in 0..32 {
        if gb.get1() {
            return ret - 1;
        }
        ret = (ret << 1) | gb.get(1);
    }
    ret.wrapping_sub(1)
}

/// `get_interleaved_se_golomb`; `None` for a code longer than 16 bit pairs (`INVALID_VLC`).
pub(super) fn se(gb: &mut Gb<'_>) -> Option<i32> {
    let mut v: u32 = 1;
    for _ in 0..16 {
        if gb.get1() {
            let v = v as i32;
            return Some(if v & 1 == 0 { v >> 1 } else { -(v >> 1) });
        }
        v = (v << 1) | gb.get(1);
    }
    None
}
