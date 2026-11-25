//! Scan a DVD VOB and fill in its navigation packs.
//!
//! The VOB comes from `cfa4_mpegts::Muxer::new_dvd`: whole 2048-byte sectors, one pack each, and
//! an empty navigation pack (private stream 2: a PCI and a DSI packet of zeros) at the start of
//! every VOBU. This module finds the VOBUs, measures them (presentation times, the end of the
//! first three reference pictures, the first audio packet of each stream) and writes the PCI
//! and DSI of every navigation pack (layouts: the public DVD-Video format notes).

use std::io::{Read, Seek, SeekFrom, Write};

use crate::{AudioKind, Error, Result, SECTOR, Timing};

/// One video object unit: the sectors from its navigation pack to the next one.
#[derive(Clone, Debug, Default)]
pub(crate) struct Vobu {
    pub first: u32,
    pub sectors: u32,
    pub scr: u64,
    pub s_ptm: u32,
    pub e_ptm: u32,
    /// Last sector of each of the first three reference pictures (I, P, P), relative to `first`.
    pub refs: Vec<u32>,
    /// Pictures in stream order: (sector of the byte before the start code, coding type).
    pics: Vec<(u32, u8)>,
    /// Last sector with video.
    last_video: u32,
    /// Sector (relative to `first`) of the audio packet of stream `n` that carries the first
    /// audio at or after the start of the VOBU's video (else its first packet).
    pub audio: [Option<u32>; 8],
    /// Audio packets: (stream, sector relative to `first`, PTS if it starts an access unit).
    audio_packets: Vec<(usize, u32, Option<u32>)>,
    pub pictures: u32,
}

/// What a VOB scan found.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scan {
    pub vobus: Vec<Vobu>,
    pub total_sectors: u32,
}

/// The 33-bit PTS / DTS of the 5 bytes at `at`, or `None` when the marker bits are wrong.
fn timestamp(b: &[u8], at: usize) -> Option<u64> {
    let d = b.get(at..at + 5)?;
    if d[0] & 1 == 0 || d[2] & 1 == 0 || d[4] & 1 == 0 {
        return None;
    }
    Some(u64::from(d[0] >> 1 & 7) << 30 | u64::from(d[1]) << 22 | u64::from(d[2] >> 1) << 15 | u64::from(d[3]) << 7 | u64::from(d[4] >> 1))
}

/// Finds MPEG-2 picture start codes (00 00 01 00) and the coding type behind them in a video
/// byte stream that arrives in pieces, remembering in which sector each byte was.
#[derive(Default)]
struct PictureFinder {
    zeros: u8,
    /// A 00 00 01 was just read.
    start_code: bool,
    /// Sectors of the last five bytes.
    ring: [u32; 5],
    /// Bytes still to read after a picture start code (temporal reference, coding type).
    pending: u8,
    before: u32,
}

impl PictureFinder {
    /// `on_picture(sector of the byte before the start code, coding type)` for every picture.
    fn feed(&mut self, data: &[u8], sector: u32, mut on_picture: impl FnMut(u32, u8)) {
        for &b in data {
            self.ring.rotate_left(1);
            if let Some(last) = self.ring.last_mut() {
                *last = sector;
            }
            if self.pending > 0 {
                self.pending -= 1;
                if self.pending == 0 {
                    // the second byte holds the last two bits of the temporal reference, then
                    // picture_coding_type
                    on_picture(self.before, b >> 3 & 7);
                }
                continue;
            }
            if self.start_code {
                self.start_code = false;
                self.zeros = 0;
                if b == 0 {
                    self.before = self.ring.first().copied().unwrap_or(sector);
                    self.pending = 2;
                }
                continue;
            }
            match b {
                0 => self.zeros = (self.zeros + 1).min(2),
                1 if self.zeros >= 2 => {
                    self.start_code = true;
                    self.zeros = 0;
                }
                _ => self.zeros = 0,
            }
        }
    }
}

