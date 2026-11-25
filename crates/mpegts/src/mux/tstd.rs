//! The transport stream system target decoder (T-STD, H.222.0 §2.4.2) as the muxer's admission
//! control: for every elementary stream a transport buffer TB (512 bytes, drained at Rx), then
//! for video a multiplexing buffer MB (drained at Rbx into) the elementary stream buffer EB, for
//! audio the main buffer B. The model is fluid at transport packet granularity: a packet enters
//! TB when it has arrived completely, leaves it at Rx, and its PES bytes are in MB / B when they
//! have left TB. An access unit is removed from EB / B at its decode time.
//!
//! Audio and video that are not modelled (unknown codec parameters) are sent as soon as the
//! multiplex clock allows.

use super::SYS_PER_90K;

/// Transport buffer size, bytes (§2.4.2.3).
const TBS: usize = 512;
/// A transport packet.
const TS_PACKET: usize = 188;

/// 27 MHz ticks to move `bytes` at `bps` bit/s.
pub(super) fn ticks_at(bytes: usize, bps: u64) -> i64 {
    i64::try_from(bytes as u128 * 8 * 27_000_000 / u128::from(bps.max(1))).unwrap_or(i64::MAX)
}

/// The T-STD buffers of a video stream (§2.4.2.3): `rx` is the rate TB is drained at, `mbs` the
/// size of MB, `rbx` the leak rate from MB to EB (the leak method; bit/s) and `ebs` the size of
/// EB, the decoder's VBV / CPB (bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoBuffers {
    pub rx: u64,
    pub mbs: usize,
    pub rbx: u64,
    pub ebs: usize,
}

/// `(Rmax bit/s, VBVmax bits, High-1440 / High level)` of the MPEG-2 profile_and_level_indication
/// (H.262 Tables 8-13 and 8-14): Main Profile at Low, Main, High-1440 and High level, the 4:2:2
/// Profile at Main and High level.
fn mpeg2_limits(pli: u8) -> Option<(u64, u64, bool)> {
    Some(match pli {
        0x4A => (4_000_000, 475_136, false),
        0x48 => (15_000_000, 1_835_008, false),
        0x46 => (60_000_000, 7_340_032, true),
        0x44 => (80_000_000, 9_781_248, true),
        0x85 => (50_000_000, 9_437_184, false),
        0x82 => (300_000_000, 47_185_920, true),
        _ => return None,
    })
}

impl VideoBuffers {
    /// MPEG-2 video of `profile_and_level` (0x48 Main Profile @ Main Level, …) whose sequence
    /// header signals a `vbv_buffer_bits` buffer and a `bit_rate` bit/s. `None` for a profile or
    /// level without a table here.
    pub fn mpeg2(profile_and_level: u8, vbv_buffer_bits: u64, bit_rate: u64) -> Option<VideoBuffers> {
        let (rmax, vbv_max, high) = mpeg2_limits(profile_and_level)?;
        // BSmux = 4 ms and BSoh = 1/750 s of Rmax
        let overhead_bits = rmax * 4 / 1000 + rmax / 750;
        let mbs_bits = if high { overhead_bits } else { overhead_bits + vbv_max.saturating_sub(vbv_buffer_bits) };
        // High-1440 and High level: the leak is the signalled rate plus 5 %, at most Rmax
        let rbx = if high { (bit_rate + bit_rate / 20).min(rmax) } else { rmax };
        Some(VideoBuffers {
            rx: rmax + rmax / 5,
            mbs: usize::try_from(mbs_bits / 8).unwrap_or(usize::MAX),
            rbx,
            ebs: usize::try_from(vbv_buffer_bits / 8).unwrap_or(usize::MAX),
        })
    }

    /// H.264 video of `level_idc` (§2.14.3.1): `cpb_bits` is the CpbSize of the NAL HRD
    /// parameters, or `None` for the level's 1200 × MaxCPB. `None` for a level without a table.
    pub fn avc(level_idc: u8, cpb_bits: Option<u64>) -> Option<VideoBuffers> {
        // (MaxBR in 1000 bit/s, MaxCPB in 1000 bits), Table A-1
        let (max_br, max_cpb): (u64, u64) = match level_idc {
            9 | 10 => (64, 175),
            11 => (192, 500),
            12 => (384, 1_000),
            13 => (768, 2_000),
            20 => (2_000, 2_000),
            21 | 22 => (4_000, 4_000),
            30 => (10_000, 10_000),
            31 => (14_000, 14_000),
            32 => (20_000, 20_000),
            40 => (20_000, 25_000),
            41 | 42 => (50_000, 62_500),
            50 => (135_000, 135_000),
            51 | 52 | 60 | 61 | 62 => (240_000, 240_000),
            _ => return None,
        };
        let br = 1200 * max_br;
        let level_cpb = 1200 * max_cpb;
        let cpb = cpb_bits.unwrap_or(level_cpb);
        let busy = br.max(2_000_000);
        let mbs_bits = busy * 4 / 1000 + busy / 750 + level_cpb.saturating_sub(cpb);
        Some(VideoBuffers {
            rx: br + br / 5,
            mbs: usize::try_from(mbs_bits / 8).unwrap_or(usize::MAX),
            rbx: br,
            ebs: usize::try_from(cpb / 8).unwrap_or(usize::MAX),
        })
    }
}

