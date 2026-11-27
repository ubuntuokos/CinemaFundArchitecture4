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
/// only, one slice, a fixed QP. `clean` lets the encoder refresh poorly predicted blocks with
/// intra blocks of the new picture; without it every block is moved from the reference, so the
/// mosh stays dirtier for longer.
fn config(w: usize, h: usize, qp: u8, clean: bool) -> EncoderConfig {
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
    cfg.inter_only = !clean;
    cfg
}

fn yuv_frame(f: &Yuv) -> YuvFrame<'_> {
    YuvFrame { y: &f.y, u: &f.u, v: &f.v, y_stride: f.w, uv_stride: f.w / 2 }
}

/// A running mosh: B's pictures are fed one at a time and each comes back predicted from A (and
/// the moshed pictures before it). Feeding is incremental, so a transition that plays forward
/// pays one encode and one decode per frame, not the whole run again.
pub struct Session {
    w: usize,
    h: usize,
    enc: Encoder,
    dec: cfa4_h264::Decoder,
    /// Pictures of B fed so far, `b[0]` included.
    fed: usize,
}

impl Session {
    /// Start a mosh of `b0` (B's first picture, the key frame that gets dropped) onto `a`.
    /// `clean`: B's P pictures may carry intra blocks (see [`config`]).
    pub fn new(a: &Image, b0: &Image, qp: u8, clean: bool) -> Result<Self, MoshError> {
        let (w, h) = (a.w, a.h);
        if (b0.w, b0.h) != (w, h) {
            return Err(MoshError::BadInput);
        }
        let ya = to_yuv(a).ok_or(MoshError::BadInput)?;
        let yb = to_yuv(b0).ok_or(MoshError::BadInput)?;
        let cfg = config(ya.w, ya.h, qp, clean);
        let enc_err = |e: cfa4_h264enc::Error| MoshError::Encode(e.to_string());
        // A's key frame from its own encoder (same configuration, so the same SPS / PPS)
        let mut ea = Encoder::new(cfg.clone()).map_err(enc_err)?;
        let mut pa = ea.try_encode(&yuv_frame(&ya), 0).map_err(enc_err)?;
        pa.extend(ea.flush());
        let key = match pa.as_slice() {
            [p] if p.frame_type == FrameType::Idr => p,
            _ => return Err(MoshError::Encode("no IDR for the reference".into())),
        };
        // B's key frame is encoded (so its P pictures follow it) but never decoded
        let mut enc = Encoder::new(cfg).map_err(enc_err)?;
        let pb = enc.try_encode(&yuv_frame(&yb), 0).map_err(enc_err)?;
        if !matches!(pb.as_slice(), [p] if p.frame_type == FrameType::Idr) {
            return Err(MoshError::Encode("B does not start with one IDR".into()));
        }
        let mut dec = cfa4_h264::Decoder::with_threads(1);
        let pics = dec.decode(&key.data, 0).map_err(|e| MoshError::Decode(e.to_string()))?;
        if pics.len() != 1 {
            return Err(MoshError::MissingPictures);
        }
        Ok(Self { w, h, enc, dec, fed: 1 })
    }

    /// Pictures of B fed so far (`b[0]` included).
    pub fn fed(&self) -> usize {
        self.fed
    }

    /// Feed B's next picture; returns it moshed (opaque, `a`'s size).
    pub fn push(&mut self, b: &Image) -> Result<Image, MoshError> {
        if (b.w, b.h) != (self.w, self.h) || self.fed >= MAX_FRAMES {
            return Err(MoshError::BadInput);
        }
        let yb = to_yuv(b).ok_or(MoshError::BadInput)?;
        let pts = self.fed as i64;
        let packets = self.enc.try_encode(&yuv_frame(&yb), pts).map_err(|e| MoshError::Encode(e.to_string()))?;
        if packets.iter().any(|p| p.frame_type != FrameType::P) {
            return Err(MoshError::Encode("B is not one IDR followed by P pictures".into()));
        }
        let mut pics = Vec::new();
        for p in &packets {
            pics.extend(self.dec.decode(&p.data, pts).map_err(|e| MoshError::Decode(e.to_string()))?);
        }
        if let Some(e) = self.dec.take_error() {
            return Err(MoshError::Decode(e.to_string()));
        }
        self.fed += 1;
        match pics.as_slice() {
            [p] => from_picture(p, self.w, self.h).ok_or(MoshError::MissingPictures),
            _ => Err(MoshError::MissingPictures),
        }
    }
}

