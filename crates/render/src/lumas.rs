//! The shipped luma masks (plan KDN-21): 18 greyscale PGM images from the Kdenlive data set (CC0-1.0,
//! Massimo Stella, `assets/lumas/PAL/`), embedded in the binary and used by Gradient Wipe's
//! "Luma Mask" choice. Bright pixels switch to the incoming clip last (a pixel is "revealed" when the
//! progress passes its grey value), the same convention as the built-in diagonal gradient.
//!
//! The masks are sampled bilinearly in normalised coordinates, so they fit any frame size.

use std::sync::{Arc, OnceLock};

/// The embedded files, in the order of `cfa4_project::vtransition::LUMA_OPTS` (index + 1).
pub const FILES: &[(&str, &[u8])] = &[
    ("bi-linear_x", include_bytes!("../../../assets/lumas/PAL/bi-linear_x.pgm")),
    ("bi-linear_y", include_bytes!("../../../assets/lumas/PAL/bi-linear_y.pgm")),
    ("burst", include_bytes!("../../../assets/lumas/PAL/burst.pgm")),
    ("checkerboard_small", include_bytes!("../../../assets/lumas/PAL/checkerboard_small.pgm")),
    ("clock", include_bytes!("../../../assets/lumas/PAL/clock.pgm")),
    ("cloud", include_bytes!("../../../assets/lumas/PAL/cloud.pgm")),
    ("curtain", include_bytes!("../../../assets/lumas/PAL/curtain.pgm")),
    ("horizontal_blinds", include_bytes!("../../../assets/lumas/PAL/horizontal_blinds.pgm")),
    ("linear_x", include_bytes!("../../../assets/lumas/PAL/linear_x.pgm")),
    ("linear_y", include_bytes!("../../../assets/lumas/PAL/linear_y.pgm")),
    ("radial", include_bytes!("../../../assets/lumas/PAL/radial.pgm")),
    ("radial-bars", include_bytes!("../../../assets/lumas/PAL/radial-bars.pgm")),
    ("spiral", include_bytes!("../../../assets/lumas/PAL/spiral.pgm")),
    ("spiral2", include_bytes!("../../../assets/lumas/PAL/spiral2.pgm")),
    ("square", include_bytes!("../../../assets/lumas/PAL/square.pgm")),
    ("square2", include_bytes!("../../../assets/lumas/PAL/square2.pgm")),
    ("square2-bars", include_bytes!("../../../assets/lumas/PAL/square2-bars.pgm")),
    ("symmetric_clock", include_bytes!("../../../assets/lumas/PAL/symmetric_clock.pgm")),
];

/// A decoded 8-bit greyscale mask.
pub struct Luma {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Luma {
    /// Parse a binary PGM (`P5`, maxval up to 255; `#` comments in the header are skipped).
    pub fn parse_pgm(b: &[u8]) -> Option<Luma> {
        let mut pos = 0usize;
        let token = |pos: &mut usize| -> Option<String> {
            loop {
                while *pos < b.len() && b[*pos].is_ascii_whitespace() {
                    *pos += 1;
                }
                if *pos < b.len() && b[*pos] == b'#' {
                    while *pos < b.len() && b[*pos] != b'\n' {
                        *pos += 1;
                    }
                } else {
                    break;
                }
            }
            let start = *pos;
            while *pos < b.len() && !b[*pos].is_ascii_whitespace() {
                *pos += 1;
            }
            (*pos > start).then(|| String::from_utf8_lossy(&b[start..*pos]).into_owned())
        };
        if token(&mut pos)? != "P5" {
            return None;
        }
        let width: usize = token(&mut pos)?.parse().ok()?;
        let height: usize = token(&mut pos)?.parse().ok()?;
        let max: usize = token(&mut pos)?.parse().ok()?;
        if width == 0 || height == 0 || width > 16384 || height > 16384 || max == 0 || max > 255 {
            return None;
        }
        pos += 1; // the single whitespace byte after maxval
        let data = b.get(pos..pos.checked_add(width.checked_mul(height)?)?)?;
        let data = if max == 255 { data.to_vec() } else { data.iter().map(|v| ((*v as usize * 255) / max).min(255) as u8).collect() };
        Some(Luma { width, height, data })
    }

    /// Grey value 0..=1 at normalised `(u, v)` (bilinear, clamped at the edges).
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x = (u.clamp(0.0, 1.0) * (self.width - 1) as f32).max(0.0);
        let y = (v.clamp(0.0, 1.0) * (self.height - 1) as f32).max(0.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.data[y * self.width + x] as f32 / 255.0;
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bot = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bot * fy
    }
}

/// The mask for a Luma Mask choice index (0 = built-in diagonal gradient → `None`).
pub fn mask(choice: u32) -> Option<Arc<Luma>> {
    static CACHE: OnceLock<Vec<Option<Arc<Luma>>>> = OnceLock::new();
    let all = CACHE.get_or_init(|| FILES.iter().map(|(_, b)| Luma::parse_pgm(b).map(Arc::new)).collect());
    all.get((choice as usize).checked_sub(1)?)?.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_luma_parses_and_uses_a_range() {
        assert_eq!(FILES.len() + 1, cfa4_project::vtransition::LUMA_OPTS.len());
        for (i, (name, bytes)) in FILES.iter().enumerate() {
            let l = Luma::parse_pgm(bytes).unwrap_or_else(|| panic!("{name} does not parse"));
            assert_eq!(l.data.len(), l.width * l.height, "{name}");
            let (lo, hi) = (l.data.iter().min().unwrap(), l.data.iter().max().unwrap());
            assert!(hi - lo > 100, "{name}: grey range {lo}..{hi} is too flat");
            assert!(mask(i as u32 + 1).is_some(), "{name}");
        }
        assert!(mask(0).is_none() && mask(99).is_none());
    }

    #[test]
    fn labels_follow_the_file_names() {
        let norm = |s: &str| s.to_ascii_lowercase().replace([' ', '-', '_'], "");
        for ((name, _), label) in FILES.iter().zip(cfa4_project::vtransition::LUMA_OPTS.iter().skip(1)) {
            assert_eq!(norm(name), norm(label));
        }
    }

    #[test]
    fn sampling_is_bilinear_and_clamped() {
        let l = Luma { width: 2, height: 1, data: vec![0, 255] };
        assert!((l.sample(0.5, 0.0) - 0.5).abs() < 1e-6);
        assert_eq!(l.sample(-1.0, 3.0), 0.0);
        assert_eq!(l.sample(2.0, 0.0), 1.0);
    }

    #[test]
    fn rejects_bad_pgm() {
        assert!(Luma::parse_pgm(b"P6\n1 1\n255\n\0\0\0").is_none());
        assert!(Luma::parse_pgm(b"P5\n4 4\n255\n\0").is_none());
    }
}
