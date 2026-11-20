//! Real, codec-level datamosh with CFA4's own H.264 encoder and decoder.
//!
//! A datamosh drops a key frame: the decoder applies the motion vectors and residuals of one
//! picture sequence to a reference picture from somewhere else. Here no codec is changed; the
//! trick is in the order of the packets:
//!
//! 1. The frames of B are encoded with `cfa4-h264enc` as one IDR followed by P pictures only (no
//!    B pictures, no scene-cut IDRs, a fixed QP).
//! 2. A second encoder with the same configuration (so the same SPS / PPS) encodes the reference
//!    picture A as a single IDR.
//! 3. `cfa4-h264` decodes A's IDR followed by B's P pictures: the first P picture predicts from
//!    "the previous IDR", which is now A, so A's pixels move with B's motion while B's residuals
//!    bleed in.
//!
//! Pictures go through 8-bit full-range BT.709 Y'CbCr 4:2:0 of the display-encoded colour
//! (premultiplied, i.e. over black); the results come back opaque. Every failure is an
//! [`MoshError`]: callers fall back to a plain picture, nothing panics.

use cfa4_h264enc::{Encoder, EncoderConfig, FrameType, PacketFormat, Preset, Profile, RateControl, YuvFrame};

use crate::effects::{dec, enc};
use crate::image::Image;

/// The largest number of pictures one mosh encodes (memory and time stay bounded).
pub const MAX_FRAMES: usize = 240;

/// Why a mosh could not be made.
#[derive(Debug, PartialEq, Eq)]
pub enum MoshError {
    /// No pictures, pictures of different sizes, or a size the encoder cannot take.
    BadInput,
    Encode(String),
    Decode(String),
    /// The decoder gave back a different number of pictures than it was fed.
    MissingPictures,
}

impl std::fmt::Display for MoshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoshError::BadInput => write!(f, "datamosh: unusable input pictures"),
            MoshError::Encode(e) => write!(f, "datamosh: encoder: {e}"),
            MoshError::Decode(e) => write!(f, "datamosh: decoder: {e}"),
            MoshError::MissingPictures => write!(f, "datamosh: the decoder returned too few pictures"),
        }
    }
}

impl std::error::Error for MoshError {}

/// A 4:2:0 picture of even size (the image's size rounded up, edges repeated).
struct Yuv {
    w: usize,
    h: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

fn even(n: usize) -> usize {
    n.saturating_add(n & 1)
}

fn q8(v: f32) -> u8 {
    // (`as` saturates; NaN → 0)
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// BT.709 full-range Y'CbCr of display-encoded RGB (all in 0..1, chroma centred on 0.5).
fn ycbcr(c: [f32; 3]) -> [f32; 3] {
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    [y, (c[2] - y) / 1.8556 + 0.5, (c[0] - y) / 1.5748 + 0.5]
}

fn rgb(y: f32, cb: f32, cr: f32) -> [f32; 3] {
    let (cb, cr) = (cb - 0.5, cr - 0.5);
    let r = y + 1.5748 * cr;
    let b = y + 1.8556 * cb;
    let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
    [r, g, b]
}

fn to_yuv(img: &Image) -> Option<Yuv> {
    if img.w == 0 || img.h == 0 || img.px.len() != img.w * img.h * 4 {
        return None;
    }
    let (w, h) = (even(img.w), even(img.h));
    let px = |x: usize, y: usize| {
        let p = img.get(x.min(img.w - 1), y.min(img.h - 1));
        ycbcr(enc([p[0], p[1], p[2]]).map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
    };
    let mut y = vec![0u8; w * h];
    let (cw, ch) = (w / 2, h / 2);
    let (mut u, mut v) = (vec![0u8; cw * ch], vec![0u8; cw * ch]);
    for cy in 0..ch {
        for cx in 0..cw {
            let mut acc = [0f32; 2];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, yy) = (cx * 2 + dx, cy * 2 + dy);
                let c = px(x, yy);
                if let Some(o) = y.get_mut(yy * w + x) {
                    *o = q8(c[0]);
                }
                acc[0] += c[1];
                acc[1] += c[2];
            }
            if let (Some(a), Some(b)) = (u.get_mut(cy * cw + cx), v.get_mut(cy * cw + cx)) {
                *a = q8(acc[0] / 4.0);
                *b = q8(acc[1] / 4.0);
            }
        }
    }
    Some(Yuv { w, h, y, u, v })
}

/// A decoded 8-bit 4:2:0 picture back to a `w`×`h` opaque linear image.
fn from_picture(p: &cfa4_h264::Picture, w: usize, h: usize) -> Option<Image> {
    if p.bit_depth != 8 || p.chroma_format != 1 || (p.width as usize) < w || (p.height as usize) < h {
        return None;
    }
    let (ys, cs) = (p.y_stride, p.uv_stride);
    let mut out = Image::new(w, h);
    if out.w != w || out.h != h {
        return None;
    }
    for y in 0..h {
        for x in 0..w {
            let s = |v: u16| f32::from(v) / 255.0;
            let l = s(p.y.get(y * ys + x));
            let (cb, cr) = (s(p.u.get((y / 2) * cs + x / 2)), s(p.v.get((y / 2) * cs + x / 2)));
            let c = dec(rgb(l, cb, cr).map(|v| v.clamp(0.0, 1.0)));
            let i = (y * w + x) * 4;
            if let Some(o) = out.px.get_mut(i..i + 4) {
                o.copy_from_slice(&[c[0], c[1], c[2], 1.0]);
            }
        }
    }
    Some(out)
}

/// The encoder configuration both encoders share (so their SPS / PPS are identical): P pictures
/// only, one slice, a fixed QP.
fn config(w: usize, h: usize, qp: u8) -> EncoderConfig {
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.profile = Profile::High;
    cfg.preset = Preset::Speed;
    cfg.rate = RateControl::Qp(qp.min(51));
    cfg.keyint = u32::MAX / 2;
    cfg.bframes = 0;
    cfg.scenecut = false;
    cfg.threads = 1;
    cfg.slices = 1;
    cfg.aud = false;
    cfg.aq_strength = 0.0;
    cfg.format = PacketFormat::AnnexB;
    cfg.color.full_range = true;
    cfg
}

fn encode_all(cfg: EncoderConfig, frames: &[&Yuv]) -> Result<Vec<cfa4_h264enc::Packet>, MoshError> {
    let mut e = Encoder::new(cfg).map_err(|e| MoshError::Encode(e.to_string()))?;
    let mut out = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let frame = YuvFrame { y: &f.y, u: &f.u, v: &f.v, y_stride: f.w, uv_stride: f.w / 2 };
        out.extend(e.try_encode(&frame, i as i64).map_err(|e| MoshError::Encode(e.to_string()))?);
    }
    out.extend(e.flush());
    out.sort_by_key(|p| p.dts);
    Ok(out)
}

