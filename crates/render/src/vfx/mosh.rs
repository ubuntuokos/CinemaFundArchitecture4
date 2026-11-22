//! Datamosh Bloom: one P picture of the clip, repeated (the classic "P-frame duplication").
//!
//! The motion from the frame before Start Time to the frame at Start Time is encoded once with
//! CFA4's H.264 encoder ([`crate::mosh::Bloom`]) and decoded again and again onto the picture so
//! far, one more time per frame, for Repeats frames: the picture smears ("blooms") along that
//! motion. Over the last Dissolve per cent of the repeats the clip itself comes back. Outside
//! that span the clip is untouched. The effect needs the clip's other frames, so it does nothing
//! where they are not available (an adjustment layer, an isolated test), and every codec failure
//! leaves the frame as it is.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use cfa4_project::EffectInstance;

use super::*;
use crate::mosh::Bloom;
use crate::transitions::mosh::qp_for;

/// The longest bloom, in frames (each frame of the bloom costs one encode and one decode).
pub const MAX_REPEATS: usize = 120;

/// A bloom kept between frames: its key, the motion, and the last picture with its repeat count.
struct Kept {
    key: u64,
    bloom: Bloom,
    last: (usize, Image),
}

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

fn hash_image(img: &Image, h: &mut DefaultHasher) {
    (img.w, img.h).hash(h);
    for v in &img.px {
        v.to_bits().hash(h);
    }
}

/// Which repeat (1..=`repeats`) the frame `seconds` into the clip shows, if any.
pub fn repeat_at(seconds: f64, start: f64, frame_rate: f64, repeats: usize) -> Option<usize> {
    if !(seconds.is_finite() && start.is_finite() && frame_rate.is_finite() && frame_rate > 0.0) {
        return None;
    }
    let j = ((seconds - start) * frame_rate + 1e-6).floor();
    (j >= 1.0 && j <= repeats as f64).then_some(j as usize)
}

pub fn datamosh_bloom(img: &mut Image, e: &EffectInstance, cx: &FxCtx<'_>) {
    let Some(env) = cx.env else { return };
    let repeats = {
        let r = fv(e, "repeats", cx);
        if r.is_finite() { r.round().clamp(1.0, MAX_REPEATS as f32) as usize } else { 1 }
    };
    let start = fv(e, "start", cx) as f64;
    let Some(j) = repeat_at(cx.seconds, start, cx.frame_rate, repeats) else { return };
    let fd = 1.0 / cx.frame_rate;
    let (Some(prev), Some(cur)) = (env.frame(start - fd - cx.seconds, e), env.frame(start - cx.seconds, e)) else { return };
    if (prev.w, prev.h) != (img.w, img.h) || (cur.w, cur.h) != (img.w, img.h) {
        return;
    }
    let qp = qp_for(fv(e, "residual", cx));
    let mut hs = DefaultHasher::new();
    qp.hash(&mut hs);
    hash_image(&prev, &mut hs);
    hash_image(&cur, &mut hs);
    let key = hs.finish();
    let Some(bloomed) = bloom(key, &prev, &cur, qp, j) else { return };
    // Dissolve: the clip comes back over the last part of the repeats
    let d = (fv(e, "dissolve", cx) / 100.0).clamp(0.0, 1.0);
    let k = if d > 0.0 { crate::transitions::smoothstep(1.0 - d, 1.0, j as f32 / repeats as f32) } else { 0.0 };
    for (o, q) in img.px.chunks_exact_mut(4).zip(bloomed.px.chunks_exact(4)) {
        for c in 0..4 {
            o[c] = q[c] + (o[c] - q[c]) * k;
        }
    }
}

/// The picture after `j` repeats of the motion `prev` → `cur`, starting from `cur`.
fn bloom(key: u64, prev: &Image, cur: &Image, qp: u8, j: usize) -> Option<Image> {
    let mut kept = match take_kept(key) {
        Some(k) if k.last.0 <= j => k,
        _ => Kept { key, bloom: Bloom::new(prev, cur, qp).ok()?, last: (0, cur.clone()) },
    };
    while kept.last.0 < j {
        let next = kept.bloom.step(&kept.last.1).ok()?;
        kept.last = (kept.last.0 + 1, next);
    }
    let out = kept.last.1.clone();
    keep(kept);
    Some(out)
}

#[cfg(test)]
mod tests;
