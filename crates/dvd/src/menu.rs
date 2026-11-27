//! The menu VOB (CFA4 #1812, D1): a still picture repeated for about a second, a sub-picture with
//! the button graphics, and a navigation pack per VOBU that carries the button table.
//!
//! [`build_vob`] writes the packs (an empty navigation pack, then the MPEG-2 video of the picture,
//! then the sub-picture in the first VOBU); [`author_vob`] then lets the normal scan measure the
//! VOBUs, fills their PCI / DSI and writes the button table into every PCI.

use std::io::{Read, Seek, Write};

use crate::hli::{self, Hli};
use crate::{Error, Result, SECTOR, Timing, nav};

/// Frames in one menu VOBU (0.6 s at 25 fps, 0.5 s at 29.97 fps; a VOBU holds 0.4 to 1.2 s).
pub(crate) const FRAMES_PER_VOBU: usize = 15;
/// The menu plays at least this many frames, then the cell holds the last picture (still time).
pub(crate) const MENU_FRAMES: usize = 2 * FRAMES_PER_VOBU;
/// Presentation time of the first picture, 90 kHz (0.5 s: the video buffer fills before it).
const FIRST_PTS: u64 = 45_000;
/// Mux rate in 50-byte units (10.08 Mbit/s, the DVD maximum).
const MUX_RATE: u32 = 25_200;
/// A pack at the mux rate in 27 MHz ticks: 2048 * 8 * 27e6 / 10.08e6.
const PACK_TICKS: u64 = 43_886;

fn pts_bytes(prefix: u8, v: u64) -> [u8; 5] {
    [prefix << 4 | ((v >> 29) & 0x0E) as u8 | 1, (v >> 22) as u8, ((v >> 14) & 0xFE) as u8 | 1, (v >> 7) as u8, ((v << 1) & 0xFE) as u8 | 1]
}

fn pack_header(out: &mut Vec<u8>, clock: u64) {
    let (base, ext) = (clock / 300, clock % 300);
    out.extend_from_slice(&[0, 0, 1, 0xBA]);
    out.extend_from_slice(&[
        0x44 | ((base >> 27) & 0x38) as u8 | ((base >> 28) & 3) as u8,
        (base >> 20) as u8,
        ((base >> 12) & 0xF8) as u8 | 4 | ((base >> 13) & 3) as u8,
        (base >> 5) as u8,
        ((base << 3) & 0xF8) as u8 | 4 | ((ext >> 7) & 3) as u8,
        ((ext << 1) & 0xFE) as u8 | 1,
        (MUX_RATE >> 14) as u8,
        (MUX_RATE >> 6) as u8,
        ((MUX_RATE << 2) & 0xFC) as u8 | 3,
        0xF8,
    ]);
}

/// The empty navigation pack of a DVD VOBU: pack header, system header, PCI and DSI of zeros.
fn nav_pack(clock: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(SECTOR);
    pack_header(&mut p, clock);
    let bound = MUX_RATE;
    p.extend_from_slice(&[0, 0, 1, 0xBB, 0, 0x12, 0x80 | (bound >> 15) as u8, (bound >> 7) as u8, ((bound << 1) & 0xFE) as u8 | 1, 0x04, 0xE1, 0x7F]);
    p.extend_from_slice(&[0xE0, 0xE0, 0xE8, 0xC0, 0xC0, 0x20, 0xBD, 0xE0, 0x3A, 0xBF, 0xE0, 0x02]);
    p.extend_from_slice(&[0, 0, 1, 0xBF, 0x03, 0xD4, 0x00]);
    p.resize(0x400, 0);
    p.extend_from_slice(&[0, 0, 1, 0xBF, 0x03, 0xFA, 0x01]);
    p.resize(SECTOR, 0);
    p
}

/// Writes the packs of the VOB: one PES packet per pack, padded to 2048 bytes.
struct Packer {
    out: Vec<u8>,
    clock: u64,
}

impl Packer {
    fn nav(&mut self) {
        let p = nav_pack(self.clock);
        self.out.extend_from_slice(&p);
        self.clock += PACK_TICKS;
    }

    /// `stream_id` data in packs; the first packet carries `pts`. `prefix` goes in front of the
    /// payload of every packet (private stream 1 sub-stream id).
    fn pes(&mut self, stream_id: u8, pts: u64, prefix: &[u8], data: &[u8]) {
        let mut first = true;
        let mut left = data;
        while first || !left.is_empty() {
            let mut p = Vec::with_capacity(SECTOR);
            pack_header(&mut p, self.clock);
            let room = SECTOR - p.len();
            let pts_len = if first { 5 } else { 0 };
            let pre = prefix.len();
            // 6 packet header + 3 flag bytes + the optional PTS, then the payload
            let overhead = 9 + pts_len;
            let take = left.len().min(room - overhead - pre);
            let used = overhead + pre + take;
            let free = room - used;
            // up to 5 spare bytes go in as stuffing of the PES header, more as a padding packet
            let stuffing = if free > 0 && free < 6 { free } else { 0 };
            let hdl = pts_len + stuffing;
            let plen = 3 + hdl + pre + take;
            p.extend_from_slice(&[0, 0, 1, stream_id, (plen >> 8) as u8, plen as u8, 0x80, if first { 0x80 } else { 0 }, hdl as u8]);
            if first {
                p.extend_from_slice(&pts_bytes(2, pts));
            }
            p.extend(std::iter::repeat_n(0xFFu8, stuffing));
            p.extend_from_slice(prefix);
            p.extend_from_slice(&left[..take]);
            left = &left[take..];
            let free = SECTOR - p.len();
            if free >= 6 {
                p.extend_from_slice(&[0, 0, 1, 0xBE, ((free - 6) >> 8) as u8, (free - 6) as u8]);
                p.resize(SECTOR, 0xFF);
            }
            self.out.extend_from_slice(&p);
            self.clock += PACK_TICKS;
            first = false;
        }
    }
}

