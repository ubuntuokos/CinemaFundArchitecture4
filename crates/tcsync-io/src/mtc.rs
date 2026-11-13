//! MIDI Timecode between MIDI bytes and the chase/transport (no devices).

use cfa4_tcsync::chase::Observation;
use cfa4_tcsync::mtc::{MtcEvent, MtcParser, MtcRate, MtcTime};
use cfa4_time::{Tick, fields_to_frames};

use crate::{ExternalRate, Input, Transport, wrap_day};

/// MTC input: MIDI bytes in, observations and locates out.
#[derive(Clone, Debug)]
pub struct MtcReader {
    parser: MtcParser,
    rate: ExternalRate,
    events: Vec<MtcEvent>,
}

impl MtcReader {
    pub fn new(rate: ExternalRate) -> Self {
        MtcReader { parser: MtcParser::new(), rate, events: Vec::new() }
    }

    /// The time of day of an MTC label, counted at the configured external rate (MTC has no code
    /// for 23.976 or 29.97 non-drop: those arrive with the 24 and 30 codes).
    fn position(&self, t: &MtcTime) -> Tick {
        let n = fields_to_frames(i64::from(t.hours), i64::from(t.minutes), i64::from(t.seconds), i64::from(t.frames), self.rate.rate, self.rate.drop_frame);
        wrap_day(self.rate.rate.tick_of(n))
    }

