//! Tracked objects: the data model of Object Mask (OBJECT-MASK-PLAN-001, steps 2 to 6).
//!
//! An [`ObjectTrack`] is one object followed through a media item: a time-ordered list of
//! [`ObjectSample`]s (box, score and an optional mask) plus what produced it (manual box, detector,
//! segmenter) and the prompts a person gave. It is plain serde data like the rest of the project
//! model. Masks are not stored inline: a sample holds a [`MaskRef`] (the SHA-256 of the mask's
//! run-length data), and the run-length bytes live in the content-addressed cache outside the
//! project file (docs/ai/postgresql-adatreteg.md: masks stay on disk).
//!
//! Everything read from a file is hostile (AGENTS.md §0.4): [`MaskRle`] and [`ObjectTrack`]
//! validate on deserialization, every size is bounded, sums use checked arithmetic and no
//! allocation is sized by an unchecked input number.
//!
//! Time is [`Tick`] media time, never frame numbers, so speed changes and cuts stay correct.
//!
//! The run-length convention is the one libopenshot uses (LGPL-3.0-or-later, `CVObjectMask.cpp`,
//! `EncodeBinaryMaskRLE`): row-major from the top left, runs alternate 0, 1, 0, …, and the first
//! run is the (possibly empty) run of zeros. Only that convention was taken, no code.

use cfa4_geom::Rect;
use cfa4_time::Tick;
use serde::{Deserialize, Serialize};

/// Longest side of a mask in pixels.
pub const MAX_MASK_SIDE: u32 = 16_384;
/// Most pixels a mask may have (16 384 × 8 192).
pub const MAX_MASK_PIXELS: u64 = 1 << 27;
/// Most samples one track may hold.
pub const MAX_SAMPLES: usize = 1_000_000;
/// Most prompts one track may hold, and most points or rectangles in one prompt.
pub const MAX_PROMPTS: usize = 256;
/// Most points, and most rectangles, in one prompt.
pub const MAX_PROMPT_ITEMS: usize = 64;
/// Longest label, class or model name in bytes.
pub const MAX_TEXT: usize = 256;

/// Why an object value was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ObjectError {
    #[error("mask size {width}x{height} is empty or larger than the limits")]
    MaskSize { width: u32, height: u32 },
    #[error("mask data has {got} pixels but the size {width}x{height} needs {want}")]
    MaskLength { width: u32, height: u32, got: u64, want: u64 },
    #[error("run-length data covers {got} pixels, the mask has {want}")]
    RunSum { got: u64, want: u64 },
    #[error("run-length run {index} is empty (only the first run may be)")]
    EmptyRun { index: usize },
    #[error("run-length data has more runs ({got}) than the mask has pixels")]
    TooManyRuns { got: usize },
    #[error("`{0}` is not a lowercase hex SHA-256 (64 characters)")]
    BadHash(String),
    #[error("{what}: value is not finite or is out of range")]
    BadNumber { what: &'static str },
    #[error("{what}: too long or too many")]
    TooMany { what: &'static str },
    #[error("samples must be in strictly increasing time order (index {index})")]
    SampleOrder { index: usize },
}

/// Pixel count of a `width × height` mask, or the reason the size is refused.
fn mask_pixels(width: u32, height: u32) -> Result<u64, ObjectError> {
    let err = ObjectError::MaskSize { width, height };
    if width == 0 || height == 0 || width > MAX_MASK_SIDE || height > MAX_MASK_SIDE {
        return Err(err);
    }
    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| err.clone())?;
    if pixels > MAX_MASK_PIXELS {
        return Err(err);
    }
    Ok(pixels)
}

/// A binary mask as run-length data (see the module docs for the convention).
///
/// Deserializing validates; the fields are private so a value that exists is valid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MaskRleRaw")]
pub struct MaskRle {
    width: u32,
    height: u32,
    runs: Vec<u32>,
}

#[derive(Deserialize)]
struct MaskRleRaw {
    width: u32,
    height: u32,
    runs: Vec<u32>,
}

