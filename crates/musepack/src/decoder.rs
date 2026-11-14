// Ported from libmpcdec (musepack_src_r475) libmpcdec/mpc_decoder.c, requant.c and the random
// generator of synth_filter.c, BSD-3-Clause, Copyright (c) 2005-2009 The Musepack Development
// Team (`LICENSE.libmpcdec`). CFA4 changes GPL-3.0-or-later.

//! The frame decoder of libmpcdec's floating-point build: the SV7 and SV8 bitstreams,
//! requantisation and the synthesis filter, with the reference's state carried between frames.

use crate::bits::Reader;
use crate::huffman as h;
use crate::synth;

/// Samples per frame and channel.
pub const FRAME_LENGTH: usize = 36 * 32;
/// The synthesis filter's delay in samples.
pub(crate) const SYNTH_DELAY: u64 = 481;

/// Bits per sample of the quantisers 8…17 (`Res_bit`).
const RES_BIT: [u8; 18] = [0, 0, 0, 0, 0, 0, 0, 0, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// Requantisation coefficients `Cc[-1..=17]` (`__Cc`).
#[allow(clippy::excessive_precision)]
const CC: [f32; 19] = [
    111.285962475327_f32,
    65536.000000000000_f32,
    21845.333333333332_f32,
    13107.200000000001_f32,
    9362.285714285713_f32,
    7281.777777777777_f32,
    4369.066666666666_f32,
    2114.064516129032_f32,
    1040.253968253968_f32,
    516.031496062992_f32,
    257.003921568627_f32,
    128.250489236790_f32,
    64.062561094819_f32,
    32.015632633121_f32,
    16.003907203907_f32,
    8.000976681723_f32,
    4.000244155527_f32,
    2.000061037018_f32,
    1.000015259021_f32,
];

/// Requantisation offsets `Dc[-1..=17]` (`__Dc`).
const DC: [i16; 19] = [2, 0, 1, 2, 3, 4, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767];

#[rustfmt::skip]
const PARITY: [u8; 256] = [
    0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,
    1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,
    1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,
    0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,
    1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,
    0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,
    0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,
    1,0,0,1,0,1,1,0,0,1,1,0,1,0,0,1,0,1,1,0,1,0,0,1,1,0,0,1,0,1,1,0,
];

fn cc(res: i32) -> f32 {
    usize::try_from(res.saturating_add(1)).ok().and_then(|i| CC.get(i)).copied().unwrap_or(0.0)
}

fn dc(res: i32) -> i16 {
    usize::try_from(res.saturating_add(1)).ok().and_then(|i| DC.get(i)).copied().unwrap_or(0)
}

/// The stream parameters the frame decoder needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Params {
    pub sv8: bool,
    /// SV7: the highest band index; SV8: the band count (both below 32).
    pub max_band: usize,
    pub ms: bool,
    pub channels: usize,
    /// Samples in the stream (0: unknown, SV8 only).
    pub samples: u64,
    pub beg_silence: u64,
    /// SV7 only.
    pub true_gapless: bool,
}

pub(crate) struct Decoder {
    p: Params,
    samples: u64,
    decoded: u64,
    to_skip: u64,
    last_max_band: i32,
    r1: u32,
    r2: u32,
    scf_index: [[[i32; 3]; 32]; 2],
    q: [[[i16; 36]; 32]; 2],
    res: [[i32; 32]; 2],
    dscf_flag: [[bool; 32]; 2],
    scfi: [[i32; 32]; 2],
    ms_flag: [bool; 32],
    v: [Vec<f32>; 2],
    y: Box<[[[f32; 32]; 36]; 2]>,
    scf: [f32; 256],
    win: [[f32; 16]; 32],
    out: [Vec<f32>; 2],
}

