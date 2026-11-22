//! Nintendo audio streams (`.brstm`, `.bfstm`, `.bcstm`): `cfa4-nstream` reads the header and
//! decodes the PCM or DSP ADPCM blocks (CB-29.2, CB-30.1). The sound is decoded whole at open.

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource};
use cfa4_nstream::{Codec, Format, Stream};
use cfa4_time::Tick;

use crate::audio::read_resampled;
use crate::{CodecError, Result};

pub fn sniff(head: &[u8]) -> bool {
    cfa4_nstream::probe(head).is_some()
}

pub struct NstreamSource {
    info: MediaInfo,
    rate: u32,
    samples: Vec<Vec<f32>>,
}

fn container(s: &Stream) -> &'static str {
    match s.format {
        Format::Brstm => "Nintendo BRSTM",
        Format::Bfstm => "Nintendo BFSTM",
        Format::Bcstm => "Nintendo BCSTM",
    }
}

impl NstreamSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let s = cfa4_nstream::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        // only blocks the file really holds are decoded, so the sound is bounded by the file
        let d = cfa4_nstream::decode(&bytes, &s);
        let frames = d.channels.first().map_or(0, Vec::len);
        if frames == 0 {
            return Err(CodecError::Container(format!("{name}: no sound")));
        }
        let samples: Vec<Vec<f32>> = d.channels.into_iter().map(|c| c.into_iter().map(|v| f32::from(v) / 32768.0).collect()).collect();
        let (codec, bits) = match s.codec {
            Codec::Pcm8 => ("PCM signed 8-bit", Some(8)),
            Codec::Pcm16 { .. } => ("PCM signed 16-bit", Some(16)),
            Codec::Dsp { .. } => ("Nintendo DSP ADPCM", Some(16)),
        };
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::AudioOnly,
            duration: Tick::from_units(frames as i64, i64::from(s.sample_rate)),
            video: None,
            audio_streams: vec![AudioStreamInfo {
                sample_rate: s.sample_rate,
                channels: s.channels,
                codec: codec.into(),
                bits_per_sample: bits,
                ..AudioStreamInfo::default()
            }],
            container: container(&s).into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, rate: s.sample_rate, samples })
    }
}

impl MediaSource for NstreamSource {
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
    Some(NstreamSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0WAVE"));
        assert!(sniff(b"RSTM\xFE\xFF\x01\x00"));
        assert!(opener("x.brstm", Arc::from(&b"RSTM\xFE\xFF\x01\x00\0\0\0\0\0\x40"[..])).is_some_and(|r| r.is_err()));
    }
}
