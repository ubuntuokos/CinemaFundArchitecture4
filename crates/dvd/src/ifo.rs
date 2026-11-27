//! The IFO (and BUP) files of a one-title disc: VIDEO_TS.IFO and VTS_01_0.IFO. Table layouts
//! follow the public DVD-Video format notes; every table starts on a sector boundary.

use std::ops::Range;

use crate::nav::Vobu;
use crate::vm;
use crate::{Audio, AudioKind, SECTOR, TitleSetSpec};

fn put16(b: &mut Vec<u8>, at: usize, v: u16) {
    if b.len() < at + 2 {
        b.resize(at + 2, 0);
    }
    b[at..at + 2].copy_from_slice(&v.to_be_bytes());
}

fn put32(b: &mut Vec<u8>, at: usize, v: u32) {
    if b.len() < at + 4 {
        b.resize(at + 4, 0);
    }
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn pad_to_sector(b: &mut Vec<u8>) {
    b.resize(b.len().div_ceil(SECTOR) * SECTOR, 0);
}

fn sectors(b: &[u8]) -> u32 {
    (b.len() / SECTOR) as u32
}

/// The 2 bytes of video attributes (MPEG-2, 720x480 / 720x576 and its sibling sizes).
fn video_attr(spec: &TitleSetSpec) -> [u8; 2] {
    let pal = u8::from(spec.timing == crate::Timing::Pal);
    let aspect = if spec.widescreen { 3 } else { 0 };
    let res = match spec.width {
        704 => 1,
        352 => 2,
        _ => 0,
    };
    // MPEG-2, standard, aspect, automatic pan&scan and letterbox disallowed
    [0x40 | pal << 4 | aspect << 2 | 0x03, res << 3]
}

fn audio_attr(a: &Audio) -> [u8; 8] {
    let mut b = [0u8; 8];
    let mode = match a.kind {
        AudioKind::Ac3 => 0,
        AudioKind::Mpeg => 2,
        AudioKind::Lpcm => 4,
    };
    let channels = a.channels.clamp(1, 8) - 1;
    b[0] = mode << 5 | u8::from(a.language.is_some()) << 2;
    b[1] = match a.kind {
        AudioKind::Lpcm => {
            (match a.lpcm_bits {
                20 => 1,
                24 => 2,
                _ => 0,
            }) << 6
                | u8::from(a.lpcm_rate == 96_000) << 4
        }
        AudioKind::Ac3 => 0xC0,
        AudioKind::Mpeg => 0,
    } | channels;
    if let Some(l) = a.language {
        b[2] = l[0];
        b[3] = l[1];
    }
    b[5] = 1;
    b
}

/// The VTS attribute block at 0x100..0x400 of the VTS IFO (also copied into the VIDEO_TS.IFO).
fn vts_attributes(spec: &TitleSetSpec) -> Vec<u8> {
    let mut b = vec![0u8; 0x300];
    // 0x200: video attributes of the title VOBs; 0x202: number of audio streams; 0x204: audio attributes
    b[0x100..0x102].copy_from_slice(&video_attr(spec));
    put16(&mut b, 0x102, spec.audio.len().min(8) as u16);
    for (i, a) in spec.audio.iter().take(8).enumerate() {
        b[0x104 + 8 * i..0x10C + 8 * i].copy_from_slice(&audio_attr(a));
    }
    b
}

/// BCD playback time hh:mm:ss:ff with the frame rate in the top two bits of the frame byte.
fn playback_time(spec: &TitleSetSpec, ticks: u32) -> u32 {
    spec.timing.timecode(ticks)
}

/// A program chain with one program per cell: no menu, no subpictures, `Exit` after the last cell.
fn pgc(spec: &TitleSetSpec, scan_vobus: &[Vobu], cells: &[Range<usize>], vob_id: u16, with_commands: Option<&[[u8; 8]]>) -> Vec<u8> {
    let n = cells.len();
    let mut p = vec![0u8; 0xEC];
    let total = scan_vobus.last().map_or(0, |v| v.e_ptm).wrapping_sub(scan_vobus.first().map_or(0, |v| v.s_ptm));
    p[2] = n as u8;
    p[3] = n as u8;
    put32(&mut p, 4, playback_time(spec, total));
    for (i, _) in spec.audio.iter().take(8).enumerate() {
        // stream available, sub-stream (or stream) number
        p[0x0C + 2 * i] = 0x80 | i as u8;
    }
    // palette: 16 greys (0, Y, Cr, Cb)
    for i in 0..16usize {
        let y = (16 + i * 14) as u8;
        p[0xA4 + 4 * i..0xA4 + 4 * i + 4].copy_from_slice(&[0, y, 0x80, 0x80]);
    }
    // commands: no pre commands, `with_commands` post commands
    let post = with_commands.unwrap_or(&[]);
    let mut table = vec![0u8; 8];
    put16(&mut table, 2, post.len() as u16);
    for c in post {
        table.extend_from_slice(c);
    }
    let end = (table.len() - 1) as u16;
    put16(&mut table, 6, end);
    let cmd_off = p.len();
    p.extend_from_slice(&table);
    // program map: program k starts at cell k + 1
    let pm = p.len();
    for k in 0..n {
        p.push(k as u8 + 1);
    }
    if n % 2 == 1 {
        p.push(0);
    }
    // cell playback information
    let cp = p.len();
    for cell in cells {
        let (Some(first), Some(last)) = (scan_vobus.get(cell.start), scan_vobus.get(cell.end.wrapping_sub(1))) else { continue };
        let mut e = [0u8; 24];
        put_timecode(&mut e, 4, playback_time(spec, last.e_ptm.wrapping_sub(first.s_ptm)));
        e[8..12].copy_from_slice(&first.first.to_be_bytes());
        e[12..16].copy_from_slice(&(first.first + first.sectors.saturating_sub(1)).to_be_bytes());
        e[16..20].copy_from_slice(&last.first.to_be_bytes());
        e[20..24].copy_from_slice(&(last.first + last.sectors.saturating_sub(1)).to_be_bytes());
        p.extend_from_slice(&e);
    }
    // cell position information: VOB id and cell id
    let pos = p.len();
    for k in 0..n {
        p.extend_from_slice(&vob_id.to_be_bytes());
        p.push(0);
        p.push(k as u8 + 1);
    }
    put16(&mut p, 0xE4, cmd_off as u16);
    put16(&mut p, 0xE6, pm as u16);
    put16(&mut p, 0xE8, cp as u16);
    put16(&mut p, 0xEA, pos as u16);
    p
}

fn put_timecode(b: &mut [u8], at: usize, v: u32) {
    if let Some(s) = b.get_mut(at..at + 4) {
        s.copy_from_slice(&v.to_be_bytes());
    }
}

/// A cell address table (VTS_C_ADT / VMGM_C_ADT / VTSM_C_ADT), padded to whole sectors.
pub(crate) fn c_adt(vobus: &[Vobu], cells: &[Range<usize>], vob_id: u16) -> Vec<u8> {
    let mut cadt = vec![0u8; 8];
    put16(&mut cadt, 0, 1);
    for (k, cell) in cells.iter().enumerate() {
        let (Some(first), Some(last)) = (vobus.get(cell.start), vobus.get(cell.end.wrapping_sub(1))) else { continue };
        cadt.extend_from_slice(&vob_id.to_be_bytes());
        cadt.push(k as u8 + 1);
        cadt.push(0);
        cadt.extend_from_slice(&first.first.to_be_bytes());
        cadt.extend_from_slice(&(last.first + last.sectors.saturating_sub(1)).to_be_bytes());
    }
    let end = (cadt.len() - 1) as u32;
    put32(&mut cadt, 4, end);
    pad_to_sector(&mut cadt);
    cadt
}

/// A VOBU address map (VTS_VOBU_ADMAP and its menu twins), padded to whole sectors.
pub(crate) fn vobu_admap(vobus: &[Vobu]) -> Vec<u8> {
    let mut admap = vec![0u8; 4];
    for v in vobus {
        admap.extend_from_slice(&v.first.to_be_bytes());
    }
    let end = (admap.len() - 1) as u32;
    put32(&mut admap, 0, end);
    pad_to_sector(&mut admap);
    admap
}

/// `Exit`.
const EXIT: [u8; 8] = [0x30, 0x01, 0, 0, 0, 0, 0, 0];

/// VTS_01_0.IFO. `vob_sectors` is the size of the title VOBs, `ifo_sectors_hint` is not needed:
/// the IFO is laid out once and the BUP is identical.
pub(crate) fn build_vts(spec: &TitleSetSpec, vobus: &[Vobu], cells: &[Range<usize>], vob_sectors: u32, vob_id: u16, menu: Option<&MenuIfo<'_>>) -> Vec<u8> {
    let chapters = cells.len();
    // VTS_PTT_SRPT: one title with one PTT per chapter (program chain 1, program k)
    let mut ptt = vec![0u8; 12];
    put16(&mut ptt, 0, 1);
    put32(&mut ptt, 8, 12);
    for k in 0..chapters {
        ptt.extend_from_slice(&1u16.to_be_bytes());
        ptt.extend_from_slice(&(k as u16 + 1).to_be_bytes());
    }
    let end = (ptt.len() - 1) as u32;
    put32(&mut ptt, 4, end);
    pad_to_sector(&mut ptt);
    // VTS_PGCI: one entry PGC (title 1)
    let pgc = pgc(spec, vobus, cells, vob_id, Some(&[EXIT]));
    let mut pgci = vec![0u8; 16];
    put16(&mut pgci, 0, 1);
    pgci[8] = 0x81;
    put32(&mut pgci, 12, 16);
    pgci.extend_from_slice(&pgc);
    let end = (pgci.len() - 1) as u32;
    put32(&mut pgci, 4, end);
    pad_to_sector(&mut pgci);
    // VTS_TMAPTI: one time map with an entry every `unit` seconds
    let total = vobus.last().map_or(0, |v| v.e_ptm).wrapping_sub(vobus.first().map_or(0, |v| v.s_ptm));
    let secs = (u64::from(total) / 90_000 + 1) as u32;
    let unit = secs.div_ceil(0xFFFF).max(1);
    let entries = secs.div_ceil(unit);
    let mut tm = vec![0u8; 12];
    put16(&mut tm, 0, 1);
    put32(&mut tm, 8, 12);
    tm.push(unit as u8);
    tm.push(0);
    tm.extend_from_slice(&(entries as u16).to_be_bytes());
    let base = vobus.first().map_or(0, |v| v.s_ptm);
    let mut j = 0usize;
    for e in 0..entries {
        let t = u64::from(e) * u64::from(unit) * 90_000;
        while j + 1 < vobus.len() && u64::from(vobus[j + 1].s_ptm.wrapping_sub(base)) <= t {
            j += 1;
        }
        tm.extend_from_slice(&vobus.get(j).map_or(0, |v| v.first).to_be_bytes());
    }
    let end = (tm.len() - 1) as u32;
    put32(&mut tm, 4, end);
    pad_to_sector(&mut tm);
    let cadt = c_adt(vobus, cells, vob_id);
    let admap = vobu_admap(vobus);

    let mat_sectors = 1u32;
    let ptt_at = mat_sectors;
    let pgci_at = ptt_at + sectors(&ptt);
    let tmap_at = pgci_at + sectors(&pgci);
    let cadt_at = tmap_at + sectors(&tm);
    let admap_at = cadt_at + sectors(&cadt);
    // the chapter menu (VTSM) tables follow the title tables
    let m_pgci_ut = menu.map(|m| menu_pgci_ut(spec, m, 0x87));
    let m_cadt = menu.map(|m| c_adt(m.vobus, m.cells, 1));
    let m_admap = menu.map(|m| vobu_admap(m.vobus));
    let m_pgci_at = admap_at + sectors(&admap);
    let m_cadt_at = m_pgci_at + m_pgci_ut.as_ref().map_or(0, |t| sectors(t));
    let m_admap_at = m_cadt_at + m_cadt.as_ref().map_or(0, |t| sectors(t));
    let ifo_sectors = m_admap_at + m_admap.as_ref().map_or(0, |t| sectors(t));
    let menu_sectors = menu.map_or(0, |m| m.vob_sectors);

    let mut mat = vec![0u8; SECTOR];
    mat[..12].copy_from_slice(b"DVDVIDEO-VTS");
    put32(&mut mat, 0x0C, ifo_sectors * 2 + menu_sectors + vob_sectors - 1);
    put32(&mut mat, 0x1C, ifo_sectors - 1);
    put16(&mut mat, 0x20, 0x0010);
    put32(&mut mat, 0x80, ifo_sectors * SECTOR as u32 - 1);
    if menu.is_some() {
        put32(&mut mat, 0xC0, ifo_sectors); // VTS_01_0.VOB follows the IFO
        put32(&mut mat, 0xD0, m_pgci_at);
        put32(&mut mat, 0xD8, m_cadt_at);
        put32(&mut mat, 0xDC, m_admap_at);
        mat[0x100..0x102].copy_from_slice(&video_attr(spec));
        put16(&mut mat, 0x154, 1);
    }
    put32(&mut mat, 0xC4, ifo_sectors + menu_sectors);
    put32(&mut mat, 0xC8, ptt_at);
    put32(&mut mat, 0xCC, pgci_at);
    put32(&mut mat, 0xD4, tmap_at);
    put32(&mut mat, 0xE0, cadt_at);
    put32(&mut mat, 0xE4, admap_at);
    let attr = vts_attributes(spec);
    mat[0x200..0x400].copy_from_slice(&attr[0x100..0x300]);
    let mut ifo = mat;
    for t in [ptt, pgci, tm, cadt, admap].into_iter().chain(m_pgci_ut).chain(m_cadt).chain(m_admap) {
        ifo.extend_from_slice(&t);
    }
    ifo
}

/// What the video manager menu adds to `VIDEO_TS.IFO`.
pub(crate) struct MenuIfo<'a> {
    pub vobus: &'a [Vobu],
    pub cells: &'a [Range<usize>],
    /// Sectors of the menu VOB (`VIDEO_TS.VOB`).
    pub vob_sectors: u32,
    /// The 16 palette entries of the menu PGC as (Y, Cr, Cb).
    pub palette: &'a [[u8; 3]; 16],
    /// The button highlighted on entry (1-based).
    pub first_button: u8,
    /// Language of the menu language unit.
    pub language: [u8; 2],
}

