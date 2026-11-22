//! Datamosh: a real codec datamosh with CFA4's own H.264 encoder and decoder ([`crate::mosh`]).
//!
//! B's pictures from the start of the transition are encoded as P pictures and decoded onto A's
//! picture at the start instead of B's key frame, so A's pixels move with B's motion while B's
//! residuals bleed in; the last `Recovery` per cent of the transition dissolves into clean B.
//!
//! The transition needs B's earlier frames, which [`super::apply_scaled`] does not have: the
//! sequence renderer feeds them through [`MoshSource`] ([`apply_mosh`]). Without them (a
//! transition rendered from two still pictures) Datamosh is a plain dissolve. The mosh is
//! incremental: a [`crate::mosh::Session`] per transition is kept between frames, so playing
//! forward costs one encode and one decode per frame; going backwards starts the mosh again.
//! Every codec failure falls back to the plain dissolve.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::Tick;

use super::{mix, smoothstep};
use crate::image::Image;
use crate::mosh::{MAX_FRAMES, Session};
use cfa4_geom::Affine;

/// Where the mosh's pictures come from.
pub struct MoshSource<'a> {
    /// Identifies the transition (and the render scale): sessions are kept per key.
    pub key: u64,
    /// A's picture where the mosh starts (the reference that replaces B's key frame).
    pub a: &'a Image,
    /// The index of B's picture for the frame being rendered (0 = the first frame of the
    /// transition, which is A's picture itself).
    pub n: usize,
    /// B's `k`-th picture (`k` ≤ `n`; `None` falls back to the current B picture).
    pub b: &'a mut dyn FnMut(usize) -> Option<Image>,
}

/// One kept mosh: the session, the inputs it was started from, and the last picture it gave.
struct Kept {
    key: u64,
    session: Session,
    /// (index, picture) of the last picture fed.
    last: Option<(usize, Image)>,
}

/// Sessions kept between frames (a few transitions can play at once on different tracks).
static KEPT: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
const MAX_KEPT: usize = 3;

fn take_kept(key: u64) -> Option<Kept> {
    let mut v = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let i = v.iter().position(|k| k.key == key)?;
    Some(v.swap_remove(i))
}

fn keep(k: Kept) {
    let mut v = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|o| o.key != k.key);
    if v.len() >= MAX_KEPT {
        v.remove(0);
    }
    v.push(k);
}

/// The encoder's QP for a Residual of `pct` per cent: 100 % lets the most of B through (QP 18),
/// 0 % leaves only the motion (QP 51).
pub(crate) fn qp_for(pct: f32) -> u8 {
    let r = if pct.is_finite() { pct.clamp(0.0, 100.0) } else { 0.0 } / 100.0;
    (51.0 - r * 33.0).round() as u8
}

/// The encode resolution divisor for the Encode Resolution choice.
fn divisor(choice: u32) -> usize {
    match choice {
        1 => 2,
        2 => 4,
        _ => 1,
    }
}

fn num(e: &EffectInstance, id: &str) -> f32 {
    let v = e.f64_at(id, Tick::ZERO) as f32;
    if v.is_finite() { v } else { 0.0 }
}

fn choice(e: &EffectInstance, id: &str) -> u32 {
    let v = e.param(id).map(|p| p.value.clone()).or_else(|| e.def().and_then(|d| d.param(id)).map(|p| p.default.clone()));
    match v {
        Some(ParamValue::Choice(c)) => c,
        _ => 0,
    }
}

fn resized(img: &Image, w: usize, h: usize) -> Image {
    if (img.w, img.h) == (w, h) {
        return img.clone();
    }
    let m = Affine::scale(w as f64 / img.w.max(1) as f64, h as f64 / img.h.max(1) as f64);
    img.transformed(w, h, &m)
}

fn hash_image(img: &Image, h: &mut DefaultHasher) {
    (img.w, img.h).hash(h);
    for v in &img.px {
        v.to_bits().hash(h);
    }
}

