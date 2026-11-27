//! Smacker files (`.smk`, SMK2 and SMK4) with their palette video and up to seven audio tracks:
//! `cfa4-smacker` parses and decodes (backlog CB-24.2).
//!
//! Every frame is coded against the previous picture (blocks can be left as they were) and the
//! palette is cumulative, so a frame depends on every frame before it. The decoder keeps a
//! snapshot of its picture and palette every few hundred frames; a jump goes back to the nearest
//! snapshot before the frame and decodes forward, so every read equals playback from the start.
//! A frame whose video cannot be decoded shows the blocks decoded before the damage on the
//! previous picture (ffmpeg drops the frame); a frame without video repeats the picture.
//!
//! Audio chunks stand alone, one per frame and track. A track's timeline is the samples its
//! chunks hold, back to back; a chunk that cannot be decoded is silence of its length. Bink audio
//! tracks (rare) are left out.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use cfa4_frame::{AudioBuffer, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_smacker::{Smk, Snapshot, Track, TrackKind, decode_audio};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Frames between the points a restart can begin at.
const SYNC_EVERY: usize = 32;
/// Most decoder snapshots kept for a file, and the memory they may take.
const MAX_SNAPSHOTS: usize = 256;
const SNAPSHOT_BUDGET: usize = 48 << 20;
/// Decoded audio chunks kept per track.
const AUDIO_CACHE_CHUNKS: usize = 64;
/// Bytes of decoded audio kept per track (a chunk bigger than this is not kept at all).
const AUDIO_CACHE_BYTES: usize = 16 << 20;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_smacker::probe(head)
}

fn container(e: impl std::fmt::Display) -> CodecError {
    CodecError::Container(format!("Smacker: {e}"))
}

/// What the video decoders of one file share: the file, and the snapshots any of them took.
struct Shared {
    bytes: Arc<[u8]>,
    smk: Smk,
    rate: FrameRate,
    par: (u32, u32),
    /// Frames between snapshots.
    every: usize,
    /// Bytes one snapshot takes.
    per_snapshot: usize,
    snapshots: Mutex<BTreeMap<usize, Snapshot>>,
}

impl Shared {
    /// Decodes frame `k` onto `dec` (the picture after it is what the frame shows).
    fn step(&self, dec: &mut cfa4_smacker::Decoder, k: usize, frame: &[u8]) {
        let parts = self.smk.split_frame(k, frame);
        match (parts.video, parts.palette) {
            (Some(v), _) => {
                // a damaged frame keeps the blocks decoded before the damage
                let _ = dec.decode(parts.palette, v);
            }
            (None, Some(p)) => {
                let _ = dec.update_palette(p);
            }
            (None, None) => {}
        }
    }
}

/// The video decoder of one file.
struct SmkDecoder {
    shared: Arc<Shared>,
    dec: cfa4_smacker::Decoder,
    /// The frame the picture is positioned before (`usize::MAX`: unknown).
    next: usize,
}

impl SmkDecoder {
    fn new(shared: Arc<Shared>) -> Result<Self> {
        let dec = cfa4_smacker::Decoder::new(&shared.smk).map_err(|e| CodecError::Unsupported(format!("Smacker video: {e}")))?;
        Ok(Self { shared, dec, next: usize::MAX })
    }

    /// Positions the picture before frame `k`: from the nearest snapshot at or before it.
    fn position(&mut self, k: usize) -> Result<()> {
        if self.next == k {
            return Ok(());
        }
        let from = {
            let snaps = self.shared.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            snaps.range(..=k).next_back().map(|(&j, s)| (j, s.clone()))
        };
        let (mut at, snap) = match from {
            Some((j, s)) => (j, Some(s)),
            None => (0, None),
        };
        match snap {
            Some(s) => self.dec.restore(&s),
            None => self.dec = cfa4_smacker::Decoder::new(&self.shared.smk).map_err(|e| CodecError::Unsupported(format!("Smacker video: {e}")))?,
        }
        while at < k {
            if cfa4_media::cancel::cancelled() {
                self.next = usize::MAX;
                return Err(CodecError::Cancelled);
            }
            self.advance(at)?;
            at += 1;
        }
        self.next = k;
        Ok(())
    }

