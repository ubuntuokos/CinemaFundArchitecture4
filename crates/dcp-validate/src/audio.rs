//! DCP-07 (plan F09, F10, F11): sound and immersive audio track files, read with `cfa4-mxf`,
//! measured with `cfa4-audio-dsp` and parsed with `cfa4-iab`.
//!
//! - `DCP.AUD.001` (ST 428-2, ST 429-2): each main sound track file is linear PCM of 24 bits at
//!   48 or 96 kHz with 1 to 16 channels and a block align of 3 bytes a channel; the sound of a
//!   composition keeps its sample rate and channel count from reel to reel, and a reel without
//!   sound in a composition that has sound is a warning. Channel counts other than 6, 8 and 16
//!   (the ST 429-2 configurations) are warnings. The MCA labels (ST 429-2 Annex A) are not read
//!   yet: `cfa4-mxf` does not expose them.
//! - `DCP.AUD.002` (ITU-R BS.1770-4, EBU R128): the integrated loudness, loudness range and true
//!   peak of each composition, over the played range of every reel in reel order. Channels 1-6
//!   are the programme (L, R, C, LFE, Ls, Rs; the LFE is left out, the surrounds weigh 1.41);
//!   channels 7 and up (HI, VI-N, sync, …) are not. Cinema has no single loudness target, so the
//!   measurement passes and is kept in the finding; a true peak above 0 dBTP is a warning.
//!   Thresholds come with the profile files (DCP-11).
//! - `DCP.IAB.001` (ST 429-18, ST 2098-2): each immersive audio track file has the reel asset's
//!   edit rate and duration, and every frame is an IABitstream frame of that frame rate at 48 or
//!   96 kHz.

use std::collections::BTreeMap;
use std::path::Path;

use cfa4_audio_dsp::loudness::LoudnessMeter;
use cfa4_dcp::{AssetKind, Composition, Rational, ReelAsset, Uuid};
use cfa4_mxf::{Codec, MxfFile, TrackKind};

use crate::integrity::{Files, asset_element, at};
use crate::media::FileSource;
use crate::rules::*;
use crate::{Finding, Location, Status, ValidationControl};

/// Sample frames read at a time while measuring.
const CHUNK: usize = 1 << 15;

pub(crate) struct Audio {
    pub pcm: (Vec<Finding>, String),
    pub loudness: Option<(Vec<Finding>, String)>,
    pub iab: Option<(Vec<Finding>, String)>,
}

fn rational(r: cfa4_mxf::Rational) -> Rational {
    Rational { num: i64::from(r.num), den: i64::from(r.den) }
}

/// A track file opened with its first track.
struct Opened {
    src: FileSource,
    file: MxfFile,
}

fn open(path: &Path) -> Option<Opened> {
    let src = FileSource::open(path).ok()?;
    let file = cfa4_mxf::open(&src).ok()?;
    Some(Opened { src, file })
}

fn location(cpl: &str, reel: usize, a: &ReelAsset) -> Location {
    at(cpl, asset_element(reel, a), Some(a.pos), a.id)
}

