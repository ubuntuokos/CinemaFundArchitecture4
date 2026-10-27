//! Keyframed parameters.
//!
//! A parameter is a static value or a list of keyframes (clip-relative ticks). Interpolation per
//! keyframe governs the segment leaving it: Linear, Hold, Bezier (with influence/speed handles),
//! Auto Bezier, Continuous Bezier, Ease In/Out (Bezier presets) — Premiere's temporal set.

use cfa4_geom::Vec2;
use cfa4_time::Tick;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    Float(f64),
    /// A point. A coordinate may be NaN: point parameters use NaN for "auto" (the frame or source
    /// centre) until [`crate::resolve_auto_points`] fills it in. JSON has no NaN and writes it as
    /// `null`, so `null` reads back as NaN here, and only here: a `null` coordinate anywhere else
    /// in a project (a mask vertex, a layer rectangle) is still a damaged file.
    Vec2(#[serde(deserialize_with = "point_or_auto")] Vec2),
    Color([f32; 4]),
    Bool(bool),
    Choice(u32),
    Text(String),
    /// Curve control points (x, y) in 0..1, sorted by x (Lumetri curves).
    Curve(Vec<[f32; 2]>),
    /// A mask path (Bézier vertices in clip pixels); interpolates vertex-wise.
    Path(crate::mask::MaskPath),
}

/// Read a point parameter's value, taking a `null` coordinate as NaN ("auto").
fn point_or_auto<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec2, D::Error> {
    fn nan_if_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NAN))
    }
    #[derive(Deserialize)]
    struct Point {
        #[serde(deserialize_with = "nan_if_null")]
        x: f64,
        #[serde(deserialize_with = "nan_if_null")]
        y: f64,
    }
    let p = Point::deserialize(d)?;
    Ok(Vec2::new(p.x, p.y))
}

