//! The matte plane (C0 v0.1 spec, section 4): a key or an AI matte as one value.
//!
//! A matte is a single plane of **data, not colour**: values in [0, 1], no transfer function, no
//! colour-space tag, always *straight* alpha (the compositor premultiplies). Keyers (`key.*`) and
//! AI matting (`ai.matting.*`) produce the same type, so the method can change without anything
//! downstream changing. The binary run-length `MaskRef` of the object-mask plan is a different
//! type and stays as it is; [`MatteFrame::from_mask`] / [`MatteFrame::to_mask`] convert, with the
//! threshold explicit.
//!
//! Every constructor validates: a hostile size, length, value or hash is a [`MatteError`], never a
//! panic and never a silent clamp.

use std::sync::Arc;

use cfa4_time::Tick;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Largest width or height of a matte plane.
pub const MAX_MATTE_DIM: u32 = 16_384;
/// Largest number of samples in a matte plane (8192 × 8192 × 2).
pub const MAX_MATTE_PIXELS: usize = 1 << 27;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MatteError {
    #[error("matte size {0}×{1} is empty or larger than {MAX_MATTE_DIM} on a side")]
    Size(u32, u32),
    #[error("matte of {0} samples is larger than {MAX_MATTE_PIXELS}")]
    TooLarge(u64),
    #[error("matte data has {got} samples, {width}×{height} needs {need}")]
    Length { width: u32, height: u32, need: usize, got: usize },
    #[error("matte sample {index} is {value}: not finite or outside 0..=1")]
    Value { index: usize, value: f32 },
    #[error("not a SHA-256 hash (64 lower-case hex digits): {0:?}")]
    BadHash(String),
    #[error("not a matte method id: {0:?}")]
    BadMethod(String),
    #[error("matte is {got_w}×{got_h}, the picture is {want_w}×{want_h}")]
    SizeMismatch { got_w: u32, got_h: u32, want_w: u32, want_h: u32 },
    #[error("threshold {0} is not in (0, 1]")]
    Threshold(f32),
}

/// Sample type of a plane: 8 bit for preview and cache, 16 bit for alpha-keeping export, f32 for
/// internal work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatteDepth {
    U8,
    U16,
    F32,
}

