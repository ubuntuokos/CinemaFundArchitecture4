//! Pointer-gesture recognition (CFA4-CONTROL-SURFACE-GESTURE-001, MG1).
//!
//! Pure functions only: no I/O, no clock, no threads, no extra crate. The caller supplies every
//! timestamp (`t_ns`, monotonic nanoseconds). A template longer than [`MAX_TEMPLATE_POINTS`] is an
//! error; the points are not truncated.
//!
//! A straight stroke within the angle tolerance is one of the eight directions. Any other stroke
//! is matched against the caller's templates. Templates are an argument here; they are not read
//! from a profile file.

use std::fmt;

use super::DeviceId;

/// Most points a template may carry. A longer template is an error, not a clipped one.
pub const MAX_TEMPLATE_POINTS: usize = 64;

/// Most samples one recognition call accepts. A longer slice is an error, not a prefix.
pub const MAX_SAMPLES: usize = 8_192;

/// `KEY_MAX` from Linux `input-event-codes.h` (0x2ff). Button codes are not limited to this.
pub const KEY_MAX: u16 = 0x2ff;

/// Device id of the Qt pointer. Wayland's `wl_pointer` button event names no device.
pub const POINTER_SEAT: &str = "pointer:seat";

/// Lowest confidence (0..=1000) that counts as a template match.
pub const MIN_CONFIDENCE_MILLI: u16 = 800;

/// How many points a stroke and a template are resampled to before they are compared.
const RESAMPLE_N: usize = 16;

/// Normalised coordinate range (the longer side of the bounding box).
const NORM: i64 = 1024;

pub const MOD_CTRL: u8 = 0b0001;
pub const MOD_SHIFT: u8 = 0b0010;
pub const MOD_ALT: u8 = 0b0100;
pub const MOD_META: u8 = 0b1000;

/// One pointer sample. `buttons` is a bitset of logical roles (bit 0 primary, bit 1 secondary,
/// bit 2 middle, then learned roles), not a fixed-length button array. Coordinates are logical
/// pixels. `t_ns` is the caller's monotonic clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerSample {
    pub t_ns: u64,
    pub x_px: i32,
    pub y_px: i32,
    pub buttons: u32,
    pub modifiers: u8,
}

/// Thresholds the caller chooses. Times are milliseconds. Lengths are logical pixels.
/// `angle_milli_deg` is how far a straight stroke may sit from an ideal direction.
/// `hold_ms` is the button-hold deadline the engine reads; this function treats motion that
/// stays inside `slop_px` as no gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    pub min_length_px: u32,
    pub max_length_px: u32,
    pub slop_px: u32,
    pub min_ms: u64,
    pub max_ms: u64,
    pub angle_milli_deg: u32,
    pub hold_ms: u64,
}

/// A user gesture. `spatial` strokes keep their screen direction when a profile is mirrored;
/// ergonomic ones (`spatial: false`) may be mirrored with [`mirror_template`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub points: Vec<(i32, i32)>,
    pub spatial: bool,
}

/// Eight directions: the four cardinals and the four diagonals. Screen +x is right, +y is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Right,
    Left,
    Up,
    Down,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}

impl Direction {
    /// Stable id placed in [`super::SurfaceInput::Gesture`].
    pub const fn as_id(self) -> &'static str {
        match self {
            Direction::Right => "direction.right",
            Direction::Left => "direction.left",
            Direction::Up => "direction.up",
            Direction::Down => "direction.down",
            Direction::UpLeft => "direction.up-left",
            Direction::UpRight => "direction.up-right",
            Direction::DownLeft => "direction.down-left",
            Direction::DownRight => "direction.down-right",
        }
    }
}

/// What [`recognize`] decided. `confidence_milli` is an integer in 0..=1000.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recognize {
    None,
    Direction(Direction),
    Match { id: String, confidence_milli: u16 },
    Cancelled,
}

/// A template or a sample slice that cannot be recognised safely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GestureError {
    /// `points` is the length that was offered. It is not reduced to [`MAX_TEMPLATE_POINTS`].
    TemplateTooLong { id: String, points: usize },
    /// `samples` is the length that was offered. It is not reduced to [`MAX_SAMPLES`].
    TooManySamples { samples: usize },
}

impl fmt::Display for GestureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GestureError::TemplateTooLong { id, points } => {
                write!(f, "gesture template `{id}` has {points} points; the limit is {MAX_TEMPLATE_POINTS}")
            }
            GestureError::TooManySamples { samples } => {
                write!(f, "gesture has {samples} samples; the limit is {MAX_SAMPLES}")
            }
        }
    }
}