/// A P-frame bloom: the motion from `prev` to `cur` (one P picture of CFA4's encoder) applied
/// again and again. Each [`Bloom::step`] encodes the picture so far as a key frame and decodes
/// the same P picture onto it, so the motion is repeated without rewriting the packet.
pub struct Bloom {
    w: usize,
    h: usize,
    cfg: EncoderConfig,
    p: Vec<u8>,
}

impl Bloom {
    /// `clean`: the P picture may carry intra blocks (see [`config`]).
    pub fn new(prev: &Image, cur: &Image, qp: u8, clean: bool) -> Result<Self, MoshError> {
        let (w, h) = (cur.w, cur.h);
        if (prev.w, prev.h) != (w, h) {
            return Err(MoshError::BadInput);
        }
        let (y0, y1) = (to_yuv(prev).ok_or(MoshError::BadInput)?, to_yuv(cur).ok_or(MoshError::BadInput)?);
        let cfg = config(y0.w, y0.h, qp, clean);
        let enc_err = |e: cfa4_h264enc::Error| MoshError::Encode(e.to_string());
        let mut enc = Encoder::new(cfg.clone()).map_err(enc_err)?;
        let first = enc.try_encode(&yuv_frame(&y0), 0).map_err(enc_err)?;
        if !matches!(first.as_slice(), [p] if p.frame_type == FrameType::Idr) {
            return Err(MoshError::Encode("no IDR for the previous picture".into()));
        }
        let mut second = enc.try_encode(&yuv_frame(&y1), 1).map_err(enc_err)?;
        second.extend(enc.flush());
        match second.as_slice() {
            [p] if p.frame_type == FrameType::P => Ok(Self { w, h, cfg, p: p.data.clone() }),
            _ => Err(MoshError::Encode("no P picture for the motion".into())),
        }
    }

    /// The motion applied once more to `r` (opaque, the bloom's size).
    pub fn step(&self, r: &Image) -> Result<Image, MoshError> {
        if (r.w, r.h) != (self.w, self.h) {
            return Err(MoshError::BadInput);
        }
        let yr = to_yuv(r).ok_or(MoshError::BadInput)?;
        let enc_err = |e: cfa4_h264enc::Error| MoshError::Encode(e.to_string());
        let mut enc = Encoder::new(self.cfg.clone()).map_err(enc_err)?;
        let mut key = enc.try_encode(&yuv_frame(&yr), 0).map_err(enc_err)?;
        key.extend(enc.flush());
        let key = match key.as_slice() {
            [p] if p.frame_type == FrameType::Idr => p,
            _ => return Err(MoshError::Encode("no IDR for the bloom".into())),
        };
        let dec_err = |e: cfa4_h264::Error| MoshError::Decode(e.to_string());
        let mut dec = cfa4_h264::Decoder::with_threads(1);
        let mut pics = dec.decode(&key.data, 0).map_err(dec_err)?;
        pics.extend(dec.decode(&self.p, 1).map_err(dec_err)?);
        pics.extend(dec.flush());
        if let Some(e) = dec.take_error() {
            return Err(MoshError::Decode(e.to_string()));
        }
        pics.sort_by_key(|p| p.pts);
        match pics.as_slice() {
            [_, p] => from_picture(p, self.w, self.h).ok_or(MoshError::MissingPictures),
            _ => Err(MoshError::MissingPictures),
        }
    }
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
    let mut s = Session::new(a, &b[0], qp, true)?;
    b[1..].iter().map(|f| s.push(f)).collect()
}

#[cfg(test)]
mod tests;
