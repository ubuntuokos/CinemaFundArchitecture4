//! Binaural stereo export (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` §3.5, work package B-05):
//! Export ▸ Audio ▸ Channels ▸ Binaural stereo (for headphones). The sequence's Mix, in its own
//! layout (mono to 7.1.4), is played through `cfa4_hrtf::ChannelMonitor` as virtual loudspeakers,
//! exactly as the Program monitor plays it with headphones on (`cfa4-cli serve`, `audio.render
//! {monitor}`); a layout the monitor has no loudspeakers for is folded to stereo first, as there.
//!
//! The file says what it is: `binaural, HRTF: <set>` ([`Binaural::note`]) goes into the WAV
//! `LIST/INFO` `ICMT`, the FLAC `COMMENT` and the MP4 / QuickTime `©cmt`. A set that cannot be
//! made is an export error; nothing here panics.

use std::sync::{Arc, Mutex};

use cfa4_hrtf::{ChannelMonitor, HrtfSet, MonitorConfig};

use crate::{BinauralAudio, ExportError, Result};

/// Sets already made, by (name in the settings, rate): the measuring passes and the export of one
/// job, and the jobs of a queue, share them. A few entries at most.
static SETS: Mutex<Vec<(String, u32, Arc<HrtfSet>)>> = Mutex::new(Vec::new());

fn error(e: impl std::fmt::Display) -> ExportError {
    ExportError::Unsupported(format!("Binaural stereo: {e}"))
}

/// `""` and `"builtin"` are the built-in set.
fn key(hrtf: &str) -> &str {
    if hrtf.trim().is_empty() { "builtin" } else { hrtf }
}

fn make(key: &str, rate: u32) -> std::result::Result<HrtfSet, String> {
    if key == "builtin" {
        return HrtfSet::builtin(rate).map_err(|e| e.to_string());
    }
    let lower = key.to_ascii_lowercase();
    if !lower.ends_with(".maudhrtf") {
        return Err(format!("{key}: the HRTF set must be \"builtin\" or a .maudhrtf file"));
    }
    let bytes = std::fs::read(key).map_err(|e| format!("{key}: {e}"))?;
    HrtfSet::from_maudhrtf(&bytes, Some(rate)).map_err(|e| format!("{key}: {e}"))
}

/// The set `hrtf` names, at `rate`.
pub fn set(hrtf: &str, rate: u32) -> Result<Arc<HrtfSet>> {
    let key = key(hrtf);
    if let Ok(sets) = SETS.lock()
        && let Some((.., s)) = sets.iter().find(|(k, r, _)| k == key && *r == rate)
    {
        return Ok(s.clone());
    }
    let made = match std::panic::catch_unwind(|| make(key, rate)) {
        Ok(Ok(s)) => Arc::new(s),
        Ok(Err(e)) => return Err(error(e)),
        Err(_) => return Err(error("the HRTF set could not be made")),
    };
    if let Ok(mut sets) = SETS.lock() {
        if sets.len() >= 4 {
            sets.remove(0);
        }
        sets.push((key.to_owned(), rate, made.clone()));
    }
    Ok(made)
}

/// The monitor settings of `b`, checked as the headphone monitor checks them.
pub fn config(b: &BinauralAudio) -> Result<MonitorConfig> {
    if !(b.room.is_finite() && (0.0..=1.0).contains(&b.room)) {
        return Err(error("the room must be 0 to 1"));
    }
    if !(b.lfe_db.is_finite() && (-30.0..=0.0).contains(&b.lfe_db)) {
        return Err(error("the LFE level must be -30 to 0 dB"));
    }
    Ok(MonitorConfig { room: b.room, lfe_db: b.lfe_db })
}

/// What a binaural file says about itself: `binaural, HRTF: <set>`.
pub fn note(set: &HrtfSet) -> String {
    format!("binaural, HRTF: {}", set.name())
}

/// The MP4 / QuickTime `udta` items of `settings`: Export ▸ Metadata, and for a binaural export
/// `binaural, HRTF: <set>` in `©cmt` (after the user's comment, on a line of its own).
pub(crate) fn udta(settings: &crate::ExportSettings) -> Vec<(String, String)> {
    let mut items = settings.metadata.udta();
    let Some(b) = settings.audio.binaural.as_ref().filter(|_| settings.has_audio()) else { return items };
    // the set's name does not depend on the rate: the cached 48 kHz set names it
    let Ok(set) = set(&b.hrtf, 48_000) else { return items };
    let note = note(&set);
    match items.iter_mut().find(|(k, _)| k == "©cmt") {
        Some((_, c)) => *c = format!("{c}\n{note}"),
        None => items.insert(items.len().saturating_sub(1), ("©cmt".into(), note)),
    }
    items
}

/// One export's headphone renderer.
pub(crate) struct Binaural {
    monitor: ChannelMonitor,
    pub note: String,
}

impl Binaural {
    pub fn new(b: &BinauralAudio, rate: u32) -> Result<Self> {
        let config = config(b)?;
        let set = set(&b.hrtf, rate)?;
        let note = note(&set);
        Ok(Binaural { monitor: ChannelMonitor::new(set, config).map_err(error)?, note })
    }

    /// The two ears of `mix` (one block of the sequence's Mix starting at sample `start`, in the
    /// mixer's layout), `frames` long. The renderer keeps its history while the blocks follow each
    /// other and starts over on a jump (a measuring pass rewinding), as the monitor does.
    pub fn ears(&mut self, start: i64, mix: cfa4_frame::AudioBuffer, frames: usize) -> Result<Vec<Vec<f32>>> {
        let mix = if ChannelMonitor::supports(mix.channels.len()) {
            mix
        } else {
            cfa4_render::audio::to_layout(mix, cfa4_audio_dsp::channels::Layout::Stereo, cfa4_audio_dsp::channels::Mixdown::FrontRear)
        };
        let mut inputs: Vec<Vec<f32>> = mix.channels;
        for c in inputs.iter_mut() {
            c.resize(frames, 0.0);
        }
        let refs: Vec<&[f32]> = inputs.iter().map(Vec::as_slice).collect();
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        self.monitor.process(start, &refs, &mut l, &mut r).map_err(error)?;
        Ok(vec![l, r])
    }
}
