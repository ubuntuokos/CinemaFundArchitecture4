//! RealMedia files (`.rm`, `.rmvb`) with RealVideo 1.0 / 2.0 / 3.0 / 4.0 video and Cook, ATRAC3,
//! SIPR or RealAudio 1.0 / 2.0 audio: `cfa4-realmedia` demuxes, our `cfa4-realvideo` and
//! `cfa4-realaudio` decode (CB-13.1, CB-13.2, CB-14.1, CB-14.2, CB-15.1).
//!
//! RealMedia stamps packets in decode order, so the timeline is the frame count at the stream's
//! frame rate: at open every video frame is located (file offset of its first data packet) and
//! its display position worked out from the picture types (a B-frame is shown before the
//! reference picture decoded just before it).
//!
//! Audio is indexed by interleaving superblock (the data packets one deinterleaving pass takes;
//! one data packet for RealAudio 1.0); its timeline is the frame count times the frame length,
//! with the two frames Cook needs to start up as silence (as in ffmpeg). The decoders carry state
//! through the whole stream (Cook's noise generator, the CELP codecs' filter memories), so a jump
//! restores the nearest earlier decoder checkpoint and decodes forward: every read equals
//! playback from the start. AAC audio is not decoded yet.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_realaudio::{atrac3, cook, ra144, ra288, sipr};
use cfa4_realmedia::audio::{AudioInfo, Deinterleaver, parse_audio_info};
use cfa4_realmedia::{Demuxer, Header, StreamKind};
use cfa4_realvideo::{PictType, Picture, rv34};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Most video frames indexed (a day at 30 fps).
const MAX_FRAMES: usize = 30 * 60 * 60 * 24;
/// Most audio superblocks indexed (a day of 44.1 kHz Cook in the smallest superblocks; about
/// three days of RealAudio 1.0).
const MAX_BLOCKS: usize = 24 * 60 * 60 * 44_100 / 256;
/// Audio superblocks between decoder checkpoints.
const CHECKPOINT_BLOCKS: usize = 32;
/// Decoded audio superblocks kept.
const AUDIO_CACHE_BLOCKS: usize = 48;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_realmedia::probe(head)
}

/// The RealVideo versions CFA4 decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    /// RealVideo 1.0 / 2.0 (H.263 based).
    Rv12(cfa4_realvideo::Codec),
    /// RealVideo 3.0 / 4.0 (H.264 like).
    Rv34(rv34::Codec),
}

fn codec_of(fourcc: &[u8; 4]) -> Option<Codec> {
    match fourcc {
        b"RV10" => Some(Codec::Rv12(cfa4_realvideo::Codec::Rv10)),
        b"RV20" => Some(Codec::Rv12(cfa4_realvideo::Codec::Rv20)),
        b"RV30" => Some(Codec::Rv34(rv34::Codec::Rv30)),
        b"RV40" => Some(Codec::Rv34(rv34::Codec::Rv40)),
        _ => None,
    }
}

fn label(codec: Codec) -> &'static str {
    match codec {
        Codec::Rv12(cfa4_realvideo::Codec::Rv10) => "RealVideo 1.0",
        Codec::Rv12(cfa4_realvideo::Codec::Rv20) => "RealVideo 2.0",
        Codec::Rv34(rv34::Codec::Rv30) => "RealVideo 3.0",
        Codec::Rv34(rv34::Codec::Rv40) => "RealVideo 4.0",
    }
}

fn picture_type(codec: Codec, pkt: &[u8]) -> Option<PictType> {
    match codec {
        Codec::Rv12(c) => cfa4_realvideo::picture_type(c, pkt),
        Codec::Rv34(c) => rv34::picture_type(c, pkt),
    }
}

/// Either decoder.
enum AnyDecoder {
    Rv12(Box<cfa4_realvideo::Decoder>),
    Rv34(Box<rv34::Decoder>),
}

