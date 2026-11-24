//! DVD-Video authoring (CFA4 #244): the navigation packs of a VOB and the IFO / BUP files of a
//! one-title disc.
//!
//! Input is a VOB written by `cfa4_mpegts::Muxer::new_dvd`: 2048-byte packs, an empty navigation
//! pack at the start of each GOP. [`author`] scans it, fills every navigation pack's PCI and DSI
//! (VOBU times and sizes, the end of the first three reference pictures, forward / backward
//! search pointers, audio sync offsets, cell and VOB ids) and builds `VIDEO_TS.IFO` and
//! `VTS_01_0.IFO`; [`write_disc`] adds the BUP copies and splits the VOB into the
//! `VTS_01_n.VOB` files of at most 1 GiB.
//!
//! Layouts come from the public DVD-Video format notes (no menus, no subpictures, one title,
//! one program chain whose chapters are its programs and cells; no interleaving or angles).
//! The picture duration of a VOBU ignores repeat_first_field (3:2 pulldown).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod ifo;
mod nav;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// One DVD sector.
pub const SECTOR: usize = 2048;
/// Most sectors of one `VTS_01_n.VOB` file (1 GiB).
const MAX_VOB_SECTORS: u32 = 1 << 19;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a DVD VOB: {0}")]
    Vob(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// 625-line (25 fps) or 525-line (29.97 fps) video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    Pal,
    Ntsc,
}

impl Timing {
    /// One frame in 90 kHz ticks.
    pub fn frame_ticks(self) -> u32 {
        match self {
            Timing::Pal => 3600,
            Timing::Ntsc => 3003,
        }
    }

    /// BCD `hh:mm:ss:ff` of a duration, the top two bits of the frame byte give the rate
    /// (01: 25 fps, 11: 30 fps).
    pub fn timecode(self, ticks: u32) -> u32 {
        let (nominal, flag) = match self {
            Timing::Pal => (25, 0x40),
            Timing::Ntsc => (30, 0xC0),
        };
        let frames = (u64::from(ticks) + u64::from(self.frame_ticks()) / 2) / u64::from(self.frame_ticks());
        let (ff, secs) = (frames % nominal, frames / nominal);
        let bcd = |v: u64| (((v / 10) << 4) | (v % 10)) as u32;
        bcd((secs / 3600) % 100) << 24 | bcd(secs / 60 % 60) << 16 | bcd(secs % 60) << 8 | (bcd(ff) | flag)
    }
}

/// How an audio stream is carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioKind {
    /// AC-3 in private stream 1, sub-stream 0x80 + n.
    Ac3 = 0,
    /// LPCM in private stream 1, sub-stream 0xA0 + n.
    Lpcm = 1,
    /// MPEG-1 audio in stream 0xC0 + n.
    Mpeg = 2,
}

/// One audio stream, in the order the muxer was given them (the n-th of its kind is sub-stream n).
#[derive(Clone, Debug)]
pub struct Audio {
    pub kind: AudioKind,
    pub channels: u8,
    /// ISO 639-1 language, for example `*b"en"`.
    pub language: Option<[u8; 2]>,
    /// LPCM only: 16, 20 or 24.
    pub lpcm_bits: u8,
    /// LPCM only: 48000 or 96000.
    pub lpcm_rate: u32,
}

/// What `VTS_01_0.IFO` says about the title.
#[derive(Clone, Debug)]
pub struct TitleSetSpec {
    pub timing: Timing,
    pub widescreen: bool,
    /// 720, 704 or 352.
    pub width: u16,
    pub audio: Vec<Audio>,
    /// Chapter starts as 90 kHz offsets from the first picture; chapter 1 starts at 0. A chapter
    /// begins at the first VOBU that starts at or after its time.
    pub chapters: Vec<u64>,
}

/// The IFO files built for a VOB.
#[derive(Clone, Debug)]
pub struct Authored {
    /// `VIDEO_TS.IFO` (and `.BUP`).
    pub vmg_ifo: Vec<u8>,
    /// `VTS_01_0.IFO` (and `.BUP`).
    pub vts_ifo: Vec<u8>,
    pub vobus: usize,
    pub chapters: usize,
    pub sectors: u32,
}

/// Fill the navigation packs of the VOB in `vob` (in place) and build the IFO files for it.
pub fn author<F: Read + Write + Seek>(vob: &mut F, spec: &TitleSetSpec) -> Result<Authored> {
    let kinds: Vec<AudioKind> = spec.audio.iter().map(|a| a.kind).collect();
    let scan = nav::scan(vob, &kinds, spec.timing)?;
    if scan.vobus.iter().any(|v| v.s_ptm == u32::MAX) {
        return Err(Error::Vob("a VOBU without a video picture".into()));
    }
    let cells = nav::cells(&scan.vobus, &spec.chapters);
    nav::fill(vob, &scan, &cells, 1, spec.audio.len().min(8), spec.timing)?;
    let vts_ifo = ifo::build_vts(spec, &scan.vobus, &cells, scan.total_sectors, 1);
    let vmg_ifo = ifo::build_vmg(spec, cells.len(), &vts_ifo);
    Ok(Authored { vmg_ifo, vts_ifo, vobus: scan.vobus.len(), chapters: cells.len(), sectors: scan.total_sectors })
}

/// Author the VOB file at `vob` and write the disc folder `dir/VIDEO_TS`: `VIDEO_TS.IFO` and
/// `.BUP`, `VTS_01_0.IFO` and `.BUP`, and `VTS_01_1.VOB` … (the VOB split at 1 GiB).
pub fn write_disc(dir: &Path, vob: &Path, spec: &TitleSetSpec) -> Result<Authored> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(vob)?;
    let authored = author(&mut f, spec)?;
    let out = dir.join("VIDEO_TS");
    std::fs::create_dir_all(&out)?;
    for (name, data) in
        [("VIDEO_TS.IFO", &authored.vmg_ifo), ("VIDEO_TS.BUP", &authored.vmg_ifo), ("VTS_01_0.IFO", &authored.vts_ifo), ("VTS_01_0.BUP", &authored.vts_ifo)]
    {
        std::fs::write(out.join(name), data)?;
    }
    f.seek(SeekFrom::Start(0))?;
    let mut left = authored.sectors;
    let mut part = 1;
    let mut buf = vec![0u8; SECTOR * 256];
    while left > 0 {
        let take = left.min(MAX_VOB_SECTORS);
        let mut w = File::create(out.join(format!("VTS_01_{part}.VOB")))?;
        let mut todo = take as usize;
        while todo > 0 {
            let n = todo.min(256);
            let b = &mut buf[..n * SECTOR];
            f.read_exact(b)?;
            w.write_all(b)?;
            todo -= n;
        }
        left -= take;
        part += 1;
    }
    Ok(authored)
}

#[cfg(test)]
mod tests;
