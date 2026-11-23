//! Interplay MVE against ffmpeg's `ipmovie` demuxer and `interplayvideo` / `interplay_dpcm`
//! decoders (ffmpeg has no MVE writer, so the files come from this test's own).
//!
//! The writer draws every syntax element at random within the bitstream's rules: every block
//! opcode of the 8-bit and 16-bit formats 0x11, the two-pass formats 0x10 and 0x06, with motion
//! vectors that stay inside the picture and read only pixels already defined (what ffmpeg leaves
//! in a picture it does not write is undefined), palettes, hidden pictures, and sound in all four
//! codings. Pictures and samples must equal ffmpeg's. Damaged files must not panic.

use cfa4_mve::{Decoder, Input, decode_audio, parse};
use std::path::Path;
use std::process::Command;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn byte(&mut self) -> u8 {
        self.below(256) as u8
    }
}

fn opcode(kind: u8, version: u8, body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u16).to_le_bytes().to_vec();
    v.push(kind);
    v.push(version);
    v.extend_from_slice(body);
    v
}

fn chunk(kind: u16, ops: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = ops.concat();
    let mut v = (body.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&kind.to_le_bytes());
    v.extend(body);
    v
}

fn header() -> Vec<u8> {
    let mut v = b"Interplay MVE File\x1A\0".to_vec();
    v.extend_from_slice(&[0x1A, 0, 0, 1, 0x33, 0x11]);
    v
}

/// Whether ffmpeg accepts a motion from `pos` by `(dx, dy)`, and the block it reads: the same
/// rule as ffmpeg's `copy_from` (a vector off a side wraps to the next or previous row).
fn source(w: i32, h: i32, pos: usize, dx: i32, dy: i32) -> Option<(i32, i32)> {
    let (x, y) = (pos as i32 % w, pos as i32 / w);
    let tx = dx + x;
    let wrap = i32::from(tx >= w) - i32::from(tx < 0);
    let (sx, sy) = (tx - wrap * w, dy + y + wrap);
    let off = i64::from(sy) * i64::from(w) + i64::from(sx);
    (off >= 0 && off <= i64::from(h - 8) * i64::from(w) + i64::from(w - 8) && sx >= 0 && sx < w - 7).then_some((sx, sy))
}

/// Whether the 8×8 block at `(sx, sy)` is all pictures-before-this-block (in raster order of
/// blocks) and does not overlap the block at `pos`.
fn decoded_before(w: i32, pos: usize, sx: i32, sy: i32) -> bool {
    let (bx, by) = (pos as i32 % w, pos as i32 / w);
    let overlap = (sx - bx).abs() < 8 && (sy - by).abs() < 8;
    // every row of the source is above this block's band, or in its band and left of it
    !overlap && (0..8).all(|r| sy + r < by || (sy + r < by + 8 && sx + 8 <= bx))
}

/// Writes the opcode data for the blocks of one format-0x11 picture.
struct Pic<'a> {
    w: usize,
    h: usize,
    bpp16: bool,
    f: usize,
    rng: &'a mut Rng,
    stream: Vec<u8>,
    mv: Vec<u8>,
}

