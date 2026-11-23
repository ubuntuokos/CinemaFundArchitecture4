//! The Microsoft Video 1 and Microsoft RLE decoders against ffmpeg's `msvideo1` and `msrle` (the
//! external oracle). ffmpeg writes neither codec's packets in a form we can lift out of a file, so
//! the packets are made here: a first picture that covers every block / pixel (ffmpeg keeps
//! whatever its frame buffer held in the parts a picture doesn't reach), then delta pictures with
//! skip codes. Every decoded picture must be ffmpeg's, byte for byte, as RGB24.

use std::path::Path;
use std::process::Command;

use cfa4_msvideo::{rle, video1};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    chunk(b"LIST", &[kind.as_slice(), body].concat())
}

/// An AVI with one video stream of `frames`; `compression` is `biCompression`, `extra` the bytes
/// after the 40-byte header (the palette).
fn avi(w: u32, h: u32, bits: u16, compression: [u8; 4], extra: &[u8], frames: &[Vec<u8>]) -> Vec<u8> {
    let words = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let n = frames.len() as u32;
    let avih = words(&[40_000, 0, 0, 0x10, n, 0, 1, 0, w, h, 0, 0, 0, 0]);
    let strh = [b"vids".as_slice(), &compression, &words(&[0, 0, 0, 1, 25, 0, n, 0, u32::MAX, 0]), &[0u8; 8]].concat();
    let strf = [words(&[40, w, h]), [1u16.to_le_bytes(), bits.to_le_bytes()].concat(), compression.to_vec(), words(&[0; 5]), extra.to_vec()].concat();
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat())].concat());
    let (mut movi, mut idx) = (Vec::new(), Vec::new());
    for (i, f) in frames.iter().enumerate() {
        // every picture is declared a key frame: ffmpeg decodes them all in order regardless
        let flags = if i == 0 { 0x10 } else { 0 };
        idx.extend_from_slice(&[b"00dc".as_slice(), &words(&[flags, 4 + movi.len() as u32, f.len() as u32])].concat());
        movi.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi), chunk(b"idx1", &idx)].concat())
}

fn ffmpeg_rgb24(ff: &Path, file: &[u8], name: &str, w: u32, h: u32) -> Vec<Vec<u8>> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("msvideo/oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}-{}.avi", std::process::id()));
    std::fs::write(&path, file).unwrap();
    let out = Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(&path)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-c:v", "rawvideo", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .expect("run ffmpeg");
    let _ = std::fs::remove_file(&path);
    out.stdout.chunks((w * h * 3) as usize).map(<[u8]>::to_vec).collect()
}

fn rgb(p: &cfa4_msvideo::Picture) -> Vec<u8> {
    p.rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect()
}

fn palette(rng: &mut Rng, entries: usize) -> Vec<u8> {
    (0..entries).flat_map(|_| [rng.byte(), rng.byte(), rng.byte(), 0]).collect()
}

/// A Video 1 picture of 4x4 blocks: with `cover` every block is coded, otherwise skip codes
/// appear. `bits16` selects RGB555 (colours are 16-bit words) over palette indices.
fn video1_packet(rng: &mut Rng, w: u32, h: u32, bits16: bool, cover: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut left = ((w / 4) * (h / 4)) as i64;
    let word = |rng: &mut Rng, top: bool| -> u16 {
        let v = (rng.next() & 0x7FFF) as u16;
        if top { v | 0x8000 } else { v }
    };
    while left > 0 {
        match rng.below(if cover { 4 } else { 5 }) {
            0 => {
                // 1-colour
                if bits16 {
                    // a 1-colour word has its top bit set and must not look like a skip code
                    let v = loop {
                        let v = word(rng, true);
                        if (v >> 8) & 0xFC != 0x84 {
                            break v;
                        }
                    };
                    out.extend_from_slice(&v.to_le_bytes());
                } else {
                    let b = [0x80, 0x81, 0x82, 0x83, 0x88, 0x89, 0x8A, 0x8F][rng.below(8) as usize];
                    out.extend_from_slice(&[rng.byte(), b]);
                }
                left -= 1;
            }
            1 | 2 => {
                // 2-colour
                out.extend_from_slice(&[rng.byte(), (rng.byte() & 0x7F)]);
                if bits16 {
                    out.extend_from_slice(&word(rng, false).to_le_bytes());
                    out.extend_from_slice(&word(rng, false).to_le_bytes());
                } else {
                    out.extend_from_slice(&[rng.byte(), rng.byte()]);
                }
                left -= 1;
            }
            3 => {
                // 8-colour
                if bits16 {
                    out.extend_from_slice(&[rng.byte(), (rng.byte() & 0x7F)]);
                    out.extend_from_slice(&word(rng, true).to_le_bytes());
                    for _ in 0..7 {
                        out.extend_from_slice(&word(rng, false).to_le_bytes());
                    }
                } else {
                    out.extend_from_slice(&[rng.byte(), 0x90 | (rng.byte() & 0x6F)]);
                    out.extend((0..8).map(|_| rng.byte()));
                }
                left -= 1;
            }
            _ => {
                // skip n blocks (the code stands for the current block too)
                let n = 1 + rng.below(left as u32 + 2);
                out.extend_from_slice(&[(n & 0xFF) as u8, 0x84 + (n >> 8) as u8]);
                left -= i64::from(n);
            }
        }
    }
    out
}

