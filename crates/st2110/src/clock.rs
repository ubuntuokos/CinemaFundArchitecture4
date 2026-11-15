//! Time for ST 2110 (CFA4-LIVE-STUDIO-001 ST 2110 addendum, phase T1): PTP time, the RTP media
//! clock aligned to the SMPTE Epoch, frame and field timestamps, and the reference-clock state that
//! `ptp4l` reports.
//!
//! - The SMPTE Epoch is the PTP epoch: 1970-01-01 00:00:00 TAI (ST 2059-1). A PTP time is the TAI
//!   time since then; Linux gives it as `CLOCK_TAI` once `phc2sys` disciplines the system clock.
//! - The media clock is `floor(t × rate)` and the RTP timestamp is its low 32 bits (ST 2110-10
//!   §8: `mediaclk:direct=0`). Every value is integer arithmetic, so a 59.94 Hz frame advances
//!   the 90 kHz clock by 1501 and 1502 in turn, never by a rounded 1501.5.
//! - [`PtpStatus::parse_pmc`] reads the text that `pmc` (linuxptp, D9) prints; CFA4 only reads
//!   the clock and its state, it never runs a PTP slave of its own. Without a usable PTP lock the
//!   sender signals its own timebase (`ts-refclk:localmac`, `mediaclk:sender`).
//!
//! No clock is read here (L0, no system calls); the studio host reads `CLOCK_TAI` and the
//! `pmc` output and passes them in.

use crate::Error;
use crate::sdp::{MediaClock, RefClock};
use crate::video::FrameRate;

const NANOS: u128 = 1_000_000_000;

/// The 90 kHz media clock of ST 2110-20 video and -40 ancillary data.
pub const VIDEO_CLOCK: u32 = 90_000;
/// The 48 kHz media clock of ST 2110-30 audio at 48 kHz.
pub const AUDIO_CLOCK_48K: u32 = 48_000;

/// UTC − TAI is −37 s since 2017-01-01 (the last leap second); `phc2sys` and the kernel use the
/// current value, this is only the fallback when the system does not report it.
pub const TAI_UTC_OFFSET_2017: i64 = 37;

/// A PTP (TAI) time since the SMPTE Epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PtpTime {
    pub seconds: u64,
    pub nanos: u32,
}

impl PtpTime {
    pub fn new(seconds: u64, nanos: u32) -> Result<Self, Error> {
        if u128::from(nanos) >= NANOS {
            return Err(Error::Invalid("nanoseconds must be below one second"));
        }
        Ok(PtpTime { seconds, nanos })
    }

    /// TAI from a UTC (Unix) time and the TAI − UTC offset in seconds (37 since 2017).
    pub fn from_unix(unix_seconds: u64, nanos: u32, tai_minus_utc: i64) -> Result<Self, Error> {
        let s = i128::from(unix_seconds) + i128::from(tai_minus_utc);
        let s = u64::try_from(s).map_err(|_| Error::Invalid("time before the SMPTE Epoch"))?;
        Self::new(s, nanos)
    }

    /// Nanoseconds since the SMPTE Epoch.
    pub fn as_nanos(self) -> u128 {
        u128::from(self.seconds) * NANOS + u128::from(self.nanos)
    }

    pub fn from_nanos(n: u128) -> Result<Self, Error> {
        let seconds = u64::try_from(n / NANOS).map_err(|_| Error::Invalid("time out of range"))?;
        Ok(PtpTime { seconds, nanos: (n % NANOS) as u32 })
    }

    /// The media clock value at this instant: `floor(t × rate)`, all 64+ bits.
    pub fn media_clock(self, rate: u32) -> u128 {
        self.as_nanos() * u128::from(rate) / NANOS
    }

    /// The RTP timestamp at this instant (the media clock's low 32 bits).
    pub fn rtp_timestamp(self, rate: u32) -> u32 {
        self.media_clock(rate) as u32
    }
}

/// Frames (or fields) since the SMPTE Epoch for one frame rate, and their exact times and RTP
/// timestamps. Frame `n` starts at `n × den / num` seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameClock {
    rate: FrameRate,
    interlaced: bool,
}