impl Pic<'_> {
    fn px(&mut self, v: u16) {
        if self.bpp16 {
            self.stream.extend_from_slice(&v.to_le_bytes());
        } else {
            self.stream.push(v as u8);
        }
    }
    fn colour(&mut self) -> u16 {
        if self.bpp16 { self.rng.below(0x8000) as u16 } else { u16::from(self.rng.byte()) }
    }
    /// Two colours of the first coding (`first`) or the second.
    fn pair(&mut self, first: bool) -> [u16; 2] {
        let (a, b) = (self.colour(), self.colour());
        if self.bpp16 {
            [if first { a & 0x7FFF } else { a | 0x8000 }, b]
        } else if first {
            [a.min(b), a.max(b)]
        } else {
            [a.max(b).max(a.min(b) + 1).min(255), a.min(b).min(254)]
        }
    }
    fn motion_byte(&mut self, b: u8) {
        if self.bpp16 { self.mv.push(b) } else { self.stream.push(b) }
    }

    /// The block at `pos` with opcode `op`: its data, or `false` when no valid data exists.
    fn block(&mut self, op: u8, pos: usize) -> bool {
        let (w, h) = (self.w as i32, self.h as i32);
        let f = self.f;
        let pairs = |pic: &mut Self, n: usize, first: bool| -> Vec<[u16; 2]> { (0..n).map(|_| pic.pair(first)).collect() };
        let _ = pairs;
        match op {
            // copies from the last picture, the one before it, each by a vector
            0x0 | 0x4 | 0x5 if f < 1 => return false,
            0x1 | 0x2 if f < 2 => return false,
            0xF if self.bpp16 && f < 2 => return false,
            0x6 if self.bpp16 && f < 2 => return false,
            0x6 if !self.bpp16 => return false,
            0x0 | 0x1 => {}
            0xF if self.bpp16 => {}
            0x2..=0x6 if op != 0x6 || self.bpp16 => {
                for _ in 0..200 {
                    let (b, dx, dy, ok): (u8, i32, i32, bool) = match op {
                        0x2 => {
                            let b = self.rng.byte();
                            let b32 = i32::from(b);
                            let (x, y) = if b32 < 56 { (8 + b32 % 7, b32 / 7) } else { (-14 + (b32 - 56) % 29, 8 + (b32 - 56) / 29) };
                            (b, x, y, true)
                        }
                        0x3 => {
                            let b = self.rng.byte();
                            let b32 = i32::from(b);
                            let (x, y) = if b32 < 56 { (-(8 + b32 % 7), -(b32 / 7)) } else { (-(-14 + (b32 - 56) % 29), -(8 + (b32 - 56) / 29)) };
                            (b, x, y, true)
                        }
                        0x4 => {
                            let b = self.rng.byte();
                            (b, -8 + i32::from(b & 15), -8 + i32::from(b >> 4), true)
                        }
                        _ => {
                            let (x, y) = (self.rng.below(41) as i32 - 20, self.rng.below(41) as i32 - 20);
                            (0, x, y, true)
                        }
                    };
                    let _ = ok;
                    let Some((sx, sy)) = source(w, h, pos, dx, dy) else { continue };
                    if op == 0x3 && !decoded_before(w, pos, sx, sy) {
                        continue;
                    }
                    match op {
                        0x2..=0x4 => self.motion_byte(b),
                        _ => {
                            self.stream.push(dx as i8 as u8);
                            self.stream.push(dy as i8 as u8);
                        }
                    }
                    return true;
                }
                return false;
            }
            0x7 => {
                let first = self.rng.below(2) == 0;
                let p = self.pair(first);
                self.px(p[0]);
                self.px(p[1]);
                let n = if first { 8 } else { 2 };
                for _ in 0..n {
                    let b = self.rng.byte();
                    self.stream.push(b);
                }
            }
            0x8 => {
                let first = self.rng.below(2) == 0;
                if first {
                    for q in 0..4 {
                        let p = self.pair(true);
                        self.px(p[0]);
                        self.px(p[1]);
                        let _ = q;
                        for _ in 0..2 {
                            let b = self.rng.byte();
                            self.stream.push(b);
                        }
                    }
                } else {
                    let p = self.pair(false);
                    self.px(p[0]);
                    self.px(p[1]);
                    for _ in 0..4 {
                        let b = self.rng.byte();
                        self.stream.push(b);
                    }
                    let vertical = self.rng.below(2) == 0;
                    let p = self.pair(vertical);
                    self.px(p[0]);
                    self.px(p[1]);
                    for _ in 0..4 {
                        let b = self.rng.byte();
                        self.stream.push(b);
                    }
                }
            }
            0x9 => {
                let first = self.rng.below(2) == 0;
                let second = self.rng.below(2) == 0;
                let a = self.pair(first);
                let b = self.pair(second);
                for v in [a[0], a[1], b[0], b[1]] {
                    self.px(v);
                }
                let n = if first { if second { 16 } else { 4 } } else { 8 };
                for _ in 0..n {
                    let b = self.rng.byte();
                    self.stream.push(b);
                }
            }
            0xA => {
                if self.rng.below(2) == 0 {
                    for _ in 0..4 {
                        let (a, b) = (self.pair(true), self.pair(true));
                        // the first pair decides, the second is any
                        for v in [a[0], a[1], b[0], b[1]] {
                            self.px(v);
                        }
                        for _ in 0..4 {
                            let x = self.rng.byte();
                            self.stream.push(x);
                        }
                    }
                } else {
                    let (a, b) = (self.pair(false), self.pair(false));
                    for v in [a[0], a[1], b[0], b[1]] {
                        self.px(v);
                    }
                    for _ in 0..8 {
                        let x = self.rng.byte();
                        self.stream.push(x);
                    }
                    let vert = self.rng.below(2) == 0;
                    let (c, d) = (self.pair(vert), self.pair(false));
                    for v in [c[0], c[1], d[0], d[1]] {
                        self.px(v);
                    }
                    for _ in 0..8 {
                        let x = self.rng.byte();
                        self.stream.push(x);
                    }
                }
            }
            0xB => {
                for _ in 0..64 {
                    let c = self.colour();
                    self.px(c);
                }
            }
            0xC => {
                for _ in 0..16 {
                    let c = self.colour();
                    self.px(c);
                }
            }
            0xD => {
                for _ in 0..4 {
                    let c = self.colour();
                    self.px(c);
                }
            }
            0xE => {
                let c = self.colour();
                self.px(c);
            }
            _ => {
                // 0xF in 8 bits: two dither colours
                let (a, b) = (self.rng.byte(), self.rng.byte());
                self.stream.push(a);
                self.stream.push(b);
            }
        }
        true
    }
}