impl std::error::Error for GestureError {}

/// The Wayland/X11 pointer as one device. Evdev devices keep [`DeviceId::hid`].
pub fn pointer_seat() -> DeviceId {
    DeviceId(POINTER_SEAT.to_string())
}

/// Recognise `samples` against `templates`.
///
/// Returns [`Err`] when a template is longer than [`MAX_TEMPLATE_POINTS`] or the sample slice is
/// longer than [`MAX_SAMPLES`]. The inputs are not shortened. Same samples, templates and
/// thresholds always return the same value: there is no random source and no clock read.
pub fn recognize(samples: &[PointerSample], templates: &[Template], thresholds: &Thresholds) -> Result<Recognize, GestureError> {
    for template in templates {
        if template.points.len() > MAX_TEMPLATE_POINTS {
            return Err(GestureError::TemplateTooLong { id: template.id.clone(), points: template.points.len() });
        }
    }
    if samples.len() > MAX_SAMPLES {
        return Err(GestureError::TooManySamples { samples: samples.len() });
    }
    if samples.len() < 2 {
        return Ok(Recognize::None);
    }
    let mut previous = match samples.first() {
        Some(sample) => sample.t_ns,
        None => return Ok(Recognize::None),
    };
    for sample in samples.iter().skip(1) {
        if sample.t_ns < previous {
            return Ok(Recognize::None);
        }
        previous = sample.t_ns;
    }
    let start = samples.first().map(|sample| sample.t_ns).unwrap_or(0);
    let end = samples.last().map(|sample| sample.t_ns).unwrap_or(start);
    let span = end.saturating_sub(start);
    if span > thresholds.max_ms.saturating_mul(1_000_000) {
        return Ok(Recognize::Cancelled);
    }
    if span < thresholds.min_ms.saturating_mul(1_000_000) {
        return Ok(Recognize::None);
    }

    let raw: Vec<(i32, i32)> = samples.iter().map(|sample| (sample.x_px, sample.y_px)).collect();
    let filtered = dedup(&raw);
    if filtered.len() < 2 || within_slop(&filtered, thresholds.slop_px) {
        return Ok(Recognize::None);
    }
    let path = path_len(&filtered);
    if path < u64::from(thresholds.min_length_px) {
        return Ok(Recognize::None);
    }
    if path > u64::from(thresholds.max_length_px) {
        return Ok(Recognize::None);
    }
    if is_straight(&filtered, thresholds.slop_px) {
        let (x0, y0) = filtered[0];
        let (x1, y1) = filtered[filtered.len() - 1];
        return Ok(match direction_of(x1.saturating_sub(x0), y1.saturating_sub(y0), thresholds.angle_milli_deg) {
            Some(direction) => Recognize::Direction(direction),
            None => Recognize::None,
        });
    }
    if let Some((id, confidence_milli)) = best_template(&filtered, templates)
        && confidence_milli >= MIN_CONFIDENCE_MILLI
    {
        return Ok(Recognize::Match { id, confidence_milli });
    }
    Ok(Recognize::None)
}

/// Horizontal mirror. A spatial template is returned unchanged. An ergonomic template is flipped
/// around the centroid of its points; the id and the `spatial` flag stay.
pub fn mirror_template(template: &Template) -> Template {
    if template.spatial {
        return template.clone();
    }
    Template { id: template.id.clone(), points: mirror_points(&template.points), spatial: false }
}

/// Flip `points` around their horizontal centroid. An empty slice stays empty.
pub fn mirror_points(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let Some(n) = i64::try_from(points.len()).ok().filter(|n| *n > 0) else {
        return Vec::new();
    };
    let sum: i64 = points.iter().fold(0i64, |acc, (x, _)| acc.saturating_add(i64::from(*x)));
    let center = sum / n;
    points
        .iter()
        .map(|(x, y)| {
            let flipped = center.saturating_mul(2).saturating_sub(i64::from(*x));
            (clamp_i32(flipped), *y)
        })
        .collect()
}

fn best_template(stroke: &[(i32, i32)], templates: &[Template]) -> Option<(String, u16)> {
    let prepared = prepare_shape(stroke)?;
    let mut best: Option<(String, u16)> = None;
    for template in templates {
        let Some(shape) = prepare_shape(&template.points) else { continue };
        let confidence = confidence(&prepared, &shape);
        let replace = match &best {
            Some((_, previous)) => confidence > *previous,
            None => true,
        };
        if replace {
            best = Some((template.id.clone(), confidence));
        }
    }
    best
}

