// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/inv_trans.c`, `com_table.c`), Copyright (c) 2018-2022 Peking
// University Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
// Corporation. See LICENSE.uavs3d.
//! Inverse transforms: DCT-2 (up to 64x64), DCT-8 / DST-7 (position based transform), the secondary
//! transform and the alternative 4x4 transform.

use std::sync::OnceLock;

/// The transform kinds of `g_tbl_itrans`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Dct2,
    Dct8,
    Dst7,
}

/// `[kind][log2 - 1]` -> `size * size` matrix, `[freq][pos]` row major (`com_dct_coef_init`).
fn tables() -> &'static Vec<Vec<Vec<i32>>> {
    static T: OnceLock<Vec<Vec<Vec<i32>>>> = OnceLock::new();
    T.get_or_init(|| {
        let mut all = vec![Vec::new(), Vec::new(), Vec::new()];
        let mut n = 2usize;
        for _ in 0..6 {
            let s = (n as f64).sqrt() * 32.0;
            let mut m = [vec![0i32; n * n], vec![0i32; n * n], vec![0i32; n * n]];
            let round = |v: f64| (s * v + if v > 0.0 { 0.5 } else { -0.5 }) as i8 as i32;
            for k in 0..n {
                for p in 0..n {
                    let (kf, pf, nf) = (k as f64, p as f64, n as f64);
                    let w0 = if k == 0 { 0.5f64.sqrt() } else { 1.0 };
                    m[0][k * n + p] = round((std::f64::consts::PI * (pf + 0.5) * kf / nf).cos() * w0 * (2.0 / nf).sqrt());
                    m[1][k * n + p] = round((std::f64::consts::PI * (kf + 0.5) * (pf + 0.5) / (nf + 0.5)).cos() * (2.0 / (nf + 0.5)).sqrt());
                    m[2][k * n + p] = round((std::f64::consts::PI * (kf + 0.5) * (pf + 1.0) / (nf + 0.5)).sin() * (2.0 / (nf + 0.5)).sqrt());
                }
            }
            for (i, mm) in m.into_iter().enumerate() {
                all[i].push(mm);
            }
            n <<= 1;
        }
        all
    })
}

fn matrix(kind: Kind, log2: u32) -> &'static [i32] {
    let k = match kind {
        Kind::Dct2 => 0,
        Kind::Dct8 => 1,
        Kind::Dst7 => 2,
    };
    &tables()[k][log2 as usize - 1]
}

/// One stage: `src` is `n_in` rows of `line` values (`src[k * line + j]`); the result is `dst[j * n + p]`
/// with `n = 1 << log2` and `p` the output position.
#[allow(clippy::too_many_arguments)]
fn stage(src: &[i32], dst: &mut [i32], kind: Kind, log2: u32, line: usize, shift: u32, lo: i32, hi: i32) {
    let n = 1usize << log2;
    let m = matrix(kind, log2);
    let kmax = if n == 64 { 32 } else { n };
    let add = 1i32 << (shift - 1);
    for j in 0..line {
        for p in 0..n {
            let mut sum = 0i32;
            for k in 0..kmax {
                sum = sum.wrapping_add(m[k * n + p].wrapping_mul(src[k * line + j]));
            }
            dst[j * n + p] = ((sum.wrapping_add(add)) >> shift).clamp(lo, hi);
        }
    }
}

const C4: [[i32; 4]; 4] = [[34, 77, 79, 55], [58, 69, -33, -84], [72, -7, -75, 73], [81, -75, 58, -28]];
const C8: [[i32; 4]; 4] = [[123, -32, 14, 8], [-35, -120, 25, 13], [-8, 30, 123, 19], [-3, 10, -22, 126]];

fn clip16(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

fn alt_4x4(src: &[i32], resi: &mut [i32], bit_depth: u32) {
    let shift = 22 - bit_depth;
    let add = 1i32 << (shift - 1);
    let (lo, hi) = (-(1i32 << bit_depth), (1i32 << bit_depth) - 1);
    let mut tmp = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 16;
            for k in 0..4 {
                sum += C4[i][k] * src[k * 4 + j];
            }
            tmp[i][j] = clip16(sum >> 5);
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = add;
            for k in 0..4 {
                sum += C4[i][k] * tmp[j][k];
            }
            resi[j * 4 + i] = (sum >> shift).clamp(lo, hi);
        }
    }
}

fn sec_vert(src: &mut [i32], stride: usize) {
    let mut t = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            t[i][j] = src[i * stride + j];
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 64;
            for k in 0..4 {
                sum += C8[i][k] * t[k][j];
            }
            src[i * stride + j] = clip16(sum >> 7);
        }
    }
}

fn sec_hor(src: &mut [i32], stride: usize) {
    let mut t = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            t[i][j] = src[i * stride + j];
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 64;
            for k in 0..4 {
                sum += C8[i][k] * t[j][k];
            }
            src[j * stride + i] = clip16(sum >> 7);
        }
    }
}

fn sec_hor_ver(src: &mut [i32], stride: usize) {
    let mut tmp = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 64;
            for t in 0..4 {
                sum += C8[i][t] * src[j * stride + t];
            }
            tmp[i][j] = clip16(sum >> 7);
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 64;
            for t in 0..4 {
                sum += C8[i][t] * tmp[j][t];
            }
            src[i * stride + j] = clip16(sum >> 7);
        }
    }
}

/// `com_itrans`: `coef` (`w * h`, row major) to `resi`. `nxn_idx` is `Some(blk_idx)` for a luma block of a
/// position based transform, `sec_t_vh` the secondary transform selection (0 none), `alt_4x4` the
/// sequence flag of the alternative 4x4 transform.
#[allow(clippy::too_many_arguments)]
pub(crate) fn itrans(coef: &[i16], resi: &mut [i16], log2_w: u32, log2_h: u32, bit_depth: u32, nxn_idx: Option<usize>, sec_t_vh: u32, alt: bool) {
    let (w, h) = (1usize << log2_w, 1usize << log2_h);
    let mut c: Vec<i32> = coef[..w * h].iter().map(|&v| i32::from(v)).collect();
    let mut out = vec![0i32; w * h];
    if alt && log2_w == 2 && log2_h == 2 {
        alt_4x4(&c, &mut out, bit_depth);
    } else {
        if sec_t_vh == 3 {
            sec_hor_ver(&mut c, w);
        } else if sec_t_vh & 1 != 0 {
            sec_hor(&mut c, w);
        } else if sec_t_vh >> 1 != 0 {
            sec_vert(&mut c, w);
        }
        let (lo, hi) = (-(1i32 << bit_depth), (1i32 << bit_depth) - 1);
        let mut tmp = vec![0i32; w * h];
        let (kv, kh) = match nxn_idx {
            Some(idx) => (if idx >> 1 != 0 { Kind::Dst7 } else { Kind::Dct8 }, if idx & 1 != 0 { Kind::Dst7 } else { Kind::Dct8 }),
            None => (Kind::Dct2, Kind::Dct2),
        };
        stage(&c, &mut tmp, kv, log2_h, w, 5, -32768, 32767);
        // second stage: `tmp[k * h + j]`, k = horizontal frequency, j = row
        let mut o2 = vec![0i32; w * h];
        stage(&tmp, &mut o2, kh, log2_w, h, 20 - bit_depth, lo, hi);
        out.copy_from_slice(&o2);
    }
    for (r, &v) in resi.iter_mut().zip(out.iter()) {
        *r = v as i16;
    }
}