    /// Decodes frame `k` from the file, saving a snapshot first when it is due.
    fn advance(&mut self, k: usize) -> Result<()> {
        self.snapshot_if_due(k);
        let frame = self.shared.smk.frame_bytes(&self.shared.bytes, k).ok_or_else(|| container("frame beyond the file"))?;
        self.shared.step(&mut self.dec, k, frame);
        Ok(())
    }

    fn snapshot_if_due(&self, k: usize) {
        if !k.is_multiple_of(self.shared.every) {
            return;
        }
        let mut snaps = self.shared.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        // the snapshots of a file stay within the byte budget (a picture too big for it is not
        // kept, and the frames are replayed from the start)
        let room = (snaps.len() + 1).saturating_mul(self.shared.per_snapshot) <= SNAPSHOT_BUDGET;
        if room && snaps.len() < MAX_SNAPSHOTS && !snaps.contains_key(&k) {
            snaps.insert(k, self.dec.snapshot());
        }
    }

    fn picture(&self, pts: i64) -> DecodedFrame {
        let (w, h) = (self.dec.width() as u32, self.dec.height() as u32);
        let color = crate::stream_color::resolve(w, h, &[]);
        let frame = VideoFrame {
            width: w,
            height: h,
            data: PixelData::Rgba8(Arc::new(self.dec.rgba())),
            color,
            par: self.shared.par,
            pts: Tick::from_rational(pts, self.shared.rate.den, self.shared.rate.num),
            layout: Default::default(),
        };
        DecodedFrame { pts, frame, draft: false }
    }
}

impl VideoDecoder for SmkDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let k = usize::try_from(pts).map_err(|_| container("negative frame number"))?;
        self.position(k)?;
        self.snapshot_if_due(k);
        self.shared.step(&mut self.dec, k, sample);
        self.next = k + 1;
        Ok(vec![self.picture(pts)])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.next = usize::MAX;
    }
    fn name(&self) -> &str {
        "CFA4 Smacker"
    }
    fn is_random_access(&self, _sample: &[u8]) -> Option<bool> {
        // a restart restores the state itself
        Some(true)
    }
}

/// The frame table the cache works from: it makes the decoders, which share the file.
struct SmkVideo(Arc<Shared>);

impl VideoSamples for SmkVideo {
    fn count(&self) -> usize {
        self.0.smk.video_frames()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        i - i % SYNC_EVERY
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.smk.video_frames().checked_sub(1)?;
        Some(usize::try_from(t.max(0)).ok()?.min(last))
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        self.0.smk.frame_bytes(&self.0.bytes, i).map(<[u8]>::to_vec).ok_or_else(|| container("frame beyond the file"))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(SmkDecoder::new(self.0.clone())?))
    }
}

struct AudioCache {
    chunks: HashMap<usize, Arc<Vec<Vec<f32>>>>,
    order: VecDeque<usize>,
    /// Bytes of the samples held.
    bytes: usize,
}

fn planes_bytes(planes: &[Vec<f32>]) -> usize {
    planes.iter().map(|p| p.len().saturating_mul(4)).sum()
}

/// One decodable audio track.
struct ATrack {
    slot: usize,
    track: Track,
    /// The frame each chunk is in.
    frames: Vec<usize>,
    /// First sample (per channel) of each chunk, and the total at the end.
    starts: Vec<i64>,
    cache: Mutex<AudioCache>,
}

pub struct SmkSource {
    info: MediaInfo,
    shared: Arc<Shared>,
    rate: FrameRate,
    cache: GopCache,
    audio: Vec<ATrack>,
}

