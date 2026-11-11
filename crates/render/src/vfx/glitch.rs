//! CFA4's own Glitch folder: Bit Crush, Block Glitch, Databend, VHS, Scan Line Jitter and Tile
//! Jitter.
//!
//! Bit Crush, Block Glitch, Databend and VHS are ported from glitchESP
//! (<https://github.com/koupypavel/glitchESP>, `firmware/main/effects/fx_bitcrush.c`,
//! `fx_blocks.c`, `fx_databend.c`, `fx_vhs.c`, `fx_rng.h`; Copyright (c) 2026 koupypavel and
//! glitchESP contributors, MIT License). Scan Line Jitter and Tile Jitter follow the X-PostProcessing
//! Library (<https://github.com/QianMo/X-PostProcessing-Library>, `Effects/GlitchScanLineJitter`,
//! `Effects/GlitchTileJitter`; Copyright (C) 2020 QianMo, MIT License), whose scan line jitter
//! curve comes from KinoGlitch (Copyright (C) 2015 Keijiro Takahashi, MIT License). The ports
//! work on CFA4's premultiplied linear f32 images instead of RGB565, take time from the clip
//! instead of a frame counter, and replace fixed-size buffers with bounds checked from the
//! (hostile) parameters. The three MIT notices are in `glitch/LICENSE-glitch-sources`; see also
//! `docs/licensing.md` §5.
//!
//! Randomness is a pure function of (Random Seed, time step, effect stream, row or block), so a frame
//! renders the same on every core count and every re-render.

use std::f32::consts::TAU;

use rayon::prelude::*;

use super::{bv, chv, dec, enc, fv, hash01};
use crate::effects::FxCtx;
use crate::image::Image;
use cfa4_project::EffectInstance;

/// glitchESP sizes its look for a 720-pixel-wide frame; this scales it to the working width.
const REF_W: f32 = 720.0;

