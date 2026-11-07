//! Raw DV files (`.dv`, `.dif`): a plain sequence of DIF frames as a camcorder writes them over
//! FireWire, decoded by `cfa4-dv`.
//!
//! Every frame has the same size (120 000 bytes at 525/60, 144 000 at 625/50 for 25 Mbit/s DV),
//! fixed by the first frame's header, so frame `n` starts at `n * size` and is read on demand: a
//! large file is never loaded whole. A trailing partial frame is left out. The audio in the DIF
//! blocks is not read yet (the importer shows the file as video only).

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::CodecError;

const BLOCK: usize = 80;

/// True when `head` starts with the first DIF sequence of a frame: a header block, two subcode
/// blocks, three VAUX blocks, an audio block and a video block, all of DIF sequence 0, and a
/// header `cfa4-dv` recognises.
pub fn sniff(head: &[u8]) -> bool {
    use cfa4_dv::dif::{block_sct, block_sequence, sct};
    let want = [sct::HEADER, sct::SUBCODE, sct::SUBCODE, sct::VAUX, sct::VAUX, sct::VAUX, sct::AUDIO, sct::VIDEO];
    want.iter().enumerate().all(|(i, &s)| head.get(i * BLOCK..(i + 1) * BLOCK).is_some_and(|b| block_sct(b) == Some(s) && block_sequence(b) == Some(0)))
        && cfa4_dv::probe(head).is_some()
}

pub struct DifSource {
    info: MediaInfo,
    reader: cfa4_media::SharedReader,
    frames: usize,
    frame_bytes: usize,
    rate: FrameRate,
}

impl DifSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> crate::Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> crate::Result<Self> {
        let len = reader.len();
        let head =
            cfa4_media::reader::read_range(&*reader, 0, (len.min(6 * BLOCK as u64)) as usize).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        let d = crate::video::dv_stream_info(&head).ok_or_else(|| CodecError::Container(format!("{name}: DV: no DIF header block")))?;
        let p = d.profile;
        if !p.is_supported() {
            return Err(CodecError::Unsupported(format!("{name}: {} (CFA4 decodes DV, DVCAM and DVCPRO 25)", d.codec)));
        }
        let frame_bytes = p.frame_size();
        let frames = usize::try_from(len / frame_bytes.max(1) as u64).unwrap_or(usize::MAX);
        if frame_bytes == 0 || frames == 0 {
            return Err(CodecError::Container(format!("{name}: DV: no complete frame")));
        }
        let (num, den) = p.frame_rate();
        let rate = FrameRate::new(i64::from(num), i64::from(den));
        let duration = Tick::from_rational(frames as i64, rate.den, rate.num);
        let bitrate = (frame_bytes as u64 * 8).checked_mul(rate.num as u64).map(|b| b / rate.den as u64);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width: p.width,
                height: p.height,
                frame_rate: rate,
                par: d.par,
                codec: d.codec,
                pixel_format: d.pixel_format,
                color: d.color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "DV".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { info, reader, frames, frame_bytes, rate })
    }

    fn frame(&self, idx: usize) -> crate::Result<VideoFrame> {
        if idx >= self.frames {
            return Err(CodecError::Container("DV: frame index out of range".into()));
        }
        let at = idx as u64 * self.frame_bytes as u64;
        let data = cfa4_media::reader::read_range(&*self.reader, at, self.frame_bytes).map_err(|e| CodecError::Container(e.to_string()))?;
        // frames are independent: a fresh decoder per frame keeps the source shareable
        let pic = cfa4_dv::Decoder::new().decode(&data).map_err(|e| match e {
            cfa4_dv::Error::Unsupported(_) => CodecError::Unsupported(e.to_string()),
            _ => CodecError::Decode(e.to_string()),
        })?;
        let mut f = crate::video::dv_to_video_frame(pic);
        f.pts = Tick::from_rational(idx as i64, self.rate.den, self.rate.num);
        Ok(f)
    }
}

impl MediaSource for DifSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>, MediaError> {
        let last = self.frames.saturating_sub(1);
        let idx = usize::try_from(self.rate.frame_at(req.time.max(Tick::ZERO))).unwrap_or(last).min(last);
        Ok(Arc::new(self.frame(idx)?))
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> Result<AudioBuffer, MediaError> {
        Err(MediaError::NoStream("audio"))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(DifSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(DifSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_needs_a_whole_dif_sequence_head() {
        assert!(!sniff(&[]));
        assert!(!sniff(&[0u8; 8 * BLOCK]));
        assert!(!sniff(b"RIFF\0\0\0\0AVI LIST"));
    }
}