fn prepare_shape(points: &[(i32, i32)]) -> Option<Vec<(i32, i32)>> {
    let filtered = dedup(points);
    if filtered.is_empty() {
        return None;
    }
    normalize(&smooth(&resample(&filtered)))
}

fn confidence(left: &[(i32, i32)], right: &[(i32, i32)]) -> u16 {
    let n = left.len().min(right.len());
    if n == 0 {
        return 0;
    }
    let mut sum = 0i64;
    for i in 0..n {
        let (Some(a), Some(b)) = (left.get(i), right.get(i)) else { continue };
        let dx = i64::from(a.0) - i64::from(b.0);
        let dy = i64::from(a.1) - i64::from(b.1);
        sum = sum.saturating_add(dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)));
    }
    let mean = sum / i64::try_from(n).unwrap_or(1);
    let rms = i64::try_from(u64::try_from(mean.max(0)).unwrap_or(0).isqrt()).unwrap_or(i64::MAX);
    let value = (1000 - rms.saturating_mul(1000) / 256).clamp(0, 1000);
    u16::try_from(value).unwrap_or(0)
}

fn dedup(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for &point in points {
        let close = out.last().is_some_and(|&(px, py)| hypot_i64(i64::from(point.0) - i64::from(px), i64::from(point.1) - i64::from(py)) < 1);
        if !close {
            out.push(point);
        }
    }
    out
}

fn resample(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let Some(&first) = points.first() else { return Vec::new() };
    if points.len() == 1 || RESAMPLE_N <= 1 {
        return vec![first; RESAMPLE_N.max(1)];
    }
    let mut cumulative = Vec::with_capacity(points.len());
    cumulative.push(0u64);
    for window in points.windows(2) {
        let (Some(a), Some(b)) = (window.first(), window.get(1)) else { continue };
        let step = hypot_i64(i64::from(b.0) - i64::from(a.0), i64::from(b.1) - i64::from(a.1));
        let so_far = cumulative.last().copied().unwrap_or(0);
        cumulative.push(so_far.saturating_add(step));
    }
    let total = cumulative.last().copied().unwrap_or(0);
    if total == 0 {
        return vec![first; RESAMPLE_N];
    }
    let mut out = Vec::with_capacity(RESAMPLE_N);
    let last_index = points.len().saturating_sub(1);
    for i in 0..RESAMPLE_N {
        let target = total.saturating_mul(i as u64) / (RESAMPLE_N as u64 - 1);
        let found = cumulative.partition_point(|&done| done < target).saturating_sub(1).min(last_index.saturating_sub(1));
        let Some(&(x0, y0)) = points.get(found) else { continue };
        let Some(&(x1, y1)) = points.get(found + 1) else {
            out.push((x0, y0));
            continue;
        };
        let start = cumulative.get(found).copied().unwrap_or(0);
        let end = cumulative.get(found + 1).copied().unwrap_or(start);
        let segment = end.saturating_sub(start);
        if segment == 0 {
            out.push((x0, y0));
            continue;
        }
        let along = i64::try_from(target.saturating_sub(start)).unwrap_or(0);
        let segment = i64::try_from(segment).unwrap_or(1).max(1);
        let x = i64::from(x0) + (i64::from(x1) - i64::from(x0)) * along / segment;
        let y = i64::from(y0) + (i64::from(y1) - i64::from(y0)) * along / segment;
        out.push((clamp_i32(x), clamp_i32(y)));
    }
    out
}

fn smooth(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len());
    if let Some(&first) = points.first() {
        out.push(first);
    }
    for i in 1..points.len().saturating_sub(1) {
        let (Some(a), Some(b), Some(c)) = (points.get(i - 1), points.get(i), points.get(i + 1)) else { continue };
        let x = (i64::from(a.0) + i64::from(b.0) + i64::from(c.0)) / 3;
        let y = (i64::from(a.1) + i64::from(b.1) + i64::from(c.1)) / 3;
        out.push((clamp_i32(x), clamp_i32(y)));
    }
    if let Some(&last) = points.last() {
        out.push(last);
    }
    out
}