/// A finite parameter value, or `fallback`.
fn fin(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// The Random Seed parameter.
fn seed_of(e: &EffectInstance, cx: &FxCtx) -> u32 {
    fin(fv(e, "seed", cx), 0.0).clamp(0.0, 1.0e6) as u32
}

/// The time step for an effect that re-rolls `rate` times a second (0 = never).
fn step(cx: &FxCtx, rate: f32) -> u32 {
    let t = cx.seconds * f64::from(fin(rate, 0.0).max(0.0));
    if t.is_finite() && t > 0.0 { (t.floor() % 4.0e9) as u32 } else { 0 }
}

/// The whole image is there to work on.
fn usable(img: &Image) -> bool {
    img.w > 0 && img.h > 0 && img.px.len() == img.w * img.h * 4
}

/// glitchESP's `fx_hash32`.
fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// glitchESP's xorshift32 generator (`fx_rng.h`).
struct Rng(u32);

impl Rng {
    /// A generator for one row / band / tile of one time step. `stream` keeps the effects'
    /// sequences apart (not cryptographic: these are look-variation generators).
    fn at(seed: u32, frame: u32, stream: u32, index: u32) -> Self {
        let s = hash32(seed ^ frame.wrapping_mul(0x9e37_79b9) ^ stream ^ index.wrapping_mul(0x85eb_ca6b));
        Rng(if s == 0 { 0x0123_4567 } else { s })
    }
    fn u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [0, 1).
    fn f(&mut self) -> f32 {
        (self.u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
    /// Uniform integer in [lo, hi].
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo).saturating_add(1).min(u32::MAX as i64) as u32;
        lo + i64::from(self.u32() % span)
    }
}

// ------------------------------------------------------------------ Bit Crush

/// glitchESP's 4×4 Bayer matrix (values 0..15).
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Bit Crush: keeps `Bits per Channel` bits of each sRGB-encoded channel, with an ordered 4×4
/// Bayer dither (cells of `Dither Cell Size` pixels) so flat areas break into a pattern instead
/// of bands.
pub fn bit_crush(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if !usable(img) {
        return;
    }
    let bits = fin(fv(e, "bits", cx), 3.0).round().clamp(1.0, 16.0);
    let top = (2f32.powf(bits) - 1.0).max(1.0);
    let dither = bv(e, "dither");
    let cell = (fin(fv(e, "cell", cx), 1.0) * cx.px_scale).max(1.0);
    img.map_rgb(|c, x, y| {
        let t = if dither {
            let (bx, by) = (((x as f32 / cell) as usize) & 3, ((y as f32 / cell) as usize) & 3);
            (f32::from(BAYER[by][bx]) + 0.5) / 16.0
        } else {
            0.5
        };
        dec(enc(c).map(|v| (v.clamp(0.0, 1.0) * top + t).floor().min(top) / top))
    });
}

// ------------------------------------------------------------------ Block Glitch

/// Copy a `tw`×`th` rectangle of `src` at (sx, sy) into `dst` at (dx, dy); both lie inside.
#[allow(clippy::too_many_arguments)]
fn copy_rect(src: &Image, dst: &mut Image, sx: usize, sy: usize, dx: usize, dy: usize, tw: usize, th: usize) {
    let w = src.w;
    for r in 0..th {
        let (s0, d0) = (((sy + r) * w + sx) * 4, ((dy + r) * w + dx) * 4);
        if let (Some(s), Some(d)) = (src.px.get(s0..s0 + tw * 4), dst.px.get_mut(d0..d0 + tw * 4)) {
            d.copy_from_slice(s);
        }
    }
}

/// Block Glitch: `Blocks Moved` rectangular blocks are copied (or swapped) to random places
/// near where they came from, mostly snapped to the block grid, like corrupted macroblocks of a
/// broken video stream. Blocks are always read from the untouched picture.
pub fn block_glitch(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if !usable(img) {
        return;
    }
    let count = fin(fv(e, "count", cx), 0.0).round().clamp(0.0, 400.0) as usize;
    if count == 0 {
        return;
    }
    let (w, h) = (img.w, img.h);
    let size = fin(fv(e, "size", cx), 48.0).max(0.0) * cx.px_scale;
    let aspect = fin(fv(e, "aspect", cx), 2.0).clamp(0.25, 4.0);
    let th = (size.round() as usize).clamp(1, h);
    let tw = ((size * aspect).round() as usize).clamp(1, w);
    let maxd = (fin(fv(e, "distance", cx), 0.0).max(0.0) * cx.px_scale).min(w.max(h) as f32) as i64;
    let (snap, swap) = (bv(e, "snap"), bv(e, "swap"));
    let mut rng = Rng::at(seed_of(e, cx), step(cx, fv(e, "rate", cx)), 0xB10C, 0);
    let (xmax, ymax) = ((w - tw) as i64, (h - th) as i64);
    let src = img.clone();
    for _ in 0..count {
        let sx = rng.range(0, xmax);
        let sy = rng.range(0, ymax);
        let mut dx = (sx + rng.range(-maxd, maxd)).clamp(0, xmax);
        let mut dy = (sy + rng.range(-maxd, maxd)).clamp(0, ymax);
        // the draw happens either way, so Snap to Grid keeps the same blocks; a grid position
        // further than Max Distance from the source is not taken
        if rng.f() < 0.7 && snap {
            let (gx, gy) = ((dx - dx % tw as i64).min(xmax), (dy - dy % th as i64).min(ymax));
            if (gx - sx).abs() <= maxd {
                dx = gx;
            }
            if (gy - sy).abs() <= maxd {
                dy = gy;
            }
        }
        let (sx, sy, dx, dy) = (sx as usize, sy as usize, dx as usize, dy as usize);
        if swap {
            copy_rect(&src, img, dx, dy, sx, sy, tw, th);
        }
        copy_rect(&src, img, sx, sy, dx, dy, tw, th);
    }
}

// ------------------------------------------------------------------ Databend

/// The decoder state after a damaged byte: in force from that block to the next break.
#[derive(Clone, Copy, Default)]
struct Break {
    /// Where along the block stream, 0..65535.
    at: u16,
    /// Running sideways shift, in 1/256 of the width.
    shift: i64,
    /// Running colour offset (sRGB-encoded).
    dc: [f32; 3],
    /// The block at the break is repeated to the end of its run.
    smear: bool,
}

/// Databend: the look of a JPEG with a few damaged bytes. A JPEG stores blocks row by row, each
/// as the difference from the one before, so one bad byte shifts everything after it sideways and
/// offsets its colours until the next restart marker. `Breaks` damaged spots are placed along the
/// block stream; each changes the running shift and colour (or smears one block across the row).
/// An imitation, drawn directly: fast, repeatable from the seed and at any frame size.
pub fn databend(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if !usable(img) {
        return;
    }
    let shift = (fin(fv(e, "shift", cx), 0.0) / 100.0).clamp(0.0, 1.0);
    let color = (fin(fv(e, "color", cx), 0.0) / 100.0).clamp(0.0, 1.0);
    if shift <= 0.0 && color <= 0.0 {
        return;
    }
    let n = fin(fv(e, "breaks", cx), 10.0).round().clamp(1.0, 40.0) as usize;
    let mut rng = Rng::at(seed_of(e, cx), step(cx, fv(e, "rate", cx)), 0xDA7A, 0);
    let mut breaks = vec![Break::default(); n];
    for b in &mut breaks {
        b.at = (rng.u32() >> 16) as u16;
    }
    breaks.sort_by_key(|b| b.at);
    let (max_shift, max_c) = ((shift * 96.0) as i64, (color * 10.0) as i64);
    let (mut sh, mut dr, mut dg, mut db) = (0i64, 0i64, 0i64, 0i64);
    for b in &mut breaks {
        let kind = rng.u32() % 10;
        if kind == 0 {
            // a restart marker: the decoder recovers
            (sh, dr, dg, db) = (0, 0, 0, 0);
        } else {
            if max_shift > 0 {
                sh += rng.range(-max_shift, max_shift);
            }
            if max_c > 0 {
                dr = (dr + rng.range(-max_c, max_c)).clamp(-14, 14);
                dg = (dg + rng.range(-2 * max_c, 2 * max_c)).clamp(-28, 28);
                db = (db + rng.range(-max_c, max_c)).clamp(-14, 14);
            }
        }
        // colour steps of the original RGB565 channels (5, 6 and 5 bits)
        (b.shift, b.dc, b.smear) = (sh, [dr as f32 / 31.0, dg as f32 / 63.0, db as f32 / 31.0], kind == 1 && color > 0.0);
    }

    let (w, h) = (img.w, img.h);
    let bs = ((fin(fv(e, "size", cx), 16.0) * cx.px_scale).round().max(1.0) as usize).min(w.max(h));
    let cols = w.div_ceil(bs) as u64;
    let total = cols * h.div_ceil(bs) as u64;
    let block_of = |b: &Break| (u64::from(b.at) * total) >> 16;
    let clean = Break::default();
    let src = img.clone();
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let Some(srow) = src.px.get(y * w * 4..(y + 1) * w * 4) else { return };
        let first = (y / bs) as u64 * cols;
        // the state in force at the start of the row: the last break at or before it
        let mut i = 0;
        let mut state = &clean;
        while let Some(b) = breaks.get(i).filter(|b| block_of(b) <= first) {
            state = b;
            i += 1;
        }
        let mut x = 0;
        while x < w {
            // this run ends at the next break inside the row, or at the row's end
            let mut end = w;
            if let Some(b) = breaks.get(i) {
                let blk = block_of(b);
                if blk < first + cols {
                    end = ((blk.saturating_sub(first)) as usize).saturating_mul(bs).min(w);
                }
            }
            if end > x {
                let sx = (x as i64 + state.shift * w as i64 / 256).rem_euclid(w as i64) as usize;
                if let Some(dst) = row.get_mut(x * 4..end * 4) {
                    run(dst, srow, sx, state);
                }
                x = end;
            }
            if x < w {
                match breaks.get(i) {
                    Some(b) => state = b,
                    None => break,
                }
                i += 1;
            }
        }
    });
}

