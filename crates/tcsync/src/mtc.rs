//! MIDI Timecode (MTC): quarter-frame and full-frame messages, generation and parsing.
//!
//! - **Quarter frame** `F1 0nnn dddd`: eight pieces `nnn = 0..=7` carry one time over two frames,
//!   four pieces per frame: frames low/high, seconds low/high, minutes low/high, hours low, and
//!   hours high with the rate code (`0rrh`). The time is that of the frame on which piece 0 was
//!   sent, so when piece 7 arrives the sender is two frames further: the parser adds them.
//!   In reverse play the pieces come 7 → 0.
//! - **Full frame** (universal real-time SysEx) `F0 7F 7F 01 01 hr mn sc fr F7`, `hr = 0rrhhhhh`:
//!   a locate, also sent while stopped.
//!
//! Rate codes: 0 = 24, 1 = 25, 2 = 29.97 drop-frame, 3 = 30 fps. 23.976 and other rates have no
//! code; [`MtcRate::nearest`] picks the one with the same timecode base and the caller warns.

use cfa4_time::{FrameRate, fields_to_frames, frames_to_fields};

/// The MTC rate code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MtcRate {
    Fps24 = 0,
    Fps25 = 1,
    Fps2997Df = 2,
    Fps30 = 3,
}

impl MtcRate {
    pub fn from_code(code: u8) -> MtcRate {
        match code & 3 {
            0 => MtcRate::Fps24,
            1 => MtcRate::Fps25,
            2 => MtcRate::Fps2997Df,
            _ => MtcRate::Fps30,
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    /// The frame rate and drop-frame flag this code stands for.
    pub fn frame_rate(self) -> (FrameRate, bool) {
        match self {
            MtcRate::Fps24 => (FrameRate::FPS_24, false),
            MtcRate::Fps25 => (FrameRate::FPS_25, false),
            MtcRate::Fps2997Df => (FrameRate::FPS_29_97, true),
            MtcRate::Fps30 => (FrameRate::FPS_30, false),
        }
    }

    /// The code for `rate` / `drop_frame`, and whether it is exact. 23.976 maps to 24 and
    /// 29.97 non-drop to 30 (same labels, different real speed); other rates map by timecode base.
    pub fn nearest(rate: FrameRate, drop_frame: bool) -> (MtcRate, bool) {
        let rate = rate.sane();
        let exact = |r: MtcRate| {
            let (fr, df) = r.frame_rate();
            fr == rate && df == (drop_frame && rate.supports_drop_frame())
        };
        let code = match rate.timecode_base() {
            ..=24 => MtcRate::Fps24,
            25 => MtcRate::Fps25,
            _ if drop_frame && rate.supports_drop_frame() => MtcRate::Fps2997Df,
            _ => MtcRate::Fps30,
        };
        (code, exact(code))
    }

    /// Frames per timecode second.
    fn base(self) -> u8 {
        match self {
            MtcRate::Fps24 => 24,
            MtcRate::Fps25 => 25,
            _ => 30,
        }
    }
}

/// A time carried by MTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MtcTime {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub rate: MtcRate,
}

impl MtcTime {
    /// The label of frame count `frame` (wrapped into one day) at `rate`.
    pub fn from_frame_number(frame: i64, rate: MtcRate) -> MtcTime {
        let (fr, df) = rate.frame_rate();
        let day = fields_to_frames(24, 0, 0, 0, fr, df).max(1);
        let (_, h, m, s, f) = frames_to_fields(frame.rem_euclid(day), fr, df);
        let b = |v: i64, hi: i64| u8::try_from(v.clamp(0, hi)).unwrap_or(0);
        MtcTime { hours: b(h, 23), minutes: b(m, 59), seconds: b(s, 59), frames: b(f, 29), rate }
    }

    /// The frame count of this label.
    pub fn frame_number(&self) -> i64 {
        let (fr, df) = self.rate.frame_rate();
        fields_to_frames(i64::from(self.hours), i64::from(self.minutes), i64::from(self.seconds), i64::from(self.frames), fr, df)
    }

