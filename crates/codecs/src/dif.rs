//! Raw DV files (`.dv`, `.dif`): a plain sequence of DIF frames as a camcorder writes them over
//! FireWire, decoded by `cfa4-dv`.
//!
//! Every frame has the same size (120 000 bytes at 525/60, 144 000 at 625/50 for 25 Mbit/s DV),
//! fixed by the first frame's header, so frame `n` starts at `n * size` and is read on demand: a
//! large file is never loaded whole. A trailing partial frame is left out.
//!
//! Audio (DV-4): the PCM in the frames' audio DIF blocks (16-bit, or 12-bit expanded to 16; two
//! or four channels; 48, 44.1 or 32 kHz). Each frame holds a slightly different number of samples
//! (1600 or 1602 at 48 kHz, 525/60), so the open reads every frame's AAUX source pack (5 bytes)
//! to place the frames' audio on the sample timeline.

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::CodecError;

const BLOCK: usize = 80;

/// The largest DIF frame (DVCPRO HD 1080i50: 4 channels of 12 sequences): caps reads sized by a
/// container's (untrusted) sample size.
pub(crate) const MAX_FRAME_BYTES: usize = 4 * 12 * 150 * BLOCK;

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
    audio: Option<DvAudio>,
}

/// Start timecode of a DV stream: the time code pack of its first frame's subcode, as a frame
/// count at `rate` (drop-frame counted when the pack's flag says so and the rate allows it).
pub(crate) fn start_timecode(first: &[u8], rate: FrameRate) -> Option<i64> {
    let tc = cfa4_dv::timecode(first)?;
    let n = |v: u8| i64::from(v);
    Some(cfa4_time::fields_to_frames(n(tc.hours), n(tc.minutes), n(tc.seconds), n(tc.frames), rate, tc.drop_frame))
}

/// The sound carried in the DIF frames of a stream (a raw `.dv` file, or a type-1 DV AVI): its
/// format from the first frame and where each frame's samples start.
pub(crate) struct DvAudio {
    pub info: AudioStreamInfo,
    /// First sample of each frame's audio (one more entry than frames: the end).
    starts: Vec<i64>,
}

impl DvAudio {
    /// `first` is the first whole frame; `pack(i)` reads the 5-byte AAUX source pack of frame `i`
    /// (at [`cfa4_dv::SOURCE_PACK_OFFSET`]). `None` when the frames carry no audio.
    pub fn scan(first: &[u8], frames: usize, mut pack: impl FnMut(usize) -> Option<Vec<u8>>) -> Option<Self> {
        let a = cfa4_dv::audio_format(first)?;
        let mut starts = Vec::with_capacity(frames + 1);
        let mut t = 0i64;
        for i in 0..frames {
            starts.push(t);
            // a frame without a readable pack keeps the nominal count
            let n = pack(i).and_then(|p| cfa4_dv::source_pack_samples(&p)).unwrap_or(a.samples);
            t += n as i64;
        }
        starts.push(t);
        let info = AudioStreamInfo {
            sample_rate: a.sample_rate,
            channels: a.channels as u32,
            codec: format!("DV audio ({}-bit PCM)", a.bits),
            bits_per_sample: Some(u32::from(a.bits)),
            speakers: Vec::new(),
        };
        Some(Self { info, starts })
    }

    /// `frames` samples at `sample_rate` from sample `start` (at the output rate); `frame(i)` reads
    /// DIF frame `i`. Linear resampling from the stream's rate.
    pub fn read(&self, start: i64, frames: usize, sample_rate: u32, frame: impl Fn(usize) -> Option<Vec<u8>>) -> AudioBuffer {
        let ainfo = &self.info;
        let starts = &self.starts;
        let count = starts.len().saturating_sub(1);
        let ch = ainfo.channels.max(1) as usize;
        // source samples covering the request, then linear resampling to `sample_rate`
        let ratio = f64::from(ainfo.sample_rate) / f64::from(sample_rate.max(1));
        let s0 = (start as f64 * ratio).floor() as i64;
        let need = (frames as f64 * ratio).ceil() as i64 + 2;
        let mut src: Vec<Vec<f32>> = vec![vec![0.0; need.max(0) as usize]; ch];
        let mut i = starts.partition_point(|&x| x <= s0).saturating_sub(1);
        while i < count && starts.get(i).is_some_and(|&t| t < s0 + need) {
            let t0 = starts.get(i).copied().unwrap_or(0);
            if let Some(a) = frame(i).and_then(|d| cfa4_dv::decode_audio(&d)) {
                for (c, dst) in src.iter_mut().enumerate() {
                    let Some(chan) = a.channels.get(c) else { continue };
                    for (k, &v) in chan.iter().enumerate() {
                        if let Some(d) = usize::try_from(t0 + k as i64 - s0).ok().and_then(|p| dst.get_mut(p)) {
                            *d = f32::from(v) / 32768.0;
                        }
                    }
                }
            }
            i += 1;
        }
        let frac = s0 as f64 - start as f64 * ratio;
        let mut out = AudioBuffer::silence(sample_rate, ch, frames);
        for (c, o) in out.channels.iter_mut().enumerate() {
            let Some(s) = src.get(c) else { continue };
            for (k, d) in o.iter_mut().enumerate() {
                let pos = k as f64 * ratio - frac;
                let i0 = pos.floor().max(0.0) as usize;
                let f = (pos - i0 as f64) as f32;
                let a = s.get(i0).copied().unwrap_or(0.0);
                let b = s.get(i0 + 1).copied().unwrap_or(a);
                *d = a + (b - a) * f;
            }
        }
        out
    }
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
        // audio: the first frame's format, every frame's sample count
        let first = cfa4_media::reader::read_range(&*reader, 0, frame_bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        let audio = DvAudio::scan(&first, frames, |i| {
            let at = i as u64 * frame_bytes as u64 + cfa4_dv::SOURCE_PACK_OFFSET as u64;
            cfa4_media::reader::read_range(&*reader, at, 5).ok()
        });
        let audio_streams = audio.iter().map(|a| a.info.clone()).collect();
        let start_timecode = start_timecode(&first, rate);
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
            audio_streams,
            container: "DV".into(),
            start_timecode,
            file_size: Some(len),
        };
        Ok(Self { info, reader, frames, frame_bytes, rate, audio })
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

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer, MediaError> {
        let a = self.audio.as_ref().ok_or(MediaError::NoStream("audio"))?;
        cfa4_media::validate_audio_buffer_shape(a.info.channels.max(1) as usize, frames)?;
        Ok(a.read(start, frames, sample_rate, |i| {
            (i < self.frames).then(|| cfa4_media::reader::read_range(&*self.reader, i as u64 * self.frame_bytes as u64, self.frame_bytes).ok()).flatten()
        }))
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
