//! Long feature trajectories through a clip: the shared analysis of the Subspace Warp stabilizer.
//!
//! Points are found with the Shi–Tomasi detector and followed frame to frame with the pyramidal
//! Lucas–Kanade tracker (forward–backward checked), both from [`crate::track`]. A point that is
//! lost ends its trajectory; new points are seeded where the picture has too few live ones, away
//! from the existing points, so the whole frame stays covered while the clip plays. The result is
//! a set of `(start frame, positions)` tracks, the input of the subspace factorization
//! (Liu et al., "Subspace Video Stabilization", ACM TOG 2011).
//!
//! The pass is sequential over frames (a track needs its previous position) and parallel over
//! points inside a frame, so it never starves other work and checks the cancel flag every frame.

use crate::track::{Gray, Prepared, features, track_points};
use cfa4_geom::Vec2;
use rayon::prelude::*;

/// Live points the analysis aims for.
pub const TARGET_POINTS: usize = 400;
/// Seed new points when fewer than this share of [`TARGET_POINTS`] is alive.
const RESEED_BELOW: f64 = 0.8;
/// New points keep at least this far (pixels of the analysis frame) from live ones.
const SPACING: f64 = 8.0;
/// Features closer than this to the frame border are not seeded (the tracker window needs room).
const MARGIN: f64 = 10.0;

/// One feature followed through the clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Frame of `pts[0]`.
    pub start: usize,
    /// Position in each frame from `start` on (pixels of the analysis frame).
    pub pts: Vec<Vec2>,
}

impl Track {
    /// Frame after the last position.
    pub fn end(&self) -> usize {
        self.start + self.pts.len()
    }

    /// Position in `frame`, when the track is alive there.
    pub fn at(&self, frame: usize) -> Option<Vec2> {
        frame.checked_sub(self.start).and_then(|i| self.pts.get(i)).copied()
    }
}

/// All trajectories of a clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trajectories {
    /// Analysis frame size.
    pub w: usize,
    pub h: usize,
    pub frames: usize,
    pub tracks: Vec<Track>,
}

impl Trajectories {
    /// How many tracks are alive in `frame`.
    pub fn alive(&self, frame: usize) -> usize {
        self.tracks.iter().filter(|t| t.at(frame).is_some()).count()
    }

    /// The tracks alive in every frame of `from..to`.
    pub fn spanning(&self, from: usize, to: usize) -> Vec<&Track> {
        self.tracks.iter().filter(|t| t.start <= from && t.end() >= to).collect()
    }
}

/// Follow features through `frames` (`None` = a frame that could not be decoded: every live
/// track ends there). `cancelled` is polled once per frame; a cancelled analysis returns `None`.
pub fn analyse(frames: &[Option<Prepared>], cancelled: &dyn Fn() -> bool) -> Option<Trajectories> {
    let (w, h) = frames.iter().flatten().next().map(|p| (p.base().w, p.base().h))?;
    let region = [
        Vec2::new(MARGIN, MARGIN),
        Vec2::new(w as f64 - MARGIN, MARGIN),
        Vec2::new(w as f64 - MARGIN, h as f64 - MARGIN),
        Vec2::new(MARGIN, h as f64 - MARGIN),
    ];
    let mut done: Vec<Track> = Vec::new();
    // the tracks still alive in the previous frame
    let mut open: Vec<Track> = Vec::new();
    for (k, frame) in frames.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let Some(cur) = frame else {
            done.append(&mut open);
            continue;
        };
        // move the open tracks into this frame
        if k > 0
            && !open.is_empty()
            && let Some(prev) = &frames[k - 1]
        {
            let at: Vec<Vec2> = open.iter().filter_map(|t| t.pts.last().copied()).collect();
            let next = track_points(prev, cur, &at);
            let mut still = Vec::with_capacity(open.len());
            for (mut t, n) in open.drain(..).zip(next) {
                let inside = |q: Vec2| q.x >= MARGIN && q.y >= MARGIN && q.x <= w as f64 - MARGIN && q.y <= h as f64 - MARGIN;
                match n.filter(|&q| inside(q)) {
                    Some(q) => {
                        t.pts.push(q);
                        still.push(t);
                    }
                    None => done.push(t),
                }
            }
            open = still;
        } else if k > 0 {
            done.append(&mut open);
        }
        // seed where the frame has too few live points
        if (open.len() as f64) < TARGET_POINTS as f64 * RESEED_BELOW {
            let live: Vec<Vec2> = open.iter().filter_map(|t| t.pts.last().copied()).collect();
            let want = TARGET_POINTS - open.len();
            let cands = features(cur.base(), &region, TARGET_POINTS * 3, 5.0);
            let mut taken = live;
            for c in cands {
                if taken.len() >= TARGET_POINTS {
                    break;
                }
                let p = Vec2::new(c[0] as f64, c[1] as f64);
                if taken.iter().all(|q| (q.x - p.x).hypot(q.y - p.y) >= SPACING) {
                    taken.push(p);
                    open.push(Track { start: k, pts: vec![p] });
                    if open.len() >= TARGET_POINTS || taken.len() >= TARGET_POINTS || want == 0 {
                        break;
                    }
                }
            }
        }
    }
    done.append(&mut open);
    // a single position is not a trajectory
    done.retain(|t| t.pts.len() >= 2);
    done.sort_by_key(|t| (t.start, t.end()));
    Some(Trajectories { w, h, frames: frames.len(), tracks: done })
}