    /// The label `delta` frames later (or earlier), wrapping at 24 hours.
    pub fn offset(&self, delta: i64) -> MtcTime {
        MtcTime::from_frame_number(self.frame_number().saturating_add(delta), self.rate)
    }

    fn is_valid(&self) -> bool {
        self.hours < 24 && self.minutes < 60 && self.seconds < 60 && self.frames < self.rate.base()
    }

    /// The eight quarter-frame messages for this time, piece 0 first. Send them in order (or
    /// 7 → 0 in reverse), one every quarter frame, starting on the frame this time labels.
    pub fn quarter_frames(&self) -> [[u8; 2]; 8] {
        let nibbles = [
            self.frames & 0xF,
            (self.frames >> 4) & 0x1,
            self.seconds & 0xF,
            (self.seconds >> 4) & 0x3,
            self.minutes & 0xF,
            (self.minutes >> 4) & 0x3,
            self.hours & 0xF,
            ((self.hours >> 4) & 0x1) | (self.rate.code() << 1),
        ];
        let mut out = [[0u8; 2]; 8];
        for (i, (slot, n)) in out.iter_mut().zip(nibbles).enumerate() {
            *slot = [0xF1, ((i as u8) << 4) | n];
        }
        out
    }

    /// The full-frame SysEx message (to all devices).
    pub fn full_frame(&self) -> [u8; 10] {
        [0xF0, 0x7F, 0x7F, 0x01, 0x01, (self.rate.code() << 5) | (self.hours & 0x1F), self.minutes & 0x3F, self.seconds & 0x3F, self.frames & 0x1F, 0xF7]
    }
}

/// What the parser read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MtcEvent {
    /// A valid quarter-frame piece (four per frame: the receiver's fine clock).
    Piece { piece: u8, reverse: bool },
    /// A complete eight-piece time. Forward: the current frame (the encoded time plus the two
    /// frames the sender advanced while sending it). Reverse: the encoded time.
    Time { time: MtcTime, reverse: bool },
    /// A full-frame message: locate to this time.
    Full(MtcTime),
}

/// Longest SysEx the parser buffers; longer ones are skipped (they are not MTC).
const SYSEX_MAX: usize = 16;

/// MIDI byte-stream parser for MTC. Other messages, running status and real-time bytes are skipped.
#[derive(Clone, Debug, Default)]
pub struct MtcParser {
    /// Waiting for the data byte of a quarter frame.
    after_f1: bool,
    /// Inside a SysEx; `None` when it grew past [`SYSEX_MAX`] and is being skipped.
    sysex: Option<Option<Vec<u8>>>,
    /// Pieces of the current time, as received.
    nibbles: [u8; 8],
    /// Pieces received in sequence so far, and in which direction.
    run: u8,
    last_piece: Option<u8>,
    reverse: bool,
}

