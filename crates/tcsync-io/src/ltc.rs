//! LTC between sample blocks and the chase/transport (no devices).

use cfa4_tcsync::chase::Observation;
use cfa4_tcsync::ltc::{LtcDecoded, LtcDecoder, LtcEncoder, LtcError, LtcFrame};
use cfa4_time::Tick;

use crate::{ExternalRate, Transport, samples_ticks, wrap_day};

/// Copies channel `channel` of interleaved `data` into `out`.
pub fn pick_channel(data: &[f32], channels: usize, channel: usize, out: &mut Vec<f32>) {
    out.clear();
    if channels == 0 || channel >= channels {
        return;
    }
    out.extend(data.chunks_exact(channels).map(|f| f[channel]));
}

/// LTC input: sample blocks in, observations out.
#[derive(Clone, Debug)]
pub struct LtcReader {
    dec: LtcDecoder,
    rate: ExternalRate,
    sample_rate: u32,
    consumed: u64,
    frames: Vec<LtcDecoded>,
}

impl LtcReader {
    pub fn new(sample_rate: u32, rate: ExternalRate) -> Result<Self, LtcError> {
        Ok(LtcReader { dec: LtcDecoder::new(sample_rate)?, rate, sample_rate, consumed: 0, frames: Vec::new() })
    }

    /// Reads `mono`, whose last sample arrived at local time `end`, and appends the frames it
    /// completed. A frame completes at its last bit: going forward the clock is then at the start
    /// of the next frame, in reverse at the start of the frame read.
    pub fn push(&mut self, mono: &[f32], end: Tick, out: &mut Vec<Observation>) {
        self.frames.clear();
        self.dec.decode(mono, &mut self.frames);
        self.consumed = self.consumed.saturating_add(mono.len() as u64);
        for d in &self.frames {
            let n = d.frame.frame_number(self.rate.rate);
            let position = wrap_day(self.rate.rate.tick_of(if d.reverse { n } else { n.saturating_add(1) }));
            let lag = samples_ticks(self.consumed.saturating_sub(d.end), self.sample_rate);
            out.push(Observation { at: Tick(end.0.saturating_sub(lag.0)), position });
        }
    }
}

/// LTC output: fills sample blocks with the frames the transport is on.
#[derive(Clone, Debug)]
pub struct LtcGenerator {
    enc: LtcEncoder,
    rate: ExternalRate,
    sample_rate: u32,
    /// Added to the external time to get the sequence time.
    offset: Tick,
    /// Send the timecode this much ahead of the picture.
    latency: Tick,
    pending: Vec<f32>,
    read: usize,
    next_frame: Option<i64>,
}

impl LtcGenerator {
    pub fn new(sample_rate: u32, rate: ExternalRate, level_dbfs: f32, offset: Tick, latency: Tick) -> Result<Self, LtcError> {
        let enc = LtcEncoder::new(sample_rate, rate.rate, level_dbfs)?;
        Ok(LtcGenerator { enc, rate, sample_rate, offset, latency, pending: Vec::new(), read: 0, next_frame: None })
    }

    /// The external time of day the transport is at, `latency` after local time `t`.
    fn external_at(&self, tr: &Transport, t: Tick) -> Tick {
        let seq = tr.position_at(Tick(t.0.saturating_add(self.latency.0)));
        wrap_day(Tick(seq.0.saturating_sub(self.offset.0)))
    }

    /// Fills `out` (mono), whose first sample is heard at local time `t0`. While the transport is
    /// not rolling forward at normal speed the output is silent.
    pub fn fill(&mut self, out: &mut [f32], t0: Tick, tr: &Transport) {
        if !tr.rolling() {
            out.fill(0.0);
            self.pending.clear();
            self.read = 0;
            self.next_frame = None;
            return;
        }
        let frame = self.rate.rate.frame_duration().0.max(1);
        for (i, s) in out.iter_mut().enumerate() {
            if self.read >= self.pending.len() {
                let t = Tick(t0.0.saturating_add(samples_ticks(i as u64, self.sample_rate).0));
                self.schedule(tr, t, frame);
            }
            *s = self.pending.get(self.read).copied().unwrap_or(0.0);
            self.read += 1;
        }
    }

    /// Encodes the next frame, or re-anchors on the transport when the frame running on would be
    /// more than a frame away from where the transport is.
    fn schedule(&mut self, tr: &Transport, t: Tick, frame: i64) {
        let ext = self.external_at(tr, t);
        let day_frames = self.rate.frames_per_day();
        let on_time = |n: i64| {
            let d = (ext.0 - self.rate.rate.tick_of(n).0).rem_euclid(crate::day());
            d.min(crate::day() - d) <= frame
        };
        self.pending.clear();
        self.read = 0;
        let (n, skip) = match self.next_frame {
            Some(n) if on_time(n) => (n, 0),
            _ => {
                let n = self.rate.frame_at(ext);
                let phase = ext.0.saturating_sub(self.rate.rate.tick_of(n).0).max(0);
                let skip = phase.saturating_mul(i64::from(self.sample_rate)) / cfa4_time::TICKS_PER_SECOND;
                (n, usize::try_from(skip).unwrap_or(0))
            }
        };
        let f = LtcFrame::from_frame_number(n, self.rate.rate, self.rate.drop_frame);
        self.enc.encode(&f, &mut self.pending);
        self.read = skip.min(self.pending.len());
        self.next_frame = Some((n + 1).rem_euclid(day_frames));
    }
}

#[cfg(test)]
mod tests {
    use cfa4_time::FrameRate;

    use super::*;

    fn rate25() -> ExternalRate {
        ExternalRate::new(FrameRate::FPS_25, false)
    }