/// The analysis frames of a clip: grayscale pyramids built in parallel.
pub fn prepare(frames: Vec<Option<Gray>>) -> Vec<Option<Prepared>> {
    frames.into_par_iter().map(|g| g.map(Prepared::new)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_geom::Affine;

    fn texture(u: f64, v: f64) -> f32 {
        let a = (u * 0.21).sin() * (v * 0.17).cos() + (u * 0.07 + v * 0.11).sin() * 0.7 + ((u * 0.43).sin() * (v * 0.39).sin()) * 0.5;
        (0.5 + 0.2 * a) as f32
    }

    /// A textured plane under the per-frame pose `pose(k)` (maps scene to frame pixels).
    fn frame(w: usize, h: usize, pose: &Affine) -> Gray {
        let inv = pose.inverse().unwrap();
        let mut px = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let s = inv.apply(Vec2::new(x as f64, y as f64));
                px[y * w + x] = texture(s.x, s.y);
            }
        }
        Gray::new(w, h, px)
    }

    fn pose(k: usize) -> Affine {
        let t = k as f64;
        Affine::translate(1.3 * t + 2.0 * (t * 0.7).sin(), -0.8 * t + 1.5 * (t * 0.5).cos())
    }

    fn clip(n: usize) -> Vec<Option<Prepared>> {
        prepare((0..n).map(|k| Some(frame(160, 120, &pose(k)))).collect())
    }

    #[test]
    fn tracks_follow_the_picture_and_stay_long() {
        let frames = clip(24);
        let tr = analyse(&frames, &|| false).unwrap();
        assert_eq!((tr.w, tr.h, tr.frames), (160, 120, 24));
        assert!(tr.alive(0) >= 50, "seeded {}", tr.alive(0));
        // every track keeps its scene point: back-projected positions agree over its whole life
        let mut worst = 0.0f64;
        for t in &tr.tracks {
            let s0 = pose(t.start).inverse().unwrap().apply(t.pts[0]);
            for (i, p) in t.pts.iter().enumerate() {
                let s = pose(t.start + i).inverse().unwrap().apply(*p);
                worst = worst.max((s.x - s0.x).hypot(s.y - s0.y));
            }
        }
        assert!(worst < 1.0, "worst drift {worst}");
        // the picture moves ~1.5 px/frame, so tracks that start at 0 mostly survive to the middle
        assert!(tr.spanning(0, 12).len() >= 20, "{} long tracks", tr.spanning(0, 12).len());
        // coverage is kept up to the last frame
        assert!(tr.alive(23) >= 50, "{} alive at the end", tr.alive(23));
    }

    #[test]
    fn missing_frames_end_tracks_and_a_cancel_stops_the_analysis() {
        let mut frames = clip(10);
        frames[5] = None;
        let tr = analyse(&frames, &|| false).unwrap();
        assert!(tr.tracks.iter().all(|t| t.at(5).is_none()));
        assert!(tr.alive(4) > 0 && tr.alive(6) > 0);
        assert!(analyse(&frames, &|| true).is_none());
        assert!(analyse(&[None, None], &|| false).is_none());
    }

    #[test]
    fn a_featureless_clip_has_no_tracks() {
        let flat = (0..6).map(|_| Some(Prepared::new(Gray::new(64, 64, vec![0.5; 64 * 64])))).collect::<Vec<_>>();
        let tr = analyse(&flat, &|| false).unwrap();
        assert!(tr.tracks.is_empty());
    }
}
