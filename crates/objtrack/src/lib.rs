//! Multi-object box tracking: IoU, Kalman filter, Hungarian assignment and the SORT tracker.
//! See the crate README.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod hungarian;
pub mod kalman;
pub mod sort;

use cfa4_geom::Rect;

/// A box is usable when every number is finite and it has an area.
pub fn is_valid_box(r: &Rect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite() && r.w > 0.0 && r.h > 0.0
}

/// Intersection over union of two boxes; 0 for disjoint or invalid boxes.
pub fn iou(a: &Rect, b: &Rect) -> f64 {
    if !is_valid_box(a) || !is_valid_box(b) {
        return 0.0;
    }
    let iw = (a.right().min(b.right()) - a.x.max(b.x)).max(0.0);
    let ih = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0);
    let inter = iw * ih;
    let union = a.w * a.h + b.w * b.h - inter;
    if union > 0.0 && union.is_finite() { (inter / union).clamp(0.0, 1.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iou_basics() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(iou(&a, &a), 1.0);
        assert_eq!(iou(&a, &Rect::new(20.0, 0.0, 5.0, 5.0)), 0.0);
        // half overlap: inter 50, union 150
        assert!((iou(&a, &Rect::new(5.0, 0.0, 10.0, 10.0)) - 50.0 / 150.0).abs() < 1e-12);
        // touching edges do not overlap
        assert_eq!(iou(&a, &Rect::new(10.0, 0.0, 10.0, 10.0)), 0.0);
        // symmetric
        let b = Rect::new(3.0, 4.0, 8.0, 6.0);
        assert_eq!(iou(&a, &b), iou(&b, &a));
    }

    #[test]
    fn iou_ignores_invalid_boxes() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        for bad in [Rect::new(f64::NAN, 0.0, 1.0, 1.0), Rect::new(0.0, 0.0, -1.0, 5.0), Rect::new(0.0, 0.0, 0.0, 5.0), Rect::new(0.0, f64::INFINITY, 1.0, 1.0)]
        {
            assert_eq!(iou(&a, &bad), 0.0);
            assert_eq!(iou(&bad, &a), 0.0);
            assert!(!is_valid_box(&bad));
        }
    }
}