#[test]
fn video1_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for (w, h, bits16) in [(16u32, 16u32, true), (64, 48, true), (36, 20, true), (16, 16, false), (64, 48, false), (40, 24, false)] {
        for seed in 0..4u64 {
            let mut rng = Rng(seed * 77 + u64::from(w));
            let pal = if bits16 { Vec::new() } else { palette(&mut rng, 256) };
            let n = 8;
            let packets: Vec<Vec<u8>> = (0..n).map(|i| video1_packet(&mut rng, w, h, bits16, i == 0)).collect();
            let bits = if bits16 { 16 } else { 8 };
            let file = avi(w, h, bits, *b"MSVC", &pal, &packets);
            let want = ffmpeg_rgb24(&ff, &file, "v1", w, h);
            assert_eq!(want.len(), n, "{w}x{h} {bits}-bit seed {seed}: ffmpeg decoded {} pictures", want.len());
            let mut d = video1::Decoder::new(w, h, bits, &pal).unwrap();
            for (i, p) in packets.iter().enumerate() {
                let got = rgb(&d.decode(p).unwrap());
                if got != want[i] {
                    let k = got.iter().zip(&want[i]).position(|(a, b)| a != b).unwrap();
                    eprintln!("first diff at byte {k}: ours {:?} ffmpeg {:?}", &got[k.saturating_sub(k % 3)..][..3], &want[i][k.saturating_sub(k % 3)..][..3]);
                }
                assert!(got == want[i], "{w}x{h} {bits}-bit seed {seed}: picture {i} differs from ffmpeg's");
            }
        }
    }
}

/// One RLE line covering `w` pixels, bottom row first in the stream.
fn rle_line(rng: &mut Rng, w: usize, depth: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut x = 0;
    while x < w {
        let n = 1 + rng.below((w - x) as u32) as usize;
        let n = n.min(w - x);
        let literal = n >= 3 && rng.below(2) == 0;
        match depth {
            8 => {
                if literal {
                    out.extend_from_slice(&[0, n as u8]);
                    out.extend((0..n).map(|_| rng.byte()));
                    if n % 2 == 1 {
                        out.push(0);
                    }
                } else {
                    out.extend_from_slice(&[n as u8, rng.byte()]);
                }
            }
            24 => {
                if literal {
                    out.extend_from_slice(&[0, n as u8]);
                    out.extend((0..n * 3).map(|_| rng.byte()));
                } else {
                    out.extend_from_slice(&[n as u8, rng.byte(), rng.byte(), rng.byte()]);
                }
            }
            _ => {
                if literal {
                    out.extend_from_slice(&[0, n as u8]);
                    let bytes = n.div_ceil(2);
                    out.extend((0..bytes).map(|_| rng.byte()));
                    if bytes % 2 == 1 {
                        out.push(0);
                    }
                } else {
                    out.extend_from_slice(&[n as u8, rng.byte()]);
                }
            }
        }
        x += n;
    }
    out
}

