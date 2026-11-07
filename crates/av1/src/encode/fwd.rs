//! Forward transforms, derived from the decoder's inverse transforms.
//!
//! Every 2D inverse transform is separable: the residual is `Kc · C · Krᵀ` for the coefficient
//! block `C` (up to the integer rounding). The basis vectors are measured once per transform size
//! and type by running the decoder's own inverse on single coefficients, and the forward transform
//! is the least-squares inverse `C = (KcᵀKc)⁻¹Kcᵀ · R · Kr(KrᵀKr)⁻¹`, so it matches whatever
//! scaling the inverse has (rectangular sizes, 64-point sizes keeping 32 coefficients, ADST4's
//! slight non-orthogonality).

use std::sync::OnceLock;

use crate::spec_tables::*;
use crate::transform::inverse_transform_2d;

/// Forward transform of one size and type.
pub(crate) struct Fwd {
    pub w: usize,
    pub h: usize,
    /// Coefficients kept (min(w, 32), min(h, 32)).
    pub tw: usize,
    pub th: usize,
    /// tw x w, row major
    fr: Vec<f32>,
    /// th x h, row major
    fc: Vec<f32>,
    flip_ud: bool,
    flip_lr: bool,
    /// Residual energy per unit of coefficient energy (the transform's squared scale).
    pub gain: f32,
}

const SIZES: usize = 19;
const TYPES: usize = 16;

static CACHE: OnceLock<Vec<OnceLock<Option<Fwd>>>> = OnceLock::new();

/// The forward transform of `tx_sz` / `tx_type` (None for invalid combinations).
pub(crate) fn fwd(tx_sz: usize, tx_type: usize) -> Option<&'static Fwd> {
    if tx_sz >= SIZES || tx_type >= TYPES {
        return None;
    }
    let cache = CACHE.get_or_init(|| (0..SIZES * TYPES).map(|_| OnceLock::new()).collect());
    cache.get(tx_sz * TYPES + tx_type)?.get_or_init(|| Fwd::measure(tx_sz, tx_type)).as_ref()
}

impl Fwd {
    fn measure(tx_sz: usize, tx_type: usize) -> Option<Fwd> {
        let w = TX_WIDTH[tx_sz] as usize;
        let h = TX_HEIGHT[tx_sz] as usize;
        let (tw, th) = (w.min(32), h.min(32));
        let big = w.max(h) == 64;
        if big && tx_type != DCT_DCT {
            return None;
        }
        const V: i32 = 4096;
        const BD: u32 = 12;
        let mut deq = vec![0i32; 64 * 64];
        let mut res = vec![0i32; w * h];
        let mut probe = |i: usize, j: usize, res: &mut [i32]| {
            deq.fill(0);
            deq[i * 64 + j] = V;
            inverse_transform_2d(&deq, res, tx_sz, tx_type, false, BD);
        };
        probe(0, 0, &mut res);
        let r00 = res.clone();
        let (mut y0, mut x0, mut best) = (0, 0, 0);
        for y in 0..h {
            for x in 0..w {
                if r00[y * w + x].abs() > best {
                    best = r00[y * w + x].abs();
                    y0 = y;
                    x0 = x;
                }
            }
        }
        if best == 0 {
            return None;
        }
        let scale = V as f64 * r00[y0 * w + x0] as f64;
        let gain = (r00.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / (V as f64 * V as f64)) as f32;
        // pr: w x tw (column j = row basis j), pc: h x th (column i = column basis i)
        let mut pr = vec![0f64; w * tw];
        let mut pc = vec![0f64; h * th];
        for j in 0..tw {
            probe(0, j, &mut res);
            for x in 0..w {
                pr[x * tw + j] = res[y0 * w + x] as f64;
            }
        }
        for i in 0..th {
            probe(i, 0, &mut res);
            for y in 0..h {
                pc[y * th + i] = res[y * w + x0] as f64 / scale;
            }
        }
        let fr = pseudo_inverse(&pr, w, tw)?;
        let fc = pseudo_inverse(&pc, h, th)?;
        let flip_ud = matches!(tx_type, FLIPADST_DCT | FLIPADST_ADST | V_FLIPADST | FLIPADST_FLIPADST);
        let flip_lr = matches!(tx_type, DCT_FLIPADST | ADST_FLIPADST | H_FLIPADST | FLIPADST_FLIPADST);
        Some(Fwd { w, h, tw, th, fr: fr.iter().map(|&v| v as f32).collect(), fc: fc.iter().map(|&v| v as f32).collect(), flip_ud, flip_lr, gain })
    }

