//! SORT: Simple Online and Realtime Tracking (Bewley et al., 2016), written from the paper.
//!
//! Per frame: predict every track with its Kalman filter, match detections to predicted boxes by IoU
//! with the Hungarian algorithm (a pair needs `iou_threshold`), update the matched tracks, start a
//! track for each unmatched detection and retire tracks unmatched for more than `max_age` frames.
//! A track is reported once it has `min_hits` consecutive hits (the first frames of a clip report
//! every track, as the reference does).

use cfa4_geom::Rect;

use crate::hungarian;
use crate::iou;
use crate::is_valid_box;
use crate::kalman::BoxKalman;

/// Detections per frame beyond this are ignored (the highest scores are kept).
pub const MAX_DETECTIONS: usize = 4096;
/// Live tracks beyond this are not started.
pub const MAX_TRACKS: usize = 4096;

/// One detection (or a user's box) in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    pub bbox: Rect,
    pub score: f32,
}

/// A tracked box reported for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackedBox {
    /// Stable for the life of the track, starting at 1.
    pub id: u64,
    pub bbox: Rect,
    /// Score of the detection that matched this frame.
    pub score: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SortConfig {
    /// Frames a track survives without a match.
    pub max_age: u32,
    /// Consecutive hits before a track is reported.
    pub min_hits: u32,
    /// Smallest IoU for a match.
    pub iou_threshold: f64,
}

impl Default for SortConfig {
    fn default() -> Self {
        Self { max_age: 1, min_hits: 3, iou_threshold: 0.3 }
    }
}

#[derive(Clone, Debug)]
struct Track {
    id: u64,
    kf: BoxKalman,
    score: f32,
    time_since_update: u32,
    hit_streak: u32,
}

/// The tracker. Feed it one frame at a time with [`Sort::update`].
#[derive(Clone, Debug)]
pub struct Sort {
    cfg: SortConfig,
    tracks: Vec<Track>,
    next_id: u64,
    frame: u64,
}

impl Sort {
    pub fn new(cfg: SortConfig) -> Self {
        let cfg = SortConfig { iou_threshold: if cfg.iou_threshold.is_finite() { cfg.iou_threshold.clamp(0.0, 1.0) } else { 0.3 }, ..cfg };
        Self { cfg, tracks: Vec::new(), next_id: 1, frame: 0 }
    }

    /// Tracks currently alive (matched or coasting).
    pub fn live_tracks(&self) -> usize {
        self.tracks.len()
    }