/// Read a whole VOB and describe its VOBUs. `streams` maps the audio packets to the stream
/// numbers 0..8 of `audio`; `timing` gives the picture duration.
pub(crate) fn scan<R: Read + Seek>(src: &mut R, audio: &[AudioKind], timing: Timing) -> Result<Scan> {
    let len = src.seek(SeekFrom::End(0))?;
    src.seek(SeekFrom::Start(0))?;
    if len == 0 || len % SECTOR as u64 != 0 {
        return Err(Error::Vob(format!("{len} bytes are not a whole number of 2048-byte sectors")));
    }
    let total = u32::try_from(len / SECTOR as u64).map_err(|_| Error::Vob("a VOB of more than 2^32 sectors".into()))?;
    let frame = timing.frame_ticks();
    let mut out = Scan { vobus: Vec::new(), total_sectors: total };
    let mut finder = PictureFinder::default();
    // (kind, index within kind) → stream number
    let slot = |kind: AudioKind, idx: u8| -> Option<usize> {
        let mut n = [0u8; 3];
        for (i, k) in audio.iter().enumerate() {
            let c = &mut n[*k as usize];
            if *k == kind && *c == idx {
                return Some(i);
            }
            *c += 1;
        }
        None
    };
    let mut sector = vec![0u8; SECTOR];
    let mut reader = std::io::BufReader::with_capacity(1 << 20, src);
    for n in 0..total {
        reader.read_exact(&mut sector)?;
        if sector.get(..4) != Some(&[0, 0, 1, 0xBA]) {
            return Err(Error::Vob(format!("sector {n} does not start with a pack header")));
        }
        let scr = {
            let b = &sector[4..10];
            (u64::from(b[0] >> 3 & 7) << 30
                | u64::from(b[0] & 3) << 28
                | u64::from(b[1]) << 20
                | u64::from(b[2] >> 3) << 15
                | u64::from(b[2] & 3) << 13
                | u64::from(b[3]) << 5
                | u64::from(b[4] >> 3))
                & 0xFFFF_FFFF
        };
        let mut at = 14 + usize::from(sector[13] & 7);
        let mut nav = false;
        let mut packets: Vec<(u8, usize, usize)> = Vec::new();
        while at + 6 <= SECTOR && sector[at..at + 3] == [0, 0, 1] {
            let id = sector[at + 3];
            let plen = usize::from(sector[at + 4]) << 8 | usize::from(sector[at + 5]);
            let end = at + 6 + plen;
            if end > SECTOR {
                return Err(Error::Vob(format!("sector {n}: a packet runs past the end")));
            }
            if id == 0xBF && sector.get(at + 6) == Some(&0) {
                nav = true;
            }
            packets.push((id, at + 6, end));
            at = end;
        }
        if nav {
            if let Some(v) = out.vobus.last_mut() {
                v.sectors = n - v.first;
            }
            // a picture cannot continue into the next VOBU
            finder = PictureFinder::default();
            out.vobus.push(Vobu { first: n, scr, s_ptm: u32::MAX, ..Vobu::default() });
            continue;
        }
        let Some(v) = out.vobus.last_mut() else {
            return Err(Error::Vob("the VOB does not start with a navigation pack".into()));
        };
        for (id, start, end) in packets {
            let body = &sector[start..end];
            match id {
                0xE0..=0xEF if body.len() >= 3 => {
                    let hdl = usize::from(body[2]);
                    let flags = body[1];
                    if flags & 0x80 != 0
                        && let Some(pts) = timestamp(body, 3)
                    {
                        let pts = (pts & 0xFFFF_FFFF) as u32;
                        v.s_ptm = v.s_ptm.min(pts);
                        v.e_ptm = v.e_ptm.max(pts.wrapping_add(frame));
                    }
                    let payload = body.get(3 + hdl..).unwrap_or(&[]);
                    v.last_video = n;
                    finder.feed(payload, n, |before, kind| v.pics.push((before, kind)));
                    v.pictures = v.pics.len() as u32;
                }
                0xBD if body.len() >= 4 => {
                    let hdl = usize::from(*body.get(2).unwrap_or(&0));
                    let sub = body.get(3 + hdl).copied().unwrap_or(0);
                    let found = match sub {
                        0x80..=0x87 => slot(AudioKind::Ac3, sub & 7),
                        0xA0..=0xA7 => slot(AudioKind::Lpcm, sub & 7),
                        _ => None,
                    };
                    if let Some(s) = found.filter(|s| *s < 8) {
                        let pts = (body.get(1).is_some_and(|f| f & 0x80 != 0)).then(|| timestamp(body, 3)).flatten().map(|t| (t & 0xFFFF_FFFF) as u32);
                        v.audio_packets.push((s, n - v.first, pts));
                    }
                }
                0xC0..=0xC7 => {
                    if let Some(s) = slot(AudioKind::Mpeg, id & 7).filter(|s| *s < 8) {
                        let pts = (body.get(1).is_some_and(|f| f & 0x80 != 0)).then(|| timestamp(body, 3)).flatten().map(|t| (t & 0xFFFF_FFFF) as u32);
                        v.audio_packets.push((s, n - v.first, pts));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(v) = out.vobus.last_mut() {
        v.sectors = total - v.first;
    }
    for v in &mut out.vobus {
        for s in 0..8 {
            let mut packets = v.audio_packets.iter().filter(|p| p.0 == s);
            let first = packets.clone().next().map(|p| p.1);
            let synced = packets.find(|p| p.2.is_some_and(|t| t >= v.s_ptm)).map(|p| p.1);
            v.audio[s] = synced.or(first);
        }
        for (i, &(_, kind)) in v.pics.iter().enumerate() {
            if v.refs.len() < 3 && (kind == 1 || kind == 2) {
                let end = v.pics.get(i + 1).map_or(v.last_video, |p| p.0);
                v.refs.push(end.saturating_sub(v.first));
            }
        }
    }
    Ok(out)
}

/// Cells of a scan: a chapter starts at the VOBU that contains its time (90 kHz from the first
/// picture; chapter 1 is at 0), later chapters in the same VOBU are dropped.
pub(crate) fn cells(vobus: &[Vobu], chapters: &[u64]) -> Vec<std::ops::Range<usize>> {
    let base = u64::from(vobus.first().map_or(0, |v| v.s_ptm));
    let mut starts = vec![0usize];
    for &c in chapters {
        let t = base + c;
        let i = vobus.partition_point(|v| u64::from(v.s_ptm) <= t).saturating_sub(1);
        if starts.last().is_some_and(|&l| i > l) {
            starts.push(i);
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        out.push(s..starts.get(k + 1).copied().unwrap_or(vobus.len()));
    }
    out
}

/// Forward and backward search steps of the DSI in units of half a second.
const FORWARD: [u32; 19] = [240, 120, 60, 20, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];
const BACKWARD: [u32; 19] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 20, 60, 120, 240];

fn put32(b: &mut [u8], at: usize, v: u32) {
    if let Some(s) = b.get_mut(at..at + 4) {
        s.copy_from_slice(&v.to_be_bytes());
    }
}

fn put16(b: &mut [u8], at: usize, v: u16) {
    if let Some(s) = b.get_mut(at..at + 2) {
        s.copy_from_slice(&v.to_be_bytes());
    }
}

/// Write the PCI and DSI of every navigation pack of the VOB behind `dst`. `vob_id` is the VOB
/// number, `cells` the cell ranges (VOBU numbers, cell id = position + 1).
pub(crate) fn fill<W: Read + Write + Seek>(
    dst: &mut W,
    scan: &Scan,
    cells: &[std::ops::Range<usize>],
    vob_id: u16,
    audio_streams: usize,
    timing: Timing,
) -> Result<()> {
    let (Some(first), Some(last)) = (scan.vobus.first(), scan.vobus.last()) else {
        return Err(Error::Vob("no VOBU".into()));
    };
    let (vob_s, vob_e) = (first.s_ptm, last.e_ptm);
    let mut sector = vec![0u8; SECTOR];
    for (c, cell) in cells.iter().enumerate() {
        let Some(cell_first) = scan.vobus.get(cell.start) else { continue };
        let times: Vec<u32> = scan.vobus[cell.clone()].iter().map(|v| v.s_ptm).collect();
        let cell_end_ptm = scan.vobus.get(cell.end.wrapping_sub(1)).map_or(0, |v| v.e_ptm);
        for i in cell.clone() {
            let v = &scan.vobus[i];
            dst.seek(SeekFrom::Start(u64::from(v.first) * SECTOR as u64))?;
            dst.read_exact(&mut sector)?;
            let eltm = timing.timecode(v.s_ptm.wrapping_sub(cell_first.s_ptm));
            // PCI
            put32(&mut sector, 0x2D, v.first);
            put32(&mut sector, 0x39, v.s_ptm);
            put32(&mut sector, 0x3D, v.e_ptm);
            put32(&mut sector, 0x45, eltm);
            // DSI
            put32(&mut sector, 0x407, v.scr as u32);
            put32(&mut sector, 0x40B, v.first);
            put32(&mut sector, 0x40F, v.sectors.saturating_sub(1));
            for k in 0..3 {
                let r = v.refs.get(k).or(v.refs.last()).copied().unwrap_or(v.sectors.saturating_sub(1));
                put32(&mut sector, 0x413 + 4 * k, r);
            }
            put16(&mut sector, 0x41F, vob_id);
            sector[0x421] = 0;
            sector[0x422] = (c + 1) as u8;
            put32(&mut sector, 0x423, eltm);
            put32(&mut sector, 0x433, vob_s);
            put32(&mut sector, 0x437, vob_e);
            // search information: next VOBU in the cell and the time-based steps
            let next = (i + 1 < cell.end).then(|| scan.vobus[i + 1].first - v.first);
            let prev = (i > cell.start).then(|| v.first - scan.vobus[i - 1].first);
            put32(&mut sector, 0x4F1, next.map_or(0xBFFF_FFFF, |o| 0x8000_0000 | o));
            put32(&mut sector, 0x541, next.map_or(0x3FFF_FFFF, |o| 0x8000_0000 | o));
            put32(&mut sector, 0x545, prev.map_or(0x3FFF_FFFF, |o| 0x8000_0000 | o));
            put32(&mut sector, 0x595, prev.map_or(0xBFFF_FFFF, |o| 0x8000_0000 | o));
            // `times` holds the cell's VOBU start times: j is a cell-relative index
            let local = i - cell.start;
            // the VOBU that contains the time n half-seconds away (the current one for a short step)
            let mut prev_j = local;
            for (k, n) in FORWARD.iter().rev().enumerate() {
                let at = 0x4F5 + 4 * (FORWARD.len() - 1 - k);
                let want = u64::from(v.s_ptm) + u64::from(*n) * 45_000;
                let word = if want >= u64::from(cell_end_ptm) {
                    0x3FFF_FFFF
                } else {
                    let j = containing(&times, want).max(local);
                    let w = 0x8000_0000 | if j - prev_j > 1 { 0x4000_0000 } else { 0 } | (scan.vobus[cell.start + j].first - v.first);
                    prev_j = j;
                    w
                };
                put32(&mut sector, at, word);
            }
            let mut prev_j = local;
            for (k, n) in BACKWARD.iter().enumerate() {
                let back = u64::from(*n) * 45_000;
                let word = if u64::from(v.s_ptm) < back + u64::from(cell_first.s_ptm) {
                    0x3FFF_FFFF
                } else {
                    let j = containing(&times, u64::from(v.s_ptm) - back).min(local);
                    let w = 0x8000_0000 | if prev_j - j > 1 { 0x4000_0000 } else { 0 } | (v.first - scan.vobus[cell.start + j].first);
                    prev_j = j;
                    w
                };
                put32(&mut sector, 0x549 + 4 * k, word);
            }
            // audio synchronisation: where this VOBU's first packet of each stream is
            for s in 0..8usize {
                let w = if s >= audio_streams { 0 } else { v.audio.get(s).copied().flatten().map_or(0x3FFF, |o| o.min(0x3FFE) as u16) };
                put16(&mut sector, 0x599 + 2 * s, w);
            }
            dst.seek(SeekFrom::Start(u64::from(v.first) * SECTOR as u64))?;
            dst.write_all(&sector)?;
        }
    }
    Ok(())
}

/// Index of the last VOBU in `times` (ascending start times) that starts at or before `t`.
fn containing(times: &[u32], t: u64) -> usize {
    times.partition_point(|&s| u64::from(s) <= t).saturating_sub(1)
}
