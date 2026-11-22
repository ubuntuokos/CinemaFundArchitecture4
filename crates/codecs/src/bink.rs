//! Bink 1 files (`.bik`) with Bink video (revisions `b`, `f` to `k`, with alpha) and the RDFT or
//! DCT Bink audio: `cfa4-bink` demuxes and decodes (backlog CB-23.1, CB-24.1).
//!
//! The timeline is the frame count at the file's frame rate. A frame needs the one before it
//! (skipped and motion blocks copy from it), so a seek decodes forward from the nearest frame
//! the index flags as a key frame. Audio is indexed by frame (the packet of the first audio
//! track in it); its timeline is the sample count each packet declares, as in ffmpeg. The audio
//! decoder overlaps blocks, so it carries state through the stream: a jump restores the nearest
//! earlier decoder checkpoint and decodes forward, and every read equals playback from the
//! start. Only the first audio track is used.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use cfa4_bink::container::{self, Bink};
use cfa4_bink::{audio, video};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Audio packets between decoder checkpoints.
const CHECKPOINT_BLOCKS: usize = 64;
/// Decoded audio packets kept.
const AUDIO_CACHE_BLOCKS: usize = 48;

pub fn sniff(head: &[u8]) -> bool {
    container::probe(head)
}

fn container_error(e: impl std::fmt::Display) -> CodecError {
    CodecError::Container(format!("Bink: {e}"))
}

struct VStream {
    tag: [u8; 4],
    width: u32,
    height: u32,
    flags: u32,
    rate: FrameRate,
    /// Byte range of each frame's video packet, and the key frame flags.
    packets: Vec<Range<usize>>,
    keys: Vec<bool>,
    cache: GopCache,
}

struct AudioState {
    dec: audio::Decoder,
    next: usize,
    checkpoints: BTreeMap<usize, audio::Decoder>,
    blocks: HashMap<usize, Arc<Vec<Vec<f32>>>>,
    order: VecDeque<usize>,
}

struct AStream {
    sample_rate: u32,
    channels: usize,
    /// Byte range of each frame's audio packet (empty when the frame has none).
    packets: Vec<Range<usize>>,
    /// First sample (per channel) of each frame's packet, and the total at the end.
    starts: Vec<i64>,
    state: Mutex<AudioState>,
}

pub struct BinkSource {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    video: VStream,
    audio: Option<AStream>,
}

fn label(tag: &[u8; 4]) -> String {
    format!("Bink video (revision {})", tag[3] as char)
}

