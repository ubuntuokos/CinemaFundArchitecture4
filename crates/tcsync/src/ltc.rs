//! SMPTE linear timecode (LTC): frame layout, biphase-mark encoder and decoder.
//!
//! An LTC frame is 80 bits, sent least significant bit first, one frame per video frame:
//!
//! | bits | content |
//! |---|---|
//! | 0–3, 8–9 | frame units, frame tens (BCD) |
//! | 10, 11 | drop-frame flag, colour-frame flag |
//! | 16–19, 24–26 | seconds units, tens |
//! | 32–35, 40–42 | minutes units, tens |
//! | 48–51, 56–57 | hours units, tens |
//! | 4–7, 12–15, 20–23, 28–31, 36–39, 44–47, 52–55, 60–63 | user-bit groups 1–8 |
//! | 27, 43, 58, 59 | polarity-correction bit and binary-group flags (placement differs at 25 fps) |
//! | 64–79 | sync word `0011 1111 1111 1101` |
//!
//! Biphase-mark coding: the signal changes level at the start of every bit cell, and a `1` has a
//! second change in the middle of the cell. The level itself carries no information, so the signal
//! can be inverted, and the sync word tells the reader the direction of play.
//!
//! The reader ignores bits 27, 43, 58 and 59 and does not trust the drop-frame and colour-frame
//! flags to identify the frame rate (some generators leave them uninitialised); the writer zeroes
//! the binary-group flags and sets the polarity-correction bit so every frame holds an even number
//! of zeros, which keeps each frame starting at the same level.

use cfa4_time::{FrameRate, fields_to_frames, frames_to_fields};

/// Bits in one LTC frame.
pub const BITS_PER_FRAME: usize = 80;

/// The sync word, bits 64..=79 in transmission order.
const SYNC_BITS: [u8; 16] = [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1];

const MASK_80: u128 = (1u128 << BITS_PER_FRAME) - 1;

/// Sync word as it sits in the low 16 bits of the shift register after forward play (the newest
/// bit, bit 79, at position 0).
const fn sync_forward() -> u128 {
    let mut v = 0u128;
    let mut j = 0;
    while j < 16 {
        // Bit 64 + j sits at register position 79 - (64 + j) = 15 - j.
        v |= (SYNC_BITS[j] as u128) << (15 - j);
        j += 1;
    }
    v
}

/// Sync word as it sits in register positions 64..=79 after reverse play (bit k at position k).
const fn sync_reverse() -> u128 {
    let mut v = 0u128;
    let mut j = 0;
    while j < 16 {
        v |= (SYNC_BITS[j] as u128) << j;
        j += 1;
    }
    v
}

const SYNC_FWD: u128 = sync_forward();
const SYNC_REV: u128 = sync_reverse();

/// Errors from configuring the encoder or decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LtcError {
    /// The sample rate is outside 8 kHz..=768 kHz.
    SampleRate(u32),
    /// The frame rate is not positive or not in the LTC range (about 1..=60 frames per second).
    FrameRate(FrameRate),
}

impl std::fmt::Display for LtcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LtcError::SampleRate(r) => write!(f, "LTC: unsupported sample rate {r} Hz (8000..=768000)"),
            LtcError::FrameRate(r) => write!(f, "LTC: unsupported frame rate {}/{}", r.num, r.den),
        }
    }
}

impl std::error::Error for LtcError {}

const MIN_SAMPLE_RATE: u32 = 8_000;
const MAX_SAMPLE_RATE: u32 = 768_000;

fn check_sample_rate(sample_rate: u32) -> Result<(), LtcError> {
    if (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) { Ok(()) } else { Err(LtcError::SampleRate(sample_rate)) }
}

/// One LTC frame's content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LtcFrame {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub drop_frame: bool,
    pub color_frame: bool,
    /// The 32 user bits: nibble `i` (bits `4i..4i+4`) is user-bit group `i + 1`.
    pub user_bits: u32,
}

/// Placement of the polarity-correction bit and the binary-group flags.
fn flag_bits(rate: FrameRate) -> (usize, [usize; 3]) {
    if rate.sane().timecode_base() == 25 {
        // 25 fps: polarity 59; BGF0 27, BGF1 58, BGF2 43.
        (59, [27, 58, 43])
    } else {
        // 24 / 30 fps: polarity 27; BGF0 43, BGF1 58, BGF2 59.
        (27, [43, 58, 59])
    }
}