/// One run of a damaged row: `dst` takes the source row from `sx` on (wrapping), with the state's
/// colour offset, or the single pixel at `sx` repeated (a stuck block).
fn run(dst: &mut [f32], src: &[f32], sx: usize, b: &Break) {
    let w = src.len() / 4;
    for (i, o) in dst.chunks_exact_mut(4).enumerate() {
        let j = if b.smear { sx } else { (sx + i) % w.max(1) };
        let Some(p) = src.get(j * 4..j * 4 + 4) else { return };
        o.copy_from_slice(p);
        let a = p[3];
        if a > 1e-6 && b.dc != [0.0; 3] {
            let c = enc([p[0] / a, p[1] / a, p[2] / a]);
            let c = dec([c[0] + b.dc[0], c[1] + b.dc[1], c[2] + b.dc[2]]);
            o[..3].copy_from_slice(&[c[0] * a, c[1] * a, c[2] * a]);
        }
    }
}

// ------------------------------------------------------------------ VHS

/// VHS: what a worn tape does to a picture. Red and blue smear to the right (tape stores colour
/// with far less bandwidth than brightness), the rows wander sideways on a slow sine, a torn
/// tracking bar drifts up the picture, the bottom rows are skewed where the heads switch, white
/// snow dashes are scattered over it, and every other row is a little darker.
pub fn vhs(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if !usable(img) {
        return;
    }
    let (w, h) = (img.w, img.h);
    let k = w as f32 / REF_W;
    let bleed = fin(fv(e, "bleed", cx), 0.0).max(0.0) * cx.px_scale;
    // beyond the width every row is the clamped edge anyway; bounding it keeps the shifts in range
    let wobble = (fin(fv(e, "wobble", cx), 0.0).max(0.0) * cx.px_scale).min(w as f32);
    let frac = |id: &str| (fin(fv(e, id, cx), 0.0) / 100.0).clamp(0.0, 1.0);
    let (snow, track, head, scan) = (frac("snow"), frac("tracking"), frac("head"), frac("scanlines"));
    // the picture changes every frame, like the tape running
    let fps = if cx.frame_rate.is_finite() && cx.frame_rate > 0.0 { cx.frame_rate as f32 } else { 24.0 };
    let frame = step(cx, fps);
    let seed = seed_of(e, cx);

    // low-pass strength for red/blue: reaches a step after roughly `bleed` pixels
    let alpha = if bleed > 0.0 { (1.0 / (1.0 + bleed / 2.0)).max(8.0 / 256.0) } else { 1.0 };
    let head_rows = (h as f32 * 0.07 * head) as usize;
    let head_start = h - head_rows.min(h);
    let bar_h = (60.0 * k * track) as u64;
    let bar_top = if bar_h > 0 { h as i64 - ((u64::from(frame) * (7.0 * k + 1.0) as u64) % (h as u64 + bar_h)) as i64 } else { h as i64 };
    let phase = (f64::from(frame) * 0.21).rem_euclid(std::f64::consts::TAU) as f32;
    let inv_len = TAU / (180.0 * k).max(1e-3);
    let dim = 1.0 - 0.25 * scan;
    // glitchESP's dash colours: white, and a light grey for one dash in four
    let (white, grey) = (dec([1.0; 3])[0], dec([0.74; 3])[0]);
    let src = img.clone();
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let Some(srow) = src.px.get(y * w * 4..(y + 1) * w * 4) else { return };
        let mut rng = Rng::at(seed, frame, 0x0785, y as u32);
        // how far this row is pushed sideways
        let mut shift = (wobble * (y as f32 * inv_len + phase).sin()) as i64;
        if wobble > 0.0 && rng.u32() & 7 == 0 {
            shift += rng.range(-1, 1);
        }
        let mut torn = false;
        if head_rows > 0 && y >= head_start {
            shift += (y - head_start) as i64 * 3 + rng.range(0, (12.0 * k) as i64 + 1);
            torn = true;
        }
        if bar_h > 0 && (y as i64) >= bar_top && (y as i64) < bar_top + bar_h as i64 {
            let j = (40.0 * k) as i64 + 1;
            shift += rng.range(-j, j);
            torn = true;
        }
        let at = |x: i64| {
            let i = x.saturating_sub(shift).clamp(0, w as i64 - 1) as usize * 4;
            [srow[i], srow[i + 1], srow[i + 2], srow[i + 3]]
        };
        // red and blue are filtered as straight colour and premultiplied by each pixel's own
        // alpha, so a translucent edge never gets more colour than its coverage
        let straight = |p: [f32; 4]| Image::unpremul(p);
        let first = straight(at(0));
        let (mut r, mut b) = (first[0], first[2]);
        for (x, o) in row.chunks_exact_mut(4).enumerate() {
            let s = at(x as i64);
            let c = straight(s);
            r += (c[0] - r) * alpha;
            b += (c[2] - b) * alpha;
            let d = if y & 1 == 1 { dim } else { 1.0 };
            o.copy_from_slice(&[r * s[3] * d, s[1] * d, b * s[3] * d, s[3]]);
        }
        // snow: a few white dashes; torn rows get many
        let chance = snow * 0.08 + if torn { 0.6 } else { 0.0 };
        let dashes = if rng.f() < chance { 1 + rng.u32() % if torn { 6 } else { 2 } } else { 0 };
        for _ in 0..dashes {
            let x0 = rng.range(0, w as i64 - 1) as usize;
            let len = rng.range(2, (10.0 * k) as i64 + 2) as usize;
            let c = if rng.u32() & 3 != 0 { white } else { grey };
            for o in row.chunks_exact_mut(4).skip(x0).take(len) {
                o.copy_from_slice(&[c, c, c, 1.0]);
            }
        }
    });
}