/// One format-0x11 packet's video data (14 header bytes, [the motion offset], the stream, the
/// motion stream) and its decoding map.
fn format11(w: usize, h: usize, bpp16: bool, f: usize, rng: &mut Rng) -> (Vec<u8>, Vec<u8>) {
    let mut pic = Pic { w, h, bpp16, f, rng, stream: Vec::new(), mv: Vec::new() };
    let mut nibbles = Vec::new();
    for y in (0..h).step_by(8) {
        for x in (0..w).step_by(8) {
            loop {
                let op = pic.rng.below(16) as u8;
                let (s, m) = (pic.stream.len(), pic.mv.len());
                if pic.block(op, x + y * w) {
                    nibbles.push(op);
                    break;
                }
                pic.stream.truncate(s);
                pic.mv.truncate(m);
            }
        }
    }
    let mut dmap = Vec::new();
    for pair in nibbles.chunks(2) {
        dmap.push(pair[0] | pair.get(1).copied().unwrap_or(0) << 4);
    }
    let mut data: Vec<u8> = (0..14).map(|_| pic.rng.byte()).collect();
    if bpp16 {
        data.extend_from_slice(&(pic.stream.len() as u16 + 2).to_le_bytes());
    }
    data.extend_from_slice(&pic.stream);
    data.extend_from_slice(&pic.mv);
    (data, dmap)
}