/// Frames in 24 hours of timecode at `rate`.
fn frames_per_day(rate: FrameRate, drop_frame: bool) -> i64 {
    fields_to_frames(24, 0, 0, 0, rate, drop_frame).max(1)
}

const USER_GROUP_START: [usize; 8] = [4, 12, 20, 28, 36, 44, 52, 60];

impl LtcFrame {
    /// The frame label for frame count `frame` (wrapped into one 24-hour day, so negative counts
    /// and counts past 24 hours stay valid) at `rate`, drop-frame where the rate allows it.
    pub fn from_frame_number(frame: i64, rate: FrameRate, drop_frame: bool) -> LtcFrame {
        let rate = rate.sane();
        let df = drop_frame && rate.supports_drop_frame();
        let day = frames_per_day(rate, df);
        let n = frame.rem_euclid(day);
        let (_, h, m, s, f) = frames_to_fields(n, rate, df);
        LtcFrame {
            hours: u8::try_from(h.clamp(0, 23)).unwrap_or(0),
            minutes: u8::try_from(m.clamp(0, 59)).unwrap_or(0),
            seconds: u8::try_from(s.clamp(0, 59)).unwrap_or(0),
            frames: u8::try_from(f.clamp(0, 39)).unwrap_or(0),
            drop_frame: df,
            color_frame: false,
            user_bits: 0,
        }
    }

    /// The frame count of this label at `rate` (drop-frame counting if the flag is set and the rate
    /// allows it).
    pub fn frame_number(&self, rate: FrameRate) -> i64 {
        let rate = rate.sane();
        fields_to_frames(
            i64::from(self.hours),
            i64::from(self.minutes),
            i64::from(self.seconds),
            i64::from(self.frames),
            rate,
            self.drop_frame && rate.supports_drop_frame(),
        )
    }

    /// Whether every field fits its BCD field and the clock ranges.
    pub fn is_valid(&self) -> bool {
        self.hours < 24 && self.minutes < 60 && self.seconds < 60 && self.frames < 40
    }

    /// The 80 frame bits (bit `k` at position `k`) for transmission at `rate`, with the
    /// binary-group flags zeroed and the polarity-correction bit set for an even number of zeros.
    /// Out-of-range fields are clamped to the largest value their BCD field can hold.
    pub fn to_bits(&self, rate: FrameRate) -> u128 {
        let mut v = 0u128;
        let mut put = |start: usize, width: usize, value: u32| {
            let mask = (1u32 << width) - 1;
            v |= u128::from(value & mask) << start;
        };
        let bcd = |x: u8, tens_max: u8| (u32::from(x % 10), u32::from((x / 10).min(tens_max)));
        let (fu, ft) = bcd(self.frames, 3);
        let (su, st) = bcd(self.seconds, 7);
        let (mu, mt) = bcd(self.minutes, 7);
        let (hu, ht) = bcd(self.hours, 3);
        put(0, 4, fu);
        put(8, 2, ft);
        put(10, 1, u32::from(self.drop_frame));
        put(11, 1, u32::from(self.color_frame));
        put(16, 4, su);
        put(24, 3, st);
        put(32, 4, mu);
        put(40, 3, mt);
        put(48, 4, hu);
        put(56, 2, ht);
        for (i, &start) in USER_GROUP_START.iter().enumerate() {
            put(start, 4, (self.user_bits >> (4 * i)) & 0xF);
        }
        for (j, &b) in SYNC_BITS.iter().enumerate() {
            put(64 + j, 1, u32::from(b));
        }
        let (polarity, _) = flag_bits(rate);
        if v.count_ones() % 2 == 1 {
            v |= 1u128 << polarity;
        }
        v
    }

    /// Reads a frame from its 80 bits (bit `k` at position `k`). Returns `None` when a BCD digit
    /// or a field is out of range (noise that happened to contain a sync word). Bits 27, 43, 58
    /// and 59 are ignored.
    pub fn from_bits(bits: u128) -> Option<LtcFrame> {
        let get = |start: usize, width: usize| -> u8 { ((bits >> start) & ((1u128 << width) - 1)) as u8 };
        let digits = [get(0, 4), get(16, 4), get(32, 4), get(48, 4)];
        if digits.iter().any(|&d| d > 9) {
            return None;
        }
        let mut user_bits = 0u32;
        for (i, &start) in USER_GROUP_START.iter().enumerate() {
            user_bits |= u32::from(get(start, 4)) << (4 * i);
        }
        let f = LtcFrame {
            frames: get(8, 2) * 10 + digits[0],
            seconds: get(24, 3) * 10 + digits[1],
            minutes: get(40, 3) * 10 + digits[2],
            hours: get(56, 2) * 10 + digits[3],
            drop_frame: get(10, 1) == 1,
            color_frame: get(11, 1) == 1,
            user_bits,
        };
        f.is_valid().then_some(f)
    }
}

