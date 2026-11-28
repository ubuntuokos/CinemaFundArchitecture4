//! NTT TwinVQ (VQF) files (`.vqf`, `.vql`, `.vqe`): `cfa4-twinvq` reads the header and decodes the
//! frames (CB-31.2). The sound is decoded whole at open.

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource};
use cfa4_time::Tick;

use crate::audio::read_resampled;
use crate::{CodecError, Result};

pub fn sniff(head: &[u8]) -> bool {
    cfa4_twinvq::probe(head)
}

pub struct TwinvqSource {
    info: MediaInfo,
    rate: u32,
    samples: Vec<Vec<f32>>,
}

impl TwinvqSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let s = cfa4_twinvq::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        // only whole frames the file really holds are decoded, so the sound is bounded by the file
        let samples = cfa4_twinvq::decode(&bytes, &s).channels;
        let frames = samples.first().map_or(0, Vec::len);
        if frames == 0 {
            return Err(CodecError::Container(format!("{name}: no sound")));
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::AudioOnly,
            duration: Tick::from_units(frames as i64, i64::from(s.sample_rate)),
            video: None,
            audio_streams: vec![AudioStreamInfo {
                sample_rate: s.sample_rate,
                channels: s.channels as u32,
                codec: "NTT TwinVQ".into(),
                bits_per_sample: None,
                ..AudioStreamInfo::default()
            }],
            container: "NTT VQF".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, rate: s.sample_rate, samples })
    }
}

impl MediaSource for TwinvqSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        Err(MediaError::NoStream("video"))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        cfa4_media::validate_audio_buffer_shape(self.samples.len(), frames)?;
        Ok(read_resampled(&self.samples, self.rate, start, frames, sample_rate))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(TwinvqSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0WAVE"));
        assert!(sniff(b"TWIN97012000"));
        assert!(opener("x.vqf", Arc::from(&b"TWIN97012000\0\0\0\x08"[..])).is_some_and(|r| r.is_err()));
    }
}
