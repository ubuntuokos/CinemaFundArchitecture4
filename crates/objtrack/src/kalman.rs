//! Constant-velocity Kalman filter over a box, as in SORT (Bewley et al., 2016).
//!
//! State `[cx, cy, s, r, vcx, vcy, vs]`: centre, area `s = w·h`, aspect ratio `r = w/h` (assumed
//! constant) and the velocities of the first three. Measurement `[cx, cy, s, r]`. One step is one
//! frame (dt = 1).

use cfa4_geom::Rect;

use crate::is_valid_box;

type Vec7 = [f64; 7];
type Mat7 = [[f64; 7]; 7];

const PROCESS_NOISE: [f64; 7] = [1.0, 1.0, 1.0, 1.0, 0.01, 0.01, 0.0001];
const MEASURE_NOISE: [f64; 4] = [1.0, 1.0, 10.0, 10.0];
const INITIAL_VARIANCE: [f64; 7] = [10.0, 10.0, 10.0, 10.0, 10_000.0, 10_000.0, 10_000.0];

/// `[cx, cy, s, r]` of a box, or `None` for an invalid one.
pub fn to_measurement(b: &Rect) -> Option<[f64; 4]> {
    if !is_valid_box(b) {
        return None;
    }
    let (s, r) = (b.w * b.h, b.w / b.h);
    let m = [b.x + b.w / 2.0, b.y + b.h / 2.0, s, r];
    m.iter().all(|v| v.is_finite()).then_some(m).filter(|m| m[2] > 0.0 && m[3] > 0.0)
}

/// The box of a state, or `None` when the state no longer describes one.
pub fn to_box(cx: f64, cy: f64, s: f64, r: f64) -> Option<Rect> {
    if !(s.is_finite() && r.is_finite() && cx.is_finite() && cy.is_finite()) || s <= 0.0 || r <= 0.0 {
        return None;
    }
    let w = (s * r).sqrt();
    let h = s / w;
    let b = Rect::new(cx - w / 2.0, cy - h / 2.0, w, h);
    is_valid_box(&b).then_some(b)
}

/// The filter of one tracked box.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxKalman {
    x: Vec7,
    p: Mat7,
}

impl BoxKalman {
    /// Start from a first measurement; `None` when the box is invalid.
    pub fn new(b: &Rect) -> Option<Self> {
        let z = to_measurement(b)?;
        let mut p = [[0.0; 7]; 7];
        for (i, row) in p.iter_mut().enumerate() {
            row[i] = INITIAL_VARIANCE[i];
        }
        Some(Self { x: [z[0], z[1], z[2], z[3], 0.0, 0.0, 0.0], p })
    }

    /// Advance one frame and return the predicted box (`None` if the state has degenerated).
    pub fn predict(&mut self) -> Option<Rect> {
        if self.x[6] + self.x[2] <= 0.0 {
            self.x[6] = 0.0; // the area may not go negative
        }
        for i in 0..3 {
            self.x[i] += self.x[i + 4];
        }
        // P = F P Fᵀ + Q with F = I + shift(i → i+4) for the first three rows
        let mut fp = self.p;
        for i in 0..3 {
            for j in 0..7 {
                fp[i][j] += self.p[i + 4][j];
            }
        }
        let mut np = fp;
        for row in np.iter_mut().zip(&fp) {
            let (out, src) = row;
            for i in 0..3 {
                out[i] = src[i] + src[i + 4];
            }
        }
        for (i, row) in np.iter_mut().enumerate() {
            row[i] += PROCESS_NOISE[i];
        }
        self.p = np;
        self.current()
    }

    /// Correct the state with a measured box. An invalid box or a singular innovation leaves it as is.
    pub fn update(&mut self, b: &Rect) {
        let Some(z) = to_measurement(b) else { return };
        // innovation y = z − Hx, S = H P Hᵀ + R (4×4), K = P Hᵀ S⁻¹ (7×4)
        let y: [f64; 4] = std::array::from_fn(|i| z[i] - self.x[i]);
        let mut s = [[0.0; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                s[i][j] = self.p[i][j] + if i == j { MEASURE_NOISE[i] } else { 0.0 };
            }
        }
        let Some(si) = invert4(&s) else { return };
        let mut k = [[0.0; 4]; 7];
        for i in 0..7 {
            for j in 0..4 {
                k[i][j] = (0..4).map(|l| self.p[i][l] * si[l][j]).sum();
            }
        }
        let mut nx = self.x;
        for i in 0..7 {
            nx[i] += (0..4).map(|j| k[i][j] * y[j]).sum::<f64>();
        }
        // P = (I − K H) P
        let mut np = self.p;
        for i in 0..7 {
            for j in 0..7 {
                np[i][j] -= (0..4).map(|l| k[i][l] * self.p[l][j]).sum::<f64>();
            }
        }
        if nx.iter().all(|v| v.is_finite()) && np.iter().flatten().all(|v| v.is_finite()) {
            self.x = nx;
            self.p = np;
        }
    }

