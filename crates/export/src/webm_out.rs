//! WebM output of the stepped exporter: VP9 video and Opus audio in a Matroska file with the
//! `webm` DocType, written with [`cfa4_matroska::MkvWriter`]. Opus follows the Matroska Opus
//! mapping: `OpusHead` as CodecPrivate, the encoder delay as CodecDelay, 80 ms SeekPreRoll, and
//! DiscardPadding on the last block, trimming the padding of the final frame. Blocks are written
//! in timestamp order, so video and audio interleave.

use std::collections::VecDeque;

use cfa4_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
use cfa4_opus::Encoder;
use cfa4_time::FrameRate;

use crate::audio_out::interleave;
use crate::settings::opus_bitrate_kbps;
use crate::{EncodedPacket, ExportError, ExportSettings, Out, Result};

/// Opus decoders need this much sound before a seek point to converge (RFC 7845 §4.6).
const SEEK_PRE_ROLL_NS: u64 = 80_000_000;
const OPUS_RATE: u32 = 48_000;

fn mkv(e: cfa4_matroska::Error) -> ExportError {
    ExportError::Io(e.to_string())
}

/// `n` units of `1 / per_second` s in nanoseconds (rounded down, saturating).
fn ns(n: i128, num: i128, per_second: i128) -> i64 {
    if per_second <= 0 {
        return 0;
    }
    i64::try_from(n.saturating_mul(num).saturating_mul(1_000_000_000) / per_second).unwrap_or(if n < 0 { i64::MIN } else { i64::MAX })
}

/// Everything the WebM muxer needs from the export.
pub(crate) struct WebmSetup<'a> {
    pub settings: &'a ExportSettings,
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    /// (sample rate, channels) of the audio, if any.
    pub audio: Option<(u32, usize)>,
}

/// The Opus side of a WebM export: samples waiting for a whole frame, and how many were coded.
struct OpusTrack {
    enc: Encoder,
    channels: usize,
    pending: Vec<f32>,
    /// Samples per channel handed in.
    input: u64,
    /// Samples per channel coded (whole frames, the encoder delay included).
    coded: u64,
}

/// A queued block: presentation time, key frame, data, DiscardPadding.
type Block = (i64, bool, Vec<u8>, u64);

/// A WebM export in progress.
pub(crate) struct WebmMux {
    mkv: MkvWriter<Out>,
    rate: FrameRate,
    /// Video frames queued so far.
    frames: i64,
    video: VecDeque<Block>,
    audio: VecDeque<Block>,
    opus: Option<OpusTrack>,
}

impl WebmMux {
    pub fn new(s: WebmSetup) -> Result<Self> {
        let settings = s.settings;
        let mut v = TrackSpec::new(TrackKind::Video, "V_VP9");
        v.video_size = Some((s.width, s.height));
        let frame_ns = ns(1, i128::from(s.rate.den), i128::from(s.rate.num));
        v.default_duration_ns = u64::try_from(frame_ns).ok().filter(|d| *d > 0);
        let mut tracks = vec![v];
        let opus = match s.audio {
            Some((sr, ch)) => {
                if sr != OPUS_RATE || !(1..=2).contains(&ch) {
                    return Err(ExportError::Unsupported(format!("Opus: {sr} Hz with {ch} channels (the encoder takes 48 kHz mono or stereo)")));
                }
                let enc = Encoder::new(ch, opus_bitrate_kbps(settings.audio.bitrate_kbps).saturating_mul(1000))
                    .map_err(|e| ExportError::Unsupported(format!("Opus: {e}")))?;
                let mut a = TrackSpec::new(TrackKind::Audio, "A_OPUS");
                a.codec_private = enc.head(sr).to_bytes();
                a.audio = Some((f64::from(sr), ch as u64, None));
                a.codec_delay_ns = ns(Encoder::LOOKAHEAD as i128, 1, i128::from(OPUS_RATE)).max(0) as u64;
                a.seek_pre_roll_ns = SEEK_PRE_ROLL_NS;
                tracks.push(a);
                Some(OpusTrack { enc, channels: ch, pending: Vec::new(), input: 0, coded: 0 })
            }
            None => None,
        };
        let title = settings.metadata.title.trim();
        let opts = MuxOptions {
            doc_type: "webm".into(),
            title: (!title.is_empty()).then(|| title.to_string()),
            writing_app: format!("CFA4 {}", env!("CARGO_PKG_VERSION")),
            ..Default::default()
        };
        let mkv = MkvWriter::new(Out::create(settings)?, tracks, opts).map_err(mkv)?;
        Ok(WebmMux { mkv, rate: s.rate, frames: 0, video: VecDeque::new(), audio: VecDeque::new(), opus })
    }