/// Checks the sound of `cpls`; `None` when cancelled.
pub(crate) fn check(cpls: &[(String, Composition)], files: &Files, loudness: bool, iab: bool, control: &ValidationControl) -> Option<Audio> {
    let mut opened: BTreeMap<Uuid, Option<Opened>> = BTreeMap::new();
    let mut pcm = Vec::new();
    let mut checked = 0usize;
    for (cpl, c) in cpls {
        // (sample rate, channels) of the first sound, and the reels without sound
        let mut first: Option<(Rational, u32)> = None;
        let mut silent = Vec::new();
        for (r, reel) in c.reels.iter().enumerate() {
            let Some(a) = reel.assets.iter().find(|a| a.kind == AssetKind::MainSound) else {
                silent.push(r + 1);
                continue;
            };
            let loc = location(cpl, r, a);
            let Some(id) = a.id else { continue };
            // a reel asset outside the package (a version file) is DCP.CPL.003's
            let Some((path, file)) = files.get(&id) else { continue };
            opened.entry(id).or_insert_with(|| open(file));
            let Some(Some(o)) = opened.get(&id) else {
                pcm.push(Finding::new(AUD_PCM, Status::Unchecked, format!("'{path}' is not a readable MXF track file (see DCP.MXF.001)")).at(loc));
                continue;
            };
            let Some(t) = o.file.tracks.first().filter(|t| t.kind == TrackKind::Sound) else {
                pcm.push(Finding::new(AUD_PCM, Status::Unchecked, format!("'{path}' holds no sound track (see DCP.MXF.001)")).at(loc));
                continue;
            };
            checked += 1;
            let mut fail = |m: String| pcm.push(Finding::new(AUD_PCM, Status::Fail, m).at(loc.clone()));
            if t.codec != Codec::Pcm {
                fail(format!("'{path}' is {}, not linear PCM", t.codec.name()));
            }
            let Some(s) = &t.sound else {
                fail(format!("'{path}' has no sound descriptor"));
                continue;
            };
            let rate = rational(s.sample_rate);
            if ![48_000, 96_000].into_iter().any(|r| rate.same_value(Rational { num: r, den: 1 })) {
                fail(format!("'{path}' is sampled at {rate}; D-Cinema sound is 48 or 96 kHz"));
            }
            if s.bits != 24 {
                fail(format!("'{path}' has {} bits a sample; D-Cinema sound has 24", s.bits));
            }
            if !(1..=16).contains(&s.channels) {
                fail(format!("'{path}' has {} channels; D-Cinema sound has 1 to 16", s.channels));
            } else if s.block_align != s.channels * s.bits.div_ceil(8) {
                fail(format!("'{path}' has a block align of {} for {} channels of {} bits", s.block_align, s.channels, s.bits));
            }
            match first {
                None => first = Some((rate, s.channels)),
                Some((r0, c0)) => {
                    if !r0.same_value(rate) {
                        fail(format!("'{path}' is sampled at {rate}, an earlier reel at {r0}"));
                    }
                    if c0 != s.channels {
                        fail(format!("'{path}' has {} channels, an earlier reel {c0}", s.channels));
                    }
                }
            }
            if (1..=16).contains(&s.channels) && ![6, 8, 16].contains(&s.channels) {
                let m = format!("'{path}' has {} channels, not one of the ST 429-2 configurations (6, 8, 16)", s.channels);
                pcm.push(Finding::new(AUD_PCM, Status::Warning, m).at(loc));
            }
        }
        if first.is_some() && !silent.is_empty() {
            let reels = silent.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
            pcm.push(
                Finding::new(AUD_PCM, Status::Warning, format!("reel {reels} has no MainSound while the composition has sound"))
                    .at(Location::file(cpl.as_str())),
            );
        }
    }
    if checked == 0 && pcm.is_empty() {
        pcm.push(Finding::new(AUD_PCM, Status::NotApplicable, "no composition has sound"));
    }
    let loudness = if loudness {
        let mut out = Vec::new();
        let mut measured = 0usize;
        for (cpl, c) in cpls {
            let f = measure(cpl, c, files, &opened, control)?;
            measured += usize::from(matches!(f.status, Status::Pass | Status::Warning));
            out.push(f);
        }
        Some((out, format!("{measured} compositions measured")))
    } else {
        None
    };
    let iab = if iab { Some(immersive(cpls, files, control)?) } else { None };
    Some(Audio { pcm: (pcm, format!("{checked} sound track files are 24-bit PCM at 48 or 96 kHz")), loudness, iab })
}