/// What follows the transport buffer of a stream.
pub(super) enum Kind {
    /// Not modelled.
    Free,
    /// Audio main buffer B: size in bytes and what it holds as (removal time, bytes).
    Audio { bs: usize, held: Vec<(i64, usize)> },
    /// Video MB and EB.
    Video { buffers: VideoBuffers, mb: Vec<(i64, usize)>, mb_free_at: i64, held: Vec<(i64, usize)> },
}

/// The T-STD buffers of one elementary stream.
pub(super) struct Model {
    /// Rate TB is drained at, bit/s.
    rx: u64,
    /// When TB is empty (27 MHz).
    tb_free_at: i64,
    pub kind: Kind,
}

/// When a packet can go out and when its bytes reach the main buffer / EB.
#[derive(Clone, Copy, Debug)]
pub(super) struct Plan {
    /// Start of the packet (27 MHz).
    pub start: i64,
    /// Its bytes are out of TB.
    pub left_tb: i64,
    /// Video: its payload is in EB.
    pub in_eb: i64,
}

impl Model {
    pub fn free() -> Self {
        Model { rx: 2_000_000, tb_free_at: 0, kind: Kind::Free }
    }

    /// Audio: TB drained at `rx` bit/s, a main buffer of `bs` bytes.
    pub fn audio(rx: u64, bs: usize) -> Self {
        Model { rx, tb_free_at: 0, kind: Kind::Audio { bs, held: Vec::new() } }
    }

    pub fn video(b: VideoBuffers) -> Self {
        Model { rx: b.rx, tb_free_at: 0, kind: Kind::Video { buffers: b, mb: Vec::new(), mb_free_at: 0, held: Vec::new() } }
    }

    /// The earliest start at or after `start` of a transport packet of `wire` bytes (a BDAV
    /// packet is 4 more than the 188 TB holds) carrying `payload` PES bytes of a unit that is
    /// removed at `removal`.
    pub fn plan(&self, start: i64, wire_ticks: i64, payload: usize, removal: i64) -> Plan {
        if matches!(self.kind, Kind::Free) {
            return Plan { start, left_tb: start + wire_ticks, in_eb: start + wire_ticks };
        }
        let mut s = start;
        // each pass moves the start later, or leaves it
        loop {
            let arrived = s + wire_ticks;
            // TB must hold the packet beside what has not drained yet
            let tb_ok = self.tb_free_at - ticks_at(TBS - TS_PACKET, self.rx);
            if arrived < tb_ok {
                s = tb_ok - wire_ticks;
                continue;
            }
            let left = arrived.max(self.tb_free_at) + ticks_at(TS_PACKET, self.rx);
            match &self.kind {
                Kind::Audio { bs, held } => {
                    let used: usize = held.iter().filter(|e| e.0 > left).map(|e| e.1).sum();
                    if used + payload > *bs
                        && let Some(t) = held.iter().map(|e| e.0).filter(|&t| t > left && t < removal).min()
                    {
                        // wait until an earlier unit has been decoded
                        s += t - left;
                        continue;
                    }
                    return Plan { start: s, left_tb: left, in_eb: left };
                }
                Kind::Video { buffers, mb, mb_free_at, held } => {
                    let in_mb: usize = mb.iter().filter(|e| e.0 > left).map(|e| e.1).sum();
                    if in_mb + payload > buffers.mbs
                        && let Some(t) = mb.iter().map(|e| e.0).filter(|&t| t > left).min()
                    {
                        s += t - left;
                        continue;
                    }
                    // MB leaks into EB at Rbx while EB has room
                    let mut in_eb = left.max(*mb_free_at) + ticks_at(payload, buffers.rbx);
                    loop {
                        let used: usize = held.iter().filter(|e| e.0 > in_eb).map(|e| e.1).sum();
                        if used + payload <= buffers.ebs {
                            break;
                        }
                        match held.iter().map(|e| e.0).filter(|&t| t > in_eb && t < removal).min() {
                            Some(t) => in_eb = t,
                            None => break,
                        }
                    }
                    return Plan { start: s, left_tb: left, in_eb };
                }
                Kind::Free => return Plan { start: s, left_tb: left, in_eb: left },
            }
        }
    }

    /// Book a packet planned by [`Model::plan`]: `payload` PES bytes of a unit removed at `removal`.
    pub fn commit(&mut self, plan: &Plan, payload: usize, removal: i64) {
        if matches!(self.kind, Kind::Free) {
            return;
        }
        self.tb_free_at = plan.left_tb;
        let book = |held: &mut Vec<(i64, usize)>, at: i64| {
            held.retain(|e| e.0 > at);
            if payload > 0 {
                match held.last_mut() {
                    Some(last) if last.0 == removal => last.1 += payload,
                    _ => held.push((removal, payload)),
                }
            }
        };
        match &mut self.kind {
            Kind::Audio { held, .. } => book(held, plan.left_tb),
            Kind::Video { mb, mb_free_at, held, .. } => {
                mb.retain(|e| e.0 > plan.left_tb);
                if payload > 0 {
                    mb.push((plan.in_eb, payload));
                    *mb_free_at = plan.in_eb;
                }
                book(held, plan.in_eb);
            }
            Kind::Free => {}
        }
    }
}

/// 27 MHz time of a 90 kHz timestamp.
pub(super) fn sys(t90k: i64) -> i64 {
    t90k * SYS_PER_90K
}