impl BinkSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let bink = Bink::parse(&bytes).map_err(|e| CodecError::Unsupported(format!("{name}: {e}")))?;
        // the decoder checks the stream parameters
        video::Decoder::new(bink.width, bink.height, bink.tag, bink.flags).map_err(|e| CodecError::Unsupported(format!("{name}: {e}")))?;
        let n = bink.frame_count();
        if n == 0 {
            return Err(container_error(format!("{name}: no video frames")));
        }
        let mut packets = Vec::with_capacity(n);
        let mut keys = Vec::with_capacity(n);
        let mut apackets: Vec<Range<usize>> = Vec::with_capacity(n);
        let mut declared = Vec::with_capacity(n);
        let mut damaged = 0usize;
        for i in 0..n {
            match bink.layout(i) {
                Ok(l) => {
                    keys.push(l.keyframe);
                    packets.push(l.video);
                    let a = l.audio.into_iter().find(|(t, _)| *t == 0).map(|(_, r)| r).unwrap_or(0..0);
                    declared.push(if a.len() >= 4 {
                        u32::from_le_bytes([bytes[a.start], bytes[a.start + 1], bytes[a.start + 2], bytes[a.start + 3]])
                    } else {
                        0
                    });
                    apackets.push(a);
                }
                Err(_) => {
                    // a damaged tail ends the index
                    damaged += 1;
                    break;
                }
            }
        }
        let _ = damaged;
        if packets.is_empty() {
            return Err(container_error(format!("{name}: no readable video frames")));
        }
        let frames = packets.len();
        let rate = FrameRate::new(i64::from(bink.fps_num), i64::from(bink.fps_den));
        let mut duration = rate.tick_of(frames as i64);
        let color = crate::stream_color::resolve(
            bink.width,
            bink.height,
            &[crate::stream_color::ColorCodes { primaries: 2, transfer: 2, matrix: 2, full_range: Some(bink.tag[3] == b'k') }],
        );
        let has_alpha = bink.flags & 0x0010_0000 != 0;
        let bitrate = u64::try_from(duration.to_rational_round(1, 1)).ok().filter(|&s| s > 0).map(|s| bytes.len() as u64 * 8 / s);
        let video_info = VideoStreamInfo {
            width: bink.width,
            height: bink.height,
            frame_rate: rate,
            par: (1, 1),
            codec: label(&bink.tag),
            pixel_format: if has_alpha { "YUVA 4:2:0 8-bit, progressive".into() } else { "YUV 4:2:0 8-bit, progressive".into() },
            color,
            has_alpha,
            bitrate,
            hdr: None,
        };
        let mut audio_streams = Vec::new();
        let mut astream = None;
        if let Some(t) = bink.tracks.first()
            && let Ok(dec) = audio::Decoder::new(t.sample_rate, t.channels, t.dct, bink.tag[3])
        {
            let channels = t.channels as usize;
            let mut starts = Vec::with_capacity(frames + 1);
            let mut total = 0i64;
            starts.push(0);
            for d in &declared {
                // the decoded size in bytes of 16-bit samples
                total += i64::from(*d) / (2 * channels as i64);
                starts.push(total);
            }
            duration = duration.max(Tick::from_rational(total, 1, i64::from(t.sample_rate.max(1))));
            audio_streams.push(AudioStreamInfo {
                sample_rate: t.sample_rate,
                channels: t.channels,
                codec: format!("Bink audio ({})", if t.dct { "DCT" } else { "RDFT" }),
                bits_per_sample: None,
                speakers: Vec::new(),
            });
            astream = Some(AStream {
                sample_rate: t.sample_rate,
                channels,
                packets: apackets,
                starts,
                state: Mutex::new(AudioState {
                    dec: dec.clone(),
                    next: 0,
                    checkpoints: BTreeMap::from([(0, dec)]),
                    blocks: HashMap::new(),
                    order: VecDeque::new(),
                }),
            });
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(video_info),
            audio_streams,
            container: "Bink".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        let vs = VStream { tag: bink.tag, width: bink.width, height: bink.height, flags: bink.flags, rate, packets, keys, cache: GopCache::new(None) };
        Ok(Self { info, bytes, video: vs, audio: astream })
    }

    fn audio_block(&self, a: &AStream, st: &mut AudioState, k: usize) -> Result<Arc<Vec<Vec<f32>>>> {
        if let Some(b) = st.blocks.get(&k) {
            return Ok(b.clone());
        }
        if (st.next > k || k - st.next > CHECKPOINT_BLOCKS)
            && let Some((&j, d)) = st.checkpoints.range(..=k).next_back()
            && !(j <= st.next && st.next <= k)
        {
            st.dec = d.clone();
            st.next = j;
        }
        while st.next <= k {
            let n = st.next;
            if n.is_multiple_of(CHECKPOINT_BLOCKS) && !st.checkpoints.contains_key(&n) {
                st.checkpoints.insert(n, st.dec.clone());
            }
            let len = usize::try_from(a.starts[n + 1] - a.starts[n]).unwrap_or(0);
            let mut out = vec![Vec::with_capacity(len); a.channels];
            let range = a.packets.get(n).cloned().unwrap_or(0..0);
            if range.len() >= 4
                && let Some(pkt) = self.bytes.get(range)
            {
                // a damaged packet decodes to silence; the decoder state goes on as ffmpeg's does
                if let Ok(s) = st.dec.decode(pkt) {
                    for (i, v) in s.iter().enumerate() {
                        out[i % a.channels].push(*v);
                    }
                }
            }
            for o in &mut out {
                o.resize(len, 0.0);
            }
            st.blocks.insert(n, Arc::new(out));
            st.order.push_back(n);
            while st.order.len() > AUDIO_CACHE_BLOCKS {
                if let Some(old) = st.order.pop_front() {
                    st.blocks.remove(&old);
                }
            }
            st.next = n + 1;
        }
        st.blocks.get(&k).cloned().ok_or_else(|| container_error("audio block not decoded"))
    }

    fn read_audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        let a = self.audio.as_ref().ok_or(MediaError::NoStream("audio"))?;
        let rate = a.sample_rate.max(1);
        let ch = a.channels.max(1);
        let source_frames = cfa4_media::resample_source_frames(frames, rate, sample_rate)?;
        let need = source_frames.checked_add(2).ok_or_else(|| MediaError::Decode("audio source window overflows".into()))?;
        cfa4_media::validate_audio_buffer_shape(ch, need)?;
        cfa4_media::validate_audio_buffer_shape(ch, frames)?;
        let ratio = f64::from(rate) / f64::from(sample_rate.max(1));
        let s0 = (start as f64 * ratio).floor() as i64;
        let need_i64 = i64::try_from(need).unwrap_or(i64::MAX);
        let mut src: Vec<Vec<f32>> = vec![vec![0.0; need]; ch];
        let mut st = a.state.lock().unwrap_or_else(|e| e.into_inner());
        let blocks = a.packets.len();
        let mut k = a.starts.partition_point(|&b| b <= s0.max(0)).saturating_sub(1);
        while k < blocks {
            let b_start = a.starts.get(k).copied().unwrap_or(i64::MAX);
            if b_start >= s0.saturating_add(need_i64) {
                break;
            }
            let blk = self.audio_block(a, &mut st, k)?;
            for (c, dst) in src.iter_mut().enumerate() {
                let Some(chan) = blk.get(c) else { continue };
                for (i, v) in chan.iter().enumerate() {
                    let pos = b_start + i as i64 - s0;
                    if let Some(d) = usize::try_from(pos).ok().and_then(|p| dst.get_mut(p)) {
                        *d = *v;
                    }
                }
            }
            k += 1;
        }
        drop(st);
        let frac = s0 as f64 - start as f64 * ratio;
        let mut out = AudioBuffer::silence(sample_rate, ch, frames);
        for (c, o) in out.channels.iter_mut().enumerate() {
            let Some(s) = src.get(c) else { continue };
            for (k, d) in o.iter_mut().enumerate() {
                let pos = k as f64 * ratio - frac;
                let i0 = pos.floor().max(0.0) as usize;
                let f = (pos - i0 as f64) as f32;
                let x = s.get(i0).copied().unwrap_or(0.0);
                let y = s.get(i0 + 1).copied().unwrap_or(x);
                *d = x + (y - x) * f;
            }
        }
        Ok(out)
    }
}

