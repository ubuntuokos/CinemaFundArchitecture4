//! Forward transforms, derived from the decoder's own inverse transforms (8.7.2): the basis
//! image of every coefficient is measured once through [`inverse_transform_add`], and a residual
//! block is projected onto those images. The encoder's reconstruction always goes through the
//! exact inverse, so the forward side only has to be a good approximation of its inverse.

use std::sync::OnceLock;

use crate::transform::inverse_transform_add;

/// The separable basis of one transform size and type: `u[r]` the vertical and `v[c]` the
/// horizontal unit vectors, `gain[r * n + c]` what the inverse makes of a unit coefficient
/// (r, c) along `u[r] ⊗ v[c]`.
pub struct Basis {
    n: usize,
    u: Vec<f64>,
    v: Vec<f64>,
    gain: Vec<f64>,
}

/// The inverse of coefficient (`r`, `c`) at amplitude `a`, as an n × n image (12-bit samples
/// around mid-grey, so that the measurement does not clip).
fn image(tx_size: u8, tx_type: u8, r: usize, c: usize, a: i32) -> Vec<f64> {
    let n = 4usize << tx_size;
    let mut coefs = vec![0i32; n * n];
    if let Some(x) = coefs.get_mut(r * n + c) {
        *x = a;
    }
    let mut dst = vec![2048u16; n * n];
    inverse_transform_add(&coefs, tx_size, tx_type, false, n * n, n, 12, &mut dst, n);
    dst.iter().map(|&s| f64::from(s) - 2048.0).collect()
}

/// `image` at an amplitude that fills about half of the 12-bit range (precision of the
/// measurement), scaled back to amplitude 1.
fn unit_image(tx_size: u8, tx_type: u8, r: usize, c: usize) -> Vec<f64> {
    let probe = 1 << 12;
    let peak = image(tx_size, tx_type, r, c, probe).iter().fold(0f64, |m, v| m.max(v.abs())).max(1.0);
    let a = ((f64::from(probe) * 1000.0 / peak) as i32).clamp(1, 1 << 22);
    image(tx_size, tx_type, r, c, a).into_iter().map(|v| v / f64::from(a)).collect()
}

fn normalized(mut v: Vec<f64>) -> Vec<f64> {
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

impl Basis {
    fn measure(tx_size: u8, tx_type: u8) -> Basis {
        let n = 4usize << tx_size;
        let mut u = Vec::with_capacity(n * n);
        let mut v = Vec::with_capacity(n * n);
        for k in 0..n {
            // coefficient (k, 0) gives u[k] ⊗ v[0]: summing over x leaves u[k] (v[0] has a
            // non-zero sum for both the DCT and the ADST)
            let col = unit_image(tx_size, tx_type, k, 0);
            u.extend(normalized((0..n).map(|y| (0..n).map(|x| col[y * n + x]).sum()).collect()));
            let row = unit_image(tx_size, tx_type, 0, k);
            v.extend(normalized((0..n).map(|x| (0..n).map(|y| row[y * n + x]).sum()).collect()));
        }
        let mut gain = vec![0f64; n * n];
        for r in 0..n {
            for c in 0..n {
                let img = unit_image(tx_size, tx_type, r, c);
                let g: f64 = (0..n).flat_map(|y| (0..n).map(move |x| (y, x))).map(|(y, x)| img[y * n + x] * u[r * n + y] * v[c * n + x]).sum();
                gain[r * n + c] = if g.abs() < 1e-9 { 1.0 } else { g };
            }
        }
        Basis { n, u, v, gain }
    }

    /// The coefficients (row-major, in the dequantized domain the inverse reads) whose inverse
    /// best approximates `res` (n × n, row-major).
    pub fn forward(&self, res: &[i32], out: &mut [f64]) {
        let n = self.n;
        let mut t = [0f64; 32 * 32];
        for r in 0..n {
            for x in 0..n {
                let mut s = 0.0;
                for y in 0..n {
                    s += self.u[r * n + y] * f64::from(res.get(y * n + x).copied().unwrap_or(0));
                }
                t[r * n + x] = s;
            }
        }
        for r in 0..n {
            for c in 0..n {
                let mut s = 0.0;
                for x in 0..n {
                    s += t[r * n + x] * self.v[c * n + x];
                }
                if let Some(o) = out.get_mut(r * n + c) {
                    *o = s / self.gain[r * n + c];
                }
            }
        }
    }
}

/// The basis of transform size `tx_size` (0..3) and type `tx_type` (0..3), measured on first use.
pub fn basis(tx_size: u8, tx_type: u8) -> &'static Basis {
    static ALL: OnceLock<Vec<Basis>> = OnceLock::new();
    let all = ALL.get_or_init(|| (0..16u8).map(|i| Basis::measure(i / 4, if i / 4 == 3 { 0 } else { i % 4 })).collect());
    let i = (tx_size.min(3) as usize) * 4 + tx_type.min(3) as usize;
    &all[i.min(all.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A residual through the forward transform and back through the decoder's inverse comes
    /// back to within rounding.
    #[test]
    fn forward_then_inverse_reproduces_the_residual() {
        for tx_size in 0..4u8 {
            for tx_type in 0..if tx_size == 3 { 1 } else { 4 } {
                let n = 4usize << tx_size;
                let res: Vec<i32> = (0..n * n).map(|i| (((i * 37) % 61) as i32 - 30) + ((i / n) as i32 - (i % n) as i32)).collect();
                let mut f = vec![0f64; n * n];
                basis(tx_size, tx_type).forward(&res, &mut f);
                let coefs: Vec<i32> = f.iter().map(|v| v.round() as i32).collect();
                let mut dst = vec![128u16; n * n];
                inverse_transform_add(&coefs, tx_size, tx_type, false, n * n, n, 8, &mut dst, n);
                let worst = dst.iter().zip(&res).map(|(&d, &r)| (i32::from(d) - 128 - r).abs()).max().unwrap_or(0);
                assert!(worst <= 2, "tx {tx_size} type {tx_type}: worst error {worst}");
            }
        }
    }
}