impl AnyDecoder {
    fn new(codec: Codec, width: u32, height: u32, extradata: &[u8]) -> std::result::Result<Self, cfa4_realvideo::Error> {
        Ok(match codec {
            Codec::Rv12(c) => Self::Rv12(Box::new(cfa4_realvideo::Decoder::new(c, width, height, extradata)?)),
            Codec::Rv34(c) => Self::Rv34(Box::new(rv34::Decoder::new(c, width, height, extradata)?)),
        })
    }
    fn decode(&mut self, pkt: &[u8]) -> std::result::Result<Option<Picture>, cfa4_realvideo::Error> {
        match self {
            Self::Rv12(d) => d.decode(pkt),
            Self::Rv34(d) => d.decode(pkt),
        }
    }
    fn flush(&mut self) -> Option<Picture> {
        match self {
            Self::Rv12(d) => d.flush(),
            Self::Rv34(d) => d.flush(),
        }
    }
    fn frames_started(&self) -> u64 {
        match self {
            Self::Rv12(d) => d.frames_started(),
            Self::Rv34(d) => d.frames_started(),
        }
    }
}

/// One video frame of the file, in decode order.
#[derive(Clone, Copy, Debug)]
struct Sample {
    /// File offset of the data packet the frame starts in.
    pos: u64,
    /// Which video frame of the stream starting in that packet this is (several small frames can
    /// share a packet).
    nth: u32,
    /// Display position (frame number).
    pts: i64,
    pict: PictType,
}

/// The video stream of a file.
struct VStream {
    stream: usize,
    codec: Codec,
    width: u32,
    height: u32,
    extradata: Vec<u8>,
    rate: FrameRate,
    samples: Vec<Sample>,
    /// Sample index of each display position.
    by_pts: Vec<usize>,
    cache: GopCache,
}

/// The RealAudio decoders.
#[derive(Clone)]
enum AudioDecoder {
    Cook(Box<cook::Decoder>),
    Ra144(Box<ra144::Decoder>),
    Ra288(Box<ra288::Decoder>),
    Sipr(Box<sipr::Decoder>),
    Atrac3(Box<atrac3::Decoder>),
}

