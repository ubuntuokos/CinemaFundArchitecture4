//! AS-02 component sharing between the versions of one bundle (#1719).
//!
//! Versions that cut the same source material the same way need their picture and sound encoded
//! only once. [`plan`] gives every output frame of every version a *signature*: a hash of
//! everything that decides what the frame looks like and what its sound is (the media, the exact
//! source time, the track and clip settings). Frames with equal signatures are identical, so a
//! later version points its timeline at the component file of the earlier version that already
//! holds them; only the frames no earlier version has are encoded, each maximal run of them as
//! one new component file per picture and sound channel.
//!
//! The signature is conservative: a frame with effects, transitions, speed changes, titles,
//! graphics, nested sequences, effects other than the intrinsic ones with fixed values, track effects, mixer automation or burnt-in captions has none and
//! is always encoded by its own version. Sound is shared only at frame rates where a frame holds a
//! whole number of samples, so that a shared run is sample-exact whichever frame it starts on.

use std::collections::{BTreeMap, HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use cfa4_project::{ItemId, ItemKind, MixerStrip, Project, Sequence, TrackItem};
use cfa4_time::{FrameRate, TimeRange};

/// One stretch of a version's timeline and where its frames are stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Frames in the stretch.
    pub len: i64,
    /// Index of the version whose component files hold the frames (the version itself when the
    /// frames are its own).
    pub version: usize,
    /// Which of that version's own runs (component file sets) holds them.
    pub run: u32,
    /// First frame within that run.
    pub offset: i64,
}

/// The layout of one version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionPlan {
    pub seq: ItemId,
    /// The exported time range.
    pub range: TimeRange,
    /// The frame rate of the sequence.
    pub range_rate: FrameRate,
    /// The timeline, in order.
    pub pieces: Vec<Piece>,
    /// The runs this version encodes itself: (first frame of the version, frames).
    pub runs: Vec<(i64, i64)>,
}

impl VersionPlan {
    /// The time range of one own run.
    pub fn run_range(&self, rate: FrameRate, run: usize) -> TimeRange {
        let f0 = rate.frame_at(self.range.start);
        let (a, n) = self.runs[run];
        let start = rate.tick_of(f0 + a);
        TimeRange::new(start, rate.tick_of(f0 + a + n) - start)
    }
}

/// The layout of a whole bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub versions: Vec<VersionPlan>,
}

impl Plan {
    /// No version shares anything: every version is one run of its own, the layout of a bundle
    /// written without sharing.
    pub fn is_trivial(&self) -> bool {
        self.versions.iter().all(|v| v.runs.len() == 1 && v.pieces.len() == 1)
    }
}

/// A finished component file.
#[derive(Clone, Debug)]
pub struct Comp {
    pub umid: cfa4_mxf::Umid,
    pub rel: String,
    /// Duration in edit units.
    pub units: u64,
    pub descriptor: Vec<cfa4_mxf::as02_version::CopiedSet>,
}

/// What the runs of a bundle write down for the version files and the manifest.
#[derive(Default)]
pub struct Registry {
    /// (version index, run, channel: 0 = picture, 1.. = sound) → component.
    pub comps: BTreeMap<(usize, u32, usize), Comp>,
    /// Manifest entries of the component files written so far and not yet in a manifest.
    pub files: Vec<cfa4_mxf::as02::File>,
}

/// The plan of a bundle and the registry its runs fill; shared by the settings of every run.
pub struct Share {
    pub plan: Plan,
    pub registry: Mutex<Registry>,
}

/// [`Share`] in [`crate::ExportSettings`]: internal, never serialized.
#[derive(Clone, Default)]
pub struct ShareHandle(pub Option<Arc<Share>>);

impl std::fmt::Debug for ShareHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "ShareHandle(plan)" } else { "ShareHandle(none)" })
    }
}

fn kind_ok(project: &Project, it: &TrackItem) -> bool {
    // only the intrinsic effects every clip has (Motion, Opacity, Volume…), with fixed values
    it.effects.iter().all(|e| e.def().is_some_and(|d| d.intrinsic) && !e.is_animated() && e.layer.is_none())
        && it.multicam.is_none()
        && it.graphic.is_none()
        && it.essential.is_none()
        && !it.reverse
        && it.speed == 1.0
        && project.item(it.item).is_some_and(|p| matches!(p.kind, ItemKind::Media(_)))
}

fn covers(it: &TrackItem, t: i64) -> bool {
    it.start.0 <= t && t < it.start.0.saturating_add(it.duration.0)
}

/// The media time shown or heard at timeline time `t` for a plain (speed 1) clip.
fn source_time(it: &TrackItem, t: i64) -> i64 {
    it.frame_hold.map(|h| h.0).unwrap_or_else(|| it.source_in.0.saturating_add(t - it.start.0))
}

fn video_hash(project: &Project, q: &Sequence, t: i64, h: &mut impl Hasher) -> Option<()> {
    for (ti, tr) in q.video_tracks.iter().enumerate() {
        if !tr.enabled {
            continue;
        }
        if tr.transitions.iter().any(|x| x.start.0 <= t && t < x.start.0.saturating_add(x.duration.0)) {
            return None;
        }
        let Some(it) = tr.items.iter().find(|i| covers(i, t)) else { continue };
        if !it.enabled {
            continue;
        }
        if !kind_ok(project, it) {
            return None;
        }
        (ti, it.item.0, source_time(it, t), it.scale_to_frame, it.hold_filters).hash(h);
        format!("{:?}|{:?}|{:?}", it.time_interpolation, it.field_options, it.effects).hash(h);
    }
    Some(())
}

