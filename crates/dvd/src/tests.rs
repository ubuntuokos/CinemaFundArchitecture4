use std::path::PathBuf;
use std::process::Command;

use super::*;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dvd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn tool(name: &str) -> bool {
    Command::new(name).arg("--help").output().is_ok() || Command::new(name).arg("-version").output().is_ok()
}

fn spec(chapters: Vec<u64>) -> TitleSetSpec {
    TitleSetSpec {
        timing: Timing::Pal,
        widescreen: false,
        width: 720,
        audio: vec![Audio { kind: AudioKind::Ac3, channels: 2, language: Some(*b"en"), lpcm_bits: 0, lpcm_rate: 0 }],
        chapters,
    }
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[test]
fn timecodes_are_bcd_with_the_frame_rate() {
    // 1 h 2 min 3 s 4 frames at 25 fps
    let ticks = ((3600 + 120 + 3) * 25 + 4) * 3600;
    assert_eq!(Timing::Pal.timecode(ticks), 0x0102_0344);
    assert_eq!(Timing::Ntsc.timecode(3003 * 31), 0x0000_01C1);
}

/// A PAL VOB that ffmpeg's DVD muxer wrote: 8 s of video and AC-3 with a GOP of 15.
fn ffmpeg_vob(d: &std::path::Path) -> Option<PathBuf> {
    let out = d.join("in.vob");
    let ok = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=720x576:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "8",
            "-target",
            "pal-dvd",
            "-ac",
            "2",
        ])
        .arg(&out)
        .status()
        .ok()?
        .success();
    ok.then_some(out)
}