impl AudioDecoder {
    /// The decoder for a stream, or why it cannot be decoded.
    fn new(info: &AudioInfo) -> std::result::Result<Self, String> {
        let label = match &info.fourcc {
            b"cook" => {
                let p = cook::Params {
                    channels: info.channels,
                    sample_rate: info.sample_rate,
                    block_align: info.block_align as usize,
                    extradata: info.extradata.clone(),
                };
                return cook::Decoder::new(&p).map(|d| Self::Cook(Box::new(d))).map_err(|e| format!("Cook: {e}"));
            }
            b"lpcJ" => return Ok(Self::Ra144(Box::default())),
            b"28_8" if info.block_align as usize == ra288::FRAME_SIZE => return Ok(Self::Ra288(Box::default())),
            b"28_8" => return Err("RealAudio 2.0 (28.8): unsupported frame size".into()),
            b"sipr" => {
                return sipr::Decoder::new(info.block_align as usize).map(|d| Self::Sipr(Box::new(d))).map_err(|e| format!("RealAudio SIPR: {e}"));
            }
            b"atrc" => {
                let p = atrac3::Params { channels: info.channels, block_align: info.block_align as usize, extradata: info.extradata.clone() };
                return atrac3::Decoder::new(&p).map(|d| Self::Atrac3(Box::new(d))).map_err(|e| format!("ATRAC3: {e}"));
            }
            b"raac" | b"racp" => "AAC".to_string(),
            f => format!("RealAudio {}", String::from_utf8_lossy(f)),
        };
        Err(format!("{label} audio is not decoded yet"))
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Cook(_) => "Cook",
            Self::Ra144(_) => "RealAudio 1.0 (14.4)",
            Self::Ra288(_) => "RealAudio 2.0 (28.8)",
            Self::Sipr(_) => "RealAudio SIPR",
            Self::Atrac3(_) => "ATRAC3",
        }
    }

    fn channels(&self) -> usize {
        match self {
            Self::Cook(d) => d.channels(),
            Self::Atrac3(d) => d.channels(),
            _ => 1,
        }
    }

    /// Bytes and samples per channel of one codec frame (Cook: a whole deinterleaved frame).
    fn frame(&self) -> (usize, usize) {
        match self {
            Self::Cook(d) => (usize::MAX, d.samples_per_channel()),
            Self::Ra144(_) => (ra144::FRAME_SIZE, ra144::FRAME_SAMPLES),
            Self::Ra288(_) => (ra288::FRAME_SIZE, ra288::FRAME_SAMPLES),
            Self::Sipr(d) => (d.frame_size(), d.frame_samples()),
            Self::Atrac3(d) => (d.block_align(), d.samples_per_channel()),
        }
    }

    /// Samples per channel that `len` bytes out of the deinterleaver decode to.
    fn samples_for(&self, len: usize) -> usize {
        match self.frame() {
            (usize::MAX, spc) => spc,
            (fb, spc) => (len / fb.max(1)).saturating_mul(spc),
        }
    }

    /// Decodes `data` (one frame out of the deinterleaver) onto the end of `out`; a damaged
    /// frame and Cook's start-up frames give silence.
    fn decode_into(&mut self, data: &[u8], out: &mut [Vec<f32>]) {
        let (fb, spc) = self.frame();
        let silence = |out: &mut [Vec<f32>]| {
            for o in out.iter_mut() {
                o.resize(o.len() + spc, 0.0);
            }
        };
        if let Self::Cook(d) = self {
            match d.decode(data) {
                Ok(Some(fr)) => {
                    for (c, o) in out.iter_mut().enumerate() {
                        match fr.channels.get(c) {
                            Some(s) => o.extend_from_slice(s),
                            None => o.resize(o.len() + spc, 0.0),
                        }
                    }
                }
                _ => silence(out),
            }
            return;
        }
        if let Self::Atrac3(d) = self {
            for f in data.chunks_exact(fb.max(1)) {
                match d.decode(f) {
                    Ok(p) => {
                        for (c, o) in out.iter_mut().enumerate() {
                            match p.get(c) {
                                Some(s) => o.extend_from_slice(s),
                                None => o.resize(o.len() + spc, 0.0),
                            }
                        }
                    }
                    Err(_) => silence(out),
                }
            }
            return;
        }
        let Some(o) = out.first_mut() else { return };
        for f in data.chunks_exact(fb.max(1)) {
            match self {
                Self::Ra144(d) => match d.decode(f) {
                    Ok(s) => o.extend(s.iter().map(|&v| f32::from(v) / 32768.0)),
                    Err(_) => o.resize(o.len() + spc, 0.0),
                },
                Self::Ra288(d) => match d.decode(f) {
                    Ok(s) => o.extend_from_slice(&s),
                    Err(_) => o.resize(o.len() + spc, 0.0),
                },
                Self::Sipr(d) => match d.decode(f) {
                    Ok(s) => o.extend_from_slice(&s),
                    Err(_) => o.resize(o.len() + spc, 0.0),
                },
                Self::Cook(_) | Self::Atrac3(_) => {}
            }
        }
    }
}

struct AudioState {
    /// The decoder and the superblock it decodes next.
    dec: AudioDecoder,
    next: usize,
    checkpoints: BTreeMap<usize, AudioDecoder>,
    blocks: HashMap<usize, Arc<Vec<Vec<f32>>>>,
    order: VecDeque<usize>,
}

/// The audio stream of a file.
struct AStream {
    stream: usize,
    info: AudioInfo,
    sample_rate: u32,
    channels: usize,
    /// File offset of the data packet each complete superblock starts in.
    blocks: Vec<u64>,
    /// First sample (per channel) of each superblock, and the total at the end.
    starts: Vec<i64>,
    state: Mutex<AudioState>,
}

impl AStream {
    /// Samples per channel of superblock `k`.
    fn block_len(&self, k: usize) -> usize {
        match (self.starts.get(k), self.starts.get(k + 1)) {
            (Some(a), Some(b)) => usize::try_from(b.saturating_sub(*a)).unwrap_or(0),
            _ => 0,
        }
    }
}

pub struct RmSource {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    header: Header,
    video: Option<VStream>,
    audio: Option<AStream>,
}

fn container(e: impl std::fmt::Display) -> CodecError {
    CodecError::Container(format!("RealMedia: {e}"))
}