/// The program chain of the menu: one program of one cell that holds its last picture (still
/// time 255), the first button selected by the pre command, and a post command that links back to
/// the menu itself.
fn menu_pgc(spec: &TitleSetSpec, m: &MenuIfo<'_>, pgcn: u16) -> Vec<u8> {
    let mut p = vec![0u8; 0xEC];
    p[2] = 1;
    p[3] = 1;
    let total = m.vobus.last().map_or(0, |v| v.e_ptm).wrapping_sub(m.vobus.first().map_or(0, |v| v.s_ptm));
    put32(&mut p, 4, playback_time(spec, total));
    // sub-picture stream 0 is available (the same stream number for every display mode)
    p[0x1C] = 0x80;
    for (i, c) in m.palette.iter().enumerate() {
        p[0xA4 + 4 * i..0xA4 + 4 * i + 4].copy_from_slice(&[0, c[0], c[1], c[2]]);
    }
    let pre = [vm::set_hl_btnn(m.first_button)];
    let post = [vm::link_pgcn(pgcn)];
    let mut table = vec![0u8; 8];
    put16(&mut table, 0, pre.len() as u16);
    put16(&mut table, 2, post.len() as u16);
    for c in pre.iter().chain(post.iter()) {
        table.extend_from_slice(c);
    }
    let end = (table.len() - 1) as u16;
    put16(&mut table, 6, end);
    let cmd_off = p.len();
    p.extend_from_slice(&table);
    let pm = p.len();
    p.extend_from_slice(&[1, 0]);
    let cp = p.len();
    if let (Some(first), Some(last)) = (m.vobus.first(), m.vobus.last()) {
        let mut e = [0u8; 24];
        e[2] = 0xFF; // still time: until the viewer acts
        put_timecode(&mut e, 4, playback_time(spec, last.e_ptm.wrapping_sub(first.s_ptm)));
        e[8..12].copy_from_slice(&first.first.to_be_bytes());
        e[12..16].copy_from_slice(&(first.first + first.sectors.saturating_sub(1)).to_be_bytes());
        e[16..20].copy_from_slice(&last.first.to_be_bytes());
        e[20..24].copy_from_slice(&(last.first + last.sectors.saturating_sub(1)).to_be_bytes());
        p.extend_from_slice(&e);
    }
    let pos = p.len();
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&[0, 1]);
    put16(&mut p, 0xE4, cmd_off as u16);
    put16(&mut p, 0xE6, pm as u16);
    put16(&mut p, 0xE8, cp as u16);
    put16(&mut p, 0xEA, pos as u16);
    p
}