/// A format-0x06 or 0x10 packet. Frames `0` and `1` (and `2` for 0x06) are raw, the rest mix.
fn two_pass(w: usize, h: usize, f: usize, format: u8, rng: &mut Rng) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let blocks = (w / 8) * (h / 8);
    let all_raw = if format == 0x06 { f < 3 } else { f == 0 };
    let coded_all = format == 0x10 && f < 2;
    let mut ops = Vec::new();
    let mut coded = Vec::new();
    for b in 0..blocks {
        let pos = (b % (w / 8)) * 8 + (b / (w / 8)) * 8 * w;
        let is_coded = format == 0x06 || coded_all || rng.below(3) != 0;
        coded.push(is_coded);
        if !is_coded {
            continue;
        }
        let mut op = 0u16;
        if !all_raw && rng.below(4) != 0 {
            for _ in 0..100 {
                let negative = rng.below(2) == 0 || (format == 0x10 && f < 2);
                let off = rng.below(w as u64 * h as u64 / 2) as u32;
                let (dx, dy) = ((off % w as u32) as i32, (off / w as u32) as i32);
                let Some((sx, sy)) = source(w as i32, h as i32, pos, dx, dy) else { continue };
                if !negative && (sx - (pos % w) as i32).abs() < 8 && (sy - (pos / w) as i32).abs() < 8 {
                    continue;
                }
                op = if negative { 0xC000 + off as u16 } else { 0x4000 + off as u16 };
                break;
            }
        }
        ops.push(op);
    }
    let mut stream: Vec<u8> = (0..14).map(|_| rng.byte()).collect();
    let dmap: Vec<u8> = ops.iter().flat_map(|o| o.to_le_bytes()).collect();
    let raw = |stream: &mut Vec<u8>, rng: &mut Rng| (0..64).for_each(|_| stream.push(rng.byte()));
    if format == 0x06 {
        // the map is at the top of the data; raw blocks follow in raster order
        stream.extend_from_slice(&dmap);
        for &o in &ops {
            if o == 0 {
                raw(&mut stream, rng);
            }
        }
        return (stream, Vec::new(), Vec::new());
    }
    for &o in &ops {
        if o == 0 {
            raw(&mut stream, rng);
        }
    }
    // the skip map: 15 flags a word, most significant first, then a stop bit
    let mut smap = Vec::new();
    for bits in coded.chunks(15) {
        let mut word = 0u16;
        for (i, &c) in bits.iter().enumerate() {
            word |= u16::from(c) << (15 - i);
        }
        word |= 1 << (15 - bits.len());
        smap.extend_from_slice(&word.to_le_bytes());
    }
    smap.extend_from_slice(&0x8000u16.to_le_bytes());
    (stream, dmap, smap)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sound {
    None,
    Pcm16(u32),
    Pcm8(u32),
    Dpcm(u32),
}

struct Spec {
    w: usize,
    h: usize,
    bpp16: bool,
    format: u8,
    frames: usize,
    sound: Sound,
    seed: u64,
}

fn audio_body(sound: Sound, rng: &mut Rng) -> Vec<u8> {
    let mut v = vec![0, 0, 0xFF, 0xFF, 0, 0];
    match sound {
        Sound::None => {}
        Sound::Pcm16(c) => (0..c * 2 * (200 + rng.below(300) as u32)).for_each(|_| v.push(rng.byte())),
        Sound::Pcm8(c) => (0..c * (200 + rng.below(300) as u32)).for_each(|_| v.push(rng.byte())),
        Sound::Dpcm(c) => {
            for _ in 0..c {
                v.extend_from_slice(&((rng.below(65536)) as u16).to_le_bytes());
            }
            // an even length keeps stereo whole
            (0..2 * (100 + rng.below(200))).for_each(|_| v.push(rng.byte()));
        }
    }
    v
}

