//! MPEG-2 Systems multiplexer (H.222.0 §2.4 / §2.5): transport streams (188-byte packets, or
//! 192-byte BDAV packets for `.m2ts`) and MPEG-2 program streams.
//!
//! - Access units are queued and written in decode-time order across streams, so the caller may
//!   hand over a group of video pictures and then the audio of the same span.
//! - A system clock is modelled at the given multiplex rate: data goes out as early as the rate
//!   allows but never more than [`Muxer::DELAY`] before its decode time. The SCR / PCR (and the
//!   BDAV arrival timestamps) follow that clock, so they increase and precede the timestamps of the
//!   data they carry.
//! - Timestamps are 90 kHz from the start of the stream; the muxer adds [`Muxer::DELAY`] so the
//!   first SCR / PCR is zero and an audio stream starting slightly before zero (encoder priming)
//!   stays positive.
//! - Transport streams: a PAT and a PMT (program 1) at the start, at every video random-access
//!   point and at least every 100 ms; a PCR on the first stream's PID every 40 ms at most;
//!   continuity counters; stuffing in adaptation fields.
//! - Program streams: 2048-byte packs at most (a pack header before every PES packet, the system
//!   header in the first), the program end code at the end; DVD LPCM in private stream 1.

use std::io::Write;

use crate::ts::crc32_mpeg;

/// What a stream carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuxCodec {
    /// MPEG-1 / MPEG-2 video (ES pictures with their sequence / GOP headers).
    Mpeg2Video,
    /// AAC in ADTS frames (transport streams only).
    AacAdts,
    /// 16-bit big-endian interleaved DVD LPCM (program streams only), 1-8 channels at 48, 96, 44.1
    /// or 32 kHz.
    LpcmDvd { sample_rate: u32, channels: u32 },
}

/// The container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuxFormat {
    /// 188-byte transport stream packets.
    Ts,
    /// 192-byte BDAV packets: a 4-byte arrival timestamp before each transport packet.
    M2ts,
    /// An MPEG-2 program stream.
    Ps,
}

#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Unsupported(String),
}

pub type MuxResult<T> = std::result::Result<T, MuxError>;

const PAT_PID: u16 = 0;
const PMT_PID: u16 = 0x1000;
const FIRST_ES_PID: u16 = 0x100;
/// 27 MHz system clock ticks per 90 kHz tick.
const SYS_PER_90K: i64 = 300;
/// Largest program stream pack.
const PACK_SIZE: usize = 2048;
/// Most queued bytes before the oldest unit goes out regardless of the other streams.
const MAX_QUEUED: usize = 64 << 20;

/// One queued access unit.
struct Unit {
    stream: usize,
    data: Vec<u8>,
    /// 90 kHz, [`Muxer::DELAY`] already added.
    pts: i64,
    dts: i64,
    key: bool,
}

struct StreamState {
    codec: MuxCodec,
    pid: u16,
    cc: u8,
    /// Decode time of the last unit queued (with the delay), `None` before the first.
    last_dts: Option<i64>,
}

/// A transport or program stream being written.
pub struct Muxer<W: Write> {
    out: W,
    format: MuxFormat,
    streams: Vec<StreamState>,
    /// Bytes per second of the multiplex (the clock's pace).
    rate: u64,
    queue: Vec<Unit>,
    queued_bytes: usize,
    /// The system clock (27 MHz) at the next byte.
    clock: i64,
    last_psi: Option<i64>,
    last_pcr: Option<i64>,
    psi_cc: u8,
    /// Program stream packs written.
    packs: u64,
}

impl<W: Write> Muxer<W> {
    /// Decode times are offset by this much (90 kHz: 0.7 s), and data goes out at most this long
    /// before its decode time.
    pub const DELAY: i64 = 63_000;