    /// Reads `bytes` received at local time `at`.
    pub fn push(&mut self, bytes: &[u8], at: Tick, out: &mut Vec<Input>) {
        self.events.clear();
        self.parser.feed(bytes, &mut self.events);
        let quarter = self.rate.rate.frame_duration().0 / 4;
        for ev in &self.events {
            match ev {
                MtcEvent::Piece { .. } => {}
                MtcEvent::Time { time, reverse } => {
                    // Forward, the parser gives the frame starting with the next piece 0, a
                    // quarter frame after piece 7 arrived.
                    let p = self.position(time);
                    let position = if *reverse { p } else { wrap_day(Tick(p.0 - quarter)) };
                    out.push(Input::Frame(Observation { at, position }));
                }
                MtcEvent::Full(time) => out.push(Input::Locate(Observation { at, position: self.position(time) })),
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Run {
    /// The frame the current eight pieces describe (sent on its start).
    base: i64,
    /// Local time the base frame starts.
    base_at: Tick,
    /// Next piece to send.
    piece: u8,
}

/// MTC output: which messages are due when.
#[derive(Clone, Debug)]
pub struct MtcScheduler {
    rate: ExternalRate,
    code: MtcRate,
    offset: Tick,
    latency: Tick,
    run: Option<Run>,
    /// Frame of the last full frame sent while not rolling.
    held: Option<i64>,
}

impl MtcScheduler {
    /// `offset` is added to the external time to get the sequence time; `latency` sends ahead.
    pub fn new(rate: ExternalRate, offset: Tick, latency: Tick) -> Self {
        let (code, _) = MtcRate::nearest(rate.rate, rate.drop_frame);
        MtcScheduler { rate, code, offset, latency, run: None, held: None }
    }

    /// Whether MTC can carry this rate exactly.
    pub fn exact(rate: ExternalRate) -> bool {
        MtcRate::nearest(rate.rate, rate.drop_frame).1
    }

    fn external_at(&self, tr: &Transport, t: Tick) -> Tick {
        let seq = tr.position_at(Tick(t.0.saturating_add(self.latency.0)));
        wrap_day(Tick(seq.0.saturating_sub(self.offset.0)))
    }

    fn label(&self, n: i64) -> MtcTime {
        MtcTime::from_frame_number(n, self.code)
    }

    fn quarter(&self) -> i64 {
        self.rate.rate.frame_duration().0 / 4
    }

    /// Starts the quarter frames at the next frame boundary after `now`, with a full frame first.
    fn anchor(&mut self, tr: &Transport, now: Tick, out: &mut Vec<Vec<u8>>) {
        let ext = self.external_at(tr, now);
        let n = self.rate.frame_at(ext);
        out.push(self.label(n).full_frame().to_vec());
        // on a frame boundary start with that frame, otherwise with the next one
        let (base, to_next) = if self.rate.rate.tick_of(n) == ext { (n, 0) } else { (n + 1, self.rate.rate.tick_of(n + 1).0 - ext.0) };
        let base = base.rem_euclid(self.rate.frames_per_day());
        self.run = Some(Run { base, base_at: Tick(now.0.saturating_add(to_next.max(0))), piece: 0 });
    }

    /// Appends the messages due at local time `now` and returns when to call again.
    pub fn poll(&mut self, now: Tick, tr: &Transport, out: &mut Vec<Vec<u8>>) -> Tick {
        let frame = self.rate.rate.frame_duration().0.max(4);
        if !tr.rolling() {
            self.run = None;
            if !tr.playing {
                // A locate while stopped: one full frame for each new position.
                let n = self.rate.frame_at(self.external_at(tr, now));
                if self.held != Some(n) {
                    self.held = Some(n);
                    out.push(self.label(n).full_frame().to_vec());
                }
            }
            return Tick(now.0.saturating_add(frame));
        }
        self.held = None;
        if self.run.is_none() {
            self.anchor(tr, now, out);
        }
        let quarter = self.quarter();
        let day = self.rate.frames_per_day();
        while let Some(mut run) = self.run {
            let due = Tick(run.base_at.0.saturating_add(quarter * i64::from(run.piece)));
            if due > now {
                return due;
            }
            // Late by more than a frame (the thread stalled) or off the transport: start again.
            let label_at = self.rate.rate.tick_of(run.base).0 + quarter * i64::from(run.piece);
            let d = (self.external_at(tr, due).0 - label_at).rem_euclid(crate::day());
            if now.0 - due.0 > frame || d.min(crate::day() - d) > frame {
                self.anchor(tr, now, out);
                continue;
            }
            out.push(self.label(run.base).quarter_frames()[usize::from(run.piece)].to_vec());
            run.piece += 1;
            if run.piece == 8 {
                let next = run.base + 2;
                run.base_at = Tick(run.base_at.0 + self.rate.rate.tick_of(next).0 - self.rate.rate.tick_of(run.base).0);
                run.base = next.rem_euclid(day);
                run.piece = 0;
            }
            self.run = Some(run);
        }
        Tick(now.0.saturating_add(quarter))
    }
}

#[cfg(test)]
mod tests {
    use cfa4_tcsync::chase::{Chase, ChaseConfig, ChaseState};
    use cfa4_time::FrameRate;

    use super::*;

    fn rate25() -> ExternalRate {
        ExternalRate::new(FrameRate::FPS_25, false)
    }

    /// Runs the scheduler from 0 to `until` in 1 ms steps; returns (time, message) pairs.
    fn run(s: &mut MtcScheduler, tr: impl Fn(Tick) -> Transport, until: Tick) -> Vec<(Tick, Vec<u8>)> {
        let ms = Tick::from_units(1, 1000).0;
        let mut got = Vec::new();
        let mut now = Tick(0);
        while now < until {
            let mut out = Vec::new();
            s.poll(now, &tr(now), &mut out);
            got.extend(out.into_iter().map(|m| (now, m)));
            now = Tick(now.0 + ms);
        }
        got
    }

    #[test]
    fn rolling_sends_a_full_frame_then_quarter_frames_on_time() {
        let mut s = MtcScheduler::new(rate25(), Tick(0), Tick(0));
        let tr = Transport { playing: true, position: FrameRate::FPS_25.tick_of(250), at: Tick(0), speed: 1.0 };
        let got = run(&mut s, |_| tr, Tick::from_units(1, 1));
        assert_eq!(got[0].1, MtcTime::from_frame_number(250, MtcRate::Fps25).full_frame());
        let qf: Vec<_> = got.iter().filter(|(_, m)| m[0] == 0xF1).collect();
        // 1 s at 25 fps: 100 quarter frames (±1 for the edges).
        assert!((99..=101).contains(&qf.len()), "{}", qf.len());
        // Each piece is sent within 1 ms of its quarter-frame boundary.
        let quarter = FrameRate::FPS_25.frame_duration().0 / 4;
        let ms = Tick::from_units(1, 1000).0;
        for (t, _) in &qf {
            assert!(t.0 % quarter < ms || quarter - t.0 % quarter < ms, "{t:?}");
        }
        // The pieces read back give the transport's time.
        let mut r = MtcReader::new(rate25());
        let mut inputs = Vec::new();
        for (t, m) in &got {
            r.push(m, *t, &mut inputs);
        }
        let frames: Vec<_> = inputs.iter().filter_map(|i| if let Input::Frame(o) = i { Some(*o) } else { None }).collect();
        assert!(frames.len() >= 11);
        for o in frames {
            assert!((o.position.0 - tr.position_at(o.at).0).abs() <= ms, "{o:?}");
        }
    }

    #[test]
    fn stopped_sends_one_full_frame_per_position_and_a_jump_reanchors() {
        let mut s = MtcScheduler::new(rate25(), Tick(0), Tick(0));
        let stopped = Transport::default();
        let got = run(&mut s, |_| stopped, Tick::from_units(1, 2));
        assert_eq!(got.len(), 1);
        let jump = Tick::from_units(1, 2);
        let mut s = MtcScheduler::new(rate25(), Tick(0), Tick(0));
        let a = Transport { playing: true, ..Transport::default() };
        let b = Transport { playing: true, position: Tick::from_units(600, 1), at: jump, speed: 1.0 };
        let got = run(&mut s, |t| if t < jump { a } else { b }, Tick::from_units(1, 1));
        let fulls: Vec<_> = got.iter().filter(|(_, m)| m[0] == 0xF0).collect();
        assert_eq!(fulls.len(), 2);
        assert_eq!(fulls[1].1, MtcTime::from_frame_number(600 * 25, MtcRate::Fps25).full_frame());
    }

    #[test]
    fn chase_locks_onto_generated_mtc() {
        let offset = FrameRate::FPS_25.tick_of(25 * 3600);
        let mut s = MtcScheduler::new(rate25(), Tick(-offset.0), Tick(0));
        let tr = Transport { playing: true, position: Tick::from_units(5, 1), at: Tick(0), speed: 1.0 };
        let got = run(&mut s, |_| tr, Tick::from_units(1, 1));
        let mut r = MtcReader::new(rate25());
        let mut cfg = ChaseConfig::new(FrameRate::FPS_25);
        cfg.offset = Tick(-offset.0);
        let mut chase = Chase::new(cfg);
        let mut actions = Vec::new();
        let mut inputs = Vec::new();
        for (t, m) in &got {
            inputs.clear();
            r.push(m, *t, &mut inputs);
            for i in &inputs {
                if let Input::Frame(o) = i {
                    chase.observe(*o, &mut actions);
                }
            }
        }
        assert_eq!(chase.status().state, ChaseState::Locked);
        let cfa4_tcsync::chase::ChaseAction::Locate { to, play } = actions[0] else { panic!("{actions:?}") };
        assert!(play);
        // Locked within the first second, at the transport's position then.
        assert!(to >= tr.position && to.0 - tr.position.0 < Tick::from_units(1, 1).0, "{to:?}");
    }

    #[test]
    fn rates_without_an_mtc_code_are_flagged() {
        assert!(MtcScheduler::exact(rate25()));
        assert!(!MtcScheduler::exact(ExternalRate::new(FrameRate::FPS_23_976, false)));
        assert!(MtcScheduler::exact(ExternalRate::new(FrameRate::FPS_29_97, true)));
    }

    #[test]
    fn full_frames_become_locates() {
        let mut r = MtcReader::new(rate25());
        let mut out = Vec::new();
        r.push(&MtcTime::from_frame_number(30, MtcRate::Fps25).full_frame(), Tick(7), &mut out);
        assert_eq!(out, [Input::Locate(Observation { at: Tick(7), position: FrameRate::FPS_25.tick_of(30) })]);
    }
}