// ------------------------------------------------------------------ Scan Line Jitter / Tile Jitter

/// The Periodic interval's on/off pulse (1 for Infinite).
fn strength(e: &EffectInstance, cx: &FxCtx) -> f32 {
    if chv(e, "interval") == 1 {
        let f = fin(fv(e, "frequency", cx), 0.0);
        let s = 0.5 + 0.5 * (cx.seconds as f32 * f).cos();
        if s.is_finite() { s } else { 1.0 }
    } else {
        1.0
    }
}

/// Linear sample along a wrapped line of `n` pixels (`get(i)` for i in 0..n) at `pos` (pixel
/// centres at +0.5).
fn wrapped(n: usize, pos: f32, get: impl Fn(usize) -> [f32; 4]) -> [f32; 4] {
    let f = pos - 0.5;
    let i0 = f.floor();
    let t = f - i0;
    let i0 = (i0 as i64).rem_euclid(n as i64) as usize;
    let (a, b) = (get(i0), get((i0 + 1) % n));
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// Scan Line Jitter: single rows (or columns) jump sideways and wrap around, more of them and
/// further as Jitter rises, re-rolled `Changes per Second` times a second.
pub fn scan_line_jitter(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if !usable(img) {
        return;
    }
    let a = (fin(fv(e, "amount", cx), 0.0) / 100.0).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    // KinoGlitch's curve: small jumps at first, then many large ones
    let disp = (0.005 + a.powi(3) * 0.1) * strength(e, cx);
    let thr = (1.0 - a * 1.2).clamp(0.0, 1.0);
    let tick = step(cx, fv(e, "rate", cx));
    let seed = u64::from(seed_of(e, cx));
    let jitter = |line: usize| {
        let r = hash01(line as i64, i64::from(tick), 0x5CA1, seed) * 2.0 - 1.0;
        if r.abs() >= thr { r * disp } else { 0.0 }
    };
    let (w, h) = (img.w, img.h);
    let src = img.clone();
    let get = |x: usize, y: usize| src.get(x.min(w - 1), y.min(h - 1));
    if chv(e, "direction") == 0 {
        img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            let d = jitter(y) * w as f32;
            for (x, o) in row.chunks_exact_mut(4).enumerate() {
                o.copy_from_slice(&wrapped(w, x as f32 + 0.5 + d, |i| get(i, y)));
            }
        });
    } else {
        let shifts: Vec<f32> = (0..w).map(|x| jitter(x) * h as f32).collect();
        img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            for (x, o) in row.chunks_exact_mut(4).enumerate() {
                let d = shifts.get(x).copied().unwrap_or(0.0);
                o.copy_from_slice(&wrapped(h, y as f32 + 0.5 + d, |i| get(x, i)));
            }
        });
    }
}