impl MatteDepth {
    fn tag(self) -> u8 {
        match self {
            MatteDepth::U8 => 1,
            MatteDepth::U16 => 2,
            MatteDepth::F32 => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MatteData {
    U8(Arc<[u8]>),
    U16(Arc<[u16]>),
    F32(Arc<[f32]>),
}

impl MatteData {
    pub fn depth(&self) -> MatteDepth {
        match self {
            MatteData::U8(_) => MatteDepth::U8,
            MatteData::U16(_) => MatteDepth::U16,
            MatteData::F32(_) => MatteDepth::F32,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            MatteData::U8(v) => v.len(),
            MatteData::U16(v) => v.len(),
            MatteData::F32(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_size(width: u32, height: u32) -> Result<usize, MatteError> {
    if width == 0 || height == 0 || width > MAX_MATTE_DIM || height > MAX_MATTE_DIM {
        return Err(MatteError::Size(width, height));
    }
    let n = u64::from(width) * u64::from(height);
    if n > MAX_MATTE_PIXELS as u64 {
        return Err(MatteError::TooLarge(n));
    }
    Ok(n as usize)
}

/// How the matte was made: `key.ultra`, `key.color`, `key.luma`, or `ai.matting.<engine>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MatteMethod(String);

impl MatteMethod {
    pub fn parse(id: &str) -> Result<MatteMethod, MatteError> {
        let ok_prefix = id.starts_with("key.") || id.starts_with("ai.matting.");
        let ok_chars = id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'));
        let rest_empty = id.ends_with('.');
        if id.len() > 64 || !ok_prefix || !ok_chars || rest_empty {
            return Err(MatteError::BadMethod(id.chars().take(80).collect()));
        }
        Ok(MatteMethod(id.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_ai(&self) -> bool {
        self.0.starts_with("ai.matting.")
    }
}

impl TryFrom<String> for MatteMethod {
    type Error = MatteError;
    fn try_from(s: String) -> Result<Self, MatteError> {
        MatteMethod::parse(&s)
    }
}

impl From<MatteMethod> for String {
    fn from(m: MatteMethod) -> String {
        m.0
    }
}

/// The model an AI matte came from (identifier and the SHA-256 of its weights file).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub id: String,
    pub sha256: String,
}

/// Provenance: method, hash of the parameters, and the model for an AI matte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatteOrigin {
    pub method: MatteMethod,
    pub params_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
}

impl MatteOrigin {
    pub fn new(method: MatteMethod, params_hash: &str, model: Option<ModelRef>) -> Result<MatteOrigin, MatteError> {
        if !is_sha256_hex(params_hash) {
            return Err(MatteError::BadHash(params_hash.chars().take(80).collect()));
        }
        if let Some(m) = &model
            && (!is_sha256_hex(&m.sha256) || m.id.is_empty() || m.id.len() > 128)
        {
            return Err(MatteError::BadHash(m.sha256.chars().take(80).collect()));
        }
        Ok(MatteOrigin { method, params_hash: params_hash.to_owned(), model })
    }
}

/// Content-addressed reference to a stored matte (the store is the existing on-disk cache, not a
/// database).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MatteRefRaw")]
pub struct MatteRef {
    sha256: String,
    width: u32,
    height: u32,
    depth: MatteDepth,
}

#[derive(Deserialize)]
struct MatteRefRaw {
    sha256: String,
    width: u32,
    height: u32,
    depth: MatteDepth,
}

impl TryFrom<MatteRefRaw> for MatteRef {
    type Error = MatteError;
    fn try_from(r: MatteRefRaw) -> Result<Self, MatteError> {
        MatteRef::new(r.sha256, r.width, r.height, r.depth)
    }
}

impl MatteRef {
    pub fn new(sha256: impl Into<String>, width: u32, height: u32, depth: MatteDepth) -> Result<MatteRef, MatteError> {
        let sha256 = sha256.into();
        if !is_sha256_hex(&sha256) {
            return Err(MatteError::BadHash(sha256.chars().take(80).collect()));
        }
        check_size(width, height)?;
        Ok(MatteRef { sha256, width, height, depth })
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn depth(&self) -> MatteDepth {
        self.depth
    }
}

/// One matte plane for one picture.
#[derive(Clone, Debug, PartialEq)]
pub struct MatteFrame {
    width: u32,
    height: u32,
    data: MatteData,
    /// Presentation time of the picture this matte belongs to (no CFR rounding).
    pts: Tick,
    timecode: Option<String>,
    source: Option<String>,
    origin: MatteOrigin,
}

impl MatteFrame {
    pub fn new(width: u32, height: u32, data: MatteData, pts: Tick, origin: MatteOrigin) -> Result<MatteFrame, MatteError> {
        let need = check_size(width, height)?;
        if data.len() != need {
            return Err(MatteError::Length { width, height, need, got: data.len() });
        }
        if let MatteData::F32(v) = &data
            && let Some((index, &value)) = v.iter().enumerate().find(|(_, x)| !x.is_finite() || !(0.0..=1.0).contains(*x))
        {
            return Err(MatteError::Value { index, value });
        }
        Ok(MatteFrame { width, height, data, pts, timecode: None, source: None, origin })
    }

    pub fn with_timecode(mut self, timecode: impl Into<String>) -> MatteFrame {
        self.timecode = Some(timecode.into());
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> MatteFrame {
        self.source = Some(source.into());
        self
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn depth(&self) -> MatteDepth {
        self.data.depth()
    }
    pub fn data(&self) -> &MatteData {
        &self.data
    }
    pub fn pts(&self) -> Tick {
        self.pts
    }
    pub fn timecode(&self) -> Option<&str> {
        self.timecode.as_deref()
    }
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn origin(&self) -> &MatteOrigin {
        &self.origin
    }

    /// The matte must be exactly the size of the picture it is applied to (no silent resampling).
    pub fn check_matches(&self, width: u32, height: u32) -> Result<(), MatteError> {
        if self.width == width && self.height == height {
            Ok(())
        } else {
            Err(MatteError::SizeMismatch { got_w: self.width, got_h: self.height, want_w: width, want_h: height })
        }
    }

    /// Samples as f32 in [0, 1].
    pub fn to_f32(&self) -> Vec<f32> {
        match &self.data {
            MatteData::U8(v) => v.iter().map(|&x| f32::from(x) / 255.0).collect(),
            MatteData::U16(v) => v.iter().map(|&x| f32::from(x) / 65535.0).collect(),
            MatteData::F32(v) => v.to_vec(),
        }
    }

    /// The same matte at another depth; rounding is to nearest and deterministic (MQ-6).
    pub fn converted(&self, depth: MatteDepth) -> MatteFrame {
        let f = self.to_f32();
        let data = match depth {
            MatteDepth::U8 => MatteData::U8(f.iter().map(|&v| (v * 255.0 + 0.5).floor() as u8).collect()),
            MatteDepth::U16 => MatteData::U16(f.iter().map(|&v| (v * 65535.0 + 0.5).floor() as u16).collect()),
            MatteDepth::F32 => MatteData::F32(f.into()),
        };
        MatteFrame { data, ..self.clone() }
    }

    /// SHA-256 over a fixed header (domain, depth, size) and the samples, little-endian.
    pub fn content_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"cfa4.matte.v0\0");
        h.update([self.depth().tag()]);
        h.update(self.width.to_le_bytes());
        h.update(self.height.to_le_bytes());
        match &self.data {
            MatteData::U8(v) => h.update(v),
            MatteData::U16(v) => v.iter().for_each(|x| h.update(x.to_le_bytes())),
            MatteData::F32(v) => v.iter().for_each(|x| h.update(x.to_le_bytes())),
        }
        let digest: [u8; 32] = h.finalize().into();
        let mut s = String::with_capacity(64);
        for b in digest {
            s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            s.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
        }
        s
    }

    pub fn to_ref(&self) -> Result<MatteRef, MatteError> {
        MatteRef::new(self.content_hash(), self.width, self.height, self.depth())
    }

    /// A binary mask as a matte: 0 and 1 exactly, so the conversion loses nothing.
    pub fn from_mask(width: u32, height: u32, mask: &[bool], pts: Tick, origin: MatteOrigin) -> Result<MatteFrame, MatteError> {
        let data: Vec<u8> = mask.iter().map(|&b| u8::from(b) * 255).collect();
        MatteFrame::new(width, height, MatteData::U8(data.into()), pts, origin)
    }

    /// A matte as a binary mask: a sample at or above `threshold` is in. The threshold is a
    /// required argument, there is no default.
    pub fn to_mask(&self, threshold: f32) -> Result<Vec<bool>, MatteError> {
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
            return Err(MatteError::Threshold(threshold));
        }
        Ok(self.to_f32().into_iter().map(|v| v >= threshold).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H64: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn origin() -> MatteOrigin {
        MatteOrigin::new(MatteMethod::parse("key.ultra").unwrap(), H64, None).unwrap()
    }

    fn f32_frame(w: u32, h: u32, v: Vec<f32>) -> Result<MatteFrame, MatteError> {
        MatteFrame::new(w, h, MatteData::F32(v.into()), Tick::ZERO, origin())
    }

    #[test]
    fn hostile_sizes_lengths_and_values_are_errors() {
        assert_eq!(f32_frame(0, 4, vec![]), Err(MatteError::Size(0, 4)));
        assert_eq!(f32_frame(4, 0, vec![]), Err(MatteError::Size(4, 0)));
        assert_eq!(f32_frame(u32::MAX, 1, vec![]), Err(MatteError::Size(u32::MAX, 1)));
        assert_eq!(f32_frame(MAX_MATTE_DIM, MAX_MATTE_DIM, vec![]), Err(MatteError::TooLarge(u64::from(MAX_MATTE_DIM) * u64::from(MAX_MATTE_DIM))));
        assert!(matches!(f32_frame(2, 2, vec![0.0; 3]), Err(MatteError::Length { need: 4, got: 3, .. })));
        assert!(matches!(f32_frame(2, 1, vec![0.5, f32::NAN]), Err(MatteError::Value { index: 1, .. })));
        assert!(matches!(f32_frame(2, 1, vec![f32::INFINITY, 0.0]), Err(MatteError::Value { index: 0, .. })));
        assert!(matches!(f32_frame(2, 1, vec![0.0, 1.0001]), Err(MatteError::Value { index: 1, .. })));
        assert!(matches!(f32_frame(2, 1, vec![-0.0001, 0.0]), Err(MatteError::Value { index: 0, .. })));
        assert!(f32_frame(2, 1, vec![0.0, 1.0]).is_ok());
    }

    #[test]
    fn hashes_and_method_ids_are_checked() {
        assert!(MatteRef::new("xyz", 2, 2, MatteDepth::U8).is_err());
        assert!(MatteRef::new(H64.to_uppercase(), 2, 2, MatteDepth::U8).is_err());
        assert!(MatteRef::new(H64, 0, 2, MatteDepth::U8).is_err());
        assert!(MatteOrigin::new(MatteMethod::parse("key.color").unwrap(), "short", None).is_err());
        for bad in ["", "key.", "ai.matting.", "Key.Ultra", "vfx.key", "key. ultra", &"key.".repeat(30)] {
            assert!(MatteMethod::parse(bad).is_err(), "{bad:?}");
        }
        assert!(MatteMethod::parse("ai.matting.bmv2").unwrap().is_ai());
        // the same checks run when a value comes from JSON
        assert!(serde_json::from_str::<MatteRef>(r#"{"sha256":"zz","width":2,"height":2,"depth":"u8"}"#).is_err());
        assert!(serde_json::from_str::<MatteMethod>(r#""shell.rm""#).is_err());
    }

    #[test]
    fn depth_conversion_is_deterministic_and_keeps_the_endpoints() {
        let m = f32_frame(4, 1, vec![0.0, 0.25, 0.5, 1.0]).unwrap();
        let u8m = m.converted(MatteDepth::U8);
        assert_eq!(u8m.data(), &MatteData::U8(vec![0u8, 64, 128, 255].into()));
        let u16m = m.converted(MatteDepth::U16);
        assert_eq!(u16m.to_f32()[0], 0.0);
        assert_eq!(u16m.to_f32()[3], 1.0);
        assert_eq!(m.converted(MatteDepth::U8), m.converted(MatteDepth::U8));
        assert_eq!(m.converted(MatteDepth::F32), m);
    }

    #[test]
    fn the_hash_follows_the_content_and_the_depth() {
        let a = f32_frame(2, 1, vec![0.0, 1.0]).unwrap();
        let b = f32_frame(2, 1, vec![0.0, 0.5]).unwrap();
        assert_ne!(a.content_hash(), b.content_hash());
        assert_ne!(a.content_hash(), a.converted(MatteDepth::U8).content_hash());
        assert_eq!(a.content_hash(), a.clone().content_hash());
        let r = a.to_ref().unwrap();
        assert_eq!((r.width(), r.height(), r.depth()), (2, 1, MatteDepth::F32));
        assert_eq!(r.sha256(), a.content_hash());
    }

    #[test]
    fn mask_round_trip_needs_an_explicit_threshold() {
        let mask = [true, false, true, true];
        let m = MatteFrame::from_mask(2, 2, &mask, Tick::ZERO, origin()).unwrap();
        assert_eq!(m.to_mask(0.5).unwrap(), mask);
        assert!(m.to_mask(0.0).is_err());
        assert!(m.to_mask(f32::NAN).is_err());
        assert!(m.to_mask(1.5).is_err());
        assert!(MatteFrame::from_mask(2, 2, &mask[..3], Tick::ZERO, origin()).is_err());
    }

    #[test]
    fn a_matte_must_match_its_picture() {
        let m = f32_frame(2, 2, vec![0.0; 4]).unwrap().with_timecode("01:00:00:00").with_source("clip-1");
        assert!(m.check_matches(2, 2).is_ok());
        assert_eq!(m.check_matches(4, 2), Err(MatteError::SizeMismatch { got_w: 2, got_h: 2, want_w: 4, want_h: 2 }));
        assert_eq!((m.timecode(), m.source()), (Some("01:00:00:00"), Some("clip-1")));
    }
}