impl TryFrom<MaskRleRaw> for MaskRle {
    type Error = ObjectError;
    fn try_from(raw: MaskRleRaw) -> Result<Self, ObjectError> {
        MaskRle::from_runs(raw.width, raw.height, raw.runs)
    }
}

impl MaskRle {
    /// Builds a mask from run-length data, checking every rule: the size is within the limits,
    /// the runs sum to exactly `width × height`, and only the first run may be empty.
    pub fn from_runs(width: u32, height: u32, runs: Vec<u32>) -> Result<Self, ObjectError> {
        let pixels = mask_pixels(width, height)?;
        if runs.len() as u64 > pixels.saturating_add(1) {
            return Err(ObjectError::TooManyRuns { got: runs.len() });
        }
        let mut sum: u64 = 0;
        for (index, run) in runs.iter().enumerate() {
            if *run == 0 && index > 0 {
                return Err(ObjectError::EmptyRun { index });
            }
            sum = sum.saturating_add(u64::from(*run));
        }
        if sum != pixels {
            return Err(ObjectError::RunSum { got: sum, want: pixels });
        }
        Ok(Self { width, height, runs })
    }

    /// An all-zero mask.
    pub fn empty(width: u32, height: u32) -> Result<Self, ObjectError> {
        let pixels = mask_pixels(width, height)?;
        let run = u32::try_from(pixels).map_err(|_| ObjectError::MaskSize { width, height })?;
        Ok(Self { width, height, runs: vec![run] })
    }

    /// Encodes row-major pixels (any non-zero byte is set).
    pub fn encode(width: u32, height: u32, pixels: &[u8]) -> Result<Self, ObjectError> {
        let want = mask_pixels(width, height)?;
        if pixels.len() as u64 != want {
            return Err(ObjectError::MaskLength { width, height, got: pixels.len() as u64, want });
        }
        let mut runs = Vec::new();
        let mut current = false;
        let mut count: u32 = 0;
        for p in pixels {
            let v = *p != 0;
            if v == current {
                count += 1;
            } else {
                runs.push(count);
                current = v;
                count = 1;
            }
        }
        runs.push(count);
        Self::from_runs(width, height, runs)
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn runs(&self) -> &[u32] {
        &self.runs
    }

    /// Number of set pixels.
    pub fn set_pixels(&self) -> u64 {
        self.runs.iter().skip(1).step_by(2).map(|r| u64::from(*r)).sum()
    }

    /// Decodes to row-major bytes, 0 or 255 (a coverage plane).
    pub fn decode(&self) -> Vec<u8> {
        // the size was validated, so this allocation is bounded by MAX_MASK_PIXELS
        let total = usize::try_from(u64::from(self.width) * u64::from(self.height)).unwrap_or(0);
        let mut out = vec![0u8; total];
        let mut pos = 0usize;
        for (i, run) in self.runs.iter().enumerate() {
            let end = pos.saturating_add(*run as usize);
            if i % 2 == 1
                && let Some(slice) = out.get_mut(pos..end)
            {
                slice.fill(255);
            }
            pos = end;
        }
        out
    }

    /// Tight box of the set pixels as `(x0, y0, x1, y1)` with exclusive `x1` and `y1`, or `None`
    /// for an empty mask. Works on the runs, without decoding.
    pub fn bbox(&self) -> Option<(u32, u32, u32, u32)> {
        let w = u64::from(self.width);
        let (mut x0, mut y0, mut x1, mut y1) = (u64::MAX, u64::MAX, 0u64, 0u64);
        let mut pos: u64 = 0;
        let mut any = false;
        for (i, run) in self.runs.iter().enumerate() {
            let len = u64::from(*run);
            if i % 2 == 1 && len > 0 {
                any = true;
                let (first, last) = (pos, pos + len - 1);
                let (ry0, ry1) = (first / w, last / w);
                let (rx0, rx1) = if ry0 == ry1 { (first % w, last % w) } else { (0, w - 1) };
                x0 = x0.min(rx0);
                x1 = x1.max(rx1 + 1);
                y0 = y0.min(ry0);
                y1 = y1.max(ry1 + 1);
            }
            pos += len;
        }
        if !any {
            return None;
        }
        Some((u32::try_from(x0).ok()?, u32::try_from(y0).ok()?, u32::try_from(x1).ok()?, u32::try_from(y1).ok()?))
    }
}

/// A reference to a mask kept in the content-addressed cache: the SHA-256 of its run-length
/// data and its size (so a renderer can size buffers before it fetches the data).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MaskRefRaw")]
pub struct MaskRef {
    sha256: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct MaskRefRaw {
    sha256: String,
    width: u32,
    height: u32,
}

impl TryFrom<MaskRefRaw> for MaskRef {
    type Error = ObjectError;
    fn try_from(raw: MaskRefRaw) -> Result<Self, ObjectError> {
        MaskRef::new(raw.sha256, raw.width, raw.height)
    }
}

impl MaskRef {
    pub fn new(sha256: impl Into<String>, width: u32, height: u32) -> Result<Self, ObjectError> {
        let sha256 = sha256.into();
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(ObjectError::BadHash(sha256.chars().take(80).collect()));
        }
        mask_pixels(width, height)?;
        Ok(Self { sha256, width, height })
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
}

/// Stable id of an object within its project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub u64);

/// How a track was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectSource {
    /// A box drawn by hand, followed by the classic tracker.
    #[default]
    Manual,
    /// Boxes from a detector.
    Detected,
    /// Masks from a segmentation model (prompted or propagated).
    Segmented,
}

/// The model that produced a track, for reproducibility and the licence trail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectProvenance {
    pub model: String,
    #[serde(default)]
    pub revision: String,
    /// SHA-256 of the model file (lowercase hex), from the model manager.
    pub sha256: String,
}