impl FrameClock {
    /// `interlaced`: the two fields of a frame are sampled half a frame apart and have their own
    /// timestamps (interlaced, not PsF; a PsF frame is sampled once).
    pub fn new(rate: FrameRate, interlaced: bool) -> Self {
        FrameClock { rate, interlaced }
    }

    /// Images (frames, or fields when interlaced) per `den` seconds.
    fn images_num(&self) -> u128 {
        u128::from(self.rate.num) * if self.interlaced { 2 } else { 1 }
    }

    /// The image index that has started at or before `t`.
    pub fn image_at(&self, t: PtpTime) -> u128 {
        t.as_nanos() * self.images_num() / (u128::from(self.rate.den) * NANOS)
    }

    /// The first image that starts at or after `t`.
    pub fn next_image(&self, t: PtpTime) -> u128 {
        let n = self.image_at(t);
        if self.image_start_nanos(n) == t.as_nanos() { n } else { n + 1 }
    }

    /// The image's start in nanoseconds since the Epoch, rounded down to the nanosecond.
    pub fn image_start_nanos(&self, n: u128) -> u128 {
        n * u128::from(self.rate.den) * NANOS / self.images_num()
    }

    /// The image's start as a PTP time (rounded down to the nanosecond).
    pub fn image_start(&self, n: u128) -> Result<PtpTime, Error> {
        PtpTime::from_nanos(self.image_start_nanos(n))
    }

    /// The media clock at the image's exact sampling instant: `floor(n × den × rate / num)`,
    /// computed from the image number, not from a rounded nanosecond time.
    pub fn media_clock(&self, n: u128, rate: u32) -> u128 {
        n * u128::from(self.rate.den) * u128::from(rate) / self.images_num()
    }

    /// The RTP timestamp of image `n`. When interlaced, even `n` are first fields and odd `n`
    /// second fields of frame `n / 2`.
    pub fn rtp_timestamp(&self, n: u128, rate: u32) -> u32 {
        self.media_clock(n, rate) as u32
    }
}

/// The PTP port state as `ptp4l` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortState {
    Initializing,
    Faulty,
    Disabled,
    Listening,
    PreMaster,
    Master,
    Passive,
    Uncalibrated,
    Slave,
    GrandMaster,
}

impl PortState {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "INITIALIZING" => PortState::Initializing,
            "FAULTY" => PortState::Faulty,
            "DISABLED" => PortState::Disabled,
            "LISTENING" => PortState::Listening,
            "PRE_MASTER" | "PRE_TIME_TRANSMITTER" => PortState::PreMaster,
            "MASTER" | "TIME_TRANSMITTER" => PortState::Master,
            "PASSIVE" => PortState::Passive,
            "UNCALIBRATED" => PortState::Uncalibrated,
            "SLAVE" | "TIME_RECEIVER" => PortState::Slave,
            "GRAND_MASTER" => PortState::GrandMaster,
            _ => return None,
        })
    }
}

/// What `ptp4l` reports about the reference clock, read from `pmc` answers to `GET
/// TIME_STATUS_NP`, `GET PARENT_DATA_SET`, `GET DEFAULT_DATA_SET` and `GET PORT_DATA_SET`.
/// Fields not present in the text stay `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtpStatus {
    pub gm_present: Option<bool>,
    /// The grandmaster's clock identity (EUI-64).
    pub grandmaster: Option<[u8; 8]>,
    pub domain: Option<u8>,
    /// The offset from the master in nanoseconds.
    pub master_offset_ns: Option<i64>,
    pub port_state: Option<PortState>,
    /// TAI − UTC in seconds as the grandmaster announces it (`currentUtcOffset`, `GET
    /// TIME_PROPERTIES_DATA_SET`), and whether it is valid.
    pub utc_offset: Option<i32>,
    pub utc_offset_valid: Option<bool>,
}

