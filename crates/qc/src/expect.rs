//! What the render was supposed to produce: the render plan a checked file is compared with.
//!
//! The engine fills it from the export settings; on the command line it is a JSON file
//! (`cfa4-cli qc --expect plan.json`). Every field is optional: a check whose expectation is
//! missing reports NOT_RUN ("no render plan given") instead of guessing.

use serde::{Deserialize, Serialize};

use crate::Error;

/// Largest render-plan file read.
pub const MAX_EXPECT_BYTES: usize = 64 * 1024;

/// A frame rate as a fraction (`30000/1001`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rate {
    pub num: i64,
    pub den: i64,
}

impl Rate {
    pub fn is_valid(&self) -> bool {
        self.num > 0 && self.den > 0
    }

    /// Milliseconds of `frames` frames.
    pub fn frames_ms(&self, frames: u64) -> Option<f64> {
        self.is_valid().then(|| frames as f64 * 1000.0 * self.den as f64 / self.num as f64)
    }
}

/// The render plan.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectation {
    /// A video stream is expected (`false`: audio-only export). Unset: decided from the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<Rate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Start of the video codec label, case-insensitive (`"H.264"`, `"HEVC"`, `"AV1"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_tracks: Option<u32>,
    /// Channels of every audio track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    /// Start of the audio codec label, case-insensitive (`"AAC"`, `"PCM"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_streams: Option<u32>,
}

impl Expectation {
    pub fn from_json(text: &str) -> Result<Expectation, Error> {
        if text.len() > MAX_EXPECT_BYTES {
            return Err(Error::InvalidExpectation(format!("larger than {MAX_EXPECT_BYTES} bytes")));
        }
        let e: Expectation = serde_json::from_str(text).map_err(|e| Error::InvalidExpectation(e.to_string()))?;
        if e.frame_rate.is_some_and(|r| !r.is_valid()) {
            return Err(Error::InvalidExpectation("frameRate needs a positive num and den".into()));
        }
        Ok(e)
    }

    /// Planned duration in milliseconds (frames at the frame rate).
    pub fn duration_ms(&self) -> Option<f64> {
        self.frame_rate?.frames_ms(self.frames?)
    }

    /// Whether any stream expectation is set (the streams check can run).
    pub fn has_stream_plan(&self) -> bool {
        self.video.is_some() || self.audio_tracks.is_some() || self.audio_channels.is_some() || self.sample_rate.is_some() || self.subtitle_streams.is_some()
    }
}

/// `label` starts with `want`, ignoring case and surrounding spaces.
pub fn label_matches(label: &str, want: &str) -> bool {
    label.trim().to_lowercase().starts_with(&want.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        let e = Expectation::from_json(r#"{"frames": 48, "frameRate": {"num": 24, "den": 1}, "audioTracks": 1}"#).unwrap();
        assert_eq!(e.duration_ms(), Some(2000.0));
        assert!(e.has_stream_plan());
        assert!(Expectation::from_json(r#"{"frames": 48, "frameRate": {"num": 0, "den": 1}}"#).is_err());
        assert!(Expectation::from_json(r#"{"frame": 48}"#).is_err(), "typos are errors");
        assert!(Expectation::from_json("[").is_err());
        assert!(label_matches("H.264 (High)", "h.264"));
        assert!(!label_matches("HEVC", "H.264"));
    }
}