    /// Process one frame of detections and return the tracks to report, ordered by id.
    pub fn update(&mut self, detections: &[Detection]) -> Vec<TrackedBox> {
        self.frame = self.frame.saturating_add(1);
        // usable detections, strongest first when there are too many (stable: ties keep input order)
        let mut dets: Vec<Detection> = detections.iter().copied().filter(|d| is_valid_box(&d.bbox)).collect();
        if dets.len() > MAX_DETECTIONS {
            dets.sort_by(|a, b| b.score.total_cmp(&a.score));
            dets.truncate(MAX_DETECTIONS);
        }
        // predict; a track whose state degenerated is dropped
        let mut predicted: Vec<(usize, Rect)> = Vec::with_capacity(self.tracks.len());
        let mut dead = vec![false; self.tracks.len()];
        for (i, t) in self.tracks.iter_mut().enumerate() {
            match t.kf.predict() {
                Some(b) => predicted.push((i, b)),
                None => dead.get_mut(i).map(|d| *d = true).unwrap_or(()),
            }
        }
        // IoU matching
        let (rows, cols) = (dets.len(), predicted.len());
        let mut det_match: Vec<Option<usize>> = vec![None; rows]; // detection → track index
        if rows > 0 && cols > 0 {
            let cost: Vec<f64> = dets.iter().flat_map(|d| predicted.iter().map(move |(_, p)| -iou(&d.bbox, p))).collect();
            for (di, assigned) in hungarian::assign(&cost, rows, cols).into_iter().enumerate() {
                if let Some(pc) = assigned
                    && let (Some((ti, pbox)), Some(d)) = (predicted.get(pc), dets.get(di))
                    && iou(&d.bbox, pbox) >= self.cfg.iou_threshold
                    && let Some(m) = det_match.get_mut(di)
                {
                    *m = Some(*ti);
                }
            }
        }
        // update matched tracks
        let mut matched = vec![false; self.tracks.len()];
        for (di, m) in det_match.iter().enumerate() {
            if let (Some(ti), Some(d)) = (m, dets.get(di))
                && let Some(t) = self.tracks.get_mut(*ti)
            {
                t.kf.update(&d.bbox);
                t.score = d.score;
                t.time_since_update = 0;
                t.hit_streak = t.hit_streak.saturating_add(1);
                if let Some(f) = matched.get_mut(*ti) {
                    *f = true;
                }
            }
        }
        for (i, t) in self.tracks.iter_mut().enumerate() {
            if !matched.get(i).copied().unwrap_or(false) {
                t.time_since_update = t.time_since_update.saturating_add(1);
                t.hit_streak = 0;
            }
        }
        // new tracks for unmatched detections
        for (di, m) in det_match.iter().enumerate() {
            if m.is_none()
                && self.tracks.len() < MAX_TRACKS
                && let Some(d) = dets.get(di)
                && let Some(kf) = BoxKalman::new(&d.bbox)
            {
                self.tracks.push(Track { id: self.next_id, kf, score: d.score, time_since_update: 0, hit_streak: 1 });
                self.next_id = self.next_id.saturating_add(1);
                dead.push(false);
            }
        }
        // report, then retire
        let mut out = Vec::new();
        for (i, t) in self.tracks.iter().enumerate() {
            let alive = !dead.get(i).copied().unwrap_or(false);
            let reported = alive && t.time_since_update < 1 && (t.hit_streak >= self.cfg.min_hits || self.frame <= u64::from(self.cfg.min_hits));
            if reported && let Some(b) = t.kf.current() {
                out.push(TrackedBox { id: t.id, bbox: b, score: t.score });
            }
        }
        let max_age = self.cfg.max_age;
        let mut i = 0;
        self.tracks.retain(|t| {
            let keep = !dead.get(i).copied().unwrap_or(false) && t.time_since_update <= max_age;
            i += 1;
            keep
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(x: f64, y: f64, w: f64, h: f64) -> Detection {
        Detection { bbox: Rect::new(x, y, w, h), score: 0.9 }
    }

    fn ids(v: &[TrackedBox]) -> Vec<u64> {
        v.iter().map(|t| t.id).collect()
    }

    #[test]
    fn one_moving_object_keeps_its_id() {
        let mut s = Sort::new(SortConfig { min_hits: 1, ..SortConfig::default() });
        let mut seen = std::collections::BTreeSet::new();
        for f in 0..40 {
            let out = s.update(&[det(10.0 + 3.0 * f as f64, 20.0 + f as f64, 40.0, 30.0)]);
            assert_eq!(out.len(), 1, "frame {f}");
            seen.insert(out[0].id);
        }
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn min_hits_delays_a_new_track_after_the_first_frames() {
        let mut s = Sort::new(SortConfig { min_hits: 3, ..SortConfig::default() });
        for f in 0..5 {
            s.update(&[det(0.0, 0.0, 20.0, 20.0)]);
            let _ = f;
        }
        // a second object appears late: reported only after 3 consecutive hits
        let b = |f: f64| det(200.0 + f, 100.0, 20.0, 20.0);
        assert_eq!(ids(&s.update(&[det(0.0, 0.0, 20.0, 20.0), b(0.0)])), vec![1]);
        assert_eq!(ids(&s.update(&[det(0.0, 0.0, 20.0, 20.0), b(1.0)])), vec![1]);
        assert_eq!(ids(&s.update(&[det(0.0, 0.0, 20.0, 20.0), b(2.0)])), vec![1, 2]);
    }

    #[test]
    fn short_occlusion_keeps_the_id_and_a_long_one_does_not() {
        let cfg = SortConfig { max_age: 3, min_hits: 1, iou_threshold: 0.3 };
        let obj = |f: f64| det(10.0 + 2.0 * f, 10.0, 40.0, 40.0);
        let mut s = Sort::new(cfg);
        let first = s.update(&[obj(0.0)])[0].id;
        for f in 1..5 {
            s.update(&[obj(f as f64)]);
        }
        // gone for 3 frames, back where the motion says it is
        for _ in 0..3 {
            assert!(s.update(&[]).is_empty());
        }
        let back = s.update(&[obj(8.0)]);
        assert_eq!(ids(&back), vec![first]);
        // gone for 5 frames: the track is retired, a new id starts
        for _ in 0..5 {
            s.update(&[]);
        }
        assert_eq!(s.live_tracks(), 0);
        let again = s.update(&[obj(14.0)]);
        assert_eq!(again.len(), 1);
        assert_ne!(again[0].id, first);
    }

    #[test]
    fn two_objects_crossing_paths_keep_their_ids() {
        let mut s = Sort::new(SortConfig { min_hits: 1, ..SortConfig::default() });
        // A moves right, B moves left, on different rows so they never overlap
        let a = |f: f64| det(20.0 + 6.0 * f, 40.0, 30.0, 30.0);
        let b = |f: f64| det(300.0 - 6.0 * f, 120.0, 30.0, 30.0);
        let out = s.update(&[a(0.0), b(0.0)]);
        let (ida, idb) = (out[0].id, out[1].id);
        assert_ne!(ida, idb);
        for f in 1..40 {
            let out = s.update(&[a(f as f64), b(f as f64)]);
            assert_eq!(out.len(), 2);
            let ya = out.iter().find(|t| t.id == ida).unwrap();
            assert!(ya.bbox.y < 80.0, "A stays on its row at frame {f}");
            assert!(out.iter().find(|t| t.id == idb).unwrap().bbox.y > 80.0);
        }
    }

    #[test]
    fn output_is_deterministic() {
        let frames: Vec<Vec<Detection>> = (0..30)
            .map(|f| vec![det(f as f64 * 2.0, 5.0, 30.0, 20.0), det(100.0 - f as f64, 60.0, 25.0, 25.0), det(f as f64 * 5.0 % 90.0, 150.0, 10.0, 10.0)])
            .collect();
        let run = || {
            let mut s = Sort::new(SortConfig::default());
            frames.iter().map(|f| s.update(f)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn hostile_detections_are_dropped_or_bounded() {
        let mut s = Sort::new(SortConfig { min_hits: 1, ..SortConfig::default() });
        let bad = [Rect::new(f64::NAN, 0.0, 5.0, 5.0), Rect::new(0.0, 0.0, f64::INFINITY, 5.0), Rect::new(0.0, 0.0, 0.0, 5.0), Rect::new(0.0, 0.0, -5.0, -5.0)];
        let dets: Vec<Detection> = bad.iter().map(|b| Detection { bbox: *b, score: f32::NAN }).collect();
        assert!(s.update(&dets).is_empty());
        assert_eq!(s.live_tracks(), 0);
        // far more detections than allowed: bounded, no panic
        let many: Vec<Detection> = (0..MAX_DETECTIONS + 500).map(|i| det((i % 100) as f64 * 12.0, (i / 100) as f64 * 12.0, 10.0, 10.0)).collect();
        let out = s.update(&many);
        assert!(out.len() <= MAX_DETECTIONS);
        assert!(s.live_tracks() <= MAX_TRACKS);
        // absurd configuration
        let mut s = Sort::new(SortConfig { max_age: u32::MAX, min_hits: u32::MAX, iou_threshold: f64::NAN });
        s.update(&[det(0.0, 0.0, 5.0, 5.0)]);
        s.update(&[]);
    }

    #[test]
    fn random_streams_never_panic() {
        let mut seed = 0xdead_beef_cafe_f00d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut s = Sort::new(SortConfig { max_age: 4, min_hits: 2, iou_threshold: 0.2 });
        for _ in 0..300 {
            let n = (next() % 8) as usize;
            let dets: Vec<Detection> = (0..n)
                .map(|_| {
                    let mut pick = |m: u64| (next() % m) as f64 - (m / 4) as f64;
                    det(pick(400), pick(400), pick(60).abs() + 0.001, pick(60).abs() + 0.001)
                })
                .collect();
            let out = s.update(&dets);
            assert!(out.iter().all(|t| is_valid_box(&t.bbox)));
            assert!(out.windows(2).all(|w| w[0].id < w[1].id), "ordered by id");
        }
    }
}