fn normalize(points: &[(i32, i32)]) -> Option<Vec<(i32, i32)>> {
    let n = i64::try_from(points.len()).ok().filter(|n| *n > 0)?;
    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut min_x = i64::MAX;
    let mut max_x = i64::MIN;
    let mut min_y = i64::MAX;
    let mut max_y = i64::MIN;
    for &(x, y) in points {
        let x = i64::from(x);
        let y = i64::from(y);
        sum_x = sum_x.saturating_add(x);
        sum_y = sum_y.saturating_add(y);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let span = (max_x - min_x).max(max_y - min_y);
    if span <= 0 {
        return None;
    }
    let center_x = sum_x / n;
    let center_y = sum_y / n;
    Some(
        points
            .iter()
            .map(|&(x, y)| {
                let nx = (i64::from(x) - center_x) * NORM / span;
                let ny = (i64::from(y) - center_y) * NORM / span;
                (clamp_i32(nx), clamp_i32(ny))
            })
            .collect(),
    )
}

fn within_slop(points: &[(i32, i32)], slop: u32) -> bool {
    let Some(&(x0, y0)) = points.first() else { return true };
    let slop = u64::from(slop);
    points.iter().all(|&(x, y)| hypot_i64(i64::from(x) - i64::from(x0), i64::from(y) - i64::from(y0)) <= slop)
}

fn is_straight(points: &[(i32, i32)], slop: u32) -> bool {
    let (Some(&(x0, y0)), Some(&(x1, y1))) = (points.first(), points.last()) else { return false };
    let chord = hypot_i64(i64::from(x1) - i64::from(x0), i64::from(y1) - i64::from(y0));
    if chord == 0 {
        return false;
    }
    let limit = u64::from(slop).max(chord / 10).max(1);
    let on_chord = points.iter().all(|&(x, y)| dist_to_segment(x, y, x0, y0, x1, y1) <= limit);
    let path = path_len(points);
    on_chord && path.saturating_mul(100) <= chord.saturating_mul(110).saturating_add(u64::from(slop))
}

fn dist_to_segment(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32) -> u64 {
    let dx = i64::from(x1) - i64::from(x0);
    let dy = i64::from(y1) - i64::from(y0);
    let len2 = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
    if len2 <= 0 {
        return hypot_i64(i64::from(x) - i64::from(x0), i64::from(y) - i64::from(y0));
    }
    let dot = (i64::from(x) - i64::from(x0)).saturating_mul(dx) + (i64::from(y) - i64::from(y0)).saturating_mul(dy);
    let t = dot.clamp(0, len2);
    let proj_x = i64::from(x0) + dx * t / len2;
    let proj_y = i64::from(y0) + dy * t / len2;
    hypot_i64(i64::from(x) - proj_x, i64::from(y) - proj_y)
}

fn path_len(points: &[(i32, i32)]) -> u64 {
    points.windows(2).fold(0u64, |acc, window| {
        let (Some(a), Some(b)) = (window.first(), window.get(1)) else { return acc };
        acc.saturating_add(hypot_i64(i64::from(b.0) - i64::from(a.0), i64::from(b.1) - i64::from(a.1)))
    })
}

fn direction_of(dx: i32, dy: i32, tolerance_milli: u32) -> Option<Direction> {
    if dx == 0 && dy == 0 {
        return None;
    }
    let angle = atan2_milli(i64::from(dy), i64::from(dx));
    let tolerance = i32::try_from(tolerance_milli).unwrap_or(i32::MAX);
    // On an equal distance the earlier entry wins, so the result does not depend on hash order.
    const IDEALS: [(Direction, i32); 8] = [
        (Direction::Right, 0),
        (Direction::DownRight, 45_000),
        (Direction::Down, 90_000),
        (Direction::DownLeft, 135_000),
        (Direction::Left, 180_000),
        (Direction::UpLeft, -135_000),
        (Direction::Up, -90_000),
        (Direction::UpRight, -45_000),
    ];
    let mut best: Option<(Direction, i32)> = None;
    for (direction, ideal) in IDEALS {
        let distance = angular_distance(angle, ideal);
        if distance > tolerance {
            continue;
        }
        let replace = match best {
            Some((_, previous)) => distance < previous,
            None => true,
        };
        if replace {
            best = Some((direction, distance));
        }
    }
    best.map(|(direction, _)| direction)
}

fn angular_distance(a: i32, b: i32) -> i32 {
    let raw = i32::try_from(a.abs_diff(b)).unwrap_or(i32::MAX);
    if raw > 180_000 { 360_000 - raw } else { raw }
}

/// `atan2(y, x)` in millidegrees, range -180_000..=180_000. Integer only, so the value does not
/// depend on libm. The error on the octant tests below is under one degree.
fn atan2_milli(y: i64, x: i64) -> i32 {
    if x == 0 && y == 0 {
        return 0;
    }
    let ax = x.abs();
    let ay = y.abs();
    let (z_milli, swapped) = if ax >= ay {
        let z = if ax == 0 { 0 } else { ay.saturating_mul(1000) / ax };
        (i32::try_from(z.min(1000)).unwrap_or(1000), false)
    } else {
        let z = if ay == 0 { 0 } else { ax.saturating_mul(1000) / ay };
        (i32::try_from(z.min(1000)).unwrap_or(1000), true)
    };
    let unit = atan_unit_milli(z_milli);
    let angle = if swapped { 90_000 - unit } else { unit };
    match (x >= 0, y >= 0) {
        (true, true) => angle,
        (false, true) => 180_000 - angle,
        (false, false) => angle - 180_000,
        (true, false) => -angle,
    }
}

fn atan_unit_milli(z_milli: i32) -> i32 {
    let z = i64::from(z_milli.clamp(0, 1000));
    let linear = 45_000 * z;
    let bend = 15_640 * z * (1000 - z) / 1000;
    i32::try_from((linear + bend) / 1000).unwrap_or(0)
}

fn hypot_i64(dx: i64, dy: i64) -> u64 {
    let square = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)).max(0);
    u64::try_from(square).unwrap_or(u64::MAX).isqrt()
}