/// LTC audio generator: biphase-mark coding with a band-limited edge (about 25 µs rise time).
///
/// Bit timing is computed from a running half-bit counter in exact integer arithmetic, so the
/// signal never drifts from `rate` however long it runs (29.97 fps LTC lasts exactly 1001/30000 s
/// per frame).
#[derive(Clone, Debug)]
pub struct LtcEncoder {
    sample_rate: u32,
    rate: FrameRate,
    amplitude: f32,
    /// Half-bit cells emitted since the start.
    half_bits: u64,
    /// Samples emitted since the start.
    samples: u64,
    high: bool,
    filtered: f32,
    alpha: f32,
}

/// Rise-time filter time constant: a first-order low-pass with τ ≈ 25 µs / 2.2 gives the
/// standard 10–90 % rise time of about 25 µs.
const RISE_TAU_S: f64 = 25e-6 / 2.2;

impl LtcEncoder {
    /// A generator at `sample_rate` Hz for `rate` frames per second, peak level `level_dbfs`
    /// (clamped to -60..=0 dBFS; -18 dBFS is a usual line-level setting).
    pub fn new(sample_rate: u32, rate: FrameRate, level_dbfs: f32) -> Result<Self, LtcError> {
        check_sample_rate(sample_rate)?;
        if rate.num <= 0 || rate.den <= 0 || rate.as_f64() > 61.0 || rate.as_f64() < 1.0 {
            return Err(LtcError::FrameRate(rate));
        }
        let db = if level_dbfs.is_finite() { level_dbfs.clamp(-60.0, 0.0) } else { -18.0 };
        let alpha = 1.0 - (-1.0 / (f64::from(sample_rate) * RISE_TAU_S)).exp();
        Ok(LtcEncoder { sample_rate, rate, amplitude: 10f32.powf(db / 20.0), half_bits: 0, samples: 0, high: false, filtered: 0.0, alpha: alpha as f32 })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn frame_rate(&self) -> FrameRate {
        self.rate
    }

    /// Sample index at which half-bit cell `h` starts.
    fn boundary(&self, h: u64) -> u64 {
        // h * sample_rate * den / (num * 160), exact in 128 bits.
        let num = u128::from(self.rate.num.unsigned_abs()).max(1);
        let den = u128::from(self.rate.den.unsigned_abs());
        let v = u128::from(h) * u128::from(self.sample_rate) * den / (num * 2 * BITS_PER_FRAME as u128);
        u64::try_from(v).unwrap_or(u64::MAX)
    }

    /// Appends the samples of one frame to `out`.
    pub fn encode(&mut self, frame: &LtcFrame, out: &mut Vec<f32>) {
        let bits = frame.to_bits(self.rate);
        for k in 0..BITS_PER_FRAME {
            let one = (bits >> k) & 1 == 1;
            for half in 0..2 {
                if half == 0 || one {
                    self.high = !self.high;
                }
                let h = self.half_bits;
                let end = self.boundary(h.saturating_add(1));
                let target = if self.high { self.amplitude } else { -self.amplitude };
                while self.samples < end {
                    self.filtered += self.alpha * (target - self.filtered);
                    out.push(self.filtered);
                    self.samples += 1;
                }
                self.half_bits = h.saturating_add(1);
            }
        }
    }
}

/// A frame read by [`LtcDecoder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LtcDecoded {
    pub frame: LtcFrame,
    /// The frame was played backwards.
    pub reverse: bool,
    /// Sample index (since the decoder started) where the frame's first received bit began.
    pub start: u64,
    /// Sample index where its last received bit ended.
    pub end: u64,
}