fn write_movie(s: &Spec) -> Vec<u8> {
    let mut rng = Rng(s.seed);
    let mut out = header();
    let mut init = vec![opcode(0x05, 2, &[(s.w / 8) as u8, 0, (s.h / 8) as u8, 0, 0, 0, u8::from(s.bpp16), 0])];
    init.push(opcode(0x02, 0, &[0x40, 0x41, 0, 0, 8, 0])); // 16.7 ms × 8
    out.extend(chunk(2, &init));
    match s.sound {
        Sound::None => {}
        Sound::Pcm16(c) | Sound::Pcm8(c) | Sound::Dpcm(c) => {
            let (flags, version) = match s.sound {
                Sound::Pcm16(_) => (2, 0),
                Sound::Pcm8(_) => (0, 0),
                _ => (4 | 2, 1),
            };
            let flags: u16 = flags | u16::from(c == 2);
            let mut body = vec![0, 0];
            body.extend_from_slice(&flags.to_le_bytes());
            body.extend_from_slice(&22050u16.to_le_bytes());
            out.extend(chunk(0, &[opcode(0x03, version, &body)]));
        }
    }
    for f in 0..s.frames {
        let mut ops = Vec::new();
        if s.sound != Sound::None {
            ops.push(opcode(0x08, 0, &audio_body(s.sound, &mut rng)));
        }
        if !s.bpp16 && (f == 0 || rng.below(3) == 0) {
            // a palette of some colours (6 bits each)
            let (first, count) = if f == 0 { (0, 256) } else { (rng.below(200) as u16, 1 + rng.below(56) as u16) };
            let mut body = first.to_le_bytes().to_vec();
            body.extend_from_slice(&count.to_le_bytes());
            (0..usize::from(count) * 3).for_each(|_| body.push(rng.below(64) as u8));
            ops.push(opcode(0x0C, 0, &body));
        }
        match s.format {
            0x11 => {
                let (data, dmap) = format11(s.w, s.h, s.bpp16, f, &mut rng);
                ops.push(opcode(0x0F, 0, &dmap));
                ops.push(opcode(0x11, 0, &data));
            }
            fmt => {
                let (data, dmap, smap) = two_pass(s.w, s.h, f, fmt, &mut rng);
                if fmt == 0x10 {
                    ops.push(opcode(0x0F, 0, &dmap));
                    ops.push(opcode(0x0E, 0, &smap));
                }
                ops.push(opcode(fmt, 0, &data));
            }
        }
        // one picture in five stays hidden
        if f % 5 != 3 {
            ops.push(opcode(0x07, 0, &[]));
        }
        out.extend(chunk(3, &ops));
    }
    out.extend(chunk(5, &[]));
    out
}

fn run(ff: &Path, data: &[u8], tag: &str, args: &[&str]) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("cfa4-mve-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.mve");
    std::fs::write(&input, data).ok()?;
    let out = Command::new(ff).args(["-v", "error", "-f", "ipmovie", "-i"]).arg(&input).args(args).arg("-").output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    out.status.success().then_some(out.stdout)
}

/// ffmpeg's pictures as rgba (palette video) or raw rgb555le.
fn ffmpeg_video(ff: &Path, data: &[u8], tag: &str, bpp16: bool) -> Option<Vec<u8>> {
    let fmt = if bpp16 { "rgb555le" } else { "rgba" };
    run(ff, data, tag, &["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", fmt, "-fps_mode", "passthrough"])
}

fn ours_video(data: &[u8]) -> Vec<u8> {
    let m = parse(data).expect("parse");
    let mut d = Decoder::new(m.width, m.height, m.bpp16).expect("decoder");
    let mut out = Vec::new();
    for v in &m.video {
        let input = Input {
            format: v.format,
            send_buffer: v.send_buffer,
            video: v.video.of(data),
            decode_map: v.decode_map.of(data),
            skip_map: v.skip_map.of(data),
            palette: v.palette.as_deref(),
        };
        if let Some(f) = d.decode(&input).expect("decode") {
            if m.bpp16 {
                out.extend(f.pixels.iter().flat_map(|p| p.to_le_bytes()));
            } else {
                out.extend(f.to_rgba());
            }
        }
    }
    out
}

fn compare(tag: &str, ours: &[u8], theirs: &[u8], unit: usize) {
    assert_eq!(ours.len(), theirs.len(), "{tag}: length");
    if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
        let n = ours.iter().zip(theirs).filter(|(a, b)| a != b).count();
        panic!("{tag}: first difference at byte {i} (picture {}, offset {}), {n} differ", i / unit, i % unit);
    }
}