    /// The box of the current state.
    pub fn current(&self) -> Option<Rect> {
        to_box(self.x[0], self.x[1], self.x[2], self.x[3])
    }
}

/// Inverse of a 4×4 matrix by Gauss–Jordan elimination with partial pivoting; `None` if singular.
fn invert4(m: &[[f64; 4]; 4]) -> Option<[[f64; 4]; 4]> {
    let mut a = *m;
    let mut inv = [[0.0; 4]; 4];
    for (i, row) in inv.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for col in 0..4 {
        let piv = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        let pivot = a[piv][col].abs();
        if pivot.is_nan() || pivot <= 1e-12 {
            return None;
        }
        a.swap(col, piv);
        inv.swap(col, piv);
        let d = a[col][col];
        for j in 0..4 {
            a[col][j] /= d;
            inv[col][j] /= d;
        }
        for i in 0..4 {
            if i != col {
                let f = a[i][col];
                for j in 0..4 {
                    a[i][j] -= f * a[col][j];
                    inv[i][j] -= f * inv[col][j];
                }
            }
        }
    }
    inv.iter().flatten().all(|v| v.is_finite()).then_some(inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_round_trips_through_a_box() {
        let b = Rect::new(10.0, 20.0, 30.0, 15.0);
        let z = to_measurement(&b).unwrap();
        let back = to_box(z[0], z[1], z[2], z[3]).unwrap();
        for (a, c) in [(b.x, back.x), (b.y, back.y), (b.w, back.w), (b.h, back.h)] {
            assert!((a - c).abs() < 1e-9, "{b:?} vs {back:?}");
        }
        assert!(to_measurement(&Rect::new(0.0, 0.0, 0.0, 5.0)).is_none());
        assert!(to_box(-1.0, 0.0, -4.0, 1.0).is_none());
        assert!(BoxKalman::new(&Rect::new(f64::NAN, 0.0, 1.0, 1.0)).is_none());
    }

    #[test]
    fn invert4_inverts() {
        let m = [[4.0, 1.0, 0.0, 0.0], [1.0, 3.0, 0.5, 0.0], [0.0, 0.5, 2.0, 0.1], [0.0, 0.0, 0.1, 5.0]];
        let inv = invert4(&m).unwrap();
        for i in 0..4 {
            for j in 0..4 {
                let v: f64 = (0..4).map(|k| m[i][k] * inv[k][j]).sum();
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
        assert!(invert4(&[[0.0; 4]; 4]).is_none());
    }

    #[test]
    fn follows_constant_motion_and_predicts_ahead() {
        let box_at = |t: f64| Rect::new(100.0 + 5.0 * t, 50.0 + 2.0 * t, 40.0, 30.0);
        let mut k = BoxKalman::new(&box_at(0.0)).unwrap();
        for t in 1..=30 {
            k.predict().unwrap();
            k.update(&box_at(t as f64));
        }
        // with no measurement the box keeps moving the way it was
        let guess = k.predict().unwrap();
        let want = box_at(31.0);
        assert!((guess.x - want.x).abs() < 1.5 && (guess.y - want.y).abs() < 1.5, "{guess:?} vs {want:?}");
        assert!((guess.w - want.w).abs() < 1.5 && (guess.h - want.h).abs() < 1.5);
    }

    #[test]
    fn hostile_updates_leave_the_state_usable() {
        let mut k = BoxKalman::new(&Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        let before = k.clone();
        k.update(&Rect::new(f64::NAN, 0.0, 10.0, 10.0));
        k.update(&Rect::new(0.0, 0.0, -3.0, 10.0));
        assert_eq!(k, before);
        k.update(&Rect::new(1e300, 1e300, 1e300, 1e300));
        for _ in 0..10 {
            let _ = k.predict();
            k.update(&Rect::new(1e-300, 0.0, 1e-300, 1e-300));
        }
        // whatever it holds, asking for the box never panics
        let _ = k.current();
    }
}