impl SmkSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let smk = Smk::parse(&bytes).map_err(container)?;
        if smk.video_frames() == 0 {
            return Err(container(format!("{name}: no frames")));
        }
        // the decoder checks the header trees
        cfa4_smacker::Decoder::new(&smk).map_err(|e| CodecError::Unsupported(format!("{name}: Smacker video: {e}")))?;
        let (num, den) = smk.frame_rate();
        let rate = FrameRate::new(num, den);
        let par = smk.pixel_aspect();
        let (w, h) = (smk.width, smk.height);
        let per_snapshot = (w as usize * h as usize + 768).max(1);
        let max_snaps = (SNAPSHOT_BUDGET / per_snapshot).clamp(1, MAX_SNAPSHOTS);
        let every = smk.video_frames().div_ceil(max_snaps).max(128).div_ceil(SYNC_EVERY) * SYNC_EVERY;
        // the audio chunks of every decodable track
        let mut audio: Vec<ATrack> = smk
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(slot, t)| {
                t.filter(|t| t.kind != TrackKind::Bink).map(|track| ATrack {
                    slot,
                    track,
                    frames: Vec::new(),
                    starts: vec![0],
                    cache: Mutex::new(AudioCache { chunks: HashMap::new(), order: VecDeque::new(), bytes: 0 }),
                })
            })
            .collect();
        if !audio.is_empty() {
            for i in 0..smk.frames {
                let parts = smk.split(&bytes, i);
                for a in &mut audio {
                    if let Some(chunk) = parts.audio.get(a.slot).copied().flatten() {
                        let n = i64::try_from(cfa4_smacker::chunk_samples(&a.track, chunk)).unwrap_or(0);
                        let total = a.starts.last().copied().unwrap_or(0).saturating_add(n);
                        a.frames.push(i);
                        a.starts.push(total);
                    }
                }
            }
            // a track the media layer would refuse (its rate and channels) is left out
            audio.retain(|a| {
                a.starts.last().copied().unwrap_or(0) > 0 && cfa4_media::bounded_audio_dimensions(a.track.rate, u32::from(a.track.channels)).is_ok()
            });
        }
        let mut duration = rate.tick_of(smk.video_frames() as i64);
        let mut audio_streams = Vec::new();
        for a in &audio {
            let total = a.starts.last().copied().unwrap_or(0);
            duration = duration.max(Tick::from_rational(total, 1, i64::from(a.track.rate.max(1))));
            audio_streams.push(AudioStreamInfo {
                sample_rate: a.track.rate,
                channels: u32::from(a.track.channels),
                codec: match a.track.kind {
                    TrackKind::Packed => "Smacker Audio".into(),
                    _ => format!("PCM {}-bit", a.track.bits),
                },
                bits_per_sample: Some(u32::from(a.track.bits)),
                speakers: Vec::new(),
            });
        }
        let secs = u64::try_from(rate.tick_of(smk.video_frames() as i64).to_rational_round(1, 1)).ok().filter(|&s| s > 0);
        let video_info = VideoStreamInfo {
            width: w,
            height: h,
            frame_rate: rate,
            par,
            codec: if smk.v4 { "Smacker Video (SMK4)" } else { "Smacker Video (SMK2)" }.into(),
            pixel_format: "Palette 8-bit (256 colours), progressive".into(),
            color: crate::stream_color::resolve(w, h, &[]),
            has_alpha: false,
            bitrate: secs.map(|s| bytes.len() as u64 * 8 / s),
            hdr: None,
        };
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(video_info),
            audio_streams,
            container: "Smacker".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        let shared = Arc::new(Shared { bytes, smk, rate, par, every, per_snapshot, snapshots: Mutex::new(BTreeMap::new()) });
        Ok(Self { info, shared, rate, cache: GopCache::new(None), audio })
    }

    /// Decoded chunk `k` of `a` (planar), silence where it cannot be decoded.
    fn chunk(&self, a: &ATrack, k: usize) -> Arc<Vec<Vec<f32>>> {
        let mut cache = a.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.chunks.get(&k) {
            return c.clone();
        }
        let ch = usize::from(a.track.channels.max(1));
        let len = match (a.starts.get(k), a.starts.get(k + 1)) {
            (Some(x), Some(y)) => usize::try_from(y.saturating_sub(*x)).unwrap_or(0),
            _ => 0,
        };
        let decoded = a.frames.get(k).and_then(|&f| {
            let frame = self.shared.smk.frame_bytes(&self.shared.bytes, f)?;
            let chunk = self.shared.smk.split_frame(f, frame).audio.get(a.slot).copied().flatten()?;
            decode_audio(&a.track, chunk).ok()
        });
        // a chunk that cannot be decoded is silence: it takes no memory, the reader treats the
        // missing samples as zeros (the timeline still has its declared length)
        let mut planes: Vec<Vec<f32>> = vec![Vec::new(); ch];
        if let Some(s) = decoded {
            // the timeline holds `len` samples: a chunk that decodes to more or fewer is cut or
            // padded with silence
            let len = len.min(s.len().div_ceil(ch));
            for p in &mut planes {
                p.resize(len, 0.0);
            }
            for (i, v) in s.iter().enumerate() {
                if let Some(p) = planes.get_mut(i % ch).and_then(|p| p.get_mut(i / ch)) {
                    *p = a.track.to_f32(*v);
                }
            }
        }
        let bytes = planes_bytes(&planes);
        let c = Arc::new(planes);
        if bytes <= AUDIO_CACHE_BYTES {
            cache.chunks.insert(k, c.clone());
            cache.order.push_back(k);
            cache.bytes += bytes;
            while cache.order.len() > AUDIO_CACHE_CHUNKS || cache.bytes > AUDIO_CACHE_BYTES {
                let Some(old) = cache.order.pop_front() else { break };
                if let Some(o) = cache.chunks.remove(&old) {
                    cache.bytes = cache.bytes.saturating_sub(planes_bytes(&o));
                }
            }
        }
        c
    }

    fn read_audio(&self, a: &ATrack, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        let rate = a.track.rate.max(1);
        let ch = usize::from(a.track.channels.max(1));
        let source_frames = cfa4_media::resample_source_frames(frames, rate, sample_rate)?;
        let need = source_frames.checked_add(2).ok_or_else(|| MediaError::Decode("audio source window overflows".into()))?;
        cfa4_media::validate_audio_buffer_shape(ch, need)?;
        cfa4_media::validate_audio_buffer_shape(ch, frames)?;
        let ratio = f64::from(rate) / f64::from(sample_rate.max(1));
        let s0 = (start as f64 * ratio).floor() as i64;
        let need_i64 = i64::try_from(need).unwrap_or(i64::MAX);
        let mut src: Vec<Vec<f32>> = vec![vec![0.0; need]; ch];
        let mut k = a.starts.partition_point(|&b| b <= s0.max(0)).saturating_sub(1);
        while k < a.frames.len() {
            let b_start = a.starts.get(k).copied().unwrap_or(i64::MAX);
            if b_start >= s0.saturating_add(need_i64) {
                break;
            }
            let blk = self.chunk(a, k);
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

impl MediaSource for SmkSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let t = req.time.max(Tick::ZERO);
        let target = self.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| self.rate.frame_at(t - m));
        Ok(self.cache.frame_late(&SmkVideo(self.shared.clone()), target, late)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        self.audio_stream(0, start, frames, sample_rate)
    }

    fn audio_stream(&self, stream: usize, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        let a = self.audio.get(stream).ok_or(MediaError::NoStream("audio"))?;
        self.read_audio(a, start, frames, sample_rate)
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(SmkSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(sniff(b"SMK2"));
        assert!(opener("x.smk", Arc::from(&b"SMK2\0\0\0"[..])).is_some_and(|r| r.is_err()));
        assert!(opener("x.smk", Arc::from(&b"SMK4"[..])).is_some_and(|r| r.is_err()));
    }
}
