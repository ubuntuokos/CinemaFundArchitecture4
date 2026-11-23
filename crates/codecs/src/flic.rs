//! Autodesk Animator FLIC (`.fli`, `.flc`, `.flx`) files decoded by our pure-Rust `cfa4-flic`.
//!
//! Every frame after the first is a delta of the previous picture, so a frame is reached by
//! decoding forward from the start. The decoder's state is checkpointed along the way (an
//! 8-bit picture is small) and a seek restarts from the nearest checkpoint; the last pictures
//! asked for are cached. Files of other depths than 8 bits, and the audio of "X-COM: Terror from
//! the Deep" files, are not decoded.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use cfa4_flic::{CHUNK_MAGIC_1, CHUNK_MAGIC_2, Decoder, HEADER_SIZE, Header, PREAMBLE_SIZE};
use cfa4_frame::{AudioBuffer, PixelData, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::{CodecError, Result};

/// The codec name shown for FLIC files.
pub const NAME: &str = "Autodesk FLIC";

/// How much of a file [`sniff`] looks at.
pub const SNIFF_LEN: usize = 4096;
/// The most frames a file may index.
const MAX_FRAMES: usize = 1 << 21;
/// The most checkpoints kept; the interval between them grows with the frame count.
const MAX_CHECKPOINTS: usize = 256;
const MIN_INTERVAL: usize = 32;
/// Decoded pictures kept for repeated requests.
const CACHE: usize = 8;

/// Whether `head` starts a FLIC file this decoder takes.
pub fn sniff(head: &[u8]) -> bool {
    cfa4_flic::probe(head.get(..SNIFF_LEN).unwrap_or(head)) && Header::parse(head).is_ok_and(|h| h.depth == 8)
}

struct State {
    dec: Decoder,
    /// Frames applied to `dec`.
    next: usize,
    /// Decoder states before frame `i * interval`.
    checkpoints: Vec<(usize, Decoder)>,
    cache: VecDeque<(usize, Arc<VideoFrame>)>,
}

/// A FLIC file.
pub struct FlicSource {
    info: MediaInfo,
    reader: cfa4_media::SharedReader,
    header: Header,
    /// Offset and size of every frame chunk.
    frames: Vec<(u64, u64)>,
    rate: FrameRate,
    interval: usize,
    state: Mutex<State>,
}

impl FlicSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> Result<Self> {
        let len = reader.len();
        let read = |off: u64, n: usize| cfa4_media::reader::read_range(&*reader, off, n).map_err(|e| CodecError::Container(e.to_string()));
        let head = read(0, HEADER_SIZE + PREAMBLE_SIZE)?;
        let header = Header::parse(&head).map_err(|e| CodecError::Decode(e.to_string()))?;
        let dec = Decoder::new(&header).map_err(|e| match e {
            cfa4_flic::Error::Depth(d) => CodecError::Unsupported(format!("FLIC: {d}-bit pictures")),
            e => CodecError::Decode(e.to_string()),
        })?;

        // the frame chunks: those with a frame preamble; anything else is skipped by its size
        let mut frames = Vec::new();
        let mut pos = header.first_chunk as u64;
        while pos + PREAMBLE_SIZE as u64 <= len {
            let p = read(pos, PREAMBLE_SIZE)?;
            let size = u64::from(u32::from_le_bytes([p[0], p[1], p[2], p[3]]));
            let magic = u16::from_le_bytes([p[4], p[5]]);
            if (magic == CHUNK_MAGIC_1 || magic == CHUNK_MAGIC_2) && size > PREAMBLE_SIZE as u64 {
                frames.push((pos, size.min(len - pos)));
                if frames.len() > MAX_FRAMES {
                    return Err(CodecError::Container("FLIC: too many frames".into()));
                }
                pos += size;
            } else if magic == cfa4_flic::CHUNK_TFTD_AUDIO {
                // a 10-byte sub-header the chunk size does not count, then the samples
                pos += PREAMBLE_SIZE as u64 + 10 + size;
            } else {
                pos += size.max(PREAMBLE_SIZE as u64);
            }
        }
        if frames.is_empty() {
            return Err(CodecError::Container("FLIC: no frames".into()));
        }

        let rate = FrameRate::new(i64::from(header.frame_rate.0), i64::from(header.frame_rate.1.max(1)));
        let count = i64::try_from(frames.len()).unwrap_or(i64::MAX);
        let duration = Tick::from_rational(count.saturating_mul(rate.den), 1, rate.num);
        let secs = duration.seconds();
        let bitrate = (secs > 0.0).then(|| (len.saturating_mul(8) as f64 / secs) as u64);
        let color = crate::stream_color::resolve(header.width, header.height, &[]);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width: header.width,
                height: header.height,
                frame_rate: rate,
                par: (1, 1),
                codec: NAME.into(),
                pixel_format: "PAL8".into(),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "FLIC".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        let interval = MIN_INTERVAL.max(frames.len().div_ceil(MAX_CHECKPOINTS));
        let state = State { dec, next: 0, checkpoints: Vec::new(), cache: VecDeque::new() };
        Ok(Self { info, reader, header, frames, rate, interval, state: Mutex::new(state) })
    }

    fn packet(&self, i: usize) -> Result<Vec<u8>> {
        let &(off, size) = self.frames.get(i).ok_or_else(|| CodecError::Container("FLIC: frame index out of range".into()))?;
        let size = usize::try_from(size).map_err(|_| CodecError::Container("FLIC: frame too large".into()))?;
        cfa4_media::reader::read_range(&*self.reader, off, size).map_err(|e| CodecError::Container(e.to_string()))
    }

    fn frame(&self, i: usize) -> Result<Arc<VideoFrame>> {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, f)) = st.cache.iter().find(|(k, _)| *k == i) {
            return Ok(f.clone());
        }
        // restart from the nearest checkpoint when the picture asked for is behind the decoder
        if st.next > i + 1 {
            let (at, dec) = match st.checkpoints.iter().rev().find(|(k, _)| *k <= i) {
                Some((k, d)) => (*k, d.clone()),
                None => (0, Decoder::new(&self.header).map_err(|e| CodecError::Decode(e.to_string()))?),
            };
            st.dec = dec;
            st.next = at;
        }
        let mut last = Ok(());
        while st.next <= i {
            if st.next.is_multiple_of(self.interval) && !st.checkpoints.iter().any(|(k, _)| *k == st.next) {
                let snap = st.dec.clone();
                let at = st.next;
                st.checkpoints.push((at, snap));
            }
            let packet = self.packet(st.next)?;
            last = st.dec.decode(&packet).map_err(|e| CodecError::Decode(e.to_string()));
            st.next += 1;
            // a damaged frame leaves the picture as far as it got; only the one asked for fails
            if st.next <= i {
                last = Ok(());
            }
        }
        last?;
        let v = self.info.video.as_ref().ok_or_else(|| CodecError::Container("FLIC: no video".into()))?;
        let frame = Arc::new(VideoFrame {
            width: v.width,
            height: v.height,
            data: PixelData::Rgba8(Arc::new(st.dec.rgba())),
            color: v.color,
            par: v.par,
            pts: Tick::from_rational(i as i64, self.rate.den, self.rate.num),
            layout: Default::default(),
        });
        st.cache.push_back((i, frame.clone()));
        while st.cache.len() > CACHE {
            st.cache.pop_front();
        }
        Ok(frame)
    }
}

impl MediaSource for FlicSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let last = self.frames.len().saturating_sub(1);
        let idx = usize::try_from(self.rate.frame_at(req.time.max(Tick::ZERO))).unwrap_or(last).min(last);
        Ok(self.frame(idx)?)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        Err(MediaError::NoStream("audio"))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(FlicSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(FlicSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_files_are_not_sniffed() {
        assert!(!sniff(b""));
        assert!(!sniff(&[0u8; 200]));
        assert!(!sniff(b"RIFF\0\0\0\0AVI LIST"));
    }
}