    pub fn write_video(&mut self, packets: Vec<EncodedPacket>) -> Result<()> {
        for p in packets {
            // VP9 frames come in display order, one per output frame
            let pts = ns(i128::from(self.frames), i128::from(self.rate.den), i128::from(self.rate.num));
            self.video.push_back((pts, p.key, p.data, 0));
            self.frames += 1;
        }
        self.drain(false)
    }

    pub fn write_audio(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        if let Some(o) = self.opus.as_mut() {
            let n = planar.first().map_or(0, Vec::len);
            o.input = o.input.saturating_add(n as u64);
            o.pending.extend(interleave(planar));
            Self::code(o, &mut self.audio, false);
        }
        self.drain(false)
    }

    /// Code every whole frame waiting (all of them, padded, when `flush`) into `queue`.
    fn code(o: &mut OpusTrack, queue: &mut VecDeque<Block>, flush: bool) {
        let frame_len = Encoder::FRAME_SIZE * o.channels;
        let end = o.input.saturating_add(Encoder::LOOKAHEAD as u64);
        let mut at = 0;
        while o.pending.len() - at >= frame_len || (flush && o.coded < end) {
            let take = (o.pending.len() - at).min(frame_len);
            let data = o.enc.encode(&o.pending[at..at + take]);
            at += take;
            // the block time is that of its first decoded sample, the encoder delay not yet removed
            let pts = ns(i128::from(o.coded) - Encoder::LOOKAHEAD as i128, 1, i128::from(OPUS_RATE));
            o.coded = o.coded.saturating_add(Encoder::FRAME_SIZE as u64);
            let pad = if flush && o.coded >= end { ns(i128::from(o.coded - end), 1, i128::from(OPUS_RATE)).max(0) as u64 } else { 0 };
            queue.push_back((pts, true, data, pad));
        }
        o.pending.drain(..at);
    }

    /// Write queued blocks in timestamp order; while both tracks are open, only up to the earlier
    /// of their last queued times.
    fn drain(&mut self, all: bool) -> Result<()> {
        let has_audio = self.opus.is_some();
        loop {
            let take_video = match (self.video.front(), self.audio.front()) {
                (Some(v), Some(a)) => v.0 <= a.0,
                (Some(_), None) if all || !has_audio => true,
                (None, Some(_)) if all => false,
                _ => break,
            };
            let queue = if take_video { &mut self.video } else { &mut self.audio };
            let Some((pts, key, data, pad)) = queue.pop_front() else { break };
            let track = usize::from(!take_video);
            if pad > 0 {
                self.mkv.write_frame_discarding(track, pts, &data, pad).map_err(mkv)?;
            } else {
                self.mkv.write_frame(track, pts, key, &data, None).map_err(mkv)?;
            }
        }
        Ok(())
    }

    /// Code the last audio frames, write everything left and close the file; returns its size.
    pub fn finish(mut self, settings: &ExportSettings) -> Result<u64> {
        if let Some(o) = self.opus.as_mut() {
            Self::code(o, &mut self.audio, true);
        }
        self.drain(true)?;
        self.mkv.finish().map_err(mkv)?.finish(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::ns;

    #[test]
    fn nanoseconds_of_units() {
        assert_eq!(ns(1, 1001, 24000), 41_708_333);
        assert_eq!(ns(120, 1, 48_000), 2_500_000);
        assert_eq!(ns(-120, 1, 48_000), -2_500_000);
        assert_eq!(ns(5, 1, 0), 0, "no division by zero");
        assert_eq!(ns(i128::MAX, 2, 1), i64::MAX, "saturates");
    }
}