    /// Start a stream with `streams` (in PID / stream-id order) at `mux_rate` bits per second, the
    /// peak rate of everything together (transport or pack overhead included).
    pub fn new(out: W, format: MuxFormat, streams: &[MuxCodec], mux_rate: u64) -> MuxResult<Self> {
        if streams.is_empty() || streams.len() > 16 {
            return Err(MuxError::Unsupported(format!("{} streams: 1 to 16 are supported", streams.len())));
        }
        let mut states = Vec::with_capacity(streams.len());
        let (mut video, mut lpcm) = (0u8, 0u8);
        for (k, &codec) in streams.iter().enumerate() {
            match (format, codec) {
                (MuxFormat::Ps, MuxCodec::AacAdts) => return Err(MuxError::Unsupported("AAC is written to transport streams only".into())),
                (MuxFormat::Ts | MuxFormat::M2ts, MuxCodec::LpcmDvd { .. }) => {
                    return Err(MuxError::Unsupported("DVD LPCM is written to program streams only".into()));
                }
                (_, MuxCodec::LpcmDvd { sample_rate, channels }) => {
                    if lpcm_rate_code(sample_rate).is_none() || !(1..=8).contains(&channels) {
                        return Err(MuxError::Unsupported(format!("DVD LPCM has no {channels} channels at {sample_rate} Hz")));
                    }
                    lpcm += 1;
                }
                (_, MuxCodec::Mpeg2Video) => video += 1,
                _ => {}
            }
            states.push(StreamState { codec, pid: FIRST_ES_PID + k as u16, cc: 0, last_dts: None });
        }
        if video > 16 || lpcm > 8 {
            return Err(MuxError::Unsupported("too many streams of one kind".into()));
        }
        // the program_mux_rate field counts 50 bytes/s in 22 bits
        let rate = (mux_rate / 8).clamp(50 * 64, 50 * ((1 << 22) - 1));
        Ok(Muxer { out, format, streams: states, rate, queue: Vec::new(), queued_bytes: 0, clock: 0, last_psi: None, last_pcr: None, psi_cc: 0, packs: 0 })
    }

    /// Queue an access unit of `stream`: `pts` and `dts` (`None`: the same) in 90 kHz from the
    /// start, not before −[`Muxer::DELAY`]. Units of one stream come in decode order.
    pub fn write(&mut self, stream: usize, data: &[u8], pts: i64, dts: Option<i64>, key: bool) -> MuxResult<()> {
        let st = self.streams.get_mut(stream).ok_or_else(|| MuxError::Unsupported(format!("no stream {stream}")))?;
        let dts = dts.unwrap_or(pts);
        if !(-Self::DELAY..1 << 40).contains(&pts) || !(-Self::DELAY..=pts).contains(&dts) {
            return Err(MuxError::Unsupported(format!("timestamps out of range: pts {pts}, dts {dts}")));
        }
        let (pts, dts) = (pts + Self::DELAY, dts + Self::DELAY);
        if st.last_dts.is_some_and(|d| dts < d) {
            return Err(MuxError::Unsupported(format!("stream {stream}: decode times go backwards")));
        }
        st.last_dts = Some(dts);
        self.queued_bytes += data.len();
        self.queue.push(Unit { stream, data: data.to_vec(), pts, dts, key });
        self.flush_ready()
    }

    /// Append bytes to the last queued unit of `stream` (a sequence end code).
    pub fn append_to_last(&mut self, stream: usize, data: &[u8]) -> MuxResult<()> {
        let u = self.queue.iter_mut().rev().find(|u| u.stream == stream).ok_or_else(|| MuxError::Unsupported("nothing queued to append to".into()))?;
        u.data.extend_from_slice(data);
        self.queued_bytes += data.len();
        Ok(())
    }

    /// Write everything still queued (and the program end code of a program stream).
    pub fn finish(mut self) -> MuxResult<W> {
        self.flush_until(i64::MAX, true)?;
        if self.format == MuxFormat::Ps {
            self.out.write_all(&[0, 0, 1, 0xB9])?;
        }
        self.out.flush()?;
        Ok(self.out)
    }