impl ParamValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Float(v) => Some(*v),
            ParamValue::Choice(c) => Some(*c as f64),
            ParamValue::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
    pub fn as_vec2(&self) -> Option<Vec2> {
        match self {
            ParamValue::Vec2(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<[f32; 4]> {
        match self {
            ParamValue::Color(c) => Some(*c),
            _ => None,
        }
    }
    pub fn as_curve(&self) -> Option<&[[f32; 2]]> {
        match self {
            ParamValue::Curve(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// Whether values of this kind can be interpolated (others hold).
    pub fn interpolates(&self) -> bool {
        matches!(self, ParamValue::Float(_) | ParamValue::Vec2(_) | ParamValue::Color(_) | ParamValue::Path(_))
    }
    pub fn as_path(&self) -> Option<&crate::mask::MaskPath> {
        match self {
            ParamValue::Path(p) => Some(p),
            _ => None,
        }
    }
    fn components(&self) -> Vec<f64> {
        match self {
            ParamValue::Float(v) => vec![*v],
            ParamValue::Vec2(v) => vec![v.x, v.y],
            ParamValue::Color(c) => c.iter().map(|&x| x as f64).collect(),
            ParamValue::Path(p) => p.components(),
            _ => vec![],
        }
    }
    fn from_components(template: &ParamValue, c: &[f64]) -> ParamValue {
        match template {
            ParamValue::Float(_) => ParamValue::Float(c[0]),
            ParamValue::Vec2(_) => ParamValue::Vec2(Vec2::new(c[0], c[1])),
            ParamValue::Color(_) => ParamValue::Color([c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32]),
            ParamValue::Path(p) => ParamValue::Path(p.with_components(c)),
            other => other.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    #[default]
    Linear,
    Hold,
    Bezier,
    AutoBezier,
    ContinuousBezier,
    EaseIn,
    EaseOut,
    // CFA4 easing presets (not in Premiere): each shapes the segment leaving the keyframe with a
    // fixed curve; Overshoot, Elastic and Bounce leave the [from, to] range on purpose.
    Sine,
    Expo,
    Circ,
    Overshoot,
    Elastic,
    Bounce,
}

impl Interpolation {
    pub const ALL: [Interpolation; 13] = [
        Interpolation::Linear,
        Interpolation::Bezier,
        Interpolation::AutoBezier,
        Interpolation::ContinuousBezier,
        Interpolation::Hold,
        Interpolation::EaseIn,
        Interpolation::EaseOut,
        Interpolation::Sine,
        Interpolation::Expo,
        Interpolation::Circ,
        Interpolation::Overshoot,
        Interpolation::Elastic,
        Interpolation::Bounce,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Interpolation::Linear => "Linear",
            Interpolation::Hold => "Hold",
            Interpolation::Bezier => "Bezier",
            Interpolation::AutoBezier => "Auto Bezier",
            Interpolation::ContinuousBezier => "Continuous Bezier",
            Interpolation::EaseIn => "Ease In",
            Interpolation::EaseOut => "Ease Out",
            Interpolation::Sine => "Sine",
            Interpolation::Expo => "Exponential",
            Interpolation::Circ => "Circular",
            Interpolation::Overshoot => "Overshoot",
            Interpolation::Elastic => "Elastic",
            Interpolation::Bounce => "Bounce",
        }
    }

    /// Whether this is one of CFA4's fixed easing presets (no editable handles).
    pub fn is_preset(self) -> bool {
        self.preset_curve().is_some()
    }

    /// Fixed curve of a preset: maps u∈[0,1] to eased progress, 0 at u=0 and 1 at u=1.
    fn preset_curve(self) -> Option<fn(f64) -> f64> {
        match self {
            Interpolation::Sine => Some(ease_sine),
            Interpolation::Expo => Some(ease_expo),
            Interpolation::Circ => Some(ease_circ),
            Interpolation::Overshoot => Some(ease_overshoot),
            Interpolation::Elastic => Some(ease_elastic),
            Interpolation::Bounce => Some(ease_bounce),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Time relative to the start of the track item (clip time).
    pub time: Tick,
    pub value: ParamValue,
    /// Interpolation of the segment leaving this keyframe.
    pub interp: Interpolation,
    /// Temporal ease: influence (0..1) of the outgoing/incoming handles.
    #[serde(default = "default_influence")]
    pub out_influence: f64,
    #[serde(default = "default_influence")]
    pub in_influence: f64,
}

fn default_influence() -> f64 {
    1.0 / 3.0
}

impl Keyframe {
    pub fn new(time: Tick, value: ParamValue) -> Self {
        Self { time, value, interp: Interpolation::Linear, out_influence: default_influence(), in_influence: default_influence() }
    }
}

/// A (possibly animated) parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub value: ParamValue,
    /// Sorted by time. Empty = static.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
}

impl Param {
    pub fn new(value: ParamValue) -> Self {
        Self { value, keyframes: Vec::new() }
    }
    pub fn is_animated(&self) -> bool {
        !self.keyframes.is_empty()
    }
    /// Apply `f` to the static value and every keyframe value.
    pub fn map_values(&mut self, f: impl Fn(ParamValue) -> ParamValue) {
        self.value = f(std::mem::replace(&mut self.value, ParamValue::Float(0.0)));
        for k in &mut self.keyframes {
            k.value = f(std::mem::replace(&mut k.value, ParamValue::Float(0.0)));
        }
    }

    /// Set a value at `t`: updates the static value, or adds/replaces a keyframe when animated.
    pub fn set_at(&mut self, t: Tick, value: ParamValue) {
        if self.keyframes.is_empty() {
            self.value = value;
            return;
        }
        match self.keyframes.binary_search_by_key(&t, |k| k.time) {
            Ok(i) => self.keyframes[i].value = value,
            Err(i) => {
                let interp = self.keyframes.get(i.saturating_sub(1)).map(|k| k.interp).unwrap_or_default();
                let mut k = Keyframe::new(t, value);
                k.interp = interp;
                self.keyframes.insert(i, k);
            }
        }
    }

    /// Toggle animation (the stopwatch): on → one keyframe at `t` with the current value; off → keep value at `t`.
    pub fn toggle_animation(&mut self, t: Tick) {
        if self.keyframes.is_empty() {
            self.keyframes.push(Keyframe::new(t, self.value.clone()));
        } else {
            self.value = self.value_at(t);
            self.keyframes.clear();
        }
    }

    pub fn add_keyframe(&mut self, t: Tick) {
        let v = self.value_at(t);
        if self.keyframes.is_empty() {
            self.value = v.clone();
        }
        self.set_at_force(t, v);
    }

    /// Add or replace a keyframe at `t` (the parameter becomes animated).
    pub fn put_keyframe(&mut self, t: Tick, value: ParamValue) {
        self.set_at_force(t, value);
    }

    fn set_at_force(&mut self, t: Tick, value: ParamValue) {
        match self.keyframes.binary_search_by_key(&t, |k| k.time) {
            Ok(i) => self.keyframes[i].value = value,
            Err(i) => self.keyframes.insert(i, Keyframe::new(t, value)),
        }
    }

    pub fn remove_keyframe_at(&mut self, t: Tick) -> bool {
        if let Ok(i) = self.keyframes.binary_search_by_key(&t, |k| k.time) {
            let removed = self.keyframes.remove(i);
            if self.keyframes.is_empty() {
                self.value = removed.value;
            }
            true
        } else {
            false
        }
    }

    /// Evaluate at clip time `t`.
    pub fn value_at(&self, t: Tick) -> ParamValue {
        let k = &self.keyframes;
        if k.is_empty() {
            return self.value.clone();
        }
        if t <= k[0].time {
            return k[0].value.clone();
        }
        let last = &k[k.len() - 1];
        if t >= last.time {
            return last.value.clone();
        }
        let i = match k.binary_search_by_key(&t, |kf| kf.time) {
            Ok(i) => return k[i].value.clone(),
            Err(i) => i - 1,
        };
        let (a, b) = (&k[i], &k[i + 1]);
        if a.interp == Interpolation::Hold || !a.value.interpolates() {
            return a.value.clone();
        }
        let span = (b.time - a.time).0 as f64;
        let u = (t - a.time).0 as f64 / span;
        let e = ease(a, b, u);
        let ca = a.value.components();
        let cb = b.value.components();
        if ca.len() != cb.len() {
            // e.g. mask paths with different vertex counts: hold
            return a.value.clone();
        }
        let c: Vec<f64> = ca.iter().zip(&cb).map(|(x, y)| x + (y - x) * e).collect();
        ParamValue::from_components(&a.value, &c)
    }

    pub fn f64_at(&self, t: Tick) -> f64 {
        self.value_at(t).as_f64().unwrap_or(0.0)
    }

    /// Allocation-free [`Param::f64_at`] for scalar parameters (Float / Choice / Bool), for
    /// per-sample evaluation in the audio mixer. Non-scalar values evaluate to 0.
    pub fn scalar_at(&self, t: Tick) -> f64 {
        let k = &self.keyframes;
        let s = |v: &ParamValue| v.as_f64().unwrap_or(0.0);
        if k.is_empty() {
            return s(&self.value);
        }
        if t <= k[0].time {
            return s(&k[0].value);
        }
        let last = &k[k.len() - 1];
        if t >= last.time {
            return s(&last.value);
        }
        let i = match k.binary_search_by_key(&t, |kf| kf.time) {
            Ok(i) => return s(&k[i].value),
            Err(i) => i - 1,
        };
        let (a, b) = (&k[i], &k[i + 1]);
        let (va, vb) = (s(&a.value), s(&b.value));
        if a.interp == Interpolation::Hold || !matches!(a.value, ParamValue::Float(_)) {
            return va;
        }
        let u = (t - a.time).0 as f64 / (b.time - a.time).0 as f64;
        va + (vb - va) * ease(a, b, u)
    }

    pub fn vec2_at(&self, t: Tick) -> Vec2 {
        self.value_at(t).as_vec2().unwrap_or_default()
    }

    /// Previous/next keyframe times for navigation.
    pub fn prev_keyframe(&self, t: Tick) -> Option<Tick> {
        self.keyframes.iter().rev().map(|k| k.time).find(|&k| k < t)
    }
    pub fn next_keyframe(&self, t: Tick) -> Option<Tick> {
        self.keyframes.iter().map(|k| k.time).find(|&k| k > t)
    }
}

/// Temporal easing for u∈[0,1] between two keyframes (1D cubic Bezier in time/value space).
fn ease(a: &Keyframe, b: &Keyframe, u: f64) -> f64 {
    if let Some(curve) = a.interp.preset_curve() {
        let v = curve(if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 });
        return if v.is_finite() { v } else { u };
    }
    let out_ease = matches!(a.interp, Interpolation::Bezier | Interpolation::AutoBezier | Interpolation::ContinuousBezier | Interpolation::EaseOut);
    let in_ease = matches!(b.interp, Interpolation::Bezier | Interpolation::AutoBezier | Interpolation::ContinuousBezier | Interpolation::EaseIn)
        || matches!(a.interp, Interpolation::Bezier | Interpolation::ContinuousBezier);
    if !out_ease && !in_ease {
        return u;
    }
    // Control points in normalized (time, value) space; flat handles = ease (velocity 0).
    let x1 = if out_ease { a.out_influence.clamp(0.01, 1.0) } else { 1.0 / 3.0 };
    let y1 = if out_ease { 0.0 } else { 1.0 / 3.0 };
    let x2 = if in_ease { 1.0 - b.in_influence.clamp(0.01, 1.0) } else { 2.0 / 3.0 };
    let y2 = if in_ease { 1.0 } else { 2.0 / 3.0 };
    cubic_bezier_y_for_x(x1, y1, x2, y2, u)
}

// Easing presets, after Robert Penner's easing equations (the curves CSS and most motion tools
// share). In-out for the smooth ones; out-only for the ones that overshoot or bounce at the end.

fn ease_sine(u: f64) -> f64 {
    (1.0 - (std::f64::consts::PI * u).cos()) / 2.0
}

fn ease_expo(u: f64) -> f64 {
    if u <= 0.0 {
        0.0
    } else if u >= 1.0 {
        1.0
    } else if u < 0.5 {
        2f64.powf(20.0 * u - 10.0) / 2.0
    } else {
        (2.0 - 2f64.powf(10.0 - 20.0 * u)) / 2.0
    }
}

fn ease_circ(u: f64) -> f64 {
    if u < 0.5 { (1.0 - (1.0 - (2.0 * u).powi(2)).max(0.0).sqrt()) / 2.0 } else { ((1.0 - (2.0 - 2.0 * u).powi(2)).max(0.0).sqrt() + 1.0) / 2.0 }
}

/// Passes the target by about 10% and settles back ("back out").
fn ease_overshoot(u: f64) -> f64 {
    const C1: f64 = 1.70158;
    const C3: f64 = C1 + 1.0;
    1.0 + C3 * (u - 1.0).powi(3) + C1 * (u - 1.0).powi(2)
}

/// Springs past the target and rings down ("elastic out").
fn ease_elastic(u: f64) -> f64 {
    if u <= 0.0 {
        0.0
    } else if u >= 1.0 {
        1.0
    } else {
        2f64.powf(-10.0 * u) * ((10.0 * u - 0.75) * std::f64::consts::TAU / 3.0).sin() + 1.0
    }
}

/// Lands on the target and bounces three times ("bounce out").
fn ease_bounce(u: f64) -> f64 {
    const N1: f64 = 7.5625;
    const D1: f64 = 2.75;
    if u < 1.0 / D1 {
        N1 * u * u
    } else if u < 2.0 / D1 {
        let v = u - 1.5 / D1;
        N1 * v * v + 0.75
    } else if u < 2.5 / D1 {
        let v = u - 2.25 / D1;
        N1 * v * v + 0.9375
    } else {
        let v = u - 2.625 / D1;
        N1 * v * v + 0.984375
    }
}

/// Solve a CSS-style cubic-bezier(x1,y1,x2,y2) for y at x (Newton + bisection).
pub fn cubic_bezier_y_for_x(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let bx = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * x1 + 3.0 * (1.0 - s) * s * s * x2 + s * s * s;
    let by = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * y1 + 3.0 * (1.0 - s) * s * s * y2 + s * s * s;
    let dbx = |s: f64| 3.0 * (1.0 - s) * (1.0 - s) * x1 + 6.0 * (1.0 - s) * s * (x2 - x1) + 3.0 * s * s * (1.0 - x2);
    let mut s = x;
    for _ in 0..8 {
        let d = dbx(s);
        if d.abs() < 1e-9 {
            break;
        }
        let ns = s - (bx(s) - x) / d;
        if !(0.0..=1.0).contains(&ns) {
            break;
        }
        s = ns;
    }
    if (bx(s) - x).abs() > 1e-7 {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            s = (lo + hi) / 2.0;
            if bx(s) < x { lo = s } else { hi = s }
        }
    }
    by(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_hold() {
        let mut p = Param::new(ParamValue::Float(0.0));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(100), ParamValue::Float(10.0));
        assert_eq!(p.f64_at(Tick(50)), 5.0);
        assert_eq!(p.f64_at(Tick(-5)), 0.0);
        assert_eq!(p.f64_at(Tick(500)), 10.0);
        p.keyframes[0].interp = Interpolation::Hold;
        assert_eq!(p.f64_at(Tick(99)), 0.0);
    }

    #[test]
    fn bezier_eases() {
        let mut p = Param::new(ParamValue::Vec2(Vec2::new(0.0, 0.0)));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(1000), ParamValue::Vec2(Vec2::new(100.0, 50.0)));
        p.keyframes[0].interp = Interpolation::Bezier;
        p.keyframes[1].interp = Interpolation::Bezier;
        let early = p.vec2_at(Tick(100)).x;
        let mid = p.vec2_at(Tick(500)).x;
        assert!(early < 10.0, "ease-in start should be slow: {early}");
        assert!((mid - 50.0).abs() < 1.0, "symmetric ease midpoint: {mid}");
        // monotonic
        let mut last = -1.0;
        for i in 0..=100 {
            let v = p.vec2_at(Tick(i * 10)).x;
            assert!(v >= last - 1e-9);
            last = v;
        }
    }

    #[test]
    fn presets_start_and_end_on_the_keyframes() {
        for interp in Interpolation::ALL.into_iter().filter(|i| i.is_preset()) {
            let mut p = Param::new(ParamValue::Float(10.0));
            p.toggle_animation(Tick(0));
            p.set_at(Tick(1000), ParamValue::Float(20.0));
            p.keyframes[0].interp = interp;
            assert_eq!(p.f64_at(Tick(0)), 10.0, "{interp:?}");
            assert_eq!(p.f64_at(Tick(1000)), 20.0, "{interp:?}");
            assert!((p.f64_at(Tick(1)) - 10.0).abs() < 0.1, "{interp:?} starts at the first keyframe");
            assert!((p.f64_at(Tick(999)) - 20.0).abs() < 0.1, "{interp:?} ends at the second keyframe");
            assert_eq!(p.scalar_at(Tick(250)), p.f64_at(Tick(250)), "{interp:?}: mixer path agrees");
            for i in 0..=1000 {
                let v = p.f64_at(Tick(i));
                assert!(v.is_finite() && (9.99..=24.0).contains(&v), "{interp:?} at {i}: {v}");
            }
        }
    }

    #[test]
    fn preset_shapes() {
        assert!((ease_sine(0.5) - 0.5).abs() < 1e-12);
        assert!((ease_expo(0.5) - 0.5).abs() < 1e-12);
        assert!((ease_circ(0.5) - 0.5).abs() < 1e-12);
        assert!(ease_sine(0.1) < 0.1 && ease_sine(0.9) > 0.9, "in-out curves start and end slowly");
        let peak = (0..=100).map(|i| ease_overshoot(i as f64 / 100.0)).fold(f64::MIN, f64::max);
        assert!(peak > 1.05 && peak < 1.15, "overshoot passes the target by about 10%: {peak}");
        assert!((0..=100).any(|i| ease_elastic(i as f64 / 100.0) > 1.0), "elastic rings past the target");
        let bounce: Vec<f64> = (0..=1000).map(|i| ease_bounce(i as f64 / 1000.0)).collect();
        assert!(bounce.iter().all(|v| (0.0..=1.0 + 1e-9).contains(v)), "bounce never passes the target");
        let landings = bounce.windows(3).filter(|w| w[1] > w[0] && w[1] > w[2]).count();
        assert_eq!(landings, 3, "bounce lands and comes back up three times");
    }

    #[test]
    fn presets_survive_hostile_progress() {
        for interp in Interpolation::ALL.into_iter().filter(|i| i.is_preset()) {
            let kf = |t: i64| Keyframe::new(Tick(t), ParamValue::Float(0.0));
            let mut a = kf(0);
            a.interp = interp;
            for u in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 2.0] {
                let e = ease(&a, &kf(1), u);
                assert!(e.is_finite(), "{interp:?} at {u}: {e}");
            }
        }
    }

    #[test]
    fn presets_round_trip_through_json() {
        for interp in Interpolation::ALL {
            let s = serde_json::to_string(&interp).unwrap();
            assert_eq!(serde_json::from_str::<Interpolation>(&s).unwrap(), interp);
        }
    }

    #[test]
    fn toggle_off_keeps_value() {
        let mut p = Param::new(ParamValue::Float(1.0));
        p.toggle_animation(Tick(0));
        p.set_at(Tick(10), ParamValue::Float(3.0));
        p.toggle_animation(Tick(5));
        assert!(!p.is_animated());
        assert_eq!(p.value, ParamValue::Float(2.0));
    }
}