/// [`video::Decoder`] as a [`VideoDecoder`].
pub struct BinkDecoder {
    dec: video::Decoder,
    tag: [u8; 4],
    width: u32,
    height: u32,
    flags: u32,
    rate: FrameRate,
}

impl BinkDecoder {
    fn new(s: &VStream) -> Result<Self> {
        let dec = video::Decoder::new(s.width, s.height, s.tag, s.flags).map_err(|e| CodecError::Unsupported(format!("Bink: {e}")))?;
        Ok(Self { dec, tag: s.tag, width: s.width, height: s.height, flags: s.flags, rate: s.rate })
    }
}

impl VideoDecoder for BinkDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let p = self.dec.decode(sample).map_err(|e| CodecError::Decode(format!("Bink: {e}")))?;
        let mut planes = p.planes.into_iter();
        let mut next = || planes.next().map(Arc::new).ok_or_else(|| CodecError::Decode("Bink picture without planes".into()));
        let (y, u, v) = (next()?, next()?, next()?);
        let alpha = planes.next().map(Arc::new);
        let color = crate::stream_color::resolve(
            p.width,
            p.height,
            &[crate::stream_color::ColorCodes { primaries: 2, transfer: 2, matrix: 2, full_range: Some(self.tag[3] == b'k') }],
        );
        let frame = VideoFrame {
            width: p.width,
            height: p.height,
            data: PixelData::Yuv8 { planes: [y, u, v], chroma: Chroma::C420, alpha },
            color,
            par: (1, 1),
            pts: Tick::from_rational(pts, self.rate.den, self.rate.num),
            layout: Default::default(),
        };
        Ok(vec![DecodedFrame { pts, frame, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        if let Ok(d) = video::Decoder::new(self.width, self.height, self.tag, self.flags) {
            self.dec = d;
        }
    }
    fn name(&self) -> &str {
        "CFA4 Bink"
    }
}

struct BinkVideo<'a>(&'a BinkSource);

impl VideoSamples for BinkVideo<'_> {
    fn count(&self) -> usize {
        self.0.video.packets.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        let k = &self.0.video.keys;
        (0..=i.min(k.len().saturating_sub(1))).rev().find(|&j| k.get(j).copied().unwrap_or(false)).unwrap_or(0)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.video.packets.len().checked_sub(1)?;
        Some(usize::try_from(t.max(0)).ok()?.min(last))
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let r = self.0.video.packets.get(i).ok_or_else(|| container_error("frame index out of range"))?;
        self.0.bytes.get(r.clone()).map(<[u8]>::to_vec).ok_or_else(|| container_error("frame beyond the end of the file"))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(BinkDecoder::new(&self.0.video)?))
    }
}

impl MediaSource for BinkSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let v = &self.video;
        let t = req.time.max(Tick::ZERO);
        let target = v.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| v.rate.frame_at(t - m));
        Ok(v.cache.frame_late(&BinkVideo(self), target, late)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        self.read_audio(start, frames, sample_rate)
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(BinkSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(!sniff(b"KB2i\0\0\0\0"));
        let mut head = vec![0u8; 64];
        head[..4].copy_from_slice(b"BIKi");
        // the declared file size (992 + 8) is far beyond the 64 bytes present
        head[4..8].copy_from_slice(&992u32.to_le_bytes());
        head[8..12].copy_from_slice(&1u32.to_le_bytes());
        head[20..24].copy_from_slice(&16u32.to_le_bytes());
        head[24..28].copy_from_slice(&16u32.to_le_bytes());
        head[28..32].copy_from_slice(&30u32.to_le_bytes());
        head[32..36].copy_from_slice(&1u32.to_le_bytes());
        assert!(sniff(&head));
        // a frame table that points beyond the data is an error, not a panic
        assert!(opener("x.bik", Arc::from(&head[..])).is_some_and(|r| r.is_err()));
    }
}