    /// Coefficients (th x tw, row stride tw: the decoder's `Quant` layout before quantization) of
    /// the residual `res` (h x w, row stride w).
    pub fn apply(&self, res: &[i32], out: &mut [f32], tmp: &mut Vec<f32>) {
        let (w, h, tw, th) = (self.w, self.h, self.tw, self.th);
        // tmp = R' · Frᵀ (h x tw), R' the flipped residual
        tmp.clear();
        tmp.resize(h * tw, 0.0);
        for y in 0..h {
            let sy = if self.flip_ud { h - 1 - y } else { y };
            let Some(row) = res.get(sy * w..sy * w + w) else { return };
            let trow = &mut tmp[y * tw..y * tw + tw];
            for (j, t) in trow.iter_mut().enumerate() {
                let f = &self.fr[j * w..j * w + w];
                let mut s = 0f32;
                if self.flip_lr {
                    for x in 0..w {
                        s += f[x] * row[w - 1 - x] as f32;
                    }
                } else {
                    for x in 0..w {
                        s += f[x] * row[x] as f32;
                    }
                }
                *t = s;
            }
        }
        // out = Fc · tmp (th x tw)
        for i in 0..th {
            let f = &self.fc[i * h..i * h + h];
            let Some(orow) = out.get_mut(i * tw..i * tw + tw) else { return };
            orow.fill(0.0);
            for y in 0..h {
                let k = f[y];
                if k == 0.0 {
                    continue;
                }
                for (o, &t) in orow.iter_mut().zip(&tmp[y * tw..y * tw + tw]) {
                    *o += k * t;
                }
            }
        }
    }
}

/// (BᵀB)⁻¹Bᵀ for the n x k matrix `b` (row major), as a k x n matrix.
fn pseudo_inverse(b: &[f64], n: usize, k: usize) -> Option<Vec<f64>> {
    // gram = BᵀB (k x k), augmented with the identity for Gauss-Jordan elimination
    let mut a = vec![0f64; k * 2 * k];
    for i in 0..k {
        for j in 0..k {
            let mut s = 0.0;
            for r in 0..n {
                s += b[r * k + i] * b[r * k + j];
            }
            a[i * 2 * k + j] = s;
        }
        a[i * 2 * k + k + i] = 1.0;
    }
    for col in 0..k {
        let piv = (col..k).max_by(|&x, &y| a[x * 2 * k + col].abs().total_cmp(&a[y * 2 * k + col].abs()))?;
        if a[piv * 2 * k + col].abs() < 1e-12 {
            return None;
        }
        if piv != col {
            for j in 0..2 * k {
                a.swap(piv * 2 * k + j, col * 2 * k + j);
            }
        }
        let d = a[col * 2 * k + col];
        for j in 0..2 * k {
            a[col * 2 * k + j] /= d;
        }
        for r in 0..k {
            if r != col {
                let m = a[r * 2 * k + col];
                if m != 0.0 {
                    for j in 0..2 * k {
                        a[r * 2 * k + j] -= m * a[col * 2 * k + j];
                    }
                }
            }
        }
    }
    // (BᵀB)⁻¹Bᵀ
    let mut out = vec![0f64; k * n];
    for i in 0..k {
        for r in 0..n {
            let mut s = 0.0;
            for j in 0..k {
                s += a[i * 2 * k + k + j] * b[r * k + j];
            }
            out[i * n + r] = s;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forward then inverse reproduces a residual (up to the rounding of integer coefficients).
    #[test]
    fn forward_inverts_the_decoders_transforms() {
        let mut seed = 12345u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 20) % 201) as i32 - 100
        };
        for tx_sz in 0..SIZES {
            for tx_type in 0..TYPES {
                let Some(f) = fwd(tx_sz, tx_type) else { continue };
                let (w, h) = (f.w, f.h);
                // a smooth-ish residual (64-point transforms keep only the low half)
                let base: Vec<i32> = (0..4).map(|_| rnd()).collect();
                let res: Vec<i32> = (0..w * h)
                    .map(|i| {
                        let (y, x) = ((i / w) as i32, (i % w) as i32);
                        if w.max(h) == 64 { base[0] + (base[1] * x + base[2] * y) / 64 } else { rnd() }
                    })
                    .collect();
                let mut c = vec![0f32; f.tw * f.th];
                let mut tmp = Vec::new();
                f.apply(&res, &mut c, &mut tmp);
                let mut deq = vec![0i32; 64 * 64];
                for i in 0..f.th {
                    for j in 0..f.tw {
                        deq[i * 64 + j] = c[i * f.tw + j].round() as i32;
                    }
                }
                let mut out = vec![0i32; w * h];
                inverse_transform_2d(&deq, &mut out, tx_sz, tx_type, false, 8);
                let flip_ud = f.flip_ud;
                let flip_lr = f.flip_lr;
                let mut err = 0f64;
                for y in 0..h {
                    for x in 0..w {
                        let sy = if flip_ud { h - 1 - y } else { y };
                        let sx = if flip_lr { w - 1 - x } else { x };
                        let d = (out[y * w + x] - res[sy * w + sx]) as f64;
                        err += d * d;
                    }
                }
                let rms = (err / (w * h) as f64).sqrt();
                assert!(rms < 1.5, "tx_sz {tx_sz} type {tx_type}: rms {rms}");
            }
        }
    }
}