    /// Write the queued units that no stream can still precede.
    fn flush_ready(&mut self) -> MuxResult<()> {
        // a stream with nothing queued yet holds everything back
        if let Some(h) = self.streams.iter().map(|s| s.last_dts).min().flatten() {
            self.flush_until(h, false)?;
        }
        while self.queued_bytes > MAX_QUEUED {
            let Some(first) = self.queue.iter().map(|u| u.dts).min() else { break };
            if !self.flush_until(first, false)? && !self.flush_until(first, true)? {
                break;
            }
        }
        Ok(())
    }

    /// Write the queued units with a decode time up to `until`, in decode-time order. The last
    /// unit of each stream stays queued (for [`Muxer::append_to_last`]) unless `all`.
    fn flush_until(&mut self, until: i64, all: bool) -> MuxResult<bool> {
        // stable: equal decode times keep the order they were queued in
        self.queue.sort_by_key(|u| u.dts);
        let mut held = vec![false; self.queue.len()];
        if !all {
            for s in 0..self.streams.len() {
                if let Some(i) = self.queue.iter().rposition(|u| u.stream == s)
                    && let Some(h) = held.get_mut(i)
                {
                    *h = true;
                }
            }
        }
        let n = self.queue.iter().zip(&held).take_while(|(u, h)| u.dts <= until && !**h).count();
        let ready: Vec<Unit> = self.queue.drain(..n).collect();
        for u in ready {
            self.queued_bytes = self.queued_bytes.saturating_sub(u.data.len());
            // out as early as the rate allows, not more than the delay before the decode time
            self.clock = self.clock.max((u.dts - Self::DELAY) * SYS_PER_90K);
            match self.format {
                MuxFormat::Ps => self.write_ps(&u)?,
                _ => self.write_ts(&u)?,
            }
        }
        Ok(n > 0)
    }

    /// 27 MHz ticks to send `bytes` at the multiplex rate.
    fn ticks(&self, bytes: usize) -> i64 {
        (bytes as i64).saturating_mul(27_000_000) / self.rate.max(1) as i64
    }

    // ---- transport streams -------------------------------------------------------------------

    fn write_ts(&mut self, u: &Unit) -> MuxResult<()> {
        let video = self.streams.get(u.stream).is_some_and(|s| s.codec == MuxCodec::Mpeg2Video);
        if self.last_psi.is_none_or(|t| self.clock - t >= 100 * 27_000) || (video && u.key) {
            self.write_psi()?;
        }
        let Some(st) = self.streams.get(u.stream) else { return Ok(()) };
        let (pid, codec) = (st.pid, st.codec);
        let pes = pes_packet(stream_id(codec), u, true, &[]);
        let pcr_pid = self.streams.first().map(|s| s.pid);
        let mut at = 0;
        let mut first = true;
        while at < pes.len() {
            let pcr = (Some(pid) == pcr_pid && (first || self.last_pcr.is_none_or(|t| self.clock - t >= 30 * 27_000))).then_some(self.clock);
            let random = first && u.key;
            // adaptation field: length byte + flags (+ PCR)
            let mut af = Vec::new();
            if pcr.is_some() || random {
                af.push(u8::from(random) << 6 | u8::from(pcr.is_some()) << 4);
                if let Some(t) = pcr {
                    let (base, ext) = (t / SYS_PER_90K, t % SYS_PER_90K);
                    af.extend_from_slice(&[
                        (base >> 25) as u8,
                        (base >> 17) as u8,
                        (base >> 9) as u8,
                        (base >> 1) as u8,
                        ((base & 1) << 7) as u8 | 0x7E | (ext >> 8) as u8,
                        ext as u8,
                    ]);
                    self.last_pcr = Some(t);
                }
            }
            let room = 184 - if af.is_empty() { 0 } else { 1 + af.len() };
            let take = room.min(pes.len() - at);
            if take < room {
                // stuffing: the adaptation field grows to fill the packet
                let fill = room - take;
                if af.is_empty() {
                    if fill > 1 {
                        af.push(0);
                        af.extend(std::iter::repeat_n(0xFF, fill - 2));
                    }
                    // fill == 1: an adaptation field of length 0
                } else {
                    af.extend(std::iter::repeat_n(0xFF, fill));
                }
            }
            let has_af = !af.is_empty() || take < room;
            let cc = self.next_cc(u.stream);
            let mut p = Vec::with_capacity(188);
            p.push(0x47);
            p.push(u8::from(first) << 6 | (pid >> 8) as u8 & 0x1F);
            p.push(pid as u8);
            p.push(if has_af { 0x30 } else { 0x10 } | cc);
            if has_af {
                p.push(af.len() as u8);
                p.extend_from_slice(&af);
            }
            p.extend_from_slice(pes.get(at..at + take).unwrap_or(&[]));
            self.packet(&p)?;
            at += take;
            first = false;
        }
        Ok(())
    }

