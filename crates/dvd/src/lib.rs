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
//! Layouts come from the public DVD-Video format notes and, for the VM commands, the button
//! table and the IFO tables, libdvdnav / libdvdread (GPL-2.0-or-later) field order: one title, one
//! program chain whose chapters are its programs and cells; no interleaving or angles. Menus
//! ([`DiscMenus`], [`write_disc_with_menus`]): a title menu in `VIDEO_TS.VOB` and an optional
//! chapter menu in `VTS_01_0.VOB`, each a still picture with a sub-picture and buttons whose
//! actions are VM commands. Subtitles (D2), several titles and angles (D3) and pulldown (D4) are
//! later phases of the plan.
//! The picture duration of a VOBU ignores repeat_first_field (3:2 pulldown).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod hli;
mod ifo;
mod menu;
mod nav;
mod spu;
#[allow(dead_code)]
mod vm;

pub use hli::ColorSet;
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

/// Scan the VOB in `vob`, fill its navigation packs (in place) and find its cells.
fn author_parts<F: Read + Write + Seek>(vob: &mut F, spec: &TitleSetSpec) -> Result<(nav::Scan, Vec<std::ops::Range<usize>>)> {
    let kinds: Vec<AudioKind> = spec.audio.iter().map(|a| a.kind).collect();
    let scan = nav::scan(vob, &kinds, spec.timing)?;
    if scan.vobus.iter().any(|v| v.s_ptm == u32::MAX) {
        return Err(Error::Vob("a VOBU without a video picture".into()));
    }
    let cells = nav::cells(&scan.vobus, &spec.chapters);
    nav::fill(vob, &scan, &cells, 1, spec.audio.len().min(8), spec.timing)?;
    Ok((scan, cells))
}

/// Fill the navigation packs of the VOB in `vob` (in place) and build the IFO files for it.
pub fn author<F: Read + Write + Seek>(vob: &mut F, spec: &TitleSetSpec) -> Result<Authored> {
    let (scan, cells) = author_parts(vob, spec)?;
    let vts_ifo = ifo::build_vts(spec, &scan.vobus, &cells, scan.total_sectors, 1, None);
    let vmg_ifo = ifo::build_vmg(spec, cells.len(), &vts_ifo, None);
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
    write_title_vobs(&mut f, &out, authored.sectors)?;
    Ok(authored)
}

/// What a menu button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Title menu: play title `n` from its start.
    PlayTitle(u8),
    /// Title menu: open the chapter menu.
    ShowChapterMenu,
    /// Chapter menu: play title 1 of the title set from chapter `n`.
    PlayChapter(u16),
    /// Chapter menu: back to the title menu.
    ShowMainMenu,
}

/// One button: the area that is highlighted (inclusive pixels of the 720 x 480/576 picture) and
/// its action. Buttons are numbered in the order given; up / down move through that list.
#[derive(Clone, Copy, Debug)]
pub struct MenuButton {
    pub rect: (u16, u16, u16, u16),
    pub action: MenuAction,
}

/// The button graphics: a 2-bit bitmap (values 0..=3, row by row) at (`x`, `y`) of the picture.
#[derive(Clone, Debug)]
pub struct Overlay {
    pub x: u16,
    pub y: u16,
    pub w: usize,
    pub h: usize,
    pub pixels: Vec<u8>,
}

/// One menu. The caller renders the background picture and the button graphics (the writer only
/// codes them).
#[derive(Clone, Debug)]
pub struct MenuSpec {
    /// One coded MPEG-2 I picture: a sequence header, a GOP header and the picture, 720 wide and
    /// as high as the title (480 or 576). Small pictures are better: the whole menu has to reach
    /// the decoder at the DVD rate by its presentation time.
    pub picture: Vec<u8>,
    pub overlay: Overlay,
    /// The 16 palette entries of the menu as (Y, Cr, Cb), shared by all colour sets.
    pub palette: [[u8; 3]; 16],
    /// Colours (palette index and contrast of the four overlay values) when the menu is shown,
    /// when a button is selected and when it is activated.
    pub normal: ColorSet,
    pub select: ColorSet,
    pub activate: ColorSet,
    pub buttons: Vec<MenuButton>,
    /// ISO 639-1 language of the menu, for example `*b"en"`.
    pub language: [u8; 2],
}

/// The menus of a disc: the title menu (shown first, in `VIDEO_TS.VOB`) and optionally a chapter
/// menu (in `VTS_01_0.VOB`).
#[derive(Clone, Debug)]
pub struct DiscMenus {
    pub main: MenuSpec,
    pub chapters: Option<MenuSpec>,
}

/// The menu VOBs and the IFO files built with menus.
#[derive(Clone, Debug)]
pub struct AuthoredMenus {
    pub disc: Authored,
    /// `VIDEO_TS.VOB`: the title menu.
    pub menu_vob: Vec<u8>,
    /// `VTS_01_0.VOB`: the chapter menu.
    pub chapter_vob: Option<Vec<u8>>,
}

/// Which domain a menu lives in decides which commands its buttons may use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Domain {
    /// Video manager menu; whether a chapter menu exists to open.
    Vmgm {
        chapter_menu: bool,
    },
    Vtsm,
}

/// The menu id of the chapter (part-of-title) menu in the title set.
const PTT_MENU: u8 = 7;