/// `DCP.AUD.002` for one composition: one finding. `None` when cancelled.
fn measure(cpl: &str, c: &Composition, files: &Files, opened: &BTreeMap<Uuid, Option<Opened>>, control: &ValidationControl) -> Option<Finding> {
    let here = Location::file(cpl);
    let mut parts = Vec::new();
    for reel in &c.reels {
        let Some(a) = reel.assets.iter().find(|a| a.kind == AssetKind::MainSound) else { continue };
        let Some(id) = a.id else { continue };
        let path = files.get(&id).map_or_else(|| id.to_string(), |(p, _)| p.clone());
        let Some(Some(o)) = opened.get(&id) else {
            return Some(Finding::new(AUD_LOUDNESS, Status::Unchecked, format!("'{path}' cannot be read, so the composition is not measured")).at(here));
        };
        let Some(t) = o.file.tracks.first().filter(|t| t.kind == TrackKind::Sound && t.codec == Codec::Pcm) else {
            return Some(Finding::new(AUD_LOUDNESS, Status::Unchecked, format!("'{path}' holds no PCM sound")).at(here));
        };
        if t.encryption.is_some() {
            return Some(Finding::new(AUD_LOUDNESS, Status::Unchecked, format!("'{path}' is encrypted; measuring it needs the content key")).at(here));
        }
        let Some(s) = &t.sound else { continue };
        let (Some(rate), true) = (a.edit_rate, s.sample_rate.den > 0 && s.sample_rate.num > 0) else { continue };
        if rate.num <= 0 {
            continue;
        }
        // played edit units → sample frames (ST 429-2: a whole number of samples per edit unit
        // at the D-Cinema rates)
        let per_unit = |units: u64| -> u64 {
            let v = i128::from(units) * i128::from(s.sample_rate.num) * i128::from(rate.den) / (i128::from(s.sample_rate.den) * i128::from(rate.num));
            u64::try_from(v).unwrap_or(0)
        };
        let entry = a.entry_point.unwrap_or(0);
        let units = a.duration.or(a.intrinsic_duration.map(|d| d.saturating_sub(entry))).unwrap_or(0);
        parts.push((o, f64::from(s.sample_rate.num) / f64::from(s.sample_rate.den), s.channels as usize, per_unit(entry), per_unit(units), path));
    }
    let Some(&(_, sample_rate, channels, _, _, _)) = parts.first() else {
        return Some(Finding::new(AUD_LOUDNESS, Status::NotApplicable, "the composition has no sound").at(here));
    };
    if parts.iter().any(|p| p.1 != sample_rate || p.2 != channels) {
        return Some(Finding::new(AUD_LOUDNESS, Status::Unchecked, "the reels differ in sample rate or channels (see DCP.AUD.001)").at(here));
    }
    let mut meter = LoudnessMeter::new(sample_rate, channels);
    meter.set_channel_weights(&weights(channels));
    let total: u64 = parts.iter().map(|p| p.4).sum();
    let mut done = 0u64;
    for (o, _, _, start, frames, path) in &parts {
        let mut at = 0u64;
        while at < *frames {
            control.report("loudness", done + at, total);
            if control.is_cancelled() {
                return None;
            }
            let n = usize::try_from((*frames - at).min(CHUNK as u64)).unwrap_or(CHUNK);
            let pcm = match o.file.read_pcm(&o.src, 0, start + at, n) {
                Ok(p) => p,
                Err(e) => return Some(Finding::new(AUD_LOUDNESS, Status::Unchecked, format!("'{path}' cannot be read: {e}")).at(here)),
            };
            let planes: Vec<&[f32]> = pcm.iter().map(Vec::as_slice).collect();
            meter.process(&planes);
            at += n as u64;
        }
        done += frames;
    }
    let s = meter.summary();
    let seconds = total as f64 / sample_rate;
    let integrated = if s.integrated_lufs.is_finite() { format!("{:.1} LUFS", s.integrated_lufs) } else { "below the gate".into() };
    let evidence = format!(
        "integrated {integrated}, loudness range {:.1} LU, true peak {:.1} dBTP, sample peak {:.1} dBFS, {seconds:.1} s, {channels} channels at {sample_rate} Hz",
        s.loudness_range_lu, s.true_peak_dbtp, s.sample_peak_dbfs
    );
    let (status, message) = if s.true_peak_dbtp > 0.0 {
        (Status::Warning, format!("true peak {:.1} dBTP is above full scale: the sound clips when converted", s.true_peak_dbtp))
    } else {
        (Status::Pass, format!("integrated {integrated}, true peak {:.1} dBTP", s.true_peak_dbtp))
    };
    let mut f = Finding::new(AUD_LOUDNESS, status, message).at(here);
    f.evidence = Some(evidence);
    Some(f)
}

/// BS.1770 channel weights in the ST 429-2 order: L, R, C, LFE (left out), Ls, Rs (1.41); the
/// channels after the sixth are not programme.
pub(crate) fn weights(channels: usize) -> Vec<f64> {
    (0..channels)
        .map(|c| match c {
            0..=2 => 1.0,
            4 | 5 => 1.41,
            _ => 0.0,
        })
        .collect()
}