/// One prompt point, in source pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptPoint {
    pub x: f64,
    pub y: f64,
    /// `true` marks the object, `false` marks what must stay out.
    pub positive: bool,
}

/// One prompt rectangle, in source pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptRect {
    pub rect: Rect,
    pub positive: bool,
}

/// What a person marked on one frame to select the object (positive and negative points and
/// rectangles).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectPrompt {
    pub time: Tick,
    #[serde(default)]
    pub points: Vec<PromptPoint>,
    #[serde(default)]
    pub rects: Vec<PromptRect>,
}

/// One observation of the object at one media time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectSample {
    pub time: Tick,
    /// Source pixels.
    pub bbox: Rect,
    /// Model confidence 0..=1 (1 for a manual box).
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<MaskRef>,
}

fn finite_rect(r: &Rect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite() && r.w >= 0.0 && r.h >= 0.0
}

impl ObjectSample {
    pub fn validate(&self) -> Result<(), ObjectError> {
        if !finite_rect(&self.bbox) {
            return Err(ObjectError::BadNumber { what: "sample box" });
        }
        if !self.score.is_finite() || !(0.0..=1.0).contains(&self.score) {
            return Err(ObjectError::BadNumber { what: "sample score" });
        }
        Ok(())
    }
}

/// One object followed through a media item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ObjectTrackRaw")]
pub struct ObjectTrack {
    pub id: ObjectId,
    pub label: String,
    /// The detector's class name, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default)]
    pub source: ObjectSource,
    /// Strictly increasing in time. Change it through [`ObjectTrack::insert_sample`].
    samples: Vec<ObjectSample>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<ObjectPrompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ObjectProvenance>,
}

#[derive(Deserialize)]
struct ObjectTrackRaw {
    id: ObjectId,
    label: String,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    source: ObjectSource,
    #[serde(default)]
    samples: Vec<ObjectSample>,
    #[serde(default)]
    prompts: Vec<ObjectPrompt>,
    #[serde(default)]
    provenance: Option<ObjectProvenance>,
}

impl TryFrom<ObjectTrackRaw> for ObjectTrack {
    type Error = ObjectError;
    fn try_from(raw: ObjectTrackRaw) -> Result<Self, ObjectError> {
        let track = ObjectTrack {
            id: raw.id,
            label: raw.label,
            class: raw.class,
            source: raw.source,
            samples: raw.samples,
            prompts: raw.prompts,
            provenance: raw.provenance,
        };
        track.validate()?;
        Ok(track)
    }
}