impl MtcParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets partial messages (after a port change or a gap).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Reads `bytes` and appends events to `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<MtcEvent>) {
        for &b in bytes {
            self.byte(b, out);
        }
    }

    fn byte(&mut self, b: u8, out: &mut Vec<MtcEvent>) {
        if b >= 0xF8 {
            return; // real-time bytes may appear anywhere
        }
        if let Some(buf) = self.sysex.as_mut() {
            if b == 0xF7 {
                if let Some(data) = buf.take() {
                    self.sysex = None;
                    if let Some(t) = parse_full_frame(&data) {
                        self.run = 0;
                        self.last_piece = None;
                        out.push(MtcEvent::Full(t));
                    }
                } else {
                    self.sysex = None;
                }
                return;
            }
            if b & 0x80 == 0 {
                if let Some(data) = buf.as_mut() {
                    if data.len() < SYSEX_MAX {
                        data.push(b);
                    } else {
                        *buf = None;
                    }
                }
                return;
            }
            // Any other status byte ends the SysEx without a valid message.
            self.sysex = None;
        }
        if b & 0x80 != 0 {
            self.after_f1 = b == 0xF1;
            if b == 0xF0 {
                self.sysex = Some(Some(Vec::new()));
            }
            return;
        }
        if self.after_f1 {
            self.after_f1 = false;
            self.piece(b >> 4, b & 0xF, out);
        }
    }

    fn piece(&mut self, piece: u8, nibble: u8, out: &mut Vec<MtcEvent>) {
        let piece = piece & 7;
        let continues = match self.last_piece {
            Some(p) if self.run > 0 => {
                if self.reverse {
                    piece + 1 == p
                } else {
                    piece == p + 1
                }
            }
            _ => false,
        };
        if !continues {
            // A new run can start only at piece 0 (forward) or 7 (reverse); a run of one piece
            // does not yet know its direction, so 6 after 7 or 1 after 0 decide it.
            match (self.last_piece, piece) {
                (Some(7), 6) if self.run == 1 => self.reverse = true,
                (Some(0), 1) if self.run == 1 => self.reverse = false,
                (_, 0) => {
                    self.reverse = false;
                    self.run = 0;
                }
                (_, 7) => {
                    self.reverse = true;
                    self.run = 0;
                }
                _ => {
                    self.run = 0;
                    self.last_piece = None;
                    return;
                }
            }
        }
        if let Some(n) = self.nibbles.get_mut(usize::from(piece)) {
            *n = nibble;
        }
        self.last_piece = Some(piece);
        self.run = (self.run + 1).min(8);
        out.push(MtcEvent::Piece { piece, reverse: self.reverse });
        let complete = self.run == 8 && piece == if self.reverse { 0 } else { 7 };
        if complete {
            self.run = 0;
            let n = self.nibbles;
            let time = MtcTime {
                frames: (n[0] & 0xF) | ((n[1] & 1) << 4),
                seconds: (n[2] & 0xF) | ((n[3] & 3) << 4),
                minutes: (n[4] & 0xF) | ((n[5] & 3) << 4),
                hours: (n[6] & 0xF) | ((n[7] & 1) << 4),
                rate: MtcRate::from_code(n[7] >> 1),
            };
            if time.is_valid() {
                let time = if self.reverse { time } else { time.offset(2) };
                out.push(MtcEvent::Time { time, reverse: self.reverse });
            }
        }
    }
}

/// `7F <dev> 01 01 hr mn sc fr` (the bytes between F0 and F7).
fn parse_full_frame(d: &[u8]) -> Option<MtcTime> {
    let [0x7F, _dev, 0x01, 0x01, hr, mn, sc, fr] = *d else {
        return None;
    };
    let t = MtcTime { hours: hr & 0x1F, minutes: mn, seconds: sc, frames: fr, rate: MtcRate::from_code(hr >> 5) };
    t.is_valid().then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Vec<MtcEvent> {
        let mut p = MtcParser::new();
        let mut out = Vec::new();
        p.feed(bytes, &mut out);
        out
    }

    fn times(ev: &[MtcEvent]) -> Vec<MtcTime> {
        ev.iter()
            .filter_map(|e| match e {
                MtcEvent::Time { time, .. } => Some(*time),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn quarter_frames_round_trip_with_two_frame_offset() {
        for rate in [MtcRate::Fps24, MtcRate::Fps25, MtcRate::Fps2997Df, MtcRate::Fps30] {
            let t = MtcTime { hours: 23, minutes: 58, seconds: 59, frames: 20, rate };
            let bytes: Vec<u8> = t.quarter_frames().iter().flatten().copied().collect();
            let ev = parse(&bytes);
            assert_eq!(ev.iter().filter(|e| matches!(e, MtcEvent::Piece { .. })).count(), 8);
            assert_eq!(times(&ev), vec![t.offset(2)], "{rate:?}");
        }
    }

    #[test]
    fn reverse_order() {
        let t = MtcTime { hours: 1, minutes: 2, seconds: 3, frames: 4, rate: MtcRate::Fps25 };
        let bytes: Vec<u8> = t.quarter_frames().iter().rev().flatten().copied().collect();
        let ev = parse(&bytes);
        assert_eq!(ev.last(), Some(&MtcEvent::Time { time: t, reverse: true }));
    }

    #[test]
    fn continuous_stream_gives_a_time_every_two_frames() {
        let mut bytes = Vec::new();
        for k in 0..10 {
            let t = MtcTime::from_frame_number(1000 + 2 * k, MtcRate::Fps30);
            bytes.extend(t.quarter_frames().iter().flatten());
        }
        let got: Vec<i64> = times(&parse(&bytes)).iter().map(MtcTime::frame_number).collect();
        let want: Vec<i64> = (0..10).map(|k| 1002 + 2 * k).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn starting_mid_sequence_waits_for_piece_zero() {
        let t = MtcTime::from_frame_number(500, MtcRate::Fps24);
        let q = t.quarter_frames();
        let mut bytes: Vec<u8> = q[3..].iter().flatten().copied().collect();
        bytes.extend(q.iter().flatten());
        assert_eq!(times(&parse(&bytes)), vec![t.offset(2)]);
    }

    #[test]
    fn out_of_order_piece_breaks_the_run() {
        let t = MtcTime::from_frame_number(500, MtcRate::Fps24);
        let q = t.quarter_frames();
        let mut bytes: Vec<u8> = Vec::new();
        for (i, m) in q.iter().enumerate() {
            if i != 4 {
                bytes.extend(m);
            }
        }
        assert!(times(&parse(&bytes)).is_empty());
    }

    #[test]
    fn full_frame_and_interleaved_realtime_bytes() {
        let t = MtcTime { hours: 10, minutes: 0, seconds: 0, frames: 0, rate: MtcRate::Fps2997Df };
        let mut bytes = vec![0xF8, 0x90, 0x40]; // clock, note on (data byte skipped)
        for b in t.full_frame() {
            bytes.push(b);
            bytes.push(0xFE); // active sensing inside the SysEx
        }
        assert_eq!(parse(&bytes), vec![MtcEvent::Full(t)]);
    }

    #[test]
    fn other_sysex_and_invalid_times_are_ignored() {
        assert!(parse(&[0xF0, 0x43, 0x10, 0x4C, 0x00, 0xF7]).is_empty());
        let mut bad = MtcTime { hours: 0, minutes: 0, seconds: 0, frames: 0, rate: MtcRate::Fps24 }.full_frame();
        bad[8] = 29; // frame 29 at 24 fps
        assert!(parse(&bad).is_empty());
        let long: Vec<u8> = std::iter::once(0xF0).chain(std::iter::repeat_n(0x01, 1000)).chain([0xF7]).collect();
        assert!(parse(&long).is_empty());
    }

    #[test]
    fn rate_mapping() {
        assert_eq!(MtcRate::nearest(FrameRate::FPS_29_97, true), (MtcRate::Fps2997Df, true));
        assert_eq!(MtcRate::nearest(FrameRate::FPS_29_97, false), (MtcRate::Fps30, false));
        assert_eq!(MtcRate::nearest(FrameRate::FPS_23_976, false), (MtcRate::Fps24, false));
        assert_eq!(MtcRate::nearest(FrameRate::FPS_25, false), (MtcRate::Fps25, true));
        assert_eq!(MtcRate::nearest(FrameRate { num: 0, den: 0 }, false).0, MtcRate::Fps24);
    }

    #[test]
    fn drop_frame_labels_and_wrap() {
        let t = MtcTime { hours: 0, minutes: 0, seconds: 59, frames: 29, rate: MtcRate::Fps2997Df };
        let n = t.offset(1);
        assert_eq!((n.minutes, n.seconds, n.frames), (1, 0, 2));
        let last = MtcTime { hours: 23, minutes: 59, seconds: 59, frames: 24, rate: MtcRate::Fps25 };
        assert_eq!(last.offset(1).frame_number(), 0);
    }

    #[test]
    fn hostile_bytes_never_panic() {
        let mut seed = 0x2545_F491u32;
        for _ in 0..200 {
            let bytes: Vec<u8> = (0..500)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect();
            let r = std::panic::catch_unwind(|| parse(&bytes));
            assert!(r.is_ok());
        }
    }
}