/// The program chain information table with one language unit and one entry program chain
/// (`entry_id` 0x82: the title menu), padded to whole sectors.
fn menu_pgci_ut(spec: &TitleSetSpec, m: &MenuIfo<'_>, entry_id: u8) -> Vec<u8> {
    let pgc = menu_pgc(spec, m, 1);
    let mut pgcit = vec![0u8; 16];
    put16(&mut pgcit, 0, 1);
    pgcit[8] = entry_id;
    put32(&mut pgcit, 12, 16);
    pgcit.extend_from_slice(&pgc);
    let end = (pgcit.len() - 1) as u32;
    put32(&mut pgcit, 4, end);
    let mut ut = vec![0u8; 16];
    put16(&mut ut, 0, 1);
    ut[8..10].copy_from_slice(&m.language);
    ut[11] = 0x80; // the language unit holds menus
    put32(&mut ut, 12, 16);
    ut.extend_from_slice(&pgcit);
    let end = (ut.len() - 1) as u32;
    put32(&mut ut, 4, end);
    pad_to_sector(&mut ut);
    ut
}

/// VIDEO_TS.IFO of a one-title disc whose title set starts after the menu (if any). Without a
/// menu the first-play command is `JumpTT 1` and the file is what it always was.
pub(crate) fn build_vmg(spec: &TitleSetSpec, chapters: usize, vts_ifo: &[u8], menu: Option<&MenuIfo<'_>>) -> Vec<u8> {
    // sector 0: MAT and the first-play PGC; 1: TT_SRPT; 2: VMG_VTS_ATRT; then the menu tables
    let mut tail: Vec<Vec<u8>> = Vec::new();
    let pgci_ut = menu.map(|m| menu_pgci_ut(spec, m, 0x82));
    let c_adt_t = menu.map(|m| c_adt(m.vobus, m.cells, 1));
    let admap_t = menu.map(|m| vobu_admap(m.vobus));
    tail.extend(pgci_ut.iter().chain(c_adt_t.iter()).chain(admap_t.iter()).cloned());
    let n = 3 + tail.iter().map(|t| sectors(t)).sum::<u32>();
    let menu_sectors = menu.map_or(0, |m| m.vob_sectors);
    let vts_sector = 2 * n + menu_sectors;
    let mut fp = vec![0u8; 0xEC];
    // first play: the menu if there is one, else the title, as the one pre command
    let mut table = vec![0u8; 8];
    put16(&mut table, 0, 1);
    table.extend_from_slice(&if menu.is_some() { vm::jump_ss(vm::Menu::Vmgm(2)) } else { vm::jump_tt(1) });
    let end = (table.len() - 1) as u16;
    put16(&mut table, 6, end);
    let off = fp.len() as u16;
    put16(&mut fp, 0xE4, off);
    fp.extend_from_slice(&table);
    let mut mat = vec![0u8; SECTOR];
    mat[..12].copy_from_slice(b"DVDVIDEO-VMG");
    put32(&mut mat, 0x0C, 2 * n + menu_sectors - 1);
    put32(&mut mat, 0x1C, n - 1);
    put16(&mut mat, 0x20, 0x0010);
    put16(&mut mat, 0x26, 1);
    put16(&mut mat, 0x28, 1);
    mat[0x2A] = 1;
    put16(&mut mat, 0x3E, 1);
    mat[0x40..0x40 + 8].copy_from_slice(b"CFA4    ");
    put32(&mut mat, 0x80, n * SECTOR as u32 - 1);
    put32(&mut mat, 0x84, 0x400);
    put32(&mut mat, 0xC4, 1);
    put32(&mut mat, 0xD0, 2);
    if menu.is_some() {
        let mut at = 3;
        put32(&mut mat, 0xC0, n); // VIDEO_TS.VOB follows the IFO
        put32(&mut mat, 0xC8, at);
        at += pgci_ut.as_ref().map_or(0, |t| sectors(t));
        put32(&mut mat, 0xD8, at);
        at += c_adt_t.as_ref().map_or(0, |t| sectors(t));
        put32(&mut mat, 0xDC, at);
        // the menu video attributes, no audio, one sub-picture stream (2-bit RLE)
        mat[0x100..0x102].copy_from_slice(&video_attr(spec));
        put16(&mut mat, 0x154, 1);
    }
    mat[0x400..0x400 + fp.len()].copy_from_slice(&fp);
    // TT_SRPT: one title
    let mut tt = vec![0u8; 8];
    put16(&mut tt, 0, 1);
    let mut e = [0u8; 12];
    e[0] = 0x03;
    e[1] = 1;
    e[2..4].copy_from_slice(&(chapters as u16).to_be_bytes());
    e[6] = 1;
    e[7] = 1;
    e[8..12].copy_from_slice(&vts_sector.to_be_bytes());
    tt.extend_from_slice(&e);
    let end = (tt.len() - 1) as u32;
    put32(&mut tt, 4, end);
    pad_to_sector(&mut tt);
    // VMG_VTS_ATRT: a copy of the VTS attributes
    let mut at = vec![0u8; 8];
    put16(&mut at, 0, 1);
    put32(&mut at, 8, 12);
    let mut vts = vec![0u8; 8];
    put32(&mut vts, 0, 7 + 0x300);
    vts.extend_from_slice(&vts_ifo.get(0x100..0x400).map_or_else(|| vts_attributes(spec), <[u8]>::to_vec));
    at.extend_from_slice(&vts);
    let end = (at.len() - 1) as u32;
    put32(&mut at, 4, end);
    pad_to_sector(&mut at);
    let mut ifo = mat;
    ifo.extend_from_slice(&tt);
    ifo.extend_from_slice(&at);
    for t in &tail {
        ifo.extend_from_slice(t);
    }
    ifo
}