    #[test]
    fn picks_one_channel() {
        let mut out = Vec::new();
        pick_channel(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, 1, &mut out);
        assert_eq!(out, [2.0, 5.0]);
        pick_channel(&[1.0, 2.0], 2, 2, &mut out);
        assert!(out.is_empty());
        pick_channel(&[1.0, 2.0], 0, 0, &mut out);
        assert!(out.is_empty());
    }

    /// Generated output, read back, gives observations whose position matches the transport at
    /// the observation time: the two halves agree on timing to within a fraction of a frame.
    #[test]
    fn generator_output_reads_back_on_the_transport_clock() {
        let sr = 48_000;
        let offset = FrameRate::FPS_25.tick_of(25 * 3600); // the sequence starts at 01:00:00:00
        let start = Tick::from_units(10, 1); // sequence position 10 s
        let tr = Transport { playing: true, position: start, at: Tick(0), speed: 1.0 };
        let mut g = LtcGenerator::new(sr, rate25(), -18.0, Tick(-offset.0), Tick(0)).expect("generator");
        let mut r = LtcReader::new(sr, rate25()).expect("reader");
        let mut obs = Vec::new();
        let block = 480;
        let mut buf = vec![0.0f32; block];
        for b in 0..200u64 {
            let t0 = samples_ticks(b * block as u64, sr);
            g.fill(&mut buf, t0, &tr);
            let end = samples_ticks((b + 1) * block as u64, sr);
            r.push(&buf, end, &mut obs);
        }
        assert!(obs.len() >= 45, "{} frames", obs.len());
        let frame = FrameRate::FPS_25.frame_duration().0;
        for o in &obs {
            let want = Tick(tr.position_at(o.at).0 + offset.0);
            let err = (o.position.0 - want.0).abs();
            assert!(err < frame / 8, "observation {o:?} off by {err} ticks");
        }
    }

    #[test]
    fn generator_is_silent_when_not_rolling_and_reanchors_on_a_jump() {
        let sr = 48_000;
        let mut g = LtcGenerator::new(sr, rate25(), -18.0, Tick(0), Tick(0)).expect("generator");
        let mut buf = vec![1.0f32; 960];
        g.fill(&mut buf, Tick(0), &Transport::default());
        assert!(buf.iter().all(|&s| s == 0.0));
        let shuttle = Transport { playing: true, speed: 4.0, ..Transport::default() };
        g.fill(&mut buf, Tick(0), &shuttle);
        assert!(buf.iter().all(|&s| s == 0.0));

        // Play from 0, then jump to 1 hour: the frames read back follow the jump.
        let mut r = LtcReader::new(sr, rate25()).expect("reader");
        let mut obs = Vec::new();
        let a = Transport { playing: true, ..Transport::default() };
        let jump_at = samples_ticks(48_000, sr);
        let b = Transport { playing: true, position: Tick::from_units(3600, 1), at: jump_at, speed: 1.0 };
        for i in 0..200u64 {
            let t0 = samples_ticks(i * 960, sr);
            let tr = if t0 < jump_at { a } else { b };
            g.fill(&mut buf, t0, &tr);
            r.push(&buf, samples_ticks((i + 1) * 960, sr), &mut obs);
        }
        let late: Vec<_> = obs.iter().filter(|o| o.at > Tick(jump_at.0 + Tick::from_units(1, 2).0)).collect();
        assert!(late.len() > 20);
        for o in late {
            let want = b.position_at(o.at);
            assert!((o.position.0 - want.0).abs() < FrameRate::FPS_25.frame_duration().0 / 8, "{o:?}");
        }
    }

    #[test]
    fn latency_sends_ahead() {
        let sr = 48_000;
        let latency = Tick::from_units(1, 10); // 100 ms = 2.5 frames
        let tr = Transport { playing: true, ..Transport::default() };
        let mut g = LtcGenerator::new(sr, rate25(), -18.0, Tick(0), latency).expect("generator");
        let mut r = LtcReader::new(sr, rate25()).expect("reader");
        let mut obs = Vec::new();
        let mut buf = vec![0.0f32; 512];
        for b in 0..150u64 {
            g.fill(&mut buf, samples_ticks(b * 512, sr), &tr);
            r.push(&buf, samples_ticks((b + 1) * 512, sr), &mut obs);
        }
        assert!(!obs.is_empty());
        for o in &obs {
            let want = tr.position_at(Tick(o.at.0 + latency.0));
            assert!((o.position.0 - want.0).abs() < FrameRate::FPS_25.frame_duration().0 / 8, "{o:?}");
        }
    }

    #[test]
    fn reverse_frames_observe_their_start() {
        let sr = 48_000;
        let mut enc = LtcEncoder::new(sr, FrameRate::FPS_25, -18.0).expect("encoder");
        let mut s = Vec::new();
        for n in 100..130 {
            enc.encode(&LtcFrame::from_frame_number(n, FrameRate::FPS_25, false), &mut s);
        }
        s.reverse();
        let mut r = LtcReader::new(sr, rate25()).expect("reader");
        let mut obs = Vec::new();
        r.push(&s, samples_ticks(s.len() as u64, sr), &mut obs);
        assert!(obs.len() >= 25);
        // Played backwards from the end of frame 129: at sample k the clock is at 130 - k/1920.
        for o in &obs {
            let k = o.at.0 as f64 / samples_ticks(1, sr).0 as f64;
            let want = FrameRate::FPS_25.tick_of(130).0 as f64 - k * samples_ticks(1, sr).0 as f64;
            assert!((o.position.0 as f64 - want).abs() < FrameRate::FPS_25.frame_duration().0 as f64 / 8.0, "{o:?}");
        }
    }
}
