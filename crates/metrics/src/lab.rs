//! CIE 1976 L*a*b* (CIELAB) from XYZ and from RGB, D65 reference white.

use cfa4_color::Gamut;

/// A colour in CIELAB (D65 white). `l` is 0..100 for in-gamut colours.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

impl Lab {
    pub const fn new(l: f64, a: f64, b: f64) -> Lab {
        Lab { l, a, b }
    }
}

/// CIE XYZ (white Y = 1) → CIELAB, relative to the D65 white of `cfa4-color`'s gamuts
/// (CIE 15:2004 §8.2.1, with the exact 216/24389 and 24389/27 constants).
pub fn xyz_to_lab(xyz: [f64; 3]) -> Lab {
    let w = d65_white();
    let f = |t: f64| {
        const EPS: f64 = 216.0 / 24389.0;
        const KAPPA: f64 = 24389.0 / 27.0;
        if t > EPS { t.cbrt() } else { (KAPPA * t + 16.0) / 116.0 }
    };
    let (fx, fy, fz) = (f(xyz[0] / w[0]), f(xyz[1] / w[1]), f(xyz[2] / w[2]));
    Lab { l: 116.0 * fy - 16.0, a: 500.0 * (fx - fy), b: 200.0 * (fy - fz) }
}

/// Linear-light RGB in `gamut` (1.0 = reference white) → CIELAB.
pub fn linear_rgb_to_lab(rgb: [f64; 3], gamut: Gamut) -> Lab {
    let m = gamut.to_xyz();
    let row = |r: [f64; 3]| r[0] * rgb[0] + r[1] * rgb[1] + r[2] * rgb[2];
    xyz_to_lab([row(m[0]), row(m[1]), row(m[2])])
}

/// 8-bit sRGB-encoded (IEC 61966-2-1, Rec.709 primaries) → CIELAB.
pub fn srgb8_to_lab(r: u8, g: u8, b: u8) -> Lab {
    linear_rgb_to_lab([srgb_eotf(r), srgb_eotf(g), srgb_eotf(b)], Gamut::Bt709)
}

fn srgb_eotf(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// XYZ of the D65 white (the row sums of the normalised primary matrix).
fn d65_white() -> [f64; 3] {
    let m = Gamut::Bt709.to_xyz();
    [m[0][0] + m[0][1] + m[0][2], m[1][0] + m[1][1] + m[1][2], m[2][0] + m[2][1] + m[2][2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black() {
        let w = srgb8_to_lab(255, 255, 255);
        assert!((w.l - 100.0).abs() < 1e-9 && w.a.abs() < 1e-9 && w.b.abs() < 1e-9, "{w:?}");
        let k = srgb8_to_lab(0, 0, 0);
        assert!(k.l.abs() < 1e-12 && k.a.abs() < 1e-12 && k.b.abs() < 1e-12, "{k:?}");
    }
}
