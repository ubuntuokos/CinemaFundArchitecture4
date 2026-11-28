//! Procedural green-screen test material with a known true matte (CFA4-GREENSCREEN-MATTING-INTEGRATION-001, Z0).
//!
//! Everything here is generated from formulas and a fixed-seed generator: there is no external footage, so there
//! is no licence question, and every run produces the same pixels. The pictures are **synthetic**: results on them
//! compare keyers with each other; they are not a prediction of quality on camera footage.

use crate::effects::dec;
use crate::image::Image;

pub const W: usize = 192;
pub const H: usize = 128;
pub const FPS: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    /// Soft-edged disc, even screen, strong green spill on the subject.
    Disc,
    /// Same, screen brightness varies ±25 % across the frame.
    Uneven,
    /// Same, 2 % sensor noise (a different noise pattern each frame).
    Noisy,
    /// Wavy anti-aliased strands 0.6–1.6 px wide (hair), spill on them.
    Hair,
    /// A translucent blob (smoke / glass): true alpha at most 0.6.
    Smoke,
    /// Disc and hair strands move from frame to frame, with noise.
    Motion,
}

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    /// Uniform in [-0.5, 0.5).
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    }
}

/// True alpha of the scene at pixel `(x, y)` in frame `f`.
fn truth_alpha(scene: Scene, x: usize, y: usize, f: usize) -> f32 {
    let (px, py) = (x as f32, y as f32);
    let t = f as f32 / FPS;
    match scene {
        Scene::Disc | Scene::Uneven | Scene::Noisy => {
            let d = ((px - 96.0).powi(2) + (py - 64.0).powi(2)).sqrt();
            ((46.0 - d) / 6.0).clamp(0.0, 1.0)
        }
        Scene::Smoke => {
            let d2 = (px - 96.0).powi(2) + (py - 64.0).powi(2);
            0.6 * (-d2 / (2.0 * 28.0 * 28.0)).exp()
        }
        Scene::Hair | Scene::Motion => {
            let shift = if scene == Scene::Motion { 12.0 * (std::f32::consts::TAU * t * 0.5).sin() } else { 0.0 };
            let mut a = 0.0f32;
            if (16.0..112.0).contains(&py) {
                for i in 0..36 {
                    let x0 = 30.0 + i as f32 * 3.7 + shift;
                    let width = 0.6 + 1.0 * ((i * 7 % 10) as f32 / 10.0);
                    let xc = x0 + 4.0 * (py * 0.09 + i as f32 * 1.3 + t * 2.0).sin();
                    a = a.max(((width * 0.5 + 0.5) - (px - xc).abs()).clamp(0.0, 1.0));
                }
            }
            if scene == Scene::Motion {
                let cx = 96.0 + 20.0 * (std::f32::consts::TAU * t * 0.5).sin();
                let d = ((px - cx).powi(2) + (py - 64.0).powi(2)).sqrt();
                a = a.max(((26.0 - d) / 5.0).clamp(0.0, 1.0));
            }
            a
        }
    }
}

/// One frame: the picture (linear premultiplied RGBA, alpha 1) and the true alpha of the foreground.
pub fn frame(scene: Scene, f: usize) -> (Image, Vec<f32>) {
    let mut rng = Rng::new(0x5EED + f as u64 * 7919);
    let noisy = matches!(scene, Scene::Noisy | Scene::Motion);
    let fg = [0.45f32, 0.40, 0.38]; // muted subject, so that spill makes green its dominant channel
    let mut truth = vec![0.0f32; W * H];
    let mut img = Image::new(W, H);
    for y in 0..H {
        for x in 0..W {
            let a = truth_alpha(scene, x, y, f);
            let k = if scene == Scene::Uneven { 0.75 + 0.5 * (x as f32 / W as f32) } else { 1.0 };
            let screen = [0.05 * k, 0.72 * k, 0.18 * k];
            // spill: the subject picks up green, most at its rim and on thin strands
            let rim = match scene {
                Scene::Hair | Scene::Motion => 1.0,
                _ => 1.0 - ((46.0 - ((x as f32 - 96.0).powi(2) + (y as f32 - 64.0).powi(2)).sqrt()) / 30.0).clamp(0.0, 1.0),
            };
            let spill = if a > 0.0 && scene != Scene::Smoke { 0.30 * rim } else { 0.0 };
            let mut c = [0.0f32; 3];
            for i in 0..3 {
                let fv = if i == 1 { fg[i] + spill } else { fg[i] - spill * 0.2 };
                c[i] = a * fv + (1.0 - a) * screen[i];
            }
            if noisy {
                for v in &mut c {
                    *v += rng.next() * 0.04;
                }
            }
            let l = dec([c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]);
            let o = (y * W + x) * 4;
            img.px[o..o + 4].copy_from_slice(&[l[0], l[1], l[2], 1.0]);
            truth[y * W + x] = a;
        }
    }
    (img, truth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_deterministic_and_the_truth_is_a_probability() {
        for sc in [Scene::Disc, Scene::Uneven, Scene::Noisy, Scene::Hair, Scene::Smoke, Scene::Motion] {
            let (a, ta) = frame(sc, 3);
            let (b, tb) = frame(sc, 3);
            assert_eq!(a.px, b.px, "{sc:?} must be reproducible");
            assert_eq!(ta, tb);
            assert!(ta.iter().all(|v| (0.0..=1.0).contains(v)), "{sc:?}: truth outside 0..1");
            assert!(a.px.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn the_scenes_really_contain_partial_alpha_and_motion() {
        let (_, hair) = frame(Scene::Hair, 0);
        assert!(hair.iter().filter(|&&v| v > 0.05 && v < 0.95).count() > 200, "hair needs anti-aliased pixels");
        let (_, smoke) = frame(Scene::Smoke, 0);
        assert!(smoke.iter().cloned().fold(0.0, f32::max) <= 0.6 + 1e-6);
        let (_, m0) = frame(Scene::Motion, 0);
        let (_, m6) = frame(Scene::Motion, 6);
        assert!(m0.iter().zip(&m6).any(|(a, b)| (a - b).abs() > 0.5), "the motion scene must move");
        let (n0, _) = frame(Scene::Noisy, 0);
        let (n1, _) = frame(Scene::Noisy, 1);
        assert_ne!(n0.px, n1.px, "noise must differ from frame to frame");
    }
}