impl PtpStatus {
    /// Read `pmc` output (one or several responses). Unknown lines are ignored; a known field
    /// with a malformed value is an error, so a garbled answer is never taken for a lock.
    pub fn parse_pmc(text: &str) -> Result<Self, Error> {
        let mut s = PtpStatus::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(key), Some(value)) = (it.next(), it.next()) else { continue };
            if it.next().is_some() {
                continue; // header lines ("<id> seq 0 RESPONSE MANAGEMENT …")
            }
            let bad = || Error::Unsupported(format!("pmc {key} {value}"));
            match key {
                "gmPresent" => s.gm_present = Some(matches!(value, "true" | "1")),
                "gmIdentity" | "grandmasterIdentity" => s.grandmaster = Some(parse_clock_identity(value).ok_or_else(bad)?),
                "domainNumber" => s.domain = Some(value.parse().map_err(|_| bad())?),
                "master_offset" => s.master_offset_ns = Some(value.parse().map_err(|_| bad())?),
                "portState" => s.port_state = Some(PortState::parse(value).ok_or_else(bad)?),
                "currentUtcOffset" => s.utc_offset = Some(value.parse().map_err(|_| bad())?),
                "currentUtcOffsetValid" => s.utc_offset_valid = Some(matches!(value, "1" | "true")),
                _ => {}
            }
        }
        Ok(s)
    }

    /// TAI − UTC to use: the grandmaster's valid offset, else the 2017 value.
    pub fn tai_minus_utc(&self) -> i64 {
        match (self.utc_offset, self.utc_offset_valid) {
            (Some(o), Some(true)) => i64::from(o),
            _ => TAI_UTC_OFFSET_2017,
        }
    }

    /// Locked to a grandmaster: a slave port with a grandmaster present, its identity and
    /// domain known, and the offset within `max_offset_ns`.
    pub fn locked(&self, max_offset_ns: u64) -> bool {
        self.port_state == Some(PortState::Slave)
            && self.gm_present == Some(true)
            && self.grandmaster.is_some()
            && self.domain.is_some()
            && self.master_offset_ns.is_some_and(|o| o.unsigned_abs() <= max_offset_ns)
    }
}