/// Per-pixel mix of two same-size images.
fn blend(a: &Image, b: &Image, k: f32) -> Image {
    let mut out = a.clone();
    for (o, q) in out.px.chunks_exact_mut(4).zip(b.px.chunks_exact(4)) {
        let m = mix([o[0], o[1], o[2], o[3]], [q[0], q[1], q[2], q[3]], k);
        o.copy_from_slice(&m);
    }
    out
}

/// The plain dissolve every failure falls back to.
fn dissolve(a: &Image, b: &Image, p: f32) -> Image {
    blend(a, b, p)
}

/// The B picture index for output frame `f` of a transition `frames` frames long, and the B time
/// offset (in frames from the transition start) of picture `k`: Motion Boost samples B further
/// apart, and a transition longer than [`MAX_FRAMES`] pictures takes every few frames.
pub fn picture_index(f: u64, frames: u64, motion_pct: f32) -> (usize, f64) {
    let cap = (MAX_FRAMES - 1) as u64;
    let step = if frames > cap { frames as f64 / cap as f64 } else { 1.0 };
    let n = ((f as f64 / step).floor() as u64).min(cap) as usize;
    let boost = if motion_pct.is_finite() { motion_pct.clamp(25.0, 400.0) } else { 100.0 } / 100.0;
    (n, step * boost as f64)
}

/// Render the Datamosh transition at progress `p` from the current pictures `a` and `b` and the
/// mosh's inputs in `src`. `p = 0` is exactly `a`, `p = 1` exactly `b`.
pub fn apply_mosh(e: &EffectInstance, a: &Image, b: &Image, p: f32, src: &mut MoshSource<'_>) -> Image {
    let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 };
    if a.w == 0 || a.h == 0 || (a.w, a.h) != (b.w, b.h) {
        return if p < 0.5 { a.clone() } else { b.clone() };
    }
    if p <= 0.0 {
        return a.clone();
    }
    if p >= 1.0 {
        return b.clone();
    }
    let moshed = moshed(e, a, b, src).unwrap_or_else(|| dissolve(a, b, p));
    // Recovery: the last part of the transition dissolves into clean B
    let r = (num(e, "recovery") / 100.0).clamp(0.0, 1.0);
    let k = if r > 0.0 { smoothstep(1.0 - r, 1.0, p) } else { 0.0 };
    if k > 0.0 { blend(&moshed, b, k) } else { moshed }
}

/// The moshed picture `src.n`, at `a`'s size (None: fall back to a dissolve).
fn moshed(e: &EffectInstance, a: &Image, b: &Image, src: &mut MoshSource<'_>) -> Option<Image> {
    if (src.a.w, src.a.h) != (a.w, a.h) {
        return None;
    }
    let d = divisor(choice(e, "resolution"));
    let (w, h) = ((a.w / d).max(2), (a.h / d).max(2));
    let qp = qp_for(num(e, "residual"));
    let a0 = resized(src.a, w, h);
    let mut hs = DefaultHasher::new();
    (src.key, qp, w, h).hash(&mut hs);
    hash_image(&a0, &mut hs);
    let key = hs.finish();
    let n = src.n.min(MAX_FRAMES - 1);
    if n == 0 {
        // B's key frame is dropped: the first picture is A itself
        return Some(resized(&a0, a.w, a.h));
    }
    let mut b_at = |k: usize| -> Image {
        let img = (src.b)(k).filter(|i| (i.w, i.h) == (a.w, a.h));
        resized(img.as_ref().unwrap_or(b), w, h)
    };
    let mut kept = match take_kept(key) {
        Some(k) if k.session.fed() <= n + 1 || k.last.as_ref().is_some_and(|(i, _)| *i == n) => k,
        _ => Kept { key, session: Session::new(&a0, &b_at(0), qp).ok()?, last: None },
    };
    if let Some((i, img)) = &kept.last
        && *i == n
    {
        let out = resized(img, a.w, a.h);
        keep(kept);
        return Some(out);
    }
    let mut last = None;
    while kept.session.fed() <= n {
        let k = kept.session.fed();
        let pic = kept.session.push(&b_at(k)).ok()?;
        last = Some((k, pic));
    }
    let (i, img) = last?;
    let out = resized(&img, a.w, a.h);
    kept.last = Some((i, img));
    keep(kept);
    Some(out)
}

#[cfg(test)]
mod tests;