impl ObjectTrack {
    pub fn new(id: ObjectId, label: impl Into<String>, source: ObjectSource) -> Self {
        let mut label: String = label.into();
        truncate_text(&mut label);
        Self { id, label, class: None, source, samples: Vec::new(), prompts: Vec::new(), provenance: None }
    }

    pub fn samples(&self) -> &[ObjectSample] {
        &self.samples
    }

    /// Checks every rule; used after loading and by the commands before they store a track.
    pub fn validate(&self) -> Result<(), ObjectError> {
        if self.label.len() > MAX_TEXT || self.class.as_ref().is_some_and(|c| c.len() > MAX_TEXT) {
            return Err(ObjectError::TooMany { what: "label or class text" });
        }
        if let Some(p) = &self.provenance {
            if p.model.len() > MAX_TEXT || p.revision.len() > MAX_TEXT {
                return Err(ObjectError::TooMany { what: "provenance text" });
            }
            if p.sha256.len() != 64 || !p.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(ObjectError::BadHash(p.sha256.chars().take(80).collect()));
            }
        }
        if self.samples.len() > MAX_SAMPLES {
            return Err(ObjectError::TooMany { what: "samples" });
        }
        for (index, s) in self.samples.iter().enumerate() {
            s.validate()?;
            if index > 0 && self.samples.get(index - 1).is_some_and(|prev| prev.time >= s.time) {
                return Err(ObjectError::SampleOrder { index });
            }
        }
        if self.prompts.len() > MAX_PROMPTS {
            return Err(ObjectError::TooMany { what: "prompts" });
        }
        for p in &self.prompts {
            if p.points.len() > MAX_PROMPT_ITEMS || p.rects.len() > MAX_PROMPT_ITEMS {
                return Err(ObjectError::TooMany { what: "prompt points or rectangles" });
            }
            if p.points.iter().any(|q| !q.x.is_finite() || !q.y.is_finite()) || p.rects.iter().any(|q| !finite_rect(&q.rect)) {
                return Err(ObjectError::BadNumber { what: "prompt" });
            }
        }
        Ok(())
    }

    /// Adds a sample, keeping the order; a sample at the same time is replaced.
    pub fn insert_sample(&mut self, sample: ObjectSample) -> Result<(), ObjectError> {
        sample.validate()?;
        match self.samples.binary_search_by(|s| s.time.cmp(&sample.time)) {
            Ok(i) => {
                if let Some(slot) = self.samples.get_mut(i) {
                    *slot = sample;
                }
            }
            Err(i) => {
                if self.samples.len() >= MAX_SAMPLES {
                    return Err(ObjectError::TooMany { what: "samples" });
                }
                self.samples.insert(i, sample);
            }
        }
        Ok(())
    }

    /// First and last sample time.
    pub fn time_range(&self) -> Option<(Tick, Tick)> {
        Some((self.samples.first()?.time, self.samples.last()?.time))
    }

    /// The sample nearest to `time`, if it is at most `max_gap` away (an earlier sample wins a
    /// tie). This is how a renderer picks the mask for a frame between analysed frames.
    pub fn sample_near(&self, time: Tick, max_gap: Tick) -> Option<&ObjectSample> {
        let after = self.samples.partition_point(|s| s.time < time);
        let before = after.checked_sub(1).and_then(|i| self.samples.get(i));
        let next = self.samples.get(after);
        let dist = |s: &ObjectSample| (i128::from(s.time.0) - i128::from(time.0)).abs();
        let best = match (before, next) {
            (Some(b), Some(n)) => {
                if dist(n) < dist(b) {
                    n
                } else {
                    b
                }
            }
            (Some(b), None) => b,
            (None, Some(n)) => n,
            (None, None) => return None,
        };
        (dist(best) <= i128::from(max_gap.0.max(0))).then_some(best)
    }

    /// The box at `time`, linearly interpolated between the two samples around it; `None`
    /// outside the first and last sample.
    pub fn bbox_at(&self, time: Tick) -> Option<Rect> {
        let after = self.samples.partition_point(|s| s.time < time);
        let next = self.samples.get(after)?;
        if next.time == time {
            return Some(next.bbox);
        }
        let prev = self.samples.get(after.checked_sub(1)?)?;
        let span = i128::from(next.time.0) - i128::from(prev.time.0);
        if span <= 0 {
            return None;
        }
        let t = (i128::from(time.0) - i128::from(prev.time.0)) as f64 / span as f64;
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        Some(Rect::new(lerp(prev.bbox.x, next.bbox.x), lerp(prev.bbox.y, next.bbox.y), lerp(prev.bbox.w, next.bbox.w), lerp(prev.bbox.h, next.bbox.h)))
    }
}

