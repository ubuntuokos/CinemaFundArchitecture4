// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, src/near_field.{h,c}).

//! The near-field filter of one ear: how a rigid sphere of the head's radius (Duda and Martens,
//! 1998) changes the sound of a source at one distance against a source at the distance the HRTF
//! set was measured at, as a gain and a first-order high shelf. Only the head's effect: the 1/r
//! change of pressure is the caller's distance attenuation. The table's parameters are
//! interpolated bilinearly in angle and inverse distance, the set's distance taken out, and the
//! analog shelf made digital by the bilinear transform, warped to keep its corner.

use crate::near_field_table::{ANGLES, DISTANCES, INVERSE_DISTANCES, TABLE};

const ANGLE_STEP: f32 = 5.0;
const SPEED_OF_SOUND: f32 = 343.0;
/// The bilinear transform's corner stays below the Nyquist frequency.
const MAX_HALF_ANGLE: f32 = 1.55;

/// `y[n] = b0 x[n] + b1 x[n - 1] - a1 y[n - 1]`; |a1| < 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearFieldFilter {
    pub b0: f32,
    pub b1: f32,
    pub a1: f32,
}

impl NearFieldFilter {
    /// The filter that passes its input unchanged.
    pub const IDENTITY: NearFieldFilter = NearFieldFilter { b0: 1.0, b1: 0.0, a1: 0.0 };
}

/// The table row below a value in a rising axis, and how far the value lies towards the next one.
fn below(axis: &[f32], value: f32) -> (usize, f32) {
    let mut index = 0;
    while index + 2 < axis.len() && axis[index + 1] <= value {
        index += 1;
    }
    (index, (value - axis[index]) / (axis[index + 1] - axis[index]))
}

fn lookup(angle_degrees: f32, inverse_distance: f32) -> [f32; 3] {
    let angle = angle_degrees.clamp(0.0, 180.0) / ANGLE_STEP;
    let i = (angle as usize).min(ANGLES - 2);
    let fa = angle - i as f32;
    let inverse = inverse_distance.clamp(0.0, INVERSE_DISTANCES[DISTANCES - 1]);
    let (j, fd) = below(&INVERSE_DISTANCES, inverse);
    let weights = [(1.0 - fa) * (1.0 - fd), (1.0 - fa) * fd, fa * (1.0 - fd), fa * fd];
    let corners = [TABLE[i][j], TABLE[i][j + 1], TABLE[i + 1][j], TABLE[i + 1][j + 1]];
    let mut sum = [0.0f32; 3];
    for (w, c) in weights.iter().zip(corners) {
        for k in 0..3 {
            sum[k] += w * c[k];
        }
    }
    sum
}

/// The filter for an incidence angle in degrees (the angle at the head's centre between the ear
/// and the source, 0 to 180), the source's and the set's distances as the head radius over the
/// distance (0 for infinity; past 1 / 1.15 counts as 1 / 1.15), a head radius in metres and a
/// sample rate. Equal distances give the identity.
pub fn near_field(angle_degrees: f32, inverse_distance: f32, set_inverse_distance: f32, head_radius: f32, sample_rate: f32) -> NearFieldFilter {
    let angle_degrees = if angle_degrees.is_finite() { angle_degrees } else { 90.0 };
    let source = lookup(angle_degrees, if inverse_distance.is_finite() { inverse_distance } else { 0.0 });
    let set = lookup(angle_degrees, if set_inverse_distance.is_finite() { set_inverse_distance } else { 0.0 });
    let gain = 10f32.powf((source[0] - set[0]) / 20.0);
    let high = 10f32.powf((source[1] - set[1]) / 20.0);
    let corner = source[2].exp() * SPEED_OF_SOUND / head_radius;
    let k = (corner / (2.0 * sample_rate)).clamp(0.0, MAX_HALF_ANGLE).tan();
    NearFieldFilter { b0: gain * (high + k) / (1.0 + k), b1: gain * (k - high) / (1.0 + k), a1: (k - 1.0) / (1.0 + k) }
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length == 0.0 || !length.is_finite() {
        return [0.0, 0.0, -1.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

/// The direction, from the head's centre, at which an ear's response is looked up for a source at
/// `position` (metres, listener's frame): where the ray from the ear, at `ear_x` on the x axis,
/// through the source meets the sphere the set was measured on (parallax). Unit length. A source
/// at the ear, or an ear outside the sphere, looks up the source's own direction; a zero position,
/// straight ahead.
pub fn ear_direction(position: [f32; 3], ear_x: f32, set_distance: f32) -> [f32; 3] {
    let ray = [position[0] - ear_x, position[1], position[2]];
    let length = (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt();
    if length == 0.0 || ear_x.abs() >= set_distance {
        return normalized(position);
    }
    let u = [ray[0] / length, ray[1] / length, ray[2] / length];
    // |ear + t u| = R: t^2 + 2 t (ear . u) + ear^2 - R^2 = 0, the positive root (ear inside).
    let along = ear_x * u[0];
    let t = -along + (along * along - ear_x * ear_x + set_distance * set_distance).sqrt();
    normalized([ear_x + t * u[0], t * u[1], t * u[2]])
}