/// Display positions for pictures in decode order: a B-frame is shown at once, a reference
/// picture when the next reference picture arrives (or at the end).
fn display_order(types: &[PictType]) -> Vec<i64> {
    let mut pts = vec![0i64; types.len()];
    let mut next = 0i64;
    let mut held: Option<usize> = None;
    for (i, t) in types.iter().enumerate() {
        if *t == PictType::B {
            if let Some(p) = pts.get_mut(i) {
                *p = next;
            }
            next += 1;
        } else {
            if let Some(h) = held.and_then(|h| pts.get_mut(h)) {
                *h = next;
                next += 1;
            }
            held = Some(i);
        }
    }
    if let Some(h) = held.and_then(|h| pts.get_mut(h)) {
        *h = next;
    }
    pts
}

impl RmSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let mut dm = Demuxer::new(Cursor::new(bytes.clone())).map_err(container)?;
        let header = dm.header().clone();
        let found = header.streams.iter().enumerate().find_map(|(i, s)| match &s.kind {
            StreamKind::Video { fourcc, width, height, fps_16_16, extradata } => Some((i, *fourcc, *width, *height, *fps_16_16, extradata.clone())),
            _ => None,
        });
        let mut vsetup = None;
        if let Some((stream, fourcc, width, height, fps, extradata)) = found {
            let Some(codec) = codec_of(&fourcc) else {
                return Err(CodecError::Unsupported(format!(
                    "{name}: RealMedia video {} (CFA4 decodes RealVideo 1.0 to 4.0)",
                    String::from_utf8_lossy(&fourcc)
                )));
            };
            // the decoder checks the stream parameters
            AnyDecoder::new(codec, u32::from(width), u32::from(height), &extradata)
                .map_err(|e| CodecError::Unsupported(format!("{name}: {}: {e}", label(codec))))?;
            vsetup = Some((stream, codec, width, height, fps, extradata));
        }
        // the first audio stream
        let mut asetup = None;
        let mut audio_refusal = None;
        if let Some((i, info)) = header.streams.iter().enumerate().find_map(|(i, s)| match &s.kind {
            StreamKind::Audio { codec_data } => Some((i, parse_audio_info(codec_data))),
            _ => None,
        }) {
            match info.map_err(|e| e.to_string()).and_then(|a| AudioDecoder::new(&a).map(|d| (a, d))) {
                Ok((a, d)) => asetup = Some((i, a, d)),
                Err(e) => audio_refusal = Some(e),
            }
        }
        if vsetup.is_none() && asetup.is_none() {
            let why = audio_refusal.unwrap_or_else(|| "no video or audio stream".into());
            return Err(CodecError::Unsupported(format!("{name}: RealMedia: {why}")));
        }
        let vstream = vsetup.as_ref().map(|v| v.0);
        let vcodec = vsetup.as_ref().map(|v| v.1);
        let astream = asetup.as_ref().map(|a| a.0);
        let sub_h = asetup.as_ref().map_or(0, |a| usize::from(a.1.sub_packet_h));
        let blocked = asetup.as_ref().is_some_and(|a| {
            matches!(
                a.1.interleaver,
                cfa4_realmedia::audio::Interleaver::Genr | cfa4_realmedia::audio::Interleaver::Int4 | cfa4_realmedia::audio::Interleaver::Sipr
            )
        });
        let mut types = Vec::new();
        let mut places = Vec::new();
        let mut last_pos = u64::MAX;
        let mut nth = 0u32;
        let mut blocks = Vec::new();
        // samples per channel of each Int0 packet (blocked streams have a fixed superblock)
        let mut packet_samples = Vec::new();
        let (mut fill, mut block_pos) = (0usize, 0u64);
        while places.len() < MAX_FRAMES && blocks.len() < MAX_BLOCKS {
            let p = match dm.next_packet() {
                Ok(Some(p)) => p,
                // a damaged tail ends the index
                Ok(None) | Err(_) => break,
            };
            if Some(p.stream) == astream {
                // the deinterleaver's sub-packet count: a superblock counts once it is complete
                if !blocked {
                    blocks.push(p.pos);
                    packet_samples.push(asetup.as_ref().map_or(0, |a| a.2.samples_for(p.data.len())));
                    continue;
                }
                if p.keyframe {
                    fill = 0;
                }
                if fill == 0 {
                    block_pos = p.pos;
                }
                fill += 1;
                if fill >= sub_h {
                    fill = 0;
                    blocks.push(block_pos);
                }
                continue;
            }
            if Some(p.stream) != vstream || p.partial {
                continue;
            }
            let Some(codec) = vcodec else { continue };
            let Some(t) = picture_type(codec, &p.data) else { continue };
            nth = if p.pos == last_pos { nth.saturating_add(1) } else { 0 };
            last_pos = p.pos;
            places.push((p.pos, nth));
            types.push(t);
        }
        let mut duration = Tick::ZERO;
        let mut video = None;
        let mut video_info = None;
        if let Some((stream, codec, width, height, fps, extradata)) = vsetup {
            if places.is_empty() {
                return Err(container(format!("{name}: no video frames")));
            }
            let pts = display_order(&types);
            let samples: Vec<Sample> = places.iter().zip(&types).zip(&pts).map(|((&(pos, nth), &pict), &pts)| Sample { pos, nth, pts, pict }).collect();
            let mut by_pts = vec![0usize; samples.len()];
            for (i, s) in samples.iter().enumerate() {
                if let Some(slot) = usize::try_from(s.pts).ok().and_then(|p| by_pts.get_mut(p)) {
                    *slot = i;
                }
            }
            // 16.16 fixed point; RealProducer writes whole rates, keep it exact when it is one,
            // and take a standard rate the field approximates (29.97 is 0x1DF852) as that rate
            let rate = if fps == 0 {
                FrameRate::new(25, 1)
            } else if fps & 0xFFFF == 0 {
                FrameRate::new(i64::from(fps >> 16), 1)
            } else {
                let exact = FrameRate::new(i64::from(fps), 65536);
                FrameRate::COMMON.into_iter().find(|r| (r.as_f64() - exact.as_f64()).abs() < 0.005).unwrap_or(exact)
            };
            // the start of the frame after the last, so every frame lies inside the duration
            duration = rate.tick_of(samples.len() as i64);
            let color = crate::stream_color::resolve(u32::from(width), u32::from(height), &[]);
            let bitrate = u64::try_from(duration.to_rational_round(1, 1)).ok().filter(|&s| s > 0).map(|s| bytes.len() as u64 * 8 / s);
            video_info = Some(VideoStreamInfo {
                width: u32::from(width),
                height: u32::from(height),
                frame_rate: rate,
                par: (1, 1),
                codec: label(codec).into(),
                pixel_format: "YUV 4:2:0 8-bit, progressive".into(),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            });
            video = Some(VStream {
                stream,
                codec,
                width: u32::from(width),
                height: u32::from(height),
                extradata,
                rate,
                samples,
                by_pts,
                cache: GopCache::new(None),
            });
        }
        let mut audio = None;
        let mut audio_streams = Vec::new();
        if let Some((stream, info, dec)) = asetup
            && !blocks.is_empty()
        {
            let mut starts = Vec::with_capacity(blocks.len() + 1);
            let mut total = 0i64;
            starts.push(0);
            for k in 0..blocks.len() {
                let n = if blocked {
                    let frames = (usize::from(info.sub_packet_h) * info.audio_framesize as usize).checked_div(info.block_align as usize).unwrap_or(0);
                    frames.saturating_mul(dec.samples_for(info.block_align as usize))
                } else {
                    packet_samples.get(k).copied().unwrap_or(0)
                };
                total = total.saturating_add(i64::try_from(n).unwrap_or(i64::MAX));
                starts.push(total);
            }
            let channels = dec.channels();
            duration = duration.max(Tick::from_rational(total, 1, i64::from(info.sample_rate.max(1))));
            audio_streams.push(AudioStreamInfo {
                sample_rate: info.sample_rate,
                channels: channels as u32,
                codec: dec.label().into(),
                bits_per_sample: None,
                speakers: Vec::new(),
            });
            audio = Some(AStream {
                stream,
                sample_rate: info.sample_rate,
                channels,
                blocks,
                starts,
                state: Mutex::new(AudioState {
                    dec: dec.clone(),
                    next: 0,
                    checkpoints: BTreeMap::from([(0, dec)]),
                    blocks: HashMap::new(),
                    order: VecDeque::new(),
                }),
                info,
            });
        }
        if video.is_none() && audio.is_none() {
            return Err(container(format!("{name}: no audio frames")));
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: if video.is_some() { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration,
            video: video_info,
            audio_streams,
            container: "RealMedia".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, bytes, header, video, audio })
    }

    fn read(&self, v: &VStream, i: usize) -> Result<Vec<u8>> {
        let s = v.samples.get(i).ok_or_else(|| container("frame index out of range"))?;
        let mut dm = Demuxer::from_header(Cursor::new(self.bytes.clone()), self.header.clone()).map_err(container)?;
        dm.seek(s.pos).map_err(container)?;
        let mut seen = 0u32;
        // bounded: the frame starts in the packet at `pos`
        for _ in 0..4096 {
            let Some(p) = dm.next_packet().map_err(container)? else { break };
            if p.stream != v.stream || p.partial || p.pos != s.pos {
                continue;
            }
            if seen == s.nth {
                return Ok(p.data);
            }
            seen += 1;
        }
        Err(container("video frame not found"))
    }

    /// The codec frames of audio superblock `k`.
    fn audio_frames(&self, a: &AStream, k: usize) -> Result<Vec<Vec<u8>>> {
        let pos = *a.blocks.get(k).ok_or_else(|| container("audio block out of range"))?;
        let mut dm = Demuxer::from_header(Cursor::new(self.bytes.clone()), self.header.clone()).map_err(container)?;
        dm.seek(pos).map_err(container)?;
        let mut di = Deinterleaver::new(&a.info);
        // bounded: a superblock is at most a few hundred data packets
        for _ in 0..65_536 {
            let Some(p) = dm.next_packet().map_err(container)? else { break };
            if p.stream != a.stream {
                continue;
            }
            let frames = di.push(&p);
            if !frames.is_empty() {
                return Ok(frames.into_iter().map(|f| f.data).collect());
            }
        }
        Err(container("audio block not found"))
    }

    /// Decoded superblock `k` (planar), decoding forward from the nearest checkpoint.
    fn audio_block(&self, a: &AStream, st: &mut AudioState, k: usize) -> Result<Arc<Vec<Vec<f32>>>> {
        if let Some(b) = st.blocks.get(&k) {
            return Ok(b.clone());
        }
        // restore a checkpoint unless the decoder is already between it and `k`
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
            let len = a.block_len(n);
            let mut out = vec![Vec::with_capacity(len); a.channels];
            // a damaged block decodes to silence; the decoder state goes on as ffmpeg's does
            let frames = self.audio_frames(a, n).unwrap_or_default();
            for f in &frames {
                st.dec.decode_into(f, &mut out);
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
        st.blocks.get(&k).cloned().ok_or_else(|| container("audio block not decoded"))
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
        // the superblock holding `s0`
        let mut k = a.starts.partition_point(|&b| b <= s0.max(0)).saturating_sub(1);
        while k < a.blocks.len() {
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

/// [`cfa4_realvideo::Decoder`] or [`rv34::Decoder`] as a [`VideoDecoder`]: pictures come out in display order and
/// are matched to their samples by decode index.
pub struct RealVideoDecoder {
    dec: AnyDecoder,
    codec: Codec,
    width: u32,
    height: u32,
    extradata: Vec<u8>,
    rate: FrameRate,
    pts: HashMap<u64, i64>,
}

impl RealVideoDecoder {
    fn new(codec: Codec, width: u32, height: u32, extradata: &[u8], rate: FrameRate) -> Result<Self> {
        let dec = AnyDecoder::new(codec, width, height, extradata).map_err(|e| CodecError::Unsupported(format!("{}: {e}", label(codec))))?;
        Ok(Self { dec, codec, width, height, extradata: extradata.to_vec(), rate, pts: HashMap::new() })
    }

    fn frame(&mut self, p: Picture) -> Option<DecodedFrame> {
        let pts = self.pts.remove(&p.decode_index)?;
        let color = crate::stream_color::resolve(p.width, p.height, &[]);
        let data = PixelData::Yuv8 { planes: [Arc::new(p.y), Arc::new(p.u), Arc::new(p.v)], chroma: Chroma::C420, alpha: None };
        let frame = VideoFrame {
            width: p.width,
            height: p.height,
            data,
            color,
            par: (1, 1),
            pts: Tick::from_rational(pts, self.rate.den, self.rate.num),
            layout: Default::default(),
        };
        Some(DecodedFrame { pts, frame, draft: false })
    }
}

impl VideoDecoder for RealVideoDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let started = self.dec.frames_started();
        let r = self.dec.decode(sample);
        if self.dec.frames_started() > started {
            self.pts.insert(started, pts);
        }
        // a stale entry (a frame that never came out) must not grow the map
        if self.pts.len() > 16 {
            let keep = self.dec.frames_started().saturating_sub(16);
            self.pts.retain(|&k, _| k >= keep);
        }
        match r {
            Ok(Some(p)) => Ok(self.frame(p).into_iter().collect()),
            Ok(None) => Ok(Vec::new()),
            // a B-frame decoded right after the I-frame a decode started at refers to a picture
            // before it: nothing references a B-frame, so it is left out (as ffmpeg does)
            Err(_) if self.is_disposable(sample) => Ok(Vec::new()),
            Err(e) => Err(CodecError::Decode(format!("{}: {e}", label(self.codec)))),
        }
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.dec.flush().and_then(|p| self.frame(p)).into_iter().collect()
    }
    fn reset(&mut self) {
        if let Ok(d) = AnyDecoder::new(self.codec, self.width, self.height, &self.extradata) {
            self.dec = d;
        }
        self.pts.clear();
    }
    fn name(&self) -> &str {
        "CFA4 RealVideo"
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        picture_type(self.codec, sample).map(|t| t == PictType::I)
    }
    fn is_disposable(&self, sample: &[u8]) -> bool {
        picture_type(self.codec, sample) == Some(PictType::B)
    }
}

struct RmVideo<'a>(&'a RmSource, &'a VStream);

impl VideoSamples for RmVideo<'_> {
    fn count(&self) -> usize {
        self.1.samples.len()
    }
    fn pts(&self, i: usize) -> i64 {
        self.1.samples.get(i).map_or(0, |s| s.pts)
    }
    /// The I-frame at or before `i`; for a B-frame decoded right after that I-frame (it also
    /// needs the reference picture before it), the I-frame before that.
    fn sync_before(&self, i: usize) -> usize {
        let s = &self.1.samples;
        let is_i = |k: usize| s.get(k).is_some_and(|x| x.pict == PictType::I);
        let prev_i = |from: usize| (0..=from).rev().find(|&k| is_i(k)).unwrap_or(0);
        let sync = prev_i(i.min(s.len().saturating_sub(1)));
        let leading = s.get(i).is_some_and(|x| x.pict == PictType::B) && (sync + 1..i).all(|k| s.get(k).is_some_and(|x| x.pict == PictType::B));
        if leading && sync > 0 { prev_i(sync - 1) } else { sync }
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.1.by_pts.len().checked_sub(1)?;
        let t = usize::try_from(t.max(0)).ok()?.min(last);
        self.1.by_pts.get(t).copied()
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        self.0.read(self.1, i)
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        let s = self.1;
        Ok(Box::new(RealVideoDecoder::new(s.codec, s.width, s.height, &s.extradata, s.rate)?))
    }
}

impl MediaSource for RmSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let v = self.video.as_ref().ok_or(MediaError::NoStream("video"))?;
        let t = req.time.max(Tick::ZERO);
        let target = v.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| v.rate.frame_at(t - m));
        Ok(v.cache.frame_late(&RmVideo(self, v), target, late)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        self.read_audio(start, frames, sample_rate)
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(RmSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_order_moves_b_frames_before_their_reference() {
        use PictType::{B, I, P};
        assert_eq!(display_order(&[I, P, B, P, B, P]), vec![0, 2, 1, 4, 3, 5]);
        assert_eq!(display_order(&[I, P, P]), vec![0, 1, 2]);
        assert_eq!(display_order(&[]), Vec::<i64>::new());
    }

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(opener("x.rm", Arc::from(&b".RMF\0\0\0\x12\0\0"[..])).is_some_and(|r| r.is_err()));
    }
}