impl Decoder {
    pub(crate) fn new(p: Params) -> Self {
        let mut d = Self {
            p,
            samples: 0,
            decoded: 0,
            to_skip: SYNTH_DELAY.saturating_add(p.beg_silence),
            last_max_band: 0,
            r1: 1,
            r2: 1,
            scf_index: [[[0; 3]; 32]; 2],
            q: [[[0; 36]; 32]; 2],
            res: [[0; 32]; 2],
            dscf_flag: [[false; 32]; 2],
            scfi: [[0; 32]; 2],
            ms_flag: [false; 32],
            v: [vec![0.0; synth::V_LEN], vec![0.0; synth::V_LEN]],
            y: Box::new([[[0.0; 32]; 36]; 2]),
            scf: [0.0; 256],
            win: synth::window(),
            out: [vec![0.0; FRAME_LENGTH], vec![0.0; FRAME_LENGTH]],
        };
        d.samples = if !p.sv8 && p.true_gapless { p.samples.div_ceil(FRAME_LENGTH as u64).saturating_mul(FRAME_LENGTH as u64) } else { p.samples };
        d.init_scf();
        d
    }

    /// `mpc_decoder_scale_output` with factor 1.
    fn init_scf(&mut self) {
        const STEP: f64 = 0.832_980_664_765_826_7;
        let factor = 1.0 / f64::from(1u32 << 15);
        let (mut f1, mut f2) = (factor, factor);
        self.scf[1] = factor as f32;
        f1 *= STEP;
        f2 *= 1.0 / STEP;
        for n in 1..=128u32 {
            self.scf[usize::from((1 + n) as u8)] = f1 as f32;
            self.scf[usize::from(1u32.wrapping_sub(n) as u8)] = f2 as f32;
            f1 *= STEP;
            f2 *= 1.0 / STEP;
        }
    }

    /// `mpc_random_int`.
    fn random(&mut self) -> u32 {
        let t1 = u32::from(PARITY[(self.r1 & 0xF5) as usize]) << 31;
        let t2 = u32::from(PARITY[((self.r2 >> 25) & 0x63) as usize]);
        self.r1 = (self.r1 >> 1) | t1;
        self.r2 = self.r2.wrapping_add(self.r2) | t2;
        self.r1 ^ self.r2
    }

    fn noise(&mut self, ch: usize, band: usize) {
        for k in 0..36 {
            let t = self.random();
            let v = (t >> 24) + ((t >> 16) & 0xFF) + ((t >> 8) & 0xFF) + (t & 0xFF);
            self.q[ch][band][k] = (v as i32 - 510) as i16;
        }
    }

    /// `mpc_decoder_decode_frame`: decodes one frame from `r` and appends its output samples
    /// to `out` (one plane per channel). `false` once the stream's samples are complete.
    pub(crate) fn decode_frame(&mut self, r: &mut Reader<'_>, key_frame: bool, out: &mut [Vec<f32>]) -> bool {
        let fl = FRAME_LENGTH as i64;
        let mut samples_left = (self.samples as i64).wrapping_sub(self.decoded as i64).wrapping_add(SYNTH_DELAY as i64);
        if samples_left <= 0 && self.samples != 0 {
            return false;
        }
        if self.p.sv8 {
            self.read_sv8(r, key_frame);
        } else {
            self.read_sv7(r);
        }
        if self.to_skip < FRAME_LENGTH as u64 + SYNTH_DELAY {
            self.requantise();
            for ch in 0..self.p.channels.min(2) {
                synth::synthesize(&self.win, &mut self.v[ch], &self.y[ch], &mut self.out[ch]);
            }
        }
        self.decoded = self.decoded.wrapping_add(FRAME_LENGTH as u64);
        if !self.p.sv8 && self.decoded.wrapping_sub(self.samples) < FRAME_LENGTH as u64 {
            let mut last = i64::from(r.read(11));
            if self.decoded == self.samples {
                if last == 0 {
                    last = fl;
                }
                self.samples = self.samples.wrapping_add((last - fl) as u64);
                samples_left += last - fl;
            }
        }
        let mut n = samples_left.clamp(0, fl) as u64;
        let mut start = 0;
        if self.to_skip > 0 {
            if n <= self.to_skip {
                self.to_skip -= n;
                n = 0;
            } else {
                n -= self.to_skip;
                start = self.to_skip;
                self.to_skip = 0;
            }
        }
        let (start, n) = (start as usize, n as usize);
        for (ch, plane) in out.iter_mut().enumerate().take(self.p.channels.min(2)) {
            plane.extend_from_slice(self.out[ch].get(start..start + n).unwrap_or(&[]));
        }
        true
    }