/// `DCP.IAB.001`; `None` when cancelled.
fn immersive(cpls: &[(String, Composition)], files: &Files, control: &ValidationControl) -> Option<(Vec<Finding>, String)> {
    let mut out = Vec::new();
    let mut done: BTreeMap<Uuid, ()> = BTreeMap::new();
    let mut frames_checked = 0u64;
    for (cpl, c) in cpls {
        for (r, reel) in c.reels.iter().enumerate() {
            for a in reel.assets.iter().filter(|a| a.kind == AssetKind::AuxData) {
                let Some(id) = a.id else { continue };
                let Some((path, file)) = files.get(&id) else { continue };
                let Some(o) = open(file) else { continue };
                let Some(t) = o.file.tracks.first().filter(|t| t.codec == Codec::Iab) else { continue };
                let loc = location(cpl, r, a);
                let mut fail = |m: String| out.push(Finding::new(IAB_TRACK, Status::Fail, m).at(loc.clone()));
                let rate = rational(t.edit_rate);
                if let Some(er) = a.edit_rate
                    && !er.same_value(rate)
                {
                    fail(format!("'{path}' has the edit rate {rate}, the reel asset {er}"));
                }
                if let (Some(d), Some(fd)) = (a.intrinsic_duration, t.duration)
                    && i64::try_from(d).ok() != Some(fd)
                {
                    fail(format!("'{path}' is {fd} edit units long, the IntrinsicDuration is {d}"));
                }
                if done.insert(id, ()).is_some() {
                    continue;
                }
                if t.encryption.is_some() {
                    out.push(Finding::new(IAB_TRACK, Status::Unchecked, format!("'{path}' is encrypted; its frames need the content key")).at(loc));
                    continue;
                }
                let (n, f) = iab_frames(path, &o, rate, control)?;
                frames_checked += n;
                out.extend(f.into_iter().map(|f| f.at(loc.clone())));
            }
        }
    }
    if done.is_empty() && out.is_empty() {
        out.push(Finding::new(IAB_TRACK, Status::NotApplicable, "no immersive audio track file"));
    }
    Some((out, format!("{frames_checked} IABitstream frames in {} track files", done.len())))
}

/// Parses every frame of an IAB track file; `None` when cancelled.
fn iab_frames(path: &str, o: &Opened, rate: Rational, control: &ValidationControl) -> Option<(u64, Vec<Finding>)> {
    let samples = o.file.tracks[0].samples.len();
    // problem → (first frame, count)
    let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut order = Vec::new();
    let mut add = |m: String, k: usize| {
        let e = seen.entry(m.clone()).or_insert_with(|| {
            order.push(m);
            (k, 0)
        });
        e.1 += 1;
    };
    for k in 0..samples {
        if k.is_multiple_of(256) {
            control.report("iab", k as u64, samples as u64);
            if control.is_cancelled() {
                return None;
            }
        }
        let bytes = match o.file.read_sample(&o.src, 0, k) {
            Ok(b) => b,
            Err(e) => {
                add(format!("cannot be read: {e}"), k);
                continue;
            }
        };
        match cfa4_iab::parse(&bytes) {
            Ok(f) => {
                let (num, den) = f.frame_rate.rational();
                if !rate.same_value(Rational { num: i64::from(num), den: i64::from(den) }) {
                    add(format!("the IAFrame rate is {num}/{den}, the track's edit rate {rate}"), k);
                }
                if f.bit_depth != 24 {
                    add(format!("{}-bit audio; ST 429-18 immersive audio is 24-bit", f.bit_depth), k);
                }
            }
            Err(e) => add(format!("is not an IABitstream frame: {e}"), k),
        }
    }
    let findings = order
        .into_iter()
        .map(|m| {
            let (first, n) = seen[&m];
            let more = if n > 1 { format!(" ({n} frames)") } else { String::new() };
            Finding::new(IAB_TRACK, Status::Fail, format!("'{path}' frame {first}: {m}{more}"))
        })
        .collect();
    Some((samples as u64, findings))
}