fn clamp_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value > 0 { i32::MAX } else { i32::MIN })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlId, SurfaceInput, SurfaceProtocol};

    fn thresholds() -> Thresholds {
        Thresholds { min_length_px: 40, max_length_px: 2_000, slop_px: 8, min_ms: 20, max_ms: 1_500, angle_milli_deg: 20_000, hold_ms: 300 }
    }

    fn sample_line(dx: i32, dy: i32, wobble: i32, span_ms: u64) -> Vec<PointerSample> {
        let steps = 8u64;
        let step = span_ms.saturating_mul(1_000_000) / steps;
        let t0 = 1_000_000_000u64;
        (0..9)
            .map(|i| {
                let x = 500 + dx * i / 8;
                let y = 400 + dy * i / 8;
                let w = if i == 0 || i == 8 {
                    0
                } else if i % 2 == 0 {
                    wobble
                } else {
                    -wobble
                };
                let len = hypot_i64(i64::from(dx), i64::from(dy)).max(1);
                let ox = clamp_i32(-i64::from(dy) * i64::from(w) / i64::try_from(len).unwrap_or(1));
                let oy = clamp_i32(i64::from(dx) * i64::from(w) / i64::try_from(len).unwrap_or(1));
                PointerSample { t_ns: t0 + step * i as u64, x_px: x + ox, y_px: y + oy, buttons: 0x2, modifiers: 0 }
            })
            .collect()
    }

    fn poly(points: &[(i32, i32)], origin: (i32, i32), scale: i32, noise: i32, span_ms: u64) -> Vec<PointerSample> {
        let gaps = u64::try_from(points.len().saturating_sub(1)).unwrap_or(1).max(1);
        let step = span_ms.saturating_mul(1_000_000) / gaps;
        let t0 = 5_000_000_000u64;
        points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| {
                let nudge = if noise != 0 && i % 2 == 1 { noise } else { 0 };
                PointerSample {
                    t_ns: t0 + step * i as u64,
                    x_px: origin.0.saturating_add(x.saturating_mul(scale)).saturating_add(nudge),
                    y_px: origin.1.saturating_add(y.saturating_mul(scale)),
                    buttons: 0x2,
                    modifiers: 0,
                }
            })
            .collect()
    }

    fn shapes() -> Vec<Template> {
        vec![
            Template { id: "chevron".into(), points: vec![(0, 0), (50, 90), (100, 0)], spatial: false },
            Template { id: "hook".into(), points: vec![(0, 0), (0, 100), (100, 100)], spatial: false },
            Template { id: "zigzag".into(), points: vec![(0, 0), (60, 40), (20, 90), (80, 140)], spatial: false },
            Template { id: "peak".into(), points: vec![(0, 40), (40, 0), (80, 40), (40, 80)], spatial: true },
        ]
    }

    fn fit(dx: i32, dy: i32, length: i32) -> (i32, i32) {
        let h = i64::try_from(hypot_i64(i64::from(dx), i64::from(dy)).max(1)).unwrap_or(1);
        (clamp_i32(i64::from(dx) * i64::from(length) / h), clamp_i32(i64::from(dy) * i64::from(length) / h))
    }

    #[derive(Clone, Copy)]
    enum Want {
        Direction(Direction),
        Match(&'static str),
        None,
        Cancelled,
    }

    fn matches_want(want: Want, got: &Recognize) -> bool {
        match (want, got) {
            (Want::Direction(want), Recognize::Direction(got)) => want == *got,
            (Want::Match(id), Recognize::Match { id: got, .. }) => id == got,
            (Want::None, Recognize::None) => true,
            (Want::Cancelled, Recognize::Cancelled) => true,
            _ => false,
        }
    }

    fn is_hit(got: &Recognize) -> bool {
        matches!(got, Recognize::Direction(_) | Recognize::Match { .. })
    }

    /// AC-01: eight directions and the user shapes, on one fixed synthetic set.
    /// At least 95% correct, at most 1% false matches on the non-gestures, and the same result twice.
    #[test]
    fn ac01_fixed_set_is_deterministic_and_within_the_rates() {
        let thresholds = thresholds();
        let templates = shapes();
        let ideals = [
            (Direction::Right, 120, 0),
            (Direction::DownRight, 85, 85),
            (Direction::Down, 0, 120),
            (Direction::DownLeft, -85, 85),
            (Direction::Left, -120, 0),
            (Direction::UpLeft, -85, -85),
            (Direction::Up, 0, -120),
            (Direction::UpRight, 85, -85),
        ];
        // About 8° and 15° off each ideal, still inside the 20° tolerance and outside the next sector.
        let offsets = [
            (Direction::Right, 119, 17),
            (Direction::Right, 116, 31),
            (Direction::DownRight, 72, 96),
            (Direction::DownRight, 60, 104),
            (Direction::Down, -17, 119),
            (Direction::Down, -31, 116),
            (Direction::DownLeft, -96, 72),
            (Direction::DownLeft, -104, 60),
            (Direction::Left, -119, -17),
            (Direction::Left, -116, -31),
            (Direction::UpLeft, -72, -96),
            (Direction::UpLeft, -60, -104),
            (Direction::Up, 17, -119),
            (Direction::Up, 31, -116),
            (Direction::UpRight, 96, -72),
            (Direction::UpRight, 104, -60),
        ];
        // Halfway between two directions: 22.5° + k·45°, outside the 20° tolerance.
        let halfway = [(111, 46), (46, 111), (-46, 111), (-111, 46), (-111, -46), (-46, -111), (46, -111), (111, -46)];

        let mut cases: Vec<(Vec<PointerSample>, Want)> = Vec::new();
        for (direction, dx, dy) in ideals {
            cases.push((sample_line(dx, dy, 0, 240), Want::Direction(direction)));
            cases.push((sample_line(dx, dy, 2, 240), Want::Direction(direction)));
            for length in [80, 180] {
                let (x, y) = fit(dx, dy, length);
                cases.push((sample_line(x, y, 0, 240), Want::Direction(direction)));
            }
        }
        for (direction, dx, dy) in offsets {
            cases.push((sample_line(dx, dy, 0, 240), Want::Direction(direction)));
        }
        for template in &templates {
            let id: &'static str = match template.id.as_str() {
                "chevron" => "chevron",
                "hook" => "hook",
                "zigzag" => "zigzag",
                "peak" => "peak",
                other => panic!("unexpected template {other}"),
            };
            cases.push((poly(&template.points, (0, 0), 1, 0, 400), Want::Match(id)));
            cases.push((poly(&template.points, (30, -20), 1, 0, 400), Want::Match(id)));
            cases.push((poly(&template.points, (0, 0), 2, 0, 400), Want::Match(id)));
            cases.push((poly(&template.points, (10, 10), 1, 1, 400), Want::Match(id)));
        }
        for (dx, dy) in halfway {
            for length in [80, 120, 200] {
                let (x, y) = fit(dx, dy, length);
                cases.push((sample_line(x, y, 0, 240), Want::None));
            }
        }
        for n in 0..40 {
            let t0 = 3_000_000_000u64;
            let center = 100 + n * 17;
            let samples = (0..8)
                .map(|i: i32| PointerSample {
                    t_ns: t0 + u64::from(i.unsigned_abs()) * 20_000_000,
                    x_px: center + (i % 5) - 2,
                    y_px: center + (i % 3) - 1,
                    buttons: 0,
                    modifiers: 0,
                })
                .collect();
            cases.push((samples, Want::None));
        }
        for n in 0..12 {
            cases.push((sample_line(8 + n, n % 3, 0, 200), Want::None));
            cases.push((sample_line(120, 0, 0, 8), Want::None));
        }
        for (dx, dy) in [(120, 0), (0, 120), (85, 85), (-120, 0)] {
            cases.push((sample_line(dx, dy, 0, 2_000), Want::Cancelled));
        }
        let scribbles: [&[(i32, i32)]; 8] = [
            &[(40, 0), (40, 80), (40, 40), (0, 40), (80, 40)],
            &[(0, 0), (70, 10), (10, 70), (80, 80), (0, 40)],
            &[(0, 0), (20, 0), (20, 20), (40, 20), (40, 40), (60, 40), (60, 60)],
            &[(0, 0), (90, 10), (10, 90), (80, 20), (20, 70)],
            &[(10, 10), (90, 15), (15, 90), (85, 85), (10, 50), (50, 10)],
            &[(0, 50), (25, 0), (50, 50), (75, 0), (100, 50), (50, 100)],
            &[(0, 0), (30, 80), (60, 10), (90, 70), (120, 20), (150, 90)],
            &[(0, 0), (100, 20), (0, 40), (100, 60), (0, 80), (100, 100)],
        ];
        for points in scribbles {
            cases.push((poly(points, (15, 25), 1, 0, 500), Want::None));
        }

        let mut positive = 0usize;
        let mut positive_ok = 0usize;
        let mut negative = 0usize;
        let mut false_match = 0usize;
        let mut misses = Vec::new();
        for (samples, want) in &cases {
            let once = recognize(samples, &templates, &thresholds).expect("fixed set is within the caps");
            let twice = recognize(samples, &templates, &thresholds).expect("fixed set is within the caps");
            assert_eq!(once, twice, "same samples must recognise the same way");
            let positive_case = matches!(want, Want::Direction(_) | Want::Match(_));
            if positive_case {
                positive += 1;
                if matches_want(*want, &once) {
                    positive_ok += 1;
                }
            } else {
                negative += 1;
                if is_hit(&once) {
                    false_match += 1;
                }
            }
            if !matches_want(*want, &once) && misses.len() < 12 {
                misses.push(format!("want {want:?} got {once:?}"));
            }
        }
        assert!(positive >= 60, "positive cases {positive}");
        assert!(negative >= 100, "negative cases {negative}");
        assert!(positive_ok * 100 / positive >= 95, "{positive_ok}/{positive} correct; {misses:?}");
        assert!(false_match * 100 <= negative, "{false_match}/{negative} false matches; {misses:?}");
        assert!(misses.is_empty(), "{positive_ok}/{positive} correct, {false_match}/{negative} false; {misses:?}");
    }

    #[test]
    fn a_template_over_64_points_is_an_error_and_is_not_truncated() {
        let mut points = vec![(0, 0); 65];
        points[64] = (80, 40);
        let template = Template { id: "too-long".into(), points, spatial: true };
        let err = recognize(&sample_line(120, 0, 0, 240), std::slice::from_ref(&template), &thresholds()).expect_err("65 points");
        assert_eq!(err, GestureError::TemplateTooLong { id: "too-long".into(), points: 65 });
        assert_eq!(template.points.len(), 65, "the template itself is not clipped");
    }

    #[test]
    fn a_64_point_template_is_accepted() {
        let points: Vec<(i32, i32)> = (0..64).map(|i| if i < 32 { (i * 3, i * 2) } else { (96 - (i - 32) * 2, 64 + (i - 32)) }).collect();
        let template = Template { id: "long".into(), points: points.clone(), spatial: false };
        let got = recognize(&poly(&points, (0, 0), 1, 0, 800), std::slice::from_ref(&template), &thresholds()).expect("64 points");
        assert!(matches!(got, Recognize::Match { ref id, confidence_milli } if id == "long" && confidence_milli >= MIN_CONFIDENCE_MILLI), "{got:?}");
    }

    #[test]
    fn too_many_samples_is_an_error_and_is_not_a_prefix() {
        let samples = vec![PointerSample { t_ns: 0, x_px: 0, y_px: 0, buttons: 0, modifiers: 0 }; MAX_SAMPLES + 1];
        let err = recognize(&samples, &[], &thresholds()).expect_err("cap");
        assert_eq!(err, GestureError::TooManySamples { samples: MAX_SAMPLES + 1 });
    }

    #[test]
    fn empty_single_and_backwards_time_are_not_gestures() {
        let thresholds = thresholds();
        assert_eq!(recognize(&[], &[], &thresholds).unwrap(), Recognize::None);
        let one = PointerSample { t_ns: 0, x_px: 0, y_px: 0, buttons: 0, modifiers: 0 };
        assert_eq!(recognize(&[one], &[], &thresholds).unwrap(), Recognize::None);
        let mut samples = sample_line(120, 0, 0, 240);
        samples[3].t_ns = samples[2].t_ns - 1;
        assert_eq!(recognize(&samples, &[], &thresholds).unwrap(), Recognize::None);
    }

    #[test]
    fn atan2_octants_stay_within_a_degree() {
        let cases = [
            (0, 100, 0),
            (100, 100, 45_000),
            (100, 0, 90_000),
            (100, -100, 135_000),
            (0, -100, 180_000),
            (-100, -100, -135_000),
            (-100, 0, -90_000),
            (-100, 100, -45_000),
        ];
        for (y, x, want) in cases {
            let got = atan2_milli(y, x);
            let distance = angular_distance(got, want);
            assert!(distance <= 1_000, "atan2({y}, {x}) = {got}, want {want}");
        }
    }

    #[test]
    fn spatial_templates_do_not_mirror_and_ergonomic_ones_do() {
        let spatial = Template { id: "time-right".into(), points: vec![(0, 0), (100, 0)], spatial: true };
        let ergonomic = Template { id: "hook".into(), points: vec![(0, 0), (0, 80), (80, 80)], spatial: false };
        assert_eq!(mirror_template(&spatial).points, spatial.points);
        assert!(mirror_template(&spatial).spatial);
        let mirrored = mirror_template(&ergonomic);
        assert_eq!(mirrored.id, "hook");
        assert!(!mirrored.spatial);
        assert_ne!(mirrored.points, ergonomic.points);
        assert_eq!(mirrored.points.len(), ergonomic.points.len());
    }

    /// AC-18, as far as recognition goes: a mirrored ergonomic stroke matches the same id, and a
    /// rightward spatial stroke stays right.
    #[test]
    fn mirrored_ergonomic_stroke_keeps_its_id_and_spatial_direction_does_not_flip() {
        let thresholds = thresholds();
        let hook = Template { id: "hook".into(), points: vec![(0, 0), (0, 100), (100, 100)], spatial: false };
        let mirrored = mirror_template(&hook);
        let original = recognize(&poly(&hook.points, (0, 0), 1, 0, 400), std::slice::from_ref(&hook), &thresholds).unwrap();
        let flipped = recognize(&poly(&mirrored.points, (0, 0), 1, 0, 400), std::slice::from_ref(&mirrored), &thresholds).unwrap();
        assert_eq!(original, flipped);
        assert!(matches!(original, Recognize::Match { ref id, .. } if id == "hook"), "{original:?}");
        let right = recognize(&sample_line(120, 0, 0, 240), std::slice::from_ref(&mirrored), &thresholds).unwrap();
        assert_eq!(right, Recognize::Direction(Direction::Right));
        let left = recognize(&sample_line(-120, 0, 0, 240), &[], &thresholds).unwrap();
        assert_eq!(left, Recognize::Direction(Direction::Left));
    }

    #[test]
    fn gesture_input_and_pointer_protocol_round_trip_and_old_json_still_parses() {
        assert_eq!(pointer_seat().to_string(), "pointer:seat");
        assert_eq!(SurfaceProtocol::Pointer.as_str(), "pointer");
        assert_eq!(serde_json::to_value(SurfaceProtocol::Pointer).unwrap(), serde_json::json!("pointer"));
        let gesture = SurfaceInput::Gesture { id: "direction.right".into(), context: String::new() };
        let value = serde_json::to_value(&gesture).unwrap();
        assert_eq!(value["type"], "gesture");
        assert_eq!(value["id"], "direction.right");
        assert!(value.get("context").is_none(), "an empty context stays out of the JSON");
        assert_eq!(serde_json::from_value::<SurfaceInput>(value).unwrap(), gesture);
        let old = serde_json::json!({"type": "gesture", "id": "direction.right"});
        assert_eq!(serde_json::from_value::<SurfaceInput>(old).unwrap(), gesture);
        assert!(gesture.is_trigger());
        assert!(gesture.control().is_none());
        let button = SurfaceInput::Button { control: ControlId::hid_named("button1"), pressed: true };
        let stored = serde_json::to_value(&button).unwrap();
        assert_eq!(serde_json::from_value::<SurfaceInput>(stored).unwrap(), button);
    }

    impl std::fmt::Debug for Want {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Want::Direction(direction) => write!(f, "direction.{}", direction.as_id()),
                Want::Match(id) => write!(f, "match {id}"),
                Want::None => write!(f, "none"),
                Want::Cancelled => write!(f, "cancelled"),
            }
        }
    }
}