/// Datamosh `b` onto `a`: the pictures of `b` after its first, predicted from `a` instead of
/// `b[0]`. Returns `b.len() - 1` opaque pictures of `a`'s size (`b[0]` is the key frame that is
/// dropped). `qp` (0–51) is the quantizer: low lets more of B's detail through, high leaves
/// mostly the motion.
pub fn mosh(a: &Image, b: &[Image], qp: u8) -> Result<Vec<Image>, MoshError> {
    let (w, h) = (a.w, a.h);
    if b.len() < 2 || b.len() > MAX_FRAMES || b.iter().any(|f| f.w != w || f.h != h) {
        return Err(MoshError::BadInput);
    }
    let ya = to_yuv(a).ok_or(MoshError::BadInput)?;
    let yb: Vec<Yuv> = b.iter().map(to_yuv).collect::<Option<_>>().ok_or(MoshError::BadInput)?;
    let cfg = config(ya.w, ya.h, qp);
    let pb = encode_all(cfg.clone(), &yb.iter().collect::<Vec<_>>())?;
    let pa = encode_all(cfg, &[&ya])?;
    let key = pa.first().filter(|p| p.frame_type == FrameType::Idr).ok_or(MoshError::Encode("no IDR for the reference".into()))?;
    if pb.first().map(|p| p.frame_type) != Some(FrameType::Idr) || pb.iter().skip(1).any(|p| p.frame_type != FrameType::P) {
        return Err(MoshError::Encode("B is not one IDR followed by P pictures".into()));
    }
    let mut d = cfa4_h264::Decoder::with_threads(1);
    let mut pics = d.decode(&key.data, 0).map_err(|e| MoshError::Decode(e.to_string()))?;
    for (i, p) in pb.iter().enumerate().skip(1) {
        pics.extend(d.decode(&p.data, i as i64).map_err(|e| MoshError::Decode(e.to_string()))?);
    }
    pics.extend(d.flush());
    if let Some(e) = d.take_error() {
        return Err(MoshError::Decode(e.to_string()));
    }
    pics.sort_by_key(|p| p.pts);
    // the first picture is A itself
    if pics.len() != b.len() {
        return Err(MoshError::MissingPictures);
    }
    pics.iter().skip(1).map(|p| from_picture(p, w, h).ok_or(MoshError::MissingPictures)).collect()
}

#[cfg(test)]
mod tests;