    fn requantise(&mut self) {
        let last = self.p.max_band.min(31);
        for band in 0..=last {
            let (rl, rr) = (self.res[0][band], self.res[1][band]);
            for part in 0..3 {
                let fl = cc(rl) * self.scf[(self.scf_index[0][band][part] & 0xFF) as usize];
                let fr = cc(rr) * self.scf[(self.scf_index[1][band][part] & 0xFF) as usize];
                for n in part * 12..part * 12 + 12 {
                    let ql = f32::from(self.q[0][band][n]);
                    let qr = f32::from(self.q[1][band][n]);
                    let (yl, yr) = match (self.ms_flag[band], rl != 0, rr != 0) {
                        (true, true, true) => {
                            let (l, r) = (fl * ql, fr * qr);
                            (l + r, l - r)
                        }
                        (true, true, false) => (fl * ql, fl * ql),
                        (true, false, true) => {
                            let l = fr * qr;
                            (l, -l)
                        }
                        (false, true, true) => (fl * ql, fr * qr),
                        (false, true, false) => (fl * ql, 0.0),
                        (false, false, true) => (0.0, fr * qr),
                        (_, false, false) => (0.0, 0.0),
                    };
                    self.y[0][n][band] = yl;
                    self.y[1][n][band] = yr;
                }
            }
        }
    }