fn audio_bytes(data: &[u8]) -> Vec<u8> {
    let m = parse(data).expect("parse");
    let p = m.audio.expect("audio");
    let mut out = Vec::new();
    for a in &m.audio_packets {
        for v in decode_audio(&p, a.of(data)).expect("audio") {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

#[test]
fn pictures_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("mve oracle") else { return };
    let cases = [
        (64, 32, false, 0x11, 12),
        (128, 64, false, 0x11, 12),
        (192, 48, false, 0x11, 10),
        (64, 32, true, 0x11, 12),
        (128, 64, true, 0x11, 12),
        (128, 64, false, 0x10, 14),
        (192, 48, false, 0x10, 14),
        (64, 64, false, 0x06, 12),
        (128, 64, false, 0x06, 12),
    ];
    for (i, &(w, h, bpp16, format, frames)) in cases.iter().enumerate() {
        for round in 0..5u64 {
            let tag = format!("v{i}-{round}");
            let data = write_movie(&Spec { w, h, bpp16, format, frames, sound: Sound::None, seed: 0xA11CE + i as u64 * 100 + round });
            let theirs = ffmpeg_video(&ff, &data, &tag, bpp16).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
            let unit = w * h * if bpp16 { 2 } else { 4 };
            compare(&tag, &ours_video(&data), &theirs, unit);
        }
    }
}

#[test]
fn sound_matches_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("mve oracle") else { return };
    for (i, sound) in [Sound::Pcm16(1), Sound::Pcm16(2), Sound::Pcm8(1), Sound::Pcm8(2), Sound::Dpcm(1), Sound::Dpcm(2)].into_iter().enumerate() {
        let tag = format!("s{i}");
        let data = write_movie(&Spec { w: 64, h: 32, bpp16: false, format: 0x11, frames: 8, sound, seed: 0x50_0D + i as u64 });
        let theirs = run(&ff, &data, &tag, &["-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le"]).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
        compare(&format!("{tag} {sound:?}"), &audio_bytes(&data), &theirs, 2);
        // the picture is unaffected by the sound
        let video = ffmpeg_video(&ff, &data, &tag, false).unwrap();
        compare(&tag, &ours_video(&data), &video, 64 * 32 * 4);
    }
}

#[test]
fn damaged_files_do_not_panic() {
    let mut rng = Rng(0xBAD_0001);
    let cases = [(0x11, false), (0x11, true), (0x10, false), (0x06, false)];
    for round in 0..400u64 {
        let (format, bpp16) = cases[(round % 4) as usize];
        let sound = [Sound::None, Sound::Dpcm(2), Sound::Pcm16(1)][(round % 3) as usize];
        let mut data = write_movie(&Spec { w: 64, h: 32, bpp16, format, frames: 6, sound, seed: 0x99_0000 + round });
        for _ in 0..1 + rng.below(12) {
            let at = rng.below(data.len() as u64) as usize;
            data[at] = rng.byte();
        }
        if round % 5 == 0 {
            data.truncate(rng.below(data.len() as u64) as usize);
        }
        let Ok(m) = parse(&data) else { continue };
        if let Ok(mut d) = Decoder::new(m.width, m.height, m.bpp16) {
            for v in &m.video {
                let input = Input {
                    format: v.format,
                    send_buffer: v.send_buffer,
                    video: v.video.of(&data),
                    decode_map: v.decode_map.of(&data),
                    skip_map: v.skip_map.of(&data),
                    palette: v.palette.as_deref(),
                };
                let _ = d.decode(&input);
            }
        }
        if let Some(p) = m.audio {
            for a in &m.audio_packets {
                let _ = decode_audio(&p, a.of(&data));
            }
        }
    }
}

#[test]
fn probe_and_bad_headers() {
    assert!(!cfa4_mve::probe(b"RIFF"));
    assert!(cfa4_mve::probe(&header()));
    assert!(parse(b"RIFF....").is_err());
    // the first chunk must start the video
    let mut f = header();
    f.extend(chunk(3, &[]));
    assert!(parse(&f).is_err());
}