fn audio_hash(project: &Project, q: &Sequence, t: i64, fd: i64, h: &mut impl Hasher) -> Option<()> {
    if !q.master_effects.is_empty() || q.master_mixer != MixerStrip::default() || !q.submix_tracks.is_empty() {
        return None;
    }
    q.master_volume_db.to_bits().hash(h);
    q.audio_tracks.iter().any(|t| t.solo).hash(h);
    for (ti, tr) in q.audio_tracks.iter().enumerate() {
        let end = t.saturating_add(fd);
        let mut touched = tr.items.iter().filter(|i| i.start.0 < end && i.start.0.saturating_add(i.duration.0) > t).peekable();
        if touched.peek().is_none() {
            continue;
        }
        if !tr.effects.is_empty() || tr.mixer != MixerStrip::default() || !tr.transitions.is_empty() {
            return None;
        }
        (ti, tr.volume_db.to_bits(), tr.pan.to_bits(), tr.enabled, tr.muted, tr.solo, format!("{:?}", tr.channels)).hash(h);
        for it in touched {
            // a clip that starts or ends inside the frame, or one the mixer cannot treat plainly
            if it.start.0 > t || it.start.0.saturating_add(it.duration.0) < end || !kind_ok(project, it) {
                return None;
            }
            (it.item.0, source_time(it, t), it.gain_db.to_bits(), &it.source_channels, it.audio_stream, it.enabled).hash(h);
            format!("{:?}", it.effects).hash(h);
        }
    }
    Some(())
}

/// Whether a frame holds a whole number of samples at `sample_rate`.
fn whole_samples(rate: FrameRate, sample_rate: u32) -> bool {
    let (n, d) = (i128::from(rate.num.max(1)), i128::from(rate.den.max(1)));
    (i128::from(sample_rate) * d) % n == 0
}

/// The signature of each output frame of `range` (`None` where the frame is never shared).
fn signatures(project: &Project, seq: ItemId, range: TimeRange, audio: Option<u32>, burn_captions: bool) -> Vec<Option<u64>> {
    let Some(q) = project.sequence(seq) else { return Vec::new() };
    let rate = q.settings.frame_rate;
    let (f0, f1) = crate::frame_span(rate, range);
    let n = usize::try_from(f1 - f0).unwrap_or(0);
    if (burn_captions && !q.caption_tracks.is_empty()) || audio.is_some_and(|sr| !whole_samples(rate, sr)) {
        return vec![None; n];
    }
    let fd = rate.frame_duration().0;
    let s = &q.settings;
    let base = format!("{}x{}|{:?}|{:?}|{}|{:?}", s.width, s.height, s.frame_rate, s.par, s.sample_rate, s.color);
    (f0..f1)
        .map(|f| {
            let t = rate.tick_of(f).0;
            let mut h = DefaultHasher::new();
            base.hash(&mut h);
            video_hash(project, q, t, &mut h)?;
            if audio.is_some() {
                audio_hash(project, q, t, fd, &mut h)?;
            }
            Some(h.finish())
        })
        .collect()
}

/// Plan the bundle: `versions` are the sequences and their time ranges in bundle order; `audio`
/// is the output sample rate when the bundle has sound.
pub fn plan(project: &Project, versions: &[(ItemId, TimeRange)], audio: Option<u32>, burn_captions: bool) -> Plan {
    let mut known: HashMap<u64, (usize, u32, i64)> = HashMap::new();
    let mut out = Vec::new();
    for (v, &(seq, range)) in versions.iter().enumerate() {
        let sigs = signatures(project, seq, range, audio, burn_captions);
        let mut pieces: Vec<Piece> = Vec::new();
        let mut runs: Vec<(i64, i64)> = Vec::new();
        let mut origin: Vec<(usize, u32, i64)> = Vec::with_capacity(sigs.len());
        for (i, sig) in sigs.iter().enumerate() {
            let i = i as i64;
            match sig.and_then(|s| known.get(&s).copied()) {
                Some((ver, run, off)) => {
                    origin.push((ver, run, off));
                    match pieces.last_mut() {
                        Some(p) if p.version == ver && p.version != v && p.run == run && p.offset + p.len == off => p.len += 1,
                        _ => pieces.push(Piece { len: 1, version: ver, run, offset: off }),
                    }
                }
                None => match pieces.last_mut() {
                    Some(p) if p.version == v => {
                        origin.push((v, p.run, p.len));
                        p.len += 1;
                        if let Some(r) = runs.last_mut() {
                            r.1 += 1;
                        }
                    }
                    _ => {
                        let run = runs.len() as u32;
                        runs.push((i, 1));
                        origin.push((v, run, 0));
                        pieces.push(Piece { len: 1, version: v, run, offset: 0 });
                    }
                },
            }
        }
        for (sig, o) in sigs.iter().zip(origin) {
            if let Some(s) = sig {
                known.entry(*s).or_insert(o);
            }
        }
        out.push(VersionPlan { seq, range, range_rate: project.sequence(seq).map_or_else(FrameRate::default, |q| q.settings.frame_rate), pieces, runs });
    }
    Plan { versions: out }
}