fn action_command(a: MenuAction, domain: Domain) -> Result<vm::Cmd> {
    Ok(match (a, domain) {
        (MenuAction::PlayTitle(n), Domain::Vmgm { .. }) if (1..=99).contains(&n) => vm::jump_tt(n),
        (MenuAction::ShowChapterMenu, Domain::Vmgm { chapter_menu: true }) => vm::jump_ss(vm::Menu::Vtsm { vts: 1, title: 1, menu: PTT_MENU }),
        (MenuAction::PlayChapter(c), Domain::Vtsm) if c >= 1 => vm::jump_vts_ptt(1, c),
        (MenuAction::ShowMainMenu, Domain::Vtsm) => vm::jump_ss(vm::Menu::Vmgm(2)),
        (other, _) => return Err(Error::Vob(format!("{other:?} is not possible in this menu"))),
    })
}

/// A menu VOB with its navigation packs filled, and what the scan found.
struct BuiltMenu {
    vob: Vec<u8>,
    scan: nav::Scan,
    cells: Vec<std::ops::Range<usize>>,
}

fn build_menu(spec: &TitleSetSpec, menu: &MenuSpec, domain: Domain) -> Result<BuiltMenu> {
    if menu.buttons.is_empty() {
        return Err(Error::Vob("a menu needs at least one button".into()));
    }
    let o = &menu.overlay;
    let spu = spu::encode_menu(&spu::Spu { bitmap: &o.pixels, w: o.w, h: o.h, x: o.x, y: o.y, color: menu.normal.index, alpha: menu.normal.alpha })?;
    let n = menu.buttons.len();
    let mut buttons = Vec::with_capacity(n);
    for (i, b) in menu.buttons.iter().enumerate() {
        // buttons stack vertically in the order given; at the ends a button stays put
        let me = (i + 1) as u8;
        buttons.push(hli::Button {
            rect: b.rect,
            color: 1,
            auto_action: false,
            up: if i == 0 { me } else { me - 1 },
            down: if i + 1 == n { me } else { me + 1 },
            left: me,
            right: me,
            cmd: action_command(b.action, domain)?,
        });
    }
    let template =
        hli::Hli { status: 1, start_ptm: 0, end_ptm: 0, wide: spec.widescreen, colors: [[menu.select, menu.activate]; 3], buttons, forced_select: 0 };
    let raw = menu::build_vob(&menu.picture, menu::MENU_FRAMES, spec.timing, &spu)?;
    let mut cur = std::io::Cursor::new(raw);
    let m = menu::author_vob(&mut cur, &template, spec.timing, 1)?;
    Ok(BuiltMenu { vob: cur.into_inner(), scan: m.scan, cells: m.cells })
}

/// Like [`author`], and builds the menus: `VIDEO_TS.IFO` gets the title menu and a first-play
/// command that shows it, `VTS_01_0.IFO` the chapter menu if there is one.
pub fn author_with_menus<F: Read + Write + Seek>(vob: &mut F, spec: &TitleSetSpec, menus: &DiscMenus) -> Result<AuthoredMenus> {
    let (scan, cells) = author_parts(vob, spec)?;
    let main = build_menu(spec, &menus.main, Domain::Vmgm { chapter_menu: menus.chapters.is_some() })?;
    let chapters = menus.chapters.as_ref().map(|c| build_menu(spec, c, Domain::Vtsm)).transpose()?;
    let main_info = ifo::MenuIfo {
        vobus: &main.scan.vobus,
        cells: &main.cells,
        vob_sectors: main.scan.total_sectors,
        palette: &menus.main.palette,
        first_button: 1,
        language: menus.main.language,
    };
    let chapter_info = chapters.as_ref().zip(menus.chapters.as_ref()).map(|(b, m)| ifo::MenuIfo {
        vobus: &b.scan.vobus,
        cells: &b.cells,
        vob_sectors: b.scan.total_sectors,
        palette: &m.palette,
        first_button: 1,
        language: m.language,
    });
    let vts_ifo = ifo::build_vts(spec, &scan.vobus, &cells, scan.total_sectors, 1, chapter_info.as_ref());
    let vmg_ifo = ifo::build_vmg(spec, cells.len(), &vts_ifo, Some(&main_info));
    let disc = Authored { vmg_ifo, vts_ifo, vobus: scan.vobus.len(), chapters: cells.len(), sectors: scan.total_sectors };
    Ok(AuthoredMenus { disc, menu_vob: main.vob, chapter_vob: chapters.map(|c| c.vob) })
}

/// Like [`write_disc`], with menus: also writes `VIDEO_TS.VOB` / `VIDEO_TS.BUP` (title menu) and
/// `VTS_01_0.VOB` (chapter menu), and IFO files that show the title menu first.
pub fn write_disc_with_menus(dir: &Path, vob: &Path, spec: &TitleSetSpec, menus: &DiscMenus) -> Result<AuthoredMenus> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(vob)?;
    let authored = author_with_menus(&mut f, spec, menus)?;
    let out = dir.join("VIDEO_TS");
    std::fs::create_dir_all(&out)?;
    for (name, data) in [
        ("VIDEO_TS.IFO", &authored.disc.vmg_ifo),
        ("VIDEO_TS.BUP", &authored.disc.vmg_ifo),
        ("VTS_01_0.IFO", &authored.disc.vts_ifo),
        ("VTS_01_0.BUP", &authored.disc.vts_ifo),
        ("VIDEO_TS.VOB", &authored.menu_vob),
    ] {
        std::fs::write(out.join(name), data)?;
    }
    if let Some(c) = &authored.chapter_vob {
        std::fs::write(out.join("VTS_01_0.VOB"), c)?;
    }
    write_title_vobs(&mut f, &out, authored.disc.sectors)?;
    Ok(authored)
}

/// Split the authored title VOB into `VTS_01_n.VOB` files of at most 1 GiB.
fn write_title_vobs(f: &mut File, out: &Path, sectors: u32) -> Result<()> {
    f.seek(SeekFrom::Start(0))?;
    let mut left = sectors;
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
    Ok(())
}

#[cfg(test)]
mod tests;