/// Build the menu VOB: `frames` copies of the coded picture `picture` (a sequence header, a GOP
/// header and one I picture, as `cfa4-mpeg2v` writes it), in VOBUs of [`FRAMES_PER_VOBU`], the
/// sub-picture `spu` in the first VOBU. Fails when the pictures cannot reach the decoder in time
/// at the mux rate.
pub(crate) fn build_vob(picture: &[u8], frames: usize, timing: Timing, spu: &[u8]) -> Result<Vec<u8>> {
    if picture.len() < 16 || picture.get(..4) != Some(&[0, 0, 1, 0xB3]) {
        return Err(Error::Vob("the menu picture must start with an MPEG-2 sequence header".into()));
    }
    let frames = frames.max(1).div_ceil(FRAMES_PER_VOBU) * FRAMES_PER_VOBU;
    let frame = u64::from(timing.frame_ticks());
    let mut p = Packer { out: Vec::new(), clock: 0 };
    for k in 0..frames {
        let pts = FIRST_PTS + k as u64 * frame;
        if k % FRAMES_PER_VOBU == 0 {
            p.nav();
            if k == 0 {
                p.pes(0xBD, pts, &[0x20], spu);
            }
        }
        p.pes(0xE0, pts, &[], picture);
        // every byte of the picture must have arrived by its presentation time
        let arrived = p.clock / 300;
        if arrived > pts {
            return Err(Error::Vob(format!("menu picture too large: frame {} arrives {} ticks after its presentation time", k + 1, arrived - pts)));
        }
    }
    Ok(p.out)
}

/// What [`author_vob`] found.
pub(crate) struct Authored {
    pub scan: nav::Scan,
    pub cells: Vec<std::ops::Range<usize>>,
}