    fn read_sv7(&mut self, r: &mut Reader<'_>) {
        const IDX30: [i16; 27] = [-1, 0, 1, -1, 0, 1, -1, 0, 1, -1, 0, 1, -1, 0, 1, -1, 0, 1, -1, 0, 1, -1, 0, 1, -1, 0, 1];
        const IDX31: [i16; 27] = [-1, -1, -1, 0, 0, 0, 1, 1, 1, -1, -1, -1, 0, 0, 0, 1, 1, 1, -1, -1, -1, 0, 0, 0, 1, 1, 1];
        const IDX32: [i16; 27] = [-1, -1, -1, -1, -1, -1, -1, -1, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1];
        const IDX50: [i16; 25] = [-2, -1, 0, 1, 2, -2, -1, 0, 1, 2, -2, -1, 0, 1, 2, -2, -1, 0, 1, 2, -2, -1, 0, 1, 2];
        const IDX51: [i16; 25] = [-2, -2, -2, -2, -2, -1, -1, -1, -1, -1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2];
        const Q7: [[&[(u16, u8, i8)]; 2]; 7] = [
            [&h::Q1_7_0, &h::Q1_7_1],
            [&h::Q2_7_0, &h::Q2_7_1],
            [&h::Q3_7_0, &h::Q3_7_1],
            [&h::Q4_7_0, &h::Q4_7_1],
            [&h::Q5_7_0, &h::Q5_7_1],
            [&h::Q6_7_0, &h::Q6_7_1],
            [&h::Q7_7_0, &h::Q7_7_1],
        ];
        let max_band = self.p.max_band.min(31);
        let mut used = 0;
        // header
        self.res[0][0] = r.read(4) as i32;
        self.res[1][0] = r.read(4) as i32;
        if self.res[0][0] != 0 || self.res[1][0] != 0 {
            if self.p.ms {
                self.ms_flag[0] = r.read(1) != 0;
            }
            used = 1;
        }
        for n in 1..=max_band {
            for ch in 0..2 {
                let idx = r.huff(&h::HDR);
                self.res[ch][n] = if idx != 4 { self.res[ch][n - 1].wrapping_add(idx) } else { r.read(4) as i32 };
            }
            if self.res[0][n] != 0 || self.res[1][n] != 0 {
                if self.p.ms {
                    self.ms_flag[n] = r.read(1) != 0;
                }
                used = n + 1;
            }
        }
        // SCFI
        for n in 0..used {
            for ch in 0..2 {
                if self.res[ch][n] != 0 {
                    self.scfi[ch][n] = r.huff(&h::SCFI7);
                }
            }
        }
        // SCF / DSCF
        for n in 0..used {
            for ch in 0..2 {
                if self.res[ch][n] == 0 {
                    continue;
                }
                let scf = &mut self.scf_index[ch][n];
                let delta = |r: &mut Reader<'_>, prev: i32| {
                    let idx = r.huff(&h::DSCF7);
                    if idx != 8 { prev.wrapping_add(idx) } else { r.read(6) as i32 }
                };
                match self.scfi[ch][n] {
                    1 => {
                        scf[0] = delta(r, scf[2]);
                        scf[1] = delta(r, scf[0]);
                        scf[2] = scf[1];
                    }
                    3 => {
                        scf[0] = delta(r, scf[2]);
                        scf[1] = scf[0];
                        scf[2] = scf[1];
                    }
                    2 => {
                        scf[0] = delta(r, scf[2]);
                        scf[1] = scf[0];
                        scf[2] = delta(r, scf[1]);
                    }
                    0 => {
                        scf[0] = delta(r, scf[2]);
                        scf[1] = delta(r, scf[0]);
                        scf[2] = delta(r, scf[1]);
                    }
                    _ => return,
                }
                for s in scf.iter_mut() {
                    if *s > 1024 {
                        *s = 0x8080;
                    }
                }
            }
        }
        // samples
        for n in 0..used {
            for ch in 0..2 {
                let res = self.res[ch][n] as i16 as i32;
                match res {
                    -17..=-2 | 0 => {}
                    -1 => self.noise(ch, n),
                    1 => {
                        let t = Q7[0][r.read(1) as usize];
                        for k in (0..36).step_by(3) {
                            let i = r.huff(t) as usize;
                            let q = &mut self.q[ch][n];
                            q[k] = IDX30.get(i).copied().unwrap_or(0);
                            q[k + 1] = IDX31.get(i).copied().unwrap_or(0);
                            q[k + 2] = IDX32.get(i).copied().unwrap_or(0);
                        }
                    }
                    2 => {
                        let t = Q7[1][r.read(1) as usize];
                        for k in (0..36).step_by(2) {
                            let i = r.huff(t) as usize;
                            let q = &mut self.q[ch][n];
                            q[k] = IDX50.get(i).copied().unwrap_or(0);
                            q[k + 1] = IDX51.get(i).copied().unwrap_or(0);
                        }
                    }
                    3..=7 => {
                        let t = Q7[res as usize - 1][r.read(1) as usize];
                        for k in 0..36 {
                            self.q[ch][n][k] = r.huff(t) as i16;
                        }
                    }
                    8..=17 => {
                        let bits = u32::from(RES_BIT[res as usize]);
                        for k in 0..36 {
                            self.q[ch][n][k] = (r.read(bits) as i32 - i32::from(dc(res))) as i16;
                        }
                    }
                    _ => return,
                }
            }
        }
    }

    fn read_sv8(&mut self, r: &mut Reader<'_>, key_frame: bool) {
        const THRES: [u32; 9] = [0, 0, 3, 0, 0, 1, 3, 4, 8];
        #[rustfmt::skip]
        const Q2_VAR: [u32; 125] = [
            6, 5, 4, 5, 6, 5, 4, 3, 4, 5, 4, 3, 2, 3, 4, 5, 4, 3, 4, 5, 6, 5, 4, 5, 6, 5, 4, 3, 4, 5, 4, 3, 2, 3, 4, 3, 2, 1, 2, 3,
            4, 3, 2, 3, 4, 5, 4, 3, 4, 5, 4, 3, 2, 3, 4, 3, 2, 1, 2, 3, 2, 1, 0, 1, 2, 3, 2, 1, 2, 3, 4, 3, 2, 3, 4, 5, 4, 3, 4, 5,
            4, 3, 2, 3, 4, 3, 2, 1, 2, 3, 4, 3, 2, 3, 4, 5, 4, 3, 4, 5, 6, 5, 4, 5, 6, 5, 4, 3, 4, 5, 4, 3, 2, 3, 4, 5, 4, 3, 4, 5,
            6, 5, 4, 5, 6,
        ];
        type Can = (&'static [(u16, u8, i8)], &'static [i8]);
        const RES_T: [Can; 2] = [(&h::H8_RES_1, &h::S8_RES_1), (&h::H8_RES_2, &h::S8_RES_2)];
        const SCFI_T: [Can; 2] = [(&h::H8_SCFI_1, &h::S8_SCFI_1), (&h::H8_SCFI_2, &h::S8_SCFI_2)];
        const DSCF_T: [Can; 2] = [(&h::H8_DSCF_1, &h::S8_DSCF_1), (&h::H8_DSCF_2, &h::S8_DSCF_2)];
        const Q_T: [[Can; 2]; 6] = [
            [(&h::H8_Q2_1, &h::S8_Q2_1), (&h::H8_Q2_2, &h::S8_Q2_2)],
            [(&h::H8_Q3, &h::S8_Q3), (&h::H8_Q4, &h::S8_Q4)],
            [(&h::H8_Q5_1, &h::S8_Q5_1), (&h::H8_Q5_2, &h::S8_Q5_2)],
            [(&h::H8_Q6_1, &h::S8_Q6_1), (&h::H8_Q6_2, &h::S8_Q6_2)],
            [(&h::H8_Q7_1, &h::S8_Q7_1), (&h::H8_Q7_2, &h::S8_Q7_2)],
            [(&h::H8_Q8_1, &h::S8_Q8_1), (&h::H8_Q8_2, &h::S8_Q8_2)],
        ];
        let can = |r: &mut Reader<'_>, t: Can| r.can(t.0, t.1);
        let max_band = self.p.max_band.min(31);

        // header
        let mut used = if key_frame {
            r.log_dec(max_band as u32 + 1) as i32
        } else {
            let mut u = self.last_max_band.wrapping_add(can(r, (&h::H8_BANDS, &h::S8_BANDS)));
            if u > 32 {
                u -= 33;
            }
            u
        };
        // a damaged stream may name more bands than there are; the reference reads out of range
        used = used.clamp(0, 32);
        self.last_max_band = used;
        let used = used as usize;
        if used > 0 {
            for ch in 0..2 {
                let mut v = can(r, RES_T[0]);
                if v > 15 {
                    v -= 17;
                }
                self.res[ch][used - 1] = v;
            }
            for n in (0..used - 1).rev() {
                for ch in 0..2 {
                    let next = self.res[ch][n + 1];
                    let mut v = can(r, RES_T[usize::from(next > 2)]).wrapping_add(next);
                    if v > 15 {
                        v -= 17;
                    }
                    self.res[ch][n] = v;
                }
            }
            if self.p.ms {
                let tot = (0..used).filter(|&n| self.res[0][n] != 0 || self.res[1][n] != 0).count() as u32;
                let cnt = r.log_dec(tot);
                let mut tmp = 0u32;
                if cnt != 0 && cnt != tot {
                    tmp = r.enum_dec(cnt.min(tot.saturating_sub(cnt)), tot);
                }
                if cnt * 2 > tot {
                    tmp = !tmp;
                }
                for n in (0..used).rev() {
                    if self.res[0][n] != 0 || self.res[1][n] != 0 {
                        self.ms_flag[n] = tmp & 1 != 0;
                        tmp >>= 1;
                    }
                }
            }
        }
        for n in used..=max_band {
            self.res[0][n] = 0;
            self.res[1][n] = 0;
        }

        // SCFI
        if key_frame {
            self.dscf_flag = [[true; 32]; 2];
        }
        for n in 0..used {
            let (l, rr) = (self.res[0][n] != 0, self.res[1][n] != 0);
            let cnt = usize::from(l) + usize::from(rr);
            if cnt > 0 {
                let tmp = can(r, SCFI_T[cnt - 1]);
                if l {
                    self.scfi[0][n] = tmp >> (2 * (cnt - 1));
                }
                if rr {
                    self.scfi[1][n] = tmp & 3;
                }
            }
        }

        // SCF / DSCF
        for n in 0..used {
            for ch in 0..2 {
                if self.res[ch][n] == 0 {
                    continue;
                }
                let scfi = self.scfi[ch][n];
                let scf = &mut self.scf_index[ch][n];
                if self.dscf_flag[ch][n] {
                    scf[0] = r.read(7) as i32 - 6;
                    self.dscf_flag[ch][n] = false;
                } else {
                    let mut tmp = can(r, DSCF_T[1]);
                    if tmp == 64 {
                        tmp += r.read(6) as i32;
                    }
                    scf[0] = (scf[2].wrapping_sub(25).wrapping_add(tmp) & 127) - 6;
                }
                for m in 0..2 {
                    if ((scfi << m) & 2) == 0 {
                        let mut tmp = can(r, DSCF_T[0]);
                        if tmp == 31 {
                            tmp = 64 + r.read(6) as i32;
                        }
                        scf[m + 1] = (scf[m].wrapping_sub(25).wrapping_add(tmp) & 127) - 6;
                    } else {
                        scf[m + 1] = scf[m];
                    }
                }
            }
        }

        // samples
        for n in 0..used {
            for ch in 0..2 {
                let res = self.res[ch][n] as i16 as i32;
                match res {
                    0 => {}
                    2 => {
                        let mut idx = 2 * THRES[2];
                        for k in (0..36).step_by(3) {
                            let t = can(r, Q_T[0][usize::from(idx > THRES[2])]);
                            let t = usize::try_from(t).unwrap_or(0).min(124);
                            let q = &mut self.q[ch][n];
                            q[k] = t as i16 % 5 - 2;
                            q[k + 1] = (t as i16 / 5) % 5 - 2;
                            q[k + 2] = t as i16 / 25 - 2;
                            idx = (idx >> 1) + Q2_VAR[t];
                        }
                    }
                    1 => {
                        let mut k = 0;
                        while k < 36 {
                            let kmax = k + 18;
                            let cnt = can(r, (&h::H8_Q1, &h::S8_Q1));
                            let mut idx = 0u32;
                            if cnt > 0 && cnt < 18 {
                                idx = r.enum_dec(if cnt <= 9 { cnt as u32 } else { 18 - cnt as u32 }, 18);
                            }
                            if cnt > 9 {
                                idx = !idx;
                            }
                            while k < kmax {
                                let mut v = 0;
                                if idx & (1 << 17) != 0 {
                                    v = (r.read(1) as i16) * 2 - 1;
                                }
                                self.q[ch][n][k] = v;
                                idx <<= 1;
                                k += 1;
                            }
                        }
                    }
                    -1 => self.noise(ch, n),
                    3 | 4 => {
                        let t = Q_T[1][res as usize - 3];
                        for k in (0..36).step_by(2) {
                            let sym = can(r, t) as i8;
                            self.q[ch][n][k] = i16::from((sym << 4) >> 4);
                            self.q[ch][n][k + 1] = i16::from(sym >> 4);
                        }
                    }
                    5..=8 => {
                        let tables = Q_T[res as usize - 3];
                        let th = THRES[res as usize];
                        let mut idx = 2 * th;
                        for k in 0..36 {
                            let v = can(r, tables[usize::from(idx > th)]) as i16;
                            self.q[ch][n][k] = v;
                            idx = (idx >> 1).wrapping_add(u32::from(v.unsigned_abs()));
                        }
                    }
                    9..=17 => {
                        let extra = (res - 9) as u32;
                        for k in 0..36 {
                            let mut v = i16::from(can(r, (&h::H8_Q9UP, &h::S8_Q9UP)) as u8);
                            if extra != 0 {
                                v = (v << extra) | r.read(extra) as i16;
                            }
                            self.q[ch][n][k] = v.wrapping_sub(dc(res));
                        }
                    }
                    // not produced by an encoder; the reference indexes out of its tables
                    _ => {}
                }
            }
        }
    }
}