fn truncate_text(s: &mut String) {
    if s.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn sample(t: i64, x: f64) -> ObjectSample {
        ObjectSample { time: Tick(t), bbox: Rect::new(x, 0.0, 10.0, 10.0), score: 0.9, mask: None }
    }

    #[test]
    fn rle_matches_the_libopenshot_convention() {
        // 3x2: row 1 = 1 1 0, row 2 = 0 0 1 -> zeros(0), ones(2), zeros(3), ones(1)
        let m = MaskRle::encode(3, 2, &[1, 1, 0, 0, 0, 1]).unwrap();
        assert_eq!(m.runs(), &[0, 2, 3, 1]);
        assert_eq!(m.set_pixels(), 3);
        assert_eq!(m.decode(), vec![255, 255, 0, 0, 0, 255]);
        let all_zero = MaskRle::encode(2, 2, &[0; 4]).unwrap();
        assert_eq!(all_zero.runs(), &[4]);
        assert_eq!(MaskRle::empty(2, 2).unwrap(), all_zero);
    }

    #[test]
    fn rle_bbox() {
        let m = MaskRle::encode(4, 3, &[0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0]).unwrap();
        assert_eq!(m.bbox(), Some((1, 1, 3, 3)));
        assert_eq!(MaskRle::empty(4, 3).unwrap().bbox(), None);
    }

    #[test]
    fn hostile_rle_is_refused() {
        // sums that do not match, overflow, empty interior runs, impossible sizes
        assert!(MaskRle::from_runs(2, 2, vec![1, 1]).is_err());
        assert!(MaskRle::from_runs(2, 2, vec![u32::MAX, u32::MAX, u32::MAX]).is_err());
        assert!(MaskRle::from_runs(2, 2, vec![2, 0, 2]).is_err());
        assert!(MaskRle::from_runs(0, 2, vec![]).is_err());
        assert!(MaskRle::from_runs(u32::MAX, u32::MAX, vec![1]).is_err());
        assert!(MaskRle::from_runs(MAX_MASK_SIDE, MAX_MASK_SIDE, vec![1]).is_err());
        assert!(MaskRle::encode(2, 2, &[0; 3]).is_err());
        let bad: Result<MaskRle, _> = serde_json::from_str(r#"{"width":2,"height":2,"runs":[1,1]}"#);
        assert!(bad.is_err(), "deserializing validates");
        let ok: MaskRle = serde_json::from_str(r#"{"width":2,"height":2,"runs":[1,2,1]}"#).unwrap();
        assert_eq!(ok.set_pixels(), 2);
    }

    #[test]
    fn mask_ref_needs_a_real_hash() {
        assert!(MaskRef::new(HASH, 64, 64).is_ok());
        assert!(MaskRef::new("ABC", 64, 64).is_err());
        assert!(MaskRef::new(HASH.to_uppercase(), 64, 64).is_err());
        assert!(MaskRef::new(HASH, 0, 64).is_err());
    }

    #[test]
    fn samples_stay_ordered_and_replace_by_time() {
        let mut t = ObjectTrack::new(ObjectId(1), "person", ObjectSource::Manual);
        for (time, x) in [(20, 2.0), (10, 1.0), (30, 3.0), (20, 9.0)] {
            t.insert_sample(sample(time, x)).unwrap();
        }
        let times: Vec<i64> = t.samples().iter().map(|s| s.time.0).collect();
        assert_eq!(times, [10, 20, 30]);
        assert_eq!(t.samples()[1].bbox.x, 9.0, "same time replaces");
        assert_eq!(t.time_range(), Some((Tick(10), Tick(30))));
        assert!(t.validate().is_ok());
        let mut bad = sample(40, 0.0);
        bad.bbox.w = f64::NAN;
        assert!(t.insert_sample(bad).is_err());
        let mut bad = sample(40, 0.0);
        bad.score = 1.5;
        assert!(t.insert_sample(bad).is_err());
        assert_eq!(t.samples().len(), 3, "refused samples change nothing");
    }

    #[test]
    fn sample_near_respects_the_gap() {
        let mut t = ObjectTrack::new(ObjectId(1), "x", ObjectSource::Detected);
        t.insert_sample(sample(100, 0.0)).unwrap();
        t.insert_sample(sample(200, 1.0)).unwrap();
        assert_eq!(t.sample_near(Tick(140), Tick(100)).map(|s| s.time.0), Some(100));
        assert_eq!(t.sample_near(Tick(160), Tick(100)).map(|s| s.time.0), Some(200));
        assert_eq!(t.sample_near(Tick(150), Tick(100)).map(|s| s.time.0), Some(100), "tie: earlier");
        assert!(t.sample_near(Tick(400), Tick(100)).is_none());
        assert!(t.sample_near(Tick(-500), Tick(100)).is_none());
        assert!(t.sample_near(Tick(100), Tick(-5)).is_some(), "a negative gap acts as zero");
        assert!(ObjectTrack::new(ObjectId(2), "e", ObjectSource::Manual).sample_near(Tick(0), Tick(10)).is_none());
        // extreme times do not overflow
        assert!(t.sample_near(Tick(i64::MAX), Tick(i64::MAX)).is_some());
        assert!(t.sample_near(Tick(i64::MIN), Tick(0)).is_none());
    }

    #[test]
    fn bbox_interpolates_inside_the_track_only() {
        let mut t = ObjectTrack::new(ObjectId(1), "x", ObjectSource::Manual);
        t.insert_sample(sample(0, 0.0)).unwrap();
        t.insert_sample(sample(100, 10.0)).unwrap();
        assert_eq!(t.bbox_at(Tick(0)).map(|r| r.x), Some(0.0));
        assert_eq!(t.bbox_at(Tick(50)).map(|r| r.x), Some(5.0));
        assert_eq!(t.bbox_at(Tick(100)).map(|r| r.x), Some(10.0));
        assert!(t.bbox_at(Tick(-1)).is_none());
        assert!(t.bbox_at(Tick(101)).is_none());
        assert!(t.bbox_at(Tick(i64::MIN)).is_none());
        assert!(t.bbox_at(Tick(i64::MAX)).is_none());
    }

    #[test]
    fn a_track_survives_json_and_loading_validates() {
        let mut t = ObjectTrack::new(ObjectId(7), "dog", ObjectSource::Segmented);
        t.class = Some("dog".into());
        let mut s = sample(5, 1.0);
        s.mask = Some(MaskRef::new(HASH, 32, 32).unwrap());
        t.insert_sample(s).unwrap();
        t.prompts.push(ObjectPrompt {
            time: Tick(5),
            points: vec![PromptPoint { x: 3.0, y: 4.0, positive: true }],
            rects: vec![PromptRect { rect: Rect::new(0.0, 0.0, 5.0, 5.0), positive: false }],
        });
        t.provenance = Some(ObjectProvenance { model: "sam2".into(), revision: "r1".into(), sha256: HASH.into() });
        let text = serde_json::to_string(&t).unwrap();
        let back: ObjectTrack = serde_json::from_str(&text).unwrap();
        assert_eq!(back, t);
        // unordered samples, NaN-ish numbers, oversize text: refused on load
        let unordered =
            r#"{"id":1,"label":"a","samples":[{"time":5,"bbox":{"x":0,"y":0,"w":1,"h":1},"score":1},{"time":5,"bbox":{"x":0,"y":0,"w":1,"h":1},"score":1}]}"#;
        assert!(serde_json::from_str::<ObjectTrack>(unordered).is_err());
        let long = format!(r#"{{"id":1,"label":"{}"}}"#, "x".repeat(MAX_TEXT + 1));
        assert!(serde_json::from_str::<ObjectTrack>(&long).is_err());
        let minimal: ObjectTrack = serde_json::from_str(r#"{"id":1,"label":"a"}"#).unwrap();
        assert!(minimal.samples().is_empty());
        let negative_size = r#"{"id":1,"label":"a","samples":[{"time":1,"bbox":{"x":0,"y":0,"w":-1,"h":1},"score":1}]}"#;
        assert!(serde_json::from_str::<ObjectTrack>(negative_size).is_err());
    }

    #[test]
    fn labels_are_cut_at_a_char_boundary() {
        let t = ObjectTrack::new(ObjectId(1), "é".repeat(MAX_TEXT), ObjectSource::Manual);
        assert!(t.label.len() <= MAX_TEXT);
        assert!(t.validate().is_ok());
    }

    /// Truncation and byte flips of valid JSON never panic: they load or return an error.
    #[test]
    fn mutated_json_never_panics() {
        let mut t = ObjectTrack::new(ObjectId(3), "p", ObjectSource::Detected);
        t.insert_sample(sample(1, 1.0)).unwrap();
        t.insert_sample(sample(2, 2.0)).unwrap();
        let track = serde_json::to_vec(&t).unwrap();
        let mask = serde_json::to_vec(&MaskRle::encode(3, 2, &[1, 1, 0, 0, 0, 1]).unwrap()).unwrap();
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for base in [&track, &mask] {
            for _ in 0..2000 {
                let mut bytes = base.clone();
                for _ in 0..(1 + next() % 3) {
                    let i = (next() as usize) % bytes.len();
                    bytes[i] = (next() & 0xff) as u8;
                }
                if next() % 4 == 0 {
                    let cut = (next() as usize) % bytes.len();
                    bytes.truncate(cut);
                }
                let result = std::panic::catch_unwind(|| {
                    let _ = serde_json::from_slice::<ObjectTrack>(&bytes);
                    let _ = serde_json::from_slice::<MaskRle>(&bytes);
                });
                assert!(result.is_ok(), "panicked on {:?}", String::from_utf8_lossy(&bytes));
            }
        }
    }

    proptest! {
        #[test]
        fn rle_roundtrips(w in 1u32..24, h in 1u32..24, seed in any::<u64>(), density in 0u8..=255) {
            let mut s = seed | 1;
            let pixels: Vec<u8> = (0..(w * h)).map(|_| {
                s ^= s << 13; s ^= s >> 7; s ^= s << 17;
                if ((s & 0xff) as u8) < density { 255 } else { 0 }
            }).collect();
            let m = MaskRle::encode(w, h, &pixels).unwrap();
            prop_assert_eq!(m.decode(), pixels.clone());
            prop_assert_eq!(m.set_pixels(), pixels.iter().filter(|p| **p != 0).count() as u64);
            let json = serde_json::to_string(&m).unwrap();
            prop_assert_eq!(serde_json::from_str::<MaskRle>(&json).unwrap(), m.clone());
            // the box from the runs equals the box from the pixels
            let mut want: Option<(u32, u32, u32, u32)> = None;
            for y in 0..h { for x in 0..w {
                if pixels[(y * w + x) as usize] != 0 {
                    want = Some(match want {
                        None => (x, y, x + 1, y + 1),
                        Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + 1), d.max(y + 1)),
                    });
                }
            }}
            prop_assert_eq!(m.bbox(), want);
        }

        #[test]
        fn random_runs_never_panic(w in 0u32..40, h in 0u32..40, runs in proptest::collection::vec(any::<u32>(), 0..12)) {
            if let Ok(m) = MaskRle::from_runs(w, h, runs) {
                prop_assert_eq!(m.decode().len() as u64, u64::from(w) * u64::from(h));
            }
        }
    }
}