#[test]
fn an_ffmpeg_vob_gets_navigation_packs_and_ifo_files() {
    let d = dir("ffmpeg");
    let Some(vob) = ffmpeg_vob(&d) else { return };
    let disc = d.join("disc");
    let a = write_disc(&disc, &vob, &spec(vec![2 * 90_000, 5 * 90_000])).unwrap();
    assert_eq!(a.chapters, 3);
    assert!(a.vobus >= 12, "{} VOBUs", a.vobus);
    let v = std::fs::read(disc.join("VIDEO_TS/VTS_01_1.VOB")).unwrap();
    assert_eq!(v.len(), a.sectors as usize * SECTOR);
    // every navigation pack has its own sector number, a VOBU end and a first reference picture
    let mut navs = 0;
    for (n, s) in v.chunks(SECTOR).enumerate() {
        if s[0x26..0x2C] == [0, 0, 1, 0xBF, 3, 0xD4] {
            navs += 1;
            assert_eq!(be32(s, 0x2D), n as u32, "PCI lbn");
            assert_eq!(be32(s, 0x40B), n as u32, "DSI lbn");
            let ea = be32(s, 0x40F);
            assert!(ea > 0 && (n as u32 + ea) < a.sectors, "vobu_ea {ea}");
            let (r1, r2, r3) = (be32(s, 0x413), be32(s, 0x417), be32(s, 0x41B));
            assert!(0 < r1 && r1 <= r2 && r2 <= r3 && r3 <= ea, "refs {r1} {r2} {r3} of {ea}");
            assert!(be32(s, 0x3D) > be32(s, 0x39), "PCI end time after start");
        }
    }
    assert_eq!(navs, a.vobus);
    for (name, len) in
        [("VIDEO_TS.IFO", a.vmg_ifo.len()), ("VIDEO_TS.BUP", a.vmg_ifo.len()), ("VTS_01_0.IFO", a.vts_ifo.len()), ("VTS_01_0.BUP", a.vts_ifo.len())]
    {
        assert_eq!(std::fs::metadata(disc.join("VIDEO_TS").join(name)).unwrap().len() as usize, len);
        assert_eq!(len % SECTOR, 0);
    }
    if tool("lsdvd") {
        let o = Command::new("lsdvd").arg("-x").arg(&disc).output().unwrap();
        let text = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{text}");
        assert!(text.contains("Title: 01, Length: 00:00:08"), "{text}");
        assert!(text.contains("Chapters: 03"), "{text}");
        assert!(text.contains("Audio: 1, Language: en") && text.contains("Format: ac3"), "{text}");
        assert!(text.contains("Aspect ratio: 4/3") && text.contains("Format: PAL"), "{text}");
    }
    if std::env::var_os("CFA4_DVD_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// The navigation fields that do not depend on how the muxer interleaved the audio.
fn nav_fields(vob: &[u8]) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for (n, s) in vob.chunks(SECTOR).enumerate() {
        if s[0x26..0x2C] == [0, 0, 1, 0xBF, 3, 0xD4] {
            let mut f = vec![n as u32];
            f.extend([0x2D, 0x39, 0x3D, 0x45, 0x40B, 0x40F, 0x413, 0x417, 0x41B, 0x423, 0x433, 0x437, 0x4F1, 0x541, 0x545, 0x595].map(|o| be32(s, o)));
            f.extend((0..19).map(|i| be32(s, 0x4F5 + 4 * i)));
            f.extend((0..19).map(|i| be32(s, 0x549 + 4 * i)));
            f.push(u32::from(s[0x422]));
            f.push(u32::from(s[0x41F]) << 8 | u32::from(s[0x420]));
            out.push(f);
        }
    }
    out
}

#[test]
fn navigation_packs_equal_dvdauthors_for_the_same_vob() {
    if !tool("dvdauthor") {
        return;
    }
    let d = dir("dvdauthor");
    let Some(vob) = ffmpeg_vob(&d) else { return };
    let disc = d.join("disc");
    write_disc(&disc, &vob, &spec(vec![2 * 90_000, 5 * 90_000])).unwrap();
    let ok = Command::new("dvdauthor").env("VIDEO_FORMAT", "PAL").args(["-t", "-c", "0,0:02,0:05", "-o"]).arg(d.join("ref")).arg(&vob).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let ours = nav_fields(&std::fs::read(disc.join("VIDEO_TS/VTS_01_1.VOB")).unwrap());
    let theirs = nav_fields(&std::fs::read(d.join("ref/VIDEO_TS/VTS_01_1.VOB")).unwrap());
    assert_eq!(ours.len(), theirs.len());
    for (k, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        assert_eq!(a, b, "navigation pack {k}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_vob_from_our_muxer_authors_like_one_from_ffmpeg() {
    use cfa4_mpegts::{Codec, Kind, MuxCodec, Muxer, open};
    let d = dir("remux");
    let Some(src) = ffmpeg_vob(&d) else { return };
    let data = std::fs::read(&src).unwrap();
    let f = open(data.as_slice()).unwrap();
    let v = f.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
    let a = f.find(Kind::Audio, |c| *c == Codec::Ac3).unwrap();
    let mut m = Muxer::new_dvd(Vec::new(), &[MuxCodec::Mpeg2Video, MuxCodec::Ac3]).unwrap();
    let (mut ai, mut vi) = (0usize, 0usize);
    let base = f.streams[v].units[0].pts.unwrap_or(0);
    let (vu, au) = (&f.streams[v].units, &f.streams[a].units);
    // AC-3 frames are 1536 samples (2880 ticks) apart; the demuxer gives them no time stamps, so
    // they start with the video
    let apts: Vec<i64> = (0..au.len() as i64).map(|k| k * 2880).collect();
    // Picture times from the temporal references: a GOP starts at its key picture, display time
    // T + 3600 * temporal reference, decode time T + 3600 * position in decode order.
    let mut times: Vec<(i64, i64)> = Vec::new();
    let (mut t0, mut start) = (0i64, 0usize);
    for (i, u) in vu.iter().enumerate() {
        let tr = i64::from(u.picture.unwrap().temporal_reference);
        if u.key {
            t0 = u.pts.unwrap() - base - tr * 3600;
            start = i;
        }
        times.push((t0 + tr * 3600, t0 + (i - start) as i64 * 3600));
    }
    // pictures in decode order, each audio frame before the first picture it follows
    while vi < vu.len() {
        let (pts, dts) = times[vi];
        while ai < au.len() && apts[ai] <= dts {
            let t = apts[ai];
            m.write(1, &f.read_unit(data.as_slice(), a, ai).unwrap(), t, None, true).unwrap_or_else(|e| panic!("audio {ai} at {t}: {e}"));
            ai += 1;
        }
        let dts = (dts != pts).then_some(dts);
        m.write(0, &f.read_unit(data.as_slice(), v, vi).unwrap(), pts, dts, vu[vi].key).unwrap_or_else(|e| panic!("video {vi}: {e}"));
        vi += 1;
    }
    let vob = d.join("mine.vob");
    std::fs::write(&vob, m.finish().unwrap()).unwrap();
    let disc = d.join("disc");
    let a = write_disc(&disc, &vob, &spec(vec![2 * 90_000, 5 * 90_000])).unwrap();
    assert_eq!(a.chapters, 3);
    assert!(a.vobus >= 12);
    if tool("lsdvd") {
        let o = Command::new("lsdvd").arg("-x").arg(&disc).output().unwrap();
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(text.contains("Chapters: 03") && text.contains("Format: ac3"), "{text}");
        assert!(text.contains("Length: 00:00:08"), "{text}");
    }
    // the authored VOB still demuxes to the same pictures
    let out = std::fs::read(disc.join("VIDEO_TS/VTS_01_1.VOB")).unwrap();
    let g = open(out.as_slice()).unwrap();
    let w = g.find(Kind::Video, |c| *c == Codec::Mpeg2Video).unwrap();
    assert_eq!(g.streams[w].units.len(), vu.len());
    for k in [0, 7, vu.len() - 1] {
        assert_eq!(g.read_unit(out.as_slice(), w, k).unwrap(), f.read_unit(data.as_slice(), v, k).unwrap(), "picture {k}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// One MPEG-2 I picture (sequence header, GOP, picture) of a flat-colour PAL frame.
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

fn menu_spec(picture: Vec<u8>, buttons: Vec<MenuButton>) -> MenuSpec {
    let mut palette = [[0u8; 3]; 16];
    for (i, p) in palette.iter_mut().enumerate() {
        *p = [(16 + i * 14) as u8, 0x80, 0x80];
    }
    let (w, h) = (400usize, 40usize);
    MenuSpec {
        picture,
        overlay: Overlay { x: 100, y: 100, w, h, pixels: vec![1u8; w * h] },
        palette,
        normal: ColorSet { index: [0, 8, 4, 2], alpha: [0, 15, 15, 15] },
        select: ColorSet { index: [0, 12, 4, 2], alpha: [0, 15, 15, 15] },
        activate: ColorSet { index: [0, 15, 4, 2], alpha: [0, 15, 15, 15] },
        buttons,
        language: *b"en",
    }
}

#[test]
fn a_disc_with_a_title_menu_is_read_by_lsdvd() {
    let d = dir("menu");
    let (Some(vob), Some(pic)) = (ffmpeg_vob(&d), still_picture()) else { return };
    let disc = d.join("disc");
    let menu = menu_spec(pic, vec![MenuButton { rect: (100, 100, 499, 139), action: MenuAction::PlayTitle(1) }]);
    let a = write_disc_with_menu(&disc, &vob, &spec(vec![2 * 90_000]), &menu).unwrap();
    let v = disc.join("VIDEO_TS");
    for name in ["VIDEO_TS.IFO", "VIDEO_TS.BUP", "VIDEO_TS.VOB", "VTS_01_0.IFO", "VTS_01_0.BUP", "VTS_01_1.VOB"] {
        assert!(v.join(name).exists(), "{name}");
    }
    assert_eq!(std::fs::metadata(v.join("VIDEO_TS.VOB")).unwrap().len() as usize, a.menu_vob.len());
    // the title set starts after IFO, VOB and BUP of the video manager
    let ifo = &a.disc.vmg_ifo;
    let n = ifo.len() / SECTOR;
    let vob_sectors = a.menu_vob.len() / SECTOR;
    assert_eq!(be32(ifo, 0x0C) as usize, 2 * n + vob_sectors - 1, "vmg_last_sector");
    assert_eq!(be32(ifo, 0xC0) as usize, n, "VIDEO_TS.VOB starts behind the IFO");
    let tt = be32(ifo, 0xC4) as usize * SECTOR;
    assert_eq!(be32(&ifo[tt..], 8 + 8) as usize, 2 * n + vob_sectors, "title set sector");
    if tool("lsdvd") {
        let o = Command::new("lsdvd").arg("-x").arg(&disc).output().unwrap();
        let text = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "{text}");
        assert!(text.contains("Title: 01, Length: 00:00:08") && text.contains("Chapters: 02"), "{text}");
        assert!(text.contains("Aspect ratio: 4/3"), "{text}");
    }
    if std::env::var_os("CFA4_DVD_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&d);
    }
}