/// Fill the navigation packs of the menu VOB in `vob` and write the button table `hli` into every
/// PCI (`hli_ss` 1 in the first VOBU, 2, "same as before", in the others).
pub(crate) fn author_vob<F: Read + Write + Seek>(vob: &mut F, hli: &Hli, timing: Timing, vob_id: u16) -> Result<Authored> {
    let scan = nav::scan(vob, &[], timing)?;
    if scan.vobus.iter().any(|v| v.s_ptm == u32::MAX) {
        return Err(Error::Vob("a menu VOBU without a video picture".into()));
    }
    // the whole menu is one cell
    let whole = 0..scan.vobus.len();
    let cells = vec![whole];
    nav::fill(vob, &scan, &cells, vob_id, 0, timing)?;
    let first_ptm = scan.vobus.first().map_or(0, |v| v.s_ptm);
    let mut sector = vec![0u8; SECTOR];
    for (i, v) in scan.vobus.iter().enumerate() {
        vob.seek(std::io::SeekFrom::Start(u64::from(v.first) * SECTOR as u64))?;
        vob.read_exact(&mut sector)?;
        let mut h = hli.clone();
        h.start_ptm = first_ptm;
        h.end_ptm = 0xFFFF_FFFF; // an infinite still: the buttons stay
        h.status = if i == 0 { 1 } else { 2 };
        hli::write(&mut sector, &h)?;
        vob.seek(std::io::SeekFrom::Start(u64::from(v.first) * SECTOR as u64))?;
        vob.write_all(&sector)?;
    }
    Ok(Authored { scan, cells })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::process::Command;

    use super::*;
    use crate::hli::{Button, ColorSet};
    use crate::{spu, vm};

    /// One MPEG-2 I picture (sequence header, GOP, picture) of a flat-colour 720x576 frame.
    fn still_picture() -> Option<Vec<u8>> {
        let out = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0x203060:size=720x576:rate=25",
                "-frames:v",
                "1",
                "-c:v",
                "mpeg2video",
                "-q:v",
                "6",
                "-g",
                "1",
                "-f",
                "mpeg2video",
                "-",
            ])
            .output()
            .ok()?;
        (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
    }

    fn hli() -> Hli {
        let set = |i: u8| ColorSet { index: [0, i, i + 1, i + 2], alpha: [0, 15, 15, 15] };
        let b = |y: u16, up, down, cmd| Button { rect: (100, y, 500, y + 40), color: 1, auto_action: false, up, down, left: 0, right: 0, cmd };
        Hli {
            status: 1,
            start_ptm: 0,
            end_ptm: 0,
            wide: false,
            colors: [[set(1), set(5)], [set(2), set(6)], [set(3), set(7)]],
            buttons: vec![b(100, 0, 2, vm::jump_tt(1)), b(200, 1, 0, vm::jump_tt(2))],
            forced_select: 0,
        }
    }

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    #[test]
    fn a_menu_vob_has_two_vobus_with_buttons_and_the_subpicture() {
        let Some(pic) = still_picture() else { return };
        let bitmap = vec![1u8; 400 * 40];
        let s = spu::encode_menu(&spu::Spu { bitmap: &bitmap, w: 400, h: 40, x: 100, y: 100, color: [0, 1, 2, 3], alpha: [0, 15, 15, 15] }).expect("spu");
        let vob = build_vob(&pic, MENU_FRAMES, Timing::Pal, &s).expect("vob");
        assert_eq!(vob.len() % SECTOR, 0);
        let mut cur = Cursor::new(vob);
        let a = author_vob(&mut cur, &hli(), Timing::Pal, 1).expect("author");
        assert_eq!(a.scan.vobus.len(), 2);
        assert_eq!(a.cells, vec![0..2]);
        let vob = cur.into_inner();
        for (i, v) in a.scan.vobus.iter().enumerate() {
            let nav = &vob[v.first as usize * SECTOR..][..SECTOR];
            // PCI: this pack's address, highlight status, end of highlight, button count
            assert_eq!(be32(nav, 0x2D), v.first, "nv_pck_lbn");
            let hli_at = 0x2D + 0x60;
            assert_eq!(u16::from_be_bytes([nav[hli_at], nav[hli_at + 1]]), if i == 0 { 1 } else { 2 });
            assert_eq!(be32(nav, hli_at + 6), 0xFFFF_FFFF, "hli_e_ptm");
            assert_eq!(nav[hli_at + 17], 2, "btn_ns");
            // DSI: VOBU end address equals the VOBU size
            assert_eq!(be32(nav, 0x40F), v.sectors - 1);
        }
        // the sub-picture is in the first VOBU: a private stream 1 packet with sub-stream 0x20
        let first = &vob[..a.scan.vobus[0].sectors as usize * SECTOR];
        assert!(first.windows(s.len() + 1).any(|w| w[0] == 0x20 && w[1..] == s[..]));
    }

    #[test]
    fn a_picture_that_cannot_arrive_in_time_is_refused() {
        let mut big = vec![0, 0, 1, 0xB3];
        big.extend(std::iter::repeat_n(0x55u8, 200_000));
        assert!(build_vob(&big, MENU_FRAMES, Timing::Pal, &[0, 4, 0, 4]).is_err());
        assert!(build_vob(&[1, 2, 3], 1, Timing::Pal, &[]).is_err());
    }

    #[test]
    fn the_menu_vob_plays_in_ffprobe() {
        let Some(pic) = still_picture() else { return };
        let s = spu::encode_menu(&spu::Spu { bitmap: &[1u8; 80], w: 40, h: 2, x: 0, y: 0, color: [0, 1, 2, 3], alpha: [0, 15, 15, 15] }).expect("spu");
        let mut cur = Cursor::new(build_vob(&pic, MENU_FRAMES, Timing::Pal, &s).expect("vob"));
        author_vob(&mut cur, &hli(), Timing::Pal, 1).expect("author");
        let path = std::env::temp_dir().join(format!("cfa4-dvd-menu-{}.vob", std::process::id()));
        std::fs::write(&path, cur.into_inner()).expect("write");
        let Ok(out) = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height,nb_read_frames",
                "-of",
                "default=nw=1",
            ])
            .arg(&path)
            .output()
        else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("codec_name=mpeg2video") && text.contains("width=720") && text.contains("height=576"), "{text}");
        assert!(text.contains("nb_read_frames=30"), "{text}");
    }

    #[test]
    fn every_packet_of_a_long_subpicture_starts_with_the_substream_id() {
        let mut p = Packer { out: Vec::new(), clock: 0 };
        p.pes(0xBD, 90_000, &[0x20], &vec![0x55u8; 6000]);
        assert!(p.out.len() >= 3 * SECTOR && p.out.len().is_multiple_of(SECTOR));
        let mut payload = 0;
        for sector in p.out.chunks(SECTOR) {
            // pack header (14 bytes, stuffing byte count in the last) then the PES packet
            let at = 14 + usize::from(sector[13] & 7);
            assert_eq!(&sector[at..at + 4], &[0, 0, 1, 0xBD]);
            let hdl = usize::from(sector[at + 8]);
            let plen = usize::from(u16::from_be_bytes([sector[at + 4], sector[at + 5]]));
            assert_eq!(sector[at + 9 + hdl], 0x20, "sub-stream id");
            payload += plen - 3 - hdl - 1;
        }
        assert_eq!(payload, 6000);
    }
}