fn rle_packet(rng: &mut Rng, w: usize, h: usize, depth: u32, cover: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut y = 0;
    while y < h {
        let mut x0 = 0;
        // a delta picture may leave part of a line to the previous picture with a skip right and up
        if !cover && rng.below(3) == 0 && w > 4 {
            let dx = 1 + rng.below(3) as usize;
            let dy = rng.below(2) as usize;
            if y + dy < h {
                out.extend_from_slice(&[0, 2, dx as u8, dy as u8]);
                x0 = dx;
                y += dy;
            }
        }
        out.extend(rle_line(rng, w - x0, depth));
        y += 1;
        out.extend_from_slice(&[0, u8::from(y == h)]);
    }
    out
}

#[test]
fn rle_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for (w, h, depth) in [(20usize, 12usize, 8u32), (33, 9, 8), (20, 12, 24), (31, 7, 24), (20, 12, 4), (33, 9, 4)] {
        for seed in 0..4u64 {
            let mut rng = Rng(seed * 101 + w as u64 + u64::from(depth));
            let pal = if depth == 24 { Vec::new() } else { palette(&mut rng, if depth == 4 { 16 } else { 256 }) };
            let n = 6;
            // delta pictures use only whole-line runs here: a picture ffmpeg decodes may not end
            // in the picture's last line, and that is checked by the random-packet test below
            let packets: Vec<Vec<u8>> = (0..n).map(|_| rle_packet(&mut rng, w, h, depth, true)).collect();
            let compression = if depth == 4 { [2, 0, 0, 0] } else { [1, 0, 0, 0] };
            let file = avi(w as u32, h as u32, depth as u16, if depth == 24 { *b"mrle" } else { compression }, &pal, &packets);
            let want = ffmpeg_rgb24(&ff, &file, "rle", w as u32, h as u32);
            assert_eq!(want.len(), n, "{w}x{h} {depth}-bit seed {seed}: ffmpeg decoded {} pictures", want.len());
            let mut d = rle::Decoder::new(w as u32, h as u32, depth as u16, &pal).unwrap();
            for (i, p) in packets.iter().enumerate() {
                let got = rgb(&d.decode(p).unwrap());
                assert!(got == want[i], "{w}x{h} {depth}-bit seed {seed}: picture {i} differs from ffmpeg's");
            }
        }
    }
}

/// After a picture that covers everything, random bytes as delta pictures: wherever ffmpeg
/// produces a picture (Video 1 always does; RLE does unless the packet is too short), so must we,
/// and it must be the same, however damaged the packet is. (Only the 4-bit RLE path is compared:
/// the 8- and 24-bit ones let a run overshoot the visible width into ffmpeg's row padding, whose
/// size depends on the build.)
#[test]
fn random_delta_pictures_match_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let (w, h) = (24u32, 16u32);
    for (name, bits, compression, pal_entries) in [("v1-16", 16u16, *b"MSVC", 0usize), ("v1-8", 8, *b"MSVC", 256), ("rle4", 4, [2, 0, 0, 0], 16)] {
        for seed in 0..6u64 {
            let mut rng = Rng(seed + 1000 + u64::from(bits));
            let pal = palette(&mut rng, pal_entries);
            let first = match name {
                "rle4" => rle_packet(&mut rng, w as usize, h as usize, 4, true),
                _ => video1_packet(&mut rng, w, h, bits == 16, true),
            };
            let mut packets = vec![first];
            for _ in 0..10 {
                let n = 8 + rng.below(120) as usize;
                packets.push((0..n).map(|_| rng.byte()).collect());
            }
            let file = avi(w, h, bits, compression, &pal, &packets);
            let want = ffmpeg_rgb24(&ff, &file, name, w, h);
            assert_eq!(want.len(), packets.len(), "{name} seed {seed}: ffmpeg decoded {} pictures", want.len());
            let mut ours: Box<dyn FnMut(&[u8]) -> Vec<u8>> = if name == "rle4" {
                let mut d = rle::Decoder::new(w, h, bits, &pal).unwrap();
                Box::new(move |p| rgb(&d.decode(p).unwrap()))
            } else {
                let mut d = video1::Decoder::new(w, h, bits, &pal).unwrap();
                Box::new(move |p| rgb(&d.decode(p).unwrap()))
            };
            for (i, p) in packets.iter().enumerate() {
                assert!(ours(p) == want[i], "{name} seed {seed}: picture {i} differs from ffmpeg's");
            }
        }
    }
}