impl LtcDecoded {
    /// Frame length in samples; with the nominal length this gives the play speed.
    pub fn duration(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

/// LTC reader: finds the level changes, sorts their spacing into full and half bit cells with an
/// adaptive bit length, and looks for the sync word in both directions.
///
/// It needs no frame rate: it follows any speed from about 0.1× to 10× of the 24–30 fps rates.
#[derive(Clone, Debug)]
pub struct LtcDecoder {
    sample_rate: u32,
    /// Samples consumed.
    pos: u64,
    peak: f32,
    decay: f32,
    /// Current level: -1, +1, or 0 before the first edge.
    sign: i8,
    last_edge: Option<u64>,
    /// Estimated length of a full bit cell in samples (0 = unknown).
    period: f64,
    /// Start of a half-bit pair waiting for its second half.
    half_start: Option<u64>,
    reg: u128,
    starts: [u64; BITS_PER_FRAME],
    head: usize,
    /// Bits received since the last frame, sync loss or reset (capped).
    bits_seen: usize,
    /// Longest edge spacing still read as signal, in samples.
    max_interval: u64,
}

/// Quietest level still read as signal (-60 dBFS).
const NOISE_FLOOR: f32 = 0.001;
/// Hysteresis threshold as a fraction of the tracked peak.
const HYSTERESIS: f32 = 0.25;

impl LtcDecoder {
    pub fn new(sample_rate: u32) -> Result<Self, LtcError> {
        check_sample_rate(sample_rate)?;
        // Peak tracker halves in 20 ms.
        let decay = 0.5f64.powf(1.0 / (0.02 * f64::from(sample_rate))) as f32;
        // Slowest signal: 24 fps at 0.1× speed = 192 bit/s; allow some margin.
        let max_interval = u64::from(sample_rate) / 150;
        Ok(LtcDecoder {
            sample_rate,
            pos: 0,
            peak: 0.0,
            decay,
            sign: 0,
            last_edge: None,
            period: 0.0,
            half_start: None,
            reg: 0,
            starts: [0; BITS_PER_FRAME],
            head: 0,
            bits_seen: 0,
            max_interval,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Forgets the bit timing and partial frame (after a device change or a gap).
    pub fn reset(&mut self) {
        self.sign = 0;
        self.last_edge = None;
        self.period = 0.0;
        self.half_start = None;
        self.bits_seen = 0;
        self.peak = 0.0;
    }

    /// Reads `samples` (one channel) and appends each frame completed in them to `out`.
    /// Non-finite samples count as silence.
    pub fn decode(&mut self, samples: &[f32], out: &mut Vec<LtcDecoded>) {
        for &x in samples {
            let x = if x.is_finite() { x } else { 0.0 };
            self.peak = (self.peak * self.decay).max(x.abs());
            let th = (self.peak * HYSTERESIS).max(NOISE_FLOOR);
            let new_sign = if x > th {
                1
            } else if x < -th {
                -1
            } else {
                self.sign
            };
            if new_sign != self.sign && new_sign != 0 {
                if self.sign != 0 {
                    self.edge(self.pos, out);
                }
                self.sign = new_sign;
            }
            self.pos = self.pos.saturating_add(1);
        }
    }

    fn edge(&mut self, at: u64, out: &mut Vec<LtcDecoded>) {
        let Some(prev) = self.last_edge.replace(at) else {
            return;
        };
        let iv = at.saturating_sub(prev);
        if iv == 0 || iv > self.max_interval {
            self.lose_sync();
            return;
        }
        let ivf = iv as f64;
        let long = if self.period <= 0.0 || ivf > 1.5 * self.period {
            self.period = ivf;
            true
        } else if ivf >= 0.75 * self.period {
            self.period = 0.875 * self.period + 0.125 * ivf;
            true
        } else {
            self.period = 0.875 * self.period + 0.125 * 2.0 * ivf;
            false
        };
        if long {
            if self.half_start.take().is_some() {
                // A lone half cell: the bit boundary was lost.
                self.bits_seen = 0;
            }
            self.push_bit(false, prev, at, out);
        } else if let Some(start) = self.half_start.take() {
            self.push_bit(true, start, at, out);
        } else {
            self.half_start = Some(prev);
        }
    }

    fn lose_sync(&mut self) {
        self.period = 0.0;
        self.half_start = None;
        self.bits_seen = 0;
    }

    /// Whether all 80 bit cells of the frame just completed (ending at `end`) have about the same
    /// length. Rejects frames assembled from a misread cell, above all the first frame after the
    /// decoder started or lost sync, before the bit length was known.
    fn uniform_cells(&self, end: u64) -> bool {
        let first = self.starts.get(self.head).copied().unwrap_or(end);
        let avg = end.saturating_sub(first) as f64 / BITS_PER_FRAME as f64;
        if avg <= 0.0 {
            return false;
        }
        (0..BITS_PER_FRAME).all(|i| {
            let start = self.starts.get((self.head + i) % BITS_PER_FRAME).copied().unwrap_or(0);
            let next = if i + 1 == BITS_PER_FRAME { end } else { self.starts.get((self.head + i + 1) % BITS_PER_FRAME).copied().unwrap_or(end) };
            let len = next.saturating_sub(start) as f64;
            (0.6 * avg..=1.4 * avg).contains(&len)
        })
    }

    fn push_bit(&mut self, one: bool, start: u64, end: u64, out: &mut Vec<LtcDecoded>) {
        self.reg = ((self.reg << 1) | u128::from(one)) & MASK_80;
        if let Some(slot) = self.starts.get_mut(self.head) {
            *slot = start;
        }
        self.head = (self.head + 1) % BITS_PER_FRAME;
        self.bits_seen = (self.bits_seen + 1).min(BITS_PER_FRAME);
        if self.bits_seen < BITS_PER_FRAME {
            return;
        }
        let first_start = self.starts.get(self.head).copied().unwrap_or(start);
        let found = if self.reg & 0xFFFF == SYNC_FWD {
            // Forward: register position i holds bit 79 - i.
            let mut bits = 0u128;
            for k in 0..BITS_PER_FRAME {
                bits |= ((self.reg >> (BITS_PER_FRAME - 1 - k)) & 1) << k;
            }
            Some((bits, false))
        } else if (self.reg >> 64) & 0xFFFF == SYNC_REV {
            // Reverse: register position k holds bit k.
            Some((self.reg, true))
        } else {
            None
        };
        if let Some((bits, reverse)) = found {
            self.bits_seen = 0;
            if !self.uniform_cells(end) {
                return;
            }
            if let Some(frame) = LtcFrame::from_bits(bits) {
                out.push(LtcDecoded { frame, reverse, start: first_start, end });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_constants() {
        // Forward: newest bit (79 = 1) at position 0, bit 78 (= 0) at position 1.
        assert_eq!(SYNC_FWD & 1, 1);
        assert_eq!((SYNC_FWD >> 1) & 1, 0);
        assert_eq!(SYNC_FWD >> 14, 0);
        assert_eq!(SYNC_REV & 0b11, 0);
        assert_eq!(SYNC_REV >> 15, 1);
    }

    #[test]
    fn bits_round_trip_and_even_zeros() {
        let f = LtcFrame { hours: 23, minutes: 59, seconds: 58, frames: 29, drop_frame: true, color_frame: true, user_bits: 0xDEAD_BEEF };
        for rate in [FrameRate::FPS_24, FrameRate::FPS_25, FrameRate::FPS_29_97, FrameRate::FPS_30] {
            let bits = f.to_bits(rate);
            assert_eq!(bits.count_ones() % 2, 0, "{rate}");
            assert_eq!(LtcFrame::from_bits(bits), Some(f));
        }
    }

    #[test]
    fn frame_numbers_wrap_into_one_day() {
        let r = FrameRate::FPS_25;
        assert_eq!(LtcFrame::from_frame_number(-1, r, false), LtcFrame::from_frame_number(24 * 3600 * 25 - 1, r, false));
        let f = LtcFrame::from_frame_number(24 * 3600 * 25 - 1, r, false);
        assert_eq!((f.hours, f.minutes, f.seconds, f.frames), (23, 59, 59, 24));
        // Drop-frame: 00:00:59;29 is followed by 00:01:00;02.
        let r = FrameRate::FPS_29_97;
        let n = LtcFrame { minutes: 0, seconds: 59, frames: 29, drop_frame: true, ..Default::default() }.frame_number(r);
        let next = LtcFrame::from_frame_number(n + 1, r, true);
        assert_eq!((next.minutes, next.seconds, next.frames, next.drop_frame), (1, 0, 2, true));
    }

    #[test]
    fn invalid_bcd_is_rejected() {
        let mut bits = LtcFrame::default().to_bits(FrameRate::FPS_25);
        bits |= 0xA; // frame units = 10
        assert_eq!(LtcFrame::from_bits(bits), None);
    }

    #[test]
    fn bad_configuration_is_an_error() {
        assert!(LtcEncoder::new(0, FrameRate::FPS_25, -18.0).is_err());
        assert!(LtcEncoder::new(48_000, FrameRate { num: 0, den: 1 }, -18.0).is_err());
        assert!(LtcEncoder::new(48_000, FrameRate::FPS_120, -18.0).is_err());
        assert!(LtcDecoder::new(1_000_000).is_err());
        assert!(LtcEncoder::new(48_000, FrameRate::FPS_25, f32::NAN).is_ok());
    }
}