/// `001122.fffe.334455` (the linuxptp form of a clock identity) as eight bytes.
pub fn parse_clock_identity(s: &str) -> Option<[u8; 8]> {
    let hex: String = s.chars().filter(|&c| c != '.').collect();
    if hex.len() != 16 || s.split('.').count() != 3 {
        return None;
    }
    let mut out = [0u8; 8];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The largest offset from the grandmaster that still counts as locked: 1 µs, well inside the
/// receive buffers of ST 2110-21 senders.
pub const DEFAULT_LOCK_LIMIT_NS: u64 = 1_000;

/// The SDP clock signalling for a sender: the grandmaster and `direct=0` when locked, otherwise
/// the sender's own timebase named by its MAC (ST 2110-10:2022).
pub fn sender_clocks(status: Option<&PtpStatus>, local_mac: [u8; 6], max_offset_ns: u64) -> (RefClock, MediaClock) {
    match status {
        Some(s) if s.locked(max_offset_ns) => match (s.grandmaster, s.domain) {
            (Some(grandmaster), Some(domain)) => (RefClock::Ptp { grandmaster, domain }, MediaClock::Direct(0)),
            _ => (RefClock::LocalMac(local_mac), MediaClock::Sender),
        },
        _ => (RefClock::LocalMac(local_mac), MediaClock::Sender),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(n: u32, d: u32) -> FrameRate {
        FrameRate::new(n, d).unwrap()
    }

    #[test]
    fn media_clock_is_floor_of_time_times_rate() {
        let t = PtpTime::new(1, 500_000_000).unwrap();
        assert_eq!(t.media_clock(VIDEO_CLOCK), 135_000);
        assert_eq!(PtpTime::new(0, 11_111).unwrap().media_clock(VIDEO_CLOCK), 0);
        assert_eq!(PtpTime::new(0, 11_112).unwrap().media_clock(VIDEO_CLOCK), 1);
        // 2^32 / 90000 s after the Epoch the RTP timestamp wraps.
        let wrap = PtpTime::from_nanos((1u128 << 32) * NANOS / 90_000 + 1).unwrap();
        assert_eq!(wrap.rtp_timestamp(VIDEO_CLOCK), 0);
        assert!(PtpTime::new(0, 1_000_000_000).is_err());
    }

    #[test]
    fn tai_from_unix() {
        let t = PtpTime::from_unix(1_700_000_000, 5, TAI_UTC_OFFSET_2017).unwrap();
        assert_eq!(t, PtpTime { seconds: 1_700_000_037, nanos: 5 });
        assert!(PtpTime::from_unix(0, 0, -1).is_err());
    }

    #[test]
    fn frame_59_94_steps_1501_and_1502() {
        let fc = FrameClock::new(rate(60000, 1001), false);
        // Over 2 frames the 90 kHz clock advances exactly 3003.
        let n0 = 1_000_000u128;
        let steps: Vec<u32> = (n0..n0 + 8).map(|n| fc.rtp_timestamp(n + 1, VIDEO_CLOCK).wrapping_sub(fc.rtp_timestamp(n, VIDEO_CLOCK))).collect();
        assert!(steps.iter().all(|&s| s == 1501 || s == 1502), "{steps:?}");
        assert!(steps.windows(2).all(|w| w[0] != w[1]), "alternates: {steps:?}");
        assert_eq!(fc.media_clock(2, VIDEO_CLOCK), 3003);
        // 29.97: 3003 every frame; 50: 1800.
        let fc = FrameClock::new(rate(30000, 1001), false);
        assert_eq!(fc.media_clock(7, VIDEO_CLOCK) - fc.media_clock(6, VIDEO_CLOCK), 3003);
        let fc = FrameClock::new(rate(50, 1), false);
        assert_eq!(fc.media_clock(3, VIDEO_CLOCK), 5400);
    }

    #[test]
    fn frame_alignment_to_the_epoch() {
        let fc = FrameClock::new(rate(50, 1), false);
        let t = PtpTime::new(10, 30_000_000).unwrap(); // 10.03 s: frame 501 started at 10.02 s
        assert_eq!(fc.image_at(t), 501);
        assert_eq!(fc.next_image(t), 502);
        assert_eq!(fc.image_start(502).unwrap(), PtpTime::new(10, 40_000_000).unwrap());
        let on = PtpTime::new(10, 40_000_000).unwrap();
        assert_eq!(fc.next_image(on), 502);
        // The timestamp of an image equals the media clock at its start.
        for n in [0u128, 1, 501, 123_456_789] {
            assert_eq!(fc.rtp_timestamp(n, VIDEO_CLOCK), fc.image_start(n).unwrap().rtp_timestamp(VIDEO_CLOCK));
        }
        // 59.94: the start rounded down to the nanosecond never moves the timestamp past the
        // exact one.
        let fc = FrameClock::new(rate(60000, 1001), false);
        for n in [1u128, 2, 3, 59_940, 1_000_001] {
            let exact = fc.media_clock(n, VIDEO_CLOCK);
            let from_time = fc.image_start(n).unwrap().media_clock(VIDEO_CLOCK);
            assert!(exact - from_time <= 1, "{n}: {exact} {from_time}");
            let start = fc.image_start(n).unwrap();
            assert!(fc.image_at(start) == n || fc.image_at(start) + 1 == n);
            assert_eq!(fc.next_image(start), n);
        }
    }

    #[test]
    fn interlaced_fields_are_half_a_frame_apart() {
        let fc = FrameClock::new(rate(25, 1), true); // 1080i50
        assert_eq!(fc.rtp_timestamp(1, VIDEO_CLOCK) - fc.rtp_timestamp(0, VIDEO_CLOCK), 1800);
        assert_eq!(fc.rtp_timestamp(2, VIDEO_CLOCK), 3600);
        let fc = FrameClock::new(rate(30000, 1001), true); // 1080i59.94
        let steps: Vec<u32> = (0..6).map(|n| fc.rtp_timestamp(n + 1, VIDEO_CLOCK) - fc.rtp_timestamp(n, VIDEO_CLOCK)).collect();
        assert_eq!(steps, vec![1501, 1502, 1501, 1502, 1501, 1502]);
    }

    // Built for these tests in the layout `pmc -u -b 0` prints (not copied from any device).
    const PMC: &str = "sending: GET TIME_STATUS_NP
\t3c2c30.fffe.a1b2c3-0 seq 0 RESPONSE MANAGEMENT TIME_STATUS_NP
\t\tmaster_offset              -87
\t\tingress_time               1762905600000000000
\t\tcumulativeScaledRateOffset +0.000000000
\t\tgmPresent                  true
\t\tgmIdentity                 0c4f5a.fffe.10e2d4
sending: GET DEFAULT_DATA_SET
\t3c2c30.fffe.a1b2c3-0 seq 1 RESPONSE MANAGEMENT DEFAULT_DATA_SET
\t\ttwoStepFlag             1
\t\tdomainNumber            127
sending: GET PORT_DATA_SET
\t3c2c30.fffe.a1b2c3-1 seq 2 RESPONSE MANAGEMENT PORT_DATA_SET
\t\tportIdentity            3c2c30.fffe.a1b2c3-1
\t\tportState               SLAVE
sending: GET TIME_PROPERTIES_DATA_SET
\t3c2c30.fffe.a1b2c3-0 seq 3 RESPONSE MANAGEMENT TIME_PROPERTIES_DATA_SET
\t\tcurrentUtcOffset      38
\t\tleap61                0
\t\tcurrentUtcOffsetValid 1
\t\tptpTimescale          1
";

    #[test]
    fn pmc_status_and_sdp_clocks() {
        let s = PtpStatus::parse_pmc(PMC).unwrap();
        assert_eq!(s.grandmaster, Some([0x0c, 0x4f, 0x5a, 0xff, 0xfe, 0x10, 0xe2, 0xd4]));
        assert_eq!((s.domain, s.master_offset_ns, s.port_state, s.gm_present), (Some(127), Some(-87), Some(PortState::Slave), Some(true)));
        assert!(s.locked(DEFAULT_LOCK_LIMIT_NS));
        assert!(!s.locked(50));
        assert_eq!(s.tai_minus_utc(), 38);
        assert_eq!(PtpStatus::default().tai_minus_utc(), 37);
        let mac = [0x02, 0, 0, 0, 0, 1];
        let (r, m) = sender_clocks(Some(&s), mac, DEFAULT_LOCK_LIMIT_NS);
        assert_eq!(r.to_value(), "ptp=IEEE1588-2008:0C-4F-5A-FF-FE-10-E2-D4:127");
        assert_eq!(m, MediaClock::Direct(0));

        // No PTP, a listening port, or a lost grandmaster: the sender's own timebase.
        for text in ["", &PMC.replace("SLAVE", "LISTENING"), &PMC.replace("gmPresent                  true", "gmPresent false")] {
            let s = PtpStatus::parse_pmc(text).unwrap();
            let (r, m) = sender_clocks(Some(&s), mac, DEFAULT_LOCK_LIMIT_NS);
            assert_eq!((r, m), (RefClock::LocalMac(mac), MediaClock::Sender));
        }
        assert_eq!(sender_clocks(None, mac, DEFAULT_LOCK_LIMIT_NS).1, MediaClock::Sender);
    }

    #[test]
    fn garbled_pmc_is_an_error() {
        assert!(PtpStatus::parse_pmc("\t\tgmIdentity 0c4f5a.fffe").is_err());
        assert!(PtpStatus::parse_pmc("\t\tdomainNumber 300").is_err());
        assert!(PtpStatus::parse_pmc("\t\tmaster_offset x").is_err());
        assert!(PtpStatus::parse_pmc("\t\tportState SLEEPING").is_err());
        assert_eq!(parse_clock_identity("0c4f5a.fffe.10e2d4"), Some([0x0c, 0x4f, 0x5a, 0xff, 0xfe, 0x10, 0xe2, 0xd4]));
        assert_eq!(parse_clock_identity("0c4f5afffe10e2d4"), None);
        assert_eq!(parse_clock_identity("0c4f5a.fffe.10e2dz"), None);
    }
}