    fn next_cc(&mut self, stream: usize) -> u8 {
        match self.streams.get_mut(stream) {
            Some(s) => {
                let cc = s.cc;
                s.cc = (s.cc + 1) & 15;
                cc
            }
            None => 0,
        }
    }

    /// One transport packet (with its arrival timestamp for BDAV), advancing the clock.
    fn packet(&mut self, p: &[u8]) -> MuxResult<()> {
        if self.format == MuxFormat::M2ts {
            // copy_permission_indicator 0, 30-bit arrival time stamp
            let ats = (self.clock & 0x3FFF_FFFF) as u32;
            self.out.write_all(&ats.to_be_bytes())?;
        }
        self.out.write_all(p)?;
        self.clock += self.ticks(if self.format == MuxFormat::M2ts { 192 } else { 188 });
        Ok(())
    }

    fn write_psi(&mut self) -> MuxResult<()> {
        // PAT: program 1 → PMT_PID
        let mut pat = vec![0x00, 0, 0, 0x00, 0x01, 0xC1, 0, 0];
        pat.extend_from_slice(&[0, 1, 0xE0 | (PMT_PID >> 8) as u8, PMT_PID as u8]);
        let pcr_pid = self.streams.first().map_or(0x1FFF, |s| s.pid);
        let mut pmt = vec![0x02, 0, 0, 0x00, 0x01, 0xC1, 0, 0, 0xE0 | (pcr_pid >> 8) as u8, pcr_pid as u8, 0xF0, 0];
        for s in &self.streams {
            let st = match s.codec {
                MuxCodec::Mpeg2Video => 0x02,
                MuxCodec::AacAdts => 0x0F,
                MuxCodec::LpcmDvd { .. } => 0x80,
            };
            pmt.extend_from_slice(&[st, 0xE0 | (s.pid >> 8) as u8, s.pid as u8, 0xF0, 0]);
        }
        // PAT and PMT both go out every time: one continuity count serves both PIDs
        let cc = self.psi_cc;
        self.psi_cc = (cc + 1) & 15;
        for (pid, mut sec) in [(PAT_PID, pat), (PMT_PID, pmt)] {
            // section_length counts from after the field to the end of the CRC
            let len = sec.len() - 3 + 4;
            sec[1] = 0xB0 | (len >> 8) as u8 & 0x0F;
            sec[2] = len as u8;
            sec.extend_from_slice(&crc32_mpeg(&sec).to_be_bytes());
            let mut p = vec![0x47, 0x40 | (pid >> 8) as u8 & 0x1F, pid as u8, 0x10 | cc, 0];
            p.extend_from_slice(&sec);
            p.resize(188, 0xFF);
            self.packet(&p)?;
        }
        self.last_psi = Some(self.clock);
        Ok(())
    }

