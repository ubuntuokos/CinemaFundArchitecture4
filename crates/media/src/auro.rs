//! Auro-3D (#299): a source whose audio carries an Auro-Codec side channel plays the layout the
//! side channel restores (for instance 13.1 out of a 7.1 DTS-HD Master Audio or FLAC track)
//! instead of the carrier.
//!
//! [`wrap`] looks at the first samples of every audio stream of a newly opened source. A stream
//! of another declared bit depth than 24, or of a channel count no carrier uses, is not read at
//! all. Otherwise its first [`GATE_FRAMES`] samples must be exact 24-bit PCM (a lossy decode never
//! is), and its first [`SYNC_FRAMES`] must hold a block sync (sixteen samples in a row with bit 0
//! set) on some channel, so a plain track costs one or two short reads. A carrier is then
//! confirmed by `cfa4_auro`'s detector (sync, CRC and three agreeing blocks). The wrapped source then reports
//! the restored channel count and decodes every request from a carrier window that reaches one
//! block past both ends, so each block that touches the request is decoded whole.
//!
//! The restored channels are in [`cfa4_auro::cfa4::output_order`]: the bed in WAV order, then the
//! heights (HL HR HC HLs HRs), then the top.

use std::sync::Arc;

use cfa4_auro::block::{MAX_BLOCK, SYNC_SAMPLES};
use cfa4_auro::cfa4::{DETECT_FRAMES, MAX_CARRIERS, detect, output_order, pcm24_from_f32, unfold_with, wav_carrier_ids};
use cfa4_auro::{Detection, StreamId};

use crate::{AudioBuffer, FrameRequest, MediaError, MediaInfo, MediaSource, Result, SharedSource, VideoFrame};

/// Samples a stream must open with as exact 24-bit PCM to be probed further.
pub const GATE_FRAMES: usize = 256;

/// Samples that must hold a block sync: one of the largest blocks and its sync.
pub const SYNC_FRAMES: usize = MAX_BLOCK + SYNC_SAMPLES;

/// Samples the detector reads: four of the largest blocks.
const PROBE_FRAMES: usize = 4 * MAX_BLOCK + SYNC_SAMPLES;

/// Carrier samples read on each side of a request, so the blocks that touch it are whole.
const MARGIN: i64 = MAX_BLOCK as i64;

/// An Auro-3D carrier stream.
#[derive(Debug, Clone)]
struct Carrier {
    detection: Detection,
    rate: u32,
    channels: usize,
}

/// A source with at least one Auro-3D carrier among its audio streams.
pub struct AuroSource {
    inner: SharedSource,
    info: MediaInfo,
    /// Index-aligned with `info.audio_streams`.
    carriers: Vec<Option<Carrier>>,
}

/// `src` itself, or `src` wrapped in an [`AuroSource`] when one of its audio streams is an
/// Auro-3D carrier.
pub fn wrap(src: SharedSource) -> SharedSource {
    let info = src.info();
    let carriers: Vec<Option<Carrier>> = (0..info.audio_streams.len()).map(|k| probe(&*src, k)).collect();
    if carriers.iter().all(Option::is_none) {
        return src;
    }
    let mut info = info.clone();
    for (a, c) in info.audio_streams.iter_mut().zip(&carriers) {
        let Some(c) = c else { continue };
        let Some(speakers) = output_order(c.detection.original) else { continue };
        a.channels = speakers.len() as u32;
        a.speakers = speakers.iter().map(|s| s.name().to_string()).collect();
        let name = c.detection.original.auro_name().or(c.detection.original.name()).unwrap_or("?");
        a.codec = format!("{} + Auro-3D {name}", a.codec);
        a.bits_per_sample = Some(24);
    }
    Arc::new(AuroSource { inner: src, info, carriers })
}

/// Stream `k` of `src` as an Auro-3D carrier, when it is one.
fn probe(src: &dyn MediaSource, k: usize) -> Option<Carrier> {
    let a = src.info().audio_streams.get(k)?;
    let channels = a.channels as usize;
    if a.sample_rate == 0 || a.bits_per_sample.is_some_and(|b| b != 24) || channels > MAX_CARRIERS || wav_carrier_ids(channels).is_none() {
        return None;
    }
    let gate = src.audio_stream(k, 0, GATE_FRAMES, a.sample_rate).ok()?;
    if gate.channels.len() != channels || !gate.channels.iter().all(|c| pcm24_from_f32(c).is_some()) {
        return None;
    }
    let head = src.audio_stream(k, 0, SYNC_FRAMES, a.sample_rate).ok()?;
    let synced = |c: &Vec<f32>| {
        let mut run = 0;
        c.iter().any(|&v| {
            run = if ((v * 8_388_608.0) as i32) & 1 != 0 { run + 1 } else { 0 };
            run >= SYNC_SAMPLES
        })
    };
    if !head.channels.iter().any(synced) {
        return None;
    }
    let buf = src.audio_stream(k, 0, PROBE_FRAMES.min(DETECT_FRAMES), a.sample_rate).ok()?;
    let planes: Vec<Vec<i32>> = buf.channels.iter().map(|c| pcm24_from_f32(c)).collect::<Option<_>>()?;
    let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
    let detection = detect(&refs, PROBE_FRAMES)?;
    output_order(detection.original)?;
    Some(Carrier { detection, rate: a.sample_rate, channels })
}

