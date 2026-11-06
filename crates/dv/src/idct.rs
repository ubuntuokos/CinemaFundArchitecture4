//! Inverse DCTs of DV (IEC 61834-2): the 8-8 DCT and the 2-4-8 DCT of interlaced blocks,
//! both orthonormal after the inverse weighting in `quant`. Separable, in `f32`, with the
//! pixel offset of 128 added after rounding.

use std::sync::OnceLock;

struct Basis {
    /// `b8[k][n] = c(k) cos((2n + 1) k π / 16)`, orthonormal 8-point.
    b8: [[f32; 8]; 8],
    /// `b4[k][n] = c(k) cos((2n + 1) k π / 8)`, orthonormal 4-point.
    b4: [[f32; 4]; 4],
}

fn basis() -> &'static Basis {
    static B: OnceLock<Basis> = OnceLock::new();
    B.get_or_init(|| {
        use std::f64::consts::PI;
        let mut b8 = [[0f32; 8]; 8];
        let mut b4 = [[0f32; 4]; 4];
        for k in 0..8 {
            let c = if k == 0 { (1.0f64 / 8.0).sqrt() } else { 0.5 };
            for n in 0..8 {
                b8[k][n] = (c * ((2 * n + 1) as f64 * k as f64 * PI / 16.0).cos()) as f32;
            }
        }
        for k in 0..4 {
            let c = if k == 0 { 0.5 } else { (0.5f64).sqrt() };
            for n in 0..4 {
                b4[k][n] = (c * ((2 * n + 1) as f64 * k as f64 * PI / 8.0).cos()) as f32;
            }
        }
        Basis { b8, b4 }
    })
}

fn to_pixel(v: f32) -> u8 {
    (v.round() + 128.0).clamp(0.0, 255.0) as u8
}

/// Horizontal pass shared by both modes: `t[row][x] = Σ_h c[row][h] b8[h][x]`.
fn rows(c: &[f32; 64], b8: &[[f32; 8]; 8]) -> [[f32; 8]; 8] {
    let mut t = [[0f32; 8]; 8];
    for (r, out) in t.iter_mut().enumerate() {
        let row = &c[r * 8..r * 8 + 8];
        if row.iter().all(|&v| v == 0.0) {
            continue;
        }
        for (h, &cv) in row.iter().enumerate() {
            if cv != 0.0 {
                for (o, &b) in out.iter_mut().zip(&b8[h]) {
                    *o += cv * b;
                }
            }
        }
    }
    t
}

/// 8-8 IDCT: natural-order coefficients → 8x8 pixels (row-major).
pub(crate) fn idct_88(c: &[f32; 64]) -> [u8; 64] {
    let b = basis();
    let t = rows(c, &b.b8);
    let mut out = [0u8; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0f32;
            for v in 0..8 {
                s += b.b8[v][y] * t[v][x];
            }
            out[y * 8 + x] = to_pixel(s);
        }
    }
    out
}

/// 2-4-8 IDCT: rows 0-3 of `c` are the 4x8 DCT of the field sum, rows 4-7 of the field
/// difference (each scaled so the whole transform is orthonormal). Line `2z` is
/// `(sum + diff) / √2` of field line `z`, line `2z + 1` is `(sum - diff) / √2`.
pub(crate) fn idct_248(c: &[f32; 64]) -> [u8; 64] {
    let b = basis();
    let t = rows(c, &b.b8);
    let r = std::f32::consts::FRAC_1_SQRT_2;
    let mut out = [0u8; 64];
    for z in 0..4 {
        for x in 0..8 {
            let (mut s, mut d) = (0f32, 0f32);
            for v in 0..4 {
                s += b.b4[v][z] * t[v][x];
                d += b.b4[v][z] * t[v + 4][x];
            }
            out[(2 * z) * 8 + x] = to_pixel((s + d) * r);
            out[(2 * z + 1) * 8 + x] = to_pixel((s - d) * r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forward 8-8 DCT by definition, for round trips.
    fn fdct_88(p: &[f32; 64]) -> [f32; 64] {
        let b = basis();
        let mut c = [0f32; 64];
        for v in 0..8 {
            for h in 0..8 {
                let mut s = 0f32;
                for y in 0..8 {
                    for x in 0..8 {
                        s += b.b8[v][y] * b.b8[h][x] * (p[y * 8 + x] - 128.0);
                    }
                }
                c[v * 8 + h] = s;
            }
        }
        c
    }

    /// Forward 2-4-8 DCT by definition.
    fn fdct_248(p: &[f32; 64]) -> [f32; 64] {
        let b = basis();
        let r = std::f32::consts::FRAC_1_SQRT_2;
        let mut c = [0f32; 64];
        for v in 0..4 {
            for h in 0..8 {
                let (mut s, mut d) = (0f32, 0f32);
                for z in 0..4 {
                    for x in 0..8 {
                        let a = p[(2 * z) * 8 + x] - 128.0;
                        let bb = p[(2 * z + 1) * 8 + x] - 128.0;
                        let k = b.b4[v][z] * b.b8[h][x];
                        s += k * (a + bb) * r;
                        d += k * (a - bb) * r;
                    }
                }
                c[v * 8 + h] = s;
                c[(v + 4) * 8 + h] = d;
            }
        }
        c
    }

    fn pattern() -> [f32; 64] {
        let mut p = [0f32; 64];
        for (i, v) in p.iter_mut().enumerate() {
            *v = ((i * 37 + (i / 8) * 11) % 220 + 16) as f32;
        }
        p
    }

    #[test]
    fn round_trip_88() {
        let p = pattern();
        let out = idct_88(&fdct_88(&p));
        for i in 0..64 {
            assert_eq!(out[i] as f32, p[i], "sample {i}");
        }
    }

    #[test]
    fn round_trip_248() {
        let p = pattern();
        let out = idct_248(&fdct_248(&p));
        for i in 0..64 {
            assert_eq!(out[i] as f32, p[i], "sample {i}");
        }
    }

    #[test]
    fn dc_only() {
        let mut c = [0f32; 64];
        c[0] = 8.0 * 50.0; // mean +50
        assert!(idct_88(&c).iter().all(|&v| v == 178));
        assert!(idct_248(&c).iter().all(|&v| v == 178));
    }
}