    // ---- program streams ---------------------------------------------------------------------

    fn write_ps(&mut self, u: &Unit) -> MuxResult<()> {
        let Some(st) = self.streams.get(u.stream) else { return Ok(()) };
        let codec = st.codec;
        let sid = stream_id(codec);
        let lpcm_index = self.streams.iter().take(u.stream).filter(|s| matches!(s.codec, MuxCodec::LpcmDvd { .. })).count() as u8;
        let video_index = self.streams.iter().take(u.stream).filter(|s| s.codec == MuxCodec::Mpeg2Video).count() as u8;
        let mut at = 0;
        let mut first = true;
        while at < u.data.len() || first {
            let system = self.packs == 0;
            let fixed = 14 + if system { self.system_header_len() } else { 0 };
            // PES header: 6 + 3 + PTS (5) or PTS + DTS (10)
            let ts_len = if first { if u.dts != u.pts { 10 } else { 5 } } else { 0 };
            let sub_len = if matches!(codec, MuxCodec::LpcmDvd { .. }) { 7 } else { 0 };
            let room = PACK_SIZE - fixed - 9 - ts_len - sub_len;
            let mut take = room.min(u.data.len() - at);
            if let MuxCodec::LpcmDvd { channels, .. } = codec {
                // whole sample frames in every packet
                let frame = 2 * channels as usize;
                if take < u.data.len() - at {
                    take -= take % frame;
                }
            }
            let mut pack = Vec::with_capacity(PACK_SIZE);
            self.pack_header(&mut pack);
            if system {
                self.system_header(&mut pack);
            }
            let sub = match codec {
                MuxCodec::LpcmDvd { sample_rate, channels } => {
                    let rate = lpcm_rate_code(sample_rate).unwrap_or(0);
                    // sub-stream id, frame headers, first access unit pointer, then the LPCM
                    // header: frame number, 16-bit / rate / channels, dynamic range (none)
                    vec![0xA0 + lpcm_index, 1, 0, 4, 0, rate << 4 | (channels as u8 - 1), 0x80]
                }
                _ => Vec::new(),
            };
            let unit = Unit { stream: u.stream, data: u.data.get(at..at + take).unwrap_or(&[]).to_vec(), pts: u.pts, dts: u.dts, key: u.key };
            let id = if sid == 0xE0 { 0xE0 + video_index } else { sid };
            // continuation packets carry no timestamps
            pack.extend_from_slice(&pes_packet(id, &unit, first, &sub));
            self.out.write_all(&pack)?;
            self.clock += self.ticks(pack.len());
            self.packs += 1;
            at += take;
            first = false;
            if take == 0 {
                break;
            }
        }
        Ok(())
    }

    fn pack_header(&self, out: &mut Vec<u8>) {
        let (base, ext) = (self.clock / SYS_PER_90K, self.clock % SYS_PER_90K);
        let mux = (self.rate / 50) as u32;
        out.extend_from_slice(&[0, 0, 1, 0xBA]);
        out.extend_from_slice(&[
            0x44 | ((base >> 27) & 0x38) as u8 | ((base >> 28) & 3) as u8,
            (base >> 20) as u8,
            ((base >> 12) & 0xF8) as u8 | 4 | ((base >> 13) & 3) as u8,
            (base >> 5) as u8,
            ((base << 3) & 0xF8) as u8 | 4 | ((ext >> 7) & 3) as u8,
            ((ext << 1) & 0xFE) as u8 | 1,
            (mux >> 14) as u8,
            (mux >> 6) as u8,
            ((mux << 2) & 0xFC) as u8 | 3,
            0xF8,
        ]);
    }