/// Tile Jitter: the picture is cut into `Tiles` bands across (or down) the frame and every other
/// band slides back and forth by up to Amount pixels, at Speed radians a second.
pub fn tile_jitter(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    if !usable(img) {
        return;
    }
    let speed = fin(fv(e, "speed", cx), 0.0);
    let off = fin(fv(e, "amount", cx), 0.0) * cx.px_scale * (cx.seconds as f32 * speed).cos() * strength(e, cx);
    if !off.is_finite() || off.abs() < 1e-4 {
        return;
    }
    let tiles = fin(fv(e, "tiles", cx), 5.0).round().clamp(1.0, 50.0);
    let across = chv(e, "bands") == 0;
    let (dx, dy) = if chv(e, "direction") == 0 { (off, 0.0) } else { (0.0, off) };
    let (w, h) = (img.w as f32, img.h as f32);
    let src = img.clone();
    let width = img.w;
    img.px.par_chunks_mut(width * 4).enumerate().for_each(|(y, row)| {
        let py = y as f32 + 0.5;
        for (x, o) in row.chunks_exact_mut(4).enumerate() {
            let px = x as f32 + 0.5;
            let band = if across { py / h } else { px / w } * tiles;
            let p = if band.rem_euclid(2.0) < 1.0 { src.sample_bilinear_clamped(px + dx, py + dy) } else { src.get(x, y) };
            o.copy_from_slice(&p);
        }
    });
}

#[cfg(test)]
mod tests;
