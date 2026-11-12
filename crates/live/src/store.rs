//! What the server shows: the latest program picture and the live status, shared between the
//! engine (writer) and the server's threads (readers).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

/// How a cue's sound reaches the broadcast (plan C10 "Hang").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioMode {
    /// The Browser Source page plays it, in step with the picture (OBS: "Control audio via OBS").
    Embedded,
    /// Served on its own (`/audio`) for a separate media source in OBS.
    Separate,
    /// No sound from the live graphics.
    #[default]
    Ignore,
}

impl AudioMode {
    pub fn parse(s: &str) -> Option<AudioMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "embedded" | "beágyazott" => Some(AudioMode::Embedded),
            "separate" | "külön" => Some(AudioMode::Separate),
            "ignore" | "off" | "none" | "nincs" => Some(AudioMode::Ignore),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AudioMode::Embedded => "embedded",
            AudioMode::Separate => "separate",
            AudioMode::Ignore => "ignore",
        }
    }
}

/// An empty, fully transparent 1×1 PNG: what an off-air program shows.
pub const CLEAR_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
    0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01,
    0xe9, 0xfa, 0xdc, 0xd8, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[derive(Default)]
struct Inner {
    png: Option<Arc<Vec<u8>>>,
    status: Value,
    audio: Option<PathBuf>,
}

/// The program picture and status. `seq` grows with every new picture, so a page fetches only
/// what changed.
#[derive(Default)]
pub struct FrameStore {
    seq: AtomicU64,
    /// Bumped when the cue (and so its sound) changes: the page restarts the sound.
    audio_seq: AtomicU64,
    inner: Mutex<Inner>,
}

impl FrameStore {
    pub fn new() -> Arc<FrameStore> {
        Arc::new(FrameStore::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new program picture (PNG bytes).
    pub fn publish(&self, png: Vec<u8>) -> u64 {
        self.lock().png = Some(Arc::new(png));
        self.seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Off air: a transparent picture.
    pub fn clear(&self) -> u64 {
        self.publish(CLEAR_PNG.to_vec())
    }

    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }

    pub fn png(&self) -> Arc<Vec<u8>> {
        self.lock().png.clone().unwrap_or_else(|| Arc::new(CLEAR_PNG.to_vec()))
    }

    /// The live status the page and agents read (`/status.json`): on air, held, the cue and its
    /// page, the audio mode. The store adds `seq` and `audioSeq`.
    pub fn set_status(&self, status: Value) {
        self.lock().status = status;
    }

    pub fn status(&self) -> Value {
        let mut v = self.lock().status.clone();
        if !v.is_object() {
            v = json!({});
        }
        v["seq"] = json!(self.seq());
        v["audioSeq"] = json!(self.audio_seq.load(Ordering::SeqCst));
        v
    }

    /// The sound of the cue on air (None: none, or the mode is to ignore it).
    pub fn set_audio(&self, file: Option<PathBuf>) {
        let mut g = self.lock();
        if g.audio != file {
            g.audio = file;
            self.audio_seq.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn audio(&self) -> Option<PathBuf> {
        self.lock().audio.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_counts_pictures_and_keeps_the_latest() {
        let s = FrameStore::new();
        assert_eq!(s.seq(), 0);
        assert_eq!(&**s.png(), CLEAR_PNG);
        assert_eq!(s.publish(vec![1, 2, 3]), 1);
        assert_eq!(&**s.png(), &[1, 2, 3]);
        s.clear();
        assert_eq!(s.seq(), 2);
        s.set_status(json!({"onAir": true}));
        assert_eq!(s.status()["onAir"], true);
        assert_eq!(s.status()["seq"], 2);
        s.set_status(json!("not an object"));
        assert!(s.status().is_object());
        s.set_audio(Some("/a.wav".into()));
        s.set_audio(Some("/a.wav".into()));
        assert_eq!(s.status()["audioSeq"], 1);
    }

    #[test]
    fn audio_modes_parse_in_english_and_hungarian() {
        assert_eq!(AudioMode::parse("Embedded"), Some(AudioMode::Embedded));
        assert_eq!(AudioMode::parse("külön"), Some(AudioMode::Separate));
        assert_eq!(AudioMode::parse("off"), Some(AudioMode::Ignore));
        assert_eq!(AudioMode::parse("loud"), None);
        assert_eq!(AudioMode::Separate.name(), "separate");
    }
}