    fn system_header(&self, out: &mut Vec<u8>) {
        let bound = (self.rate / 50) as u32;
        let audio = self.streams.iter().filter(|s| s.codec != MuxCodec::Mpeg2Video).count() as u8;
        let video = self.streams.iter().filter(|s| s.codec == MuxCodec::Mpeg2Video).count() as u8;
        // P-STD buffer bounds, one per stream id: 1194 × 1024 bytes for video (the largest MP@HL
        // VBV), 58 × 128 for private stream 1 (shared by every LPCM sub-stream)
        let mut entries: Vec<[u8; 3]> = (0..video).map(|v| [0xE0 + v, 0xE0 | (1194 >> 8) as u8, 1194u16 as u8]).collect();
        if audio > 0 {
            entries.push([0xBD, 0xC0, 58]);
        }
        let len = 6 + 3 * entries.len();
        out.extend_from_slice(&[0, 0, 1, 0xBB, (len >> 8) as u8, len as u8]);
        out.extend_from_slice(&[0x80 | (bound >> 15) as u8, (bound >> 7) as u8, ((bound << 1) & 0xFE) as u8 | 1, audio << 2, 0x20 | video, 0x7F]);
        for e in entries {
            out.extend_from_slice(&e);
        }
    }

    /// Bytes of the system header.
    fn system_header_len(&self) -> usize {
        let video = self.streams.iter().filter(|s| s.codec == MuxCodec::Mpeg2Video).count();
        let audio = usize::from(self.streams.iter().any(|s| s.codec != MuxCodec::Mpeg2Video));
        12 + 3 * (video + audio)
    }
}

/// The PES stream_id of a codec (the first of its kind).
fn stream_id(codec: MuxCodec) -> u8 {
    match codec {
        MuxCodec::Mpeg2Video => 0xE0,
        MuxCodec::AacAdts => 0xC0,
        MuxCodec::LpcmDvd { .. } => 0xBD,
    }
}

fn lpcm_rate_code(rate: u32) -> Option<u8> {
    Some(match rate {
        48_000 => 0,
        96_000 => 1,
        44_100 => 2,
        32_000 => 3,
        _ => return None,
    })
}

/// A 5-byte PTS / DTS field with the 4-bit `prefix`.
fn timestamp(prefix: u8, t: i64) -> [u8; 5] {
    let t = t & ((1 << 33) - 1);
    [prefix << 4 | ((t >> 29) & 0x0E) as u8 | 1, (t >> 22) as u8, ((t >> 14) & 0xFE) as u8 | 1, (t >> 7) as u8, ((t << 1) & 0xFE) as u8 | 1]
}

/// A PES packet of `u` (its timestamps when `stamped`), `prefix` bytes before the payload. The
/// length field is 0 (unbounded) for a video packet too long for it.
fn pes_packet(id: u8, u: &Unit, stamped: bool, prefix: &[u8]) -> Vec<u8> {
    let mut h = Vec::with_capacity(19);
    let both = stamped && u.dts != u.pts;
    // data_alignment_indicator: a video packet starting a picture
    h.push(0x80 | if id & 0xF0 == 0xE0 && stamped { 0x04 } else { 0 });
    h.push(if both {
        0xC0
    } else if stamped {
        0x80
    } else {
        0
    });
    h.push(if both {
        10
    } else if stamped {
        5
    } else {
        0
    });
    if both {
        h.extend_from_slice(&timestamp(3, u.pts));
        h.extend_from_slice(&timestamp(1, u.dts));
    } else if stamped {
        h.extend_from_slice(&timestamp(2, u.pts));
    }
    let len = h.len() + prefix.len() + u.data.len();
    let len = if len > 0xFFFF { 0 } else { len as u16 };
    let mut p = Vec::with_capacity(6 + h.len() + prefix.len() + u.data.len());
    p.extend_from_slice(&[0, 0, 1, id, (len >> 8) as u8, len as u8]);
    p.extend_from_slice(&h);
    p.extend_from_slice(prefix);
    p.extend_from_slice(&u.data);
    p
}