/// 24-bit integers from `f32` samples, rounding: a carrier is exact, and a sample that is not
/// (a resampling source, a damaged frame) only loses its block, which then plays as authored.
fn to_pcm24(samples: &[f32]) -> Vec<i32> {
    samples.iter().map(|&v| (v * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32).collect()
}

impl AuroSource {
    fn restored(&self, c: &Carrier, k: usize, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        let Some(out_ch) = self.info.audio_streams.get(k).map(|a| a.channels as usize) else {
            return Err(MediaError::NoStream("audio"));
        };
        if sample_rate == 0 {
            return Ok(AudioBuffer::silence(sample_rate, out_ch, frames));
        }
        // The request in carrier samples, then one block more on both sides.
        let ratio = f64::from(c.rate) / f64::from(sample_rate);
        let exact = c.rate == sample_rate;
        let s0 = if exact { start } else { (start as f64 * ratio).floor() as i64 };
        let need = if exact { frames } else { (frames as f64 * ratio).ceil() as usize + 2 };
        let w0 = if s0 > MARGIN { s0 - MARGIN } else { s0.min(0) };
        let lead = usize::try_from(s0 - w0).unwrap_or(0);
        let wlen = lead.saturating_add(need).saturating_add(MAX_BLOCK);
        let carrier = self.inner.audio_stream(k, w0, wlen, c.rate)?;
        if carrier.channels.len() != c.channels {
            return Err(MediaError::Decode(format!("Auro-3D: the carrier gave {} channels, not {}", carrier.channels.len(), c.channels)));
        }
        let planes: Vec<Vec<i32>> = carrier.channels.iter().map(|ch| to_pcm24(ch)).collect();
        let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
        let Some((_, restored, _)) = unfold_with(&c.detection, &refs) else {
            return Err(MediaError::Decode("Auro-3D: unknown carrier layout".into()));
        };
        let src: Vec<&[i32]> = restored.iter().map(|ch| ch.get(lead..).unwrap_or_default()).collect();
        let mut out = AudioBuffer::silence(sample_rate, out_ch, frames);
        let frac = s0 as f64 - start as f64 * ratio;
        for (o, s) in out.channels.iter_mut().zip(&src) {
            let at = |i: usize| s.get(i).map_or(0.0, |&v| v as f32 / 8_388_608.0);
            for (k, slot) in o.iter_mut().enumerate() {
                if exact {
                    *slot = at(k);
                    continue;
                }
                let pos = k as f64 * ratio - frac;
                let i0 = pos.floor().max(0.0) as usize;
                let f = (pos - i0 as f64) as f32;
                let (a, b) = (at(i0), at(i0 + 1));
                *slot = a + (b - a) * f;
            }
        }
        Ok(out)
    }

    /// The streams stream `k` restores, in channel order (`None` for a plain stream).
    pub fn speakers(&self, k: usize) -> Option<Vec<StreamId>> {
        self.carriers.get(k)?.as_ref().and_then(|c| output_order(c.detection.original))
    }
}

impl MediaSource for AuroSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>> {
        self.inner.video_frame(req)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        self.audio_stream(0, start, frames, sample_rate)
    }

    fn audio_stream(&self, stream: usize, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        match self.carriers.get(stream) {
            Some(Some(c)) => self.restored(c, stream, start, frames, sample_rate),
            _ => self.inner.audio_stream(stream, start, frames, sample_rate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioStreamInfo, MediaKind};
    use cfa4_auro::cfa4::f32_from_pcm24;
    use cfa4_auro::synth;
    use cfa4_time::Tick;

    /// A source playing planar samples at 48 kHz (other rates: nearest sample).
    struct Planes {
        info: MediaInfo,
        planes: Vec<Vec<f32>>,
    }

    impl Planes {
        fn new(planes: Vec<Vec<f32>>, codec: &str) -> Self {
            let frames = planes.first().map_or(0, Vec::len);
            let info = MediaInfo {
                name: "carrier.wav".into(),
                kind: MediaKind::AudioOnly,
                duration: Tick::from_units(frames as i64, 48_000),
                video: None,
                audio_streams: vec![AudioStreamInfo {
                    sample_rate: 48_000,
                    channels: planes.len() as u32,
                    codec: codec.into(),
                    bits_per_sample: Some(24),
                    speakers: Vec::new(),
                }],
                container: "WAV".into(),
                start_timecode: None,
                file_size: None,
            };
            Self { info, planes }
        }
    }

    impl MediaSource for Planes {
        fn info(&self) -> &MediaInfo {
            &self.info
        }
        fn video_frame(&self, _req: FrameRequest) -> Result<Arc<VideoFrame>> {
            Err(MediaError::NoStream("video"))
        }
        fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
            let ratio = 48_000.0 / f64::from(sample_rate.max(1));
            let mut out = AudioBuffer::silence(sample_rate, self.planes.len(), frames);
            for (o, p) in out.channels.iter_mut().zip(&self.planes) {
                for (k, v) in o.iter_mut().enumerate() {
                    let at = ((start + k as i64) as f64 * ratio).round();
                    if at >= 0.0 {
                        *v = p.get(at as usize).copied().unwrap_or(0.0);
                    }
                }
            }
            Ok(out)
        }
    }

    fn carrier_51(blocks: usize) -> Vec<Vec<i32>> {
        let ids = wav_carrier_ids(6).unwrap();
        ids.iter().enumerate().map(|(c, id)| synth::channel(c as u32 + 11, blocks, 0.25, 30, id.0)).collect()
    }

    #[test]
    fn a_carrier_opens_as_its_restored_layout() {
        let planes = carrier_51(40);
        let src = wrap(Arc::new(Planes::new(planes.iter().map(|p| f32_from_pcm24(p)).collect(), "PCM")));
        let a = src.info().audio().cloned().unwrap();
        assert_eq!((a.channels, a.codec.as_str()), (10, "PCM + Auro-3D 9.1"));
        // the restored loudspeakers by name (#323)
        assert_eq!(a.speakers, ["L", "R", "C", "LFE", "Ls", "Rs", "HL", "HR", "HLs", "HRs"]);
        // Every window, wherever it starts, restores the bed (the carrier with its borrowed bits
        // cleared) and silent heights.
        for (start, frames) in [(0i64, 5000usize), (777, 300), (20_000, 4096), (39_000, 3000), (-500, 1000)] {
            let buf = src.audio(start, frames, 48_000).unwrap();
            assert_eq!((buf.channels.len(), buf.frames()), (10, frames));
            for (c, plane) in planes.iter().enumerate() {
                for k in 0..frames {
                    let at = start + k as i64;
                    let want = usize::try_from(at).ok().and_then(|i| plane.get(i)).map_or(0, |&v| v & !0xF);
                    assert_eq!((buf.channels[c][k] * 8_388_608.0) as i32, want, "start {start}, channel {c}, frame {k}");
                }
            }
            assert!(buf.channels[6..].iter().all(|c| c.iter().all(|&v| v == 0.0)));
        }
        // another output rate resamples the restored channels; hostile requests are not errors
        assert_eq!(src.audio(1000, 441, 44_100).unwrap().channels.len(), 10);
        assert_eq!(src.audio(0, 0, 48_000).unwrap().frames(), 0);
        assert_eq!(src.audio(0, 16, 0).unwrap().channels.len(), 10);
        assert!(src.audio_stream(1, 0, 16, 48_000).is_err());
        assert!(src.video_frame(FrameRequest::full(Tick::ZERO)).is_err());
    }

    #[test]
    fn plain_and_lossy_audio_is_left_alone() {
        // plain 24-bit noise: exact PCM, but no side channel
        let noise: Vec<f32> = (0..30_000).map(|i| ((i * 7919 % 65_536) - 32_768) as f32 / 8_388_608.0).collect();
        let src = wrap(Arc::new(Planes::new(vec![noise.clone(), noise.clone()], "PCM")));
        assert_eq!(src.info().audio().map(|a| a.channels), Some(2));
        // a lossy decode is not exact 24-bit PCM: one read, then left alone
        let lossy: Vec<f32> = (0..30_000).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let src = wrap(Arc::new(Planes::new(vec![lossy.clone(), lossy], "AAC")));
        assert_eq!(src.info().audio().map(|a| a.codec.clone()), Some("AAC".into()));
        // a carrier channel count no layout uses
        let planes = carrier_51(8);
        let three: Vec<Vec<f32>> = planes.iter().take(3).map(|p| f32_from_pcm24(p)).collect();
        assert_eq!(wrap(Arc::new(Planes::new(three, "PCM"))).info().audio().map(|a| a.channels), Some(3));
    }
}
