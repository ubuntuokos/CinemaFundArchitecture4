//! The MagicYUV decoder against ffmpeg's `magicyuv` decoder (the external oracle): AVI files that
//! ffmpeg's encoder writes (every 8-bit format and prediction, slices), and streams from a small
//! encoder here for what ffmpeg's encoder cannot write (10- to 14-bit formats, interlaced
//! frames, raw slices, incomplete codes). The encoder's streams also decode back to the source.
//! Damaged frames must never panic.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_magicyuv::{Decoder, Format, Picture};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
}

fn dir() -> PathBuf {
    cfa4_testkit::fixtures::fixtures_dir("magicyuv")
}

/// The video chunks of an AVI file.
fn avi_frames(path: &Path) -> Vec<Vec<u8>> {
    let data = std::fs::read(path).expect("read avi");
    let avi = cfa4_avi::open(data.as_slice()).expect("open avi");
    let stream = avi.streams.iter().position(|s| s.video.is_some()).expect("video stream");
    (0..avi.streams[stream].chunks.len()).map(|i| avi.read_chunk(data.as_slice(), stream, i).expect("chunk")).collect()
}

fn pix_fmt(p: &Picture) -> String {
    let base = match p.format {
        Format::Rgb => "gbrp",
        Format::Rgba => "gbrap",
        Format::Yuv444 => "yuv444p",
        Format::Yuv422 => "yuv422p",
        Format::Yuv420 => "yuv420p",
        Format::Yuva444 => "yuva444p",
        Format::Gray => "gray",
    };
    if p.bits == 8 { base.into() } else { format!("{base}{}le", p.bits) }
}

/// A picture as ffmpeg's raw output of its native format (G, B, R, A for RGB; 16-bit LE above 8 bits).
fn raw(p: &Picture) -> Vec<u8> {
    let order: Vec<usize> = match p.format {
        Format::Rgb => vec![1, 2, 0],
        Format::Rgba => vec![1, 2, 0, 3],
        _ => (0..p.planes.len()).collect(),
    };
    let mut out = Vec::new();
    for i in order {
        for &v in &p.planes[i] {
            if p.bits == 8 {
                out.push(v as u8);
            } else {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    out
}

fn ffmpeg_decode(ff: &Path, path: &Path, fmt: &str) -> Vec<u8> {
    let out =
        Command::new(ff).args(["-v", "error", "-threads", "1", "-i"]).arg(path).args(["-f", "rawvideo", "-pix_fmt", fmt, "-"]).output().expect("run ffmpeg");
    out.stdout
}

/// Decode every frame of `path` and compare with ffmpeg; returns the pictures.
fn compare(ff: &Path, path: &Path, tag: &str) -> Vec<Picture> {
    let mut dec = Decoder::new();
    let pics: Vec<Picture> = avi_frames(path).iter().enumerate().map(|(k, f)| dec.decode(f).unwrap_or_else(|e| panic!("{tag} frame {k}: {e}"))).collect();
    assert!(!pics.is_empty(), "{tag}: no frames");
    let ours: Vec<u8> = pics.iter().flat_map(raw).collect();
    let theirs = ffmpeg_decode(ff, path, &pix_fmt(&pics[0]));
    assert_eq!(theirs.len(), ours.len(), "{tag}: ffmpeg decoded {} bytes, we {}", theirs.len(), ours.len());
    if let Some(i) = theirs.iter().zip(&ours).position(|(a, b)| a != b) {
        panic!("{tag}: differs from ffmpeg at byte {i}");
    }
    pics
}

#[test]
fn ffmpeg_encoded_files_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("magicyuv oracle") else { return };
    let mut n = 0;
    for fmt in ["gbrp", "gbrap", "yuv444p", "yuv422p", "yuv420p", "yuva444p", "gray"] {
        for (pred, name) in [(1, "left"), (2, "gradient"), (3, "median")] {
            for (size, slices, src) in [("64x48", "1", "testsrc2"), ("37x29", "3", "mandelbrot"), ("80x40", "2", "noise")] {
                let tag = format!("{fmt}_{name}_{size}_{slices}_{src}");
                let path = dir().join(format!("{tag}.avi"));
                let input = if src == "noise" { format!("color=c=gray:s={size},noise=alls=100:allf=t") } else { format!("{src}=s={size}") };
                let ok = Command::new(&ff)
                    .args([
                        "-v",
                        "error",
                        "-y",
                        "-f",
                        "lavfi",
                        "-i",
                        &input,
                        "-frames:v",
                        "3",
                        "-pix_fmt",
                        fmt,
                        "-c:v",
                        "magicyuv",
                        "-pred",
                        &pred.to_string(),
                        "-slices",
                        slices,
                        "-threads",
                        "1",
                    ])
                    .arg(&path)
                    .status()
                    .is_ok_and(|s| s.success());
                assert!(ok, "{tag}: ffmpeg could not write it");
                let pics = compare(&ff, &path, &tag);
                assert_eq!(pics[0].width, size.split('x').next().unwrap().parse::<u32>().unwrap(), "{tag}");
                n += 1;
            }
        }
    }
    assert_eq!(n, 63);
}

// ---------------------------------------------------------------- the test encoder

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    bits: u64,
}

impl BitWriter {
    fn put(&mut self, n: u32, v: u64) {
        for k in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.out.push(0);
            }
            if (v >> k) & 1 != 0 {
                let last = self.out.len() - 1;
                self.out[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

fn mid(a: i64, b: i64, c: i64) -> i64 {
    a.max(b).min(a.min(b).max(c))
}

/// The residuals of one slice of one plane (`p` its pixels, `w` wide) under prediction `pred`.
fn residuals(p: &[u16], w: usize, pred: u8, interlaced: bool, bits: u32) -> Vec<u16> {
    let h = p.len() / w;
    let mask = (1i64 << bits) - 1;
    let first = 1 + usize::from(interlaced);
    let up = if interlaced { 2 * w } else { w };
    let mut r = vec![0u16; p.len()];
    for k in 0..h {
        for x in 0..w {
            let i = k * w + x;
            let v = i64::from(p[i]);
            let left = if x > 0 { i64::from(p[i - 1]) } else { 0 };
            let pv = if k < first {
                left
            } else {
                let t = i64::from(p[i - up]);
                match pred {
                    1 => {
                        if x == 0 {
                            t
                        } else {
                            left
                        }
                    }
                    2 => {
                        if x == 0 {
                            t
                        } else {
                            left + t - i64::from(p[i - up - 1])
                        }
                    }
                    _ => {
                        if x == 0 {
                            t
                        } else {
                            let lt = i64::from(p[i - up - 1]);
                            let grad = if bits == 8 { (left + t - lt) & 0xff } else { left + t - lt };
                            mid(left, t, grad)
                        }
                    }
                }
            };
            r[i] = ((v - pv) & mask) as u16;
        }
    }
    r
}

/// A code length per symbol: `pattern` 0 gives every symbol `bits` bits (a complete code), 1 a
/// complete code of mixed lengths, 2 an incomplete one.
fn lengths(bits: u32, pattern: u32) -> Vec<u8> {
    let n = 1usize << bits;
    let b = bits as u8;
    match pattern {
        0 => vec![b; n],
        // half the symbols one bit shorter... : n/4 at b-1 (1/2), n/2 at b+1 (1/4), n/4 at b+... keep Kraft = 1
        1 => (0..n)
            .map(|s| {
                if s < n / 4 {
                    b - 1
                } else if s < n / 2 {
                    b
                } else {
                    b + 1
                }
            })
            .collect(),
        _ => (0..n).map(|s| if s < n / 4 { b - 1 } else { b + 2 }).collect(),
    }
}

/// The canonical codes of `lens` (longest first, then by symbol, counting up from zero).
fn codes(lens: &[u8]) -> Vec<(u64, u8)> {
    let mut order: Vec<usize> = (0..lens.len()).collect();
    order.sort_by(|&a, &b| lens[b].cmp(&lens[a]).then(a.cmp(&b)));
    let mut out = vec![(0u64, 0u8); lens.len()];
    let mut code = 0u64;
    for s in order {
        let l = u32::from(lens[s]);
        out[s] = (code >> (32 - l), lens[s]);
        code += 1 << (32 - l);
    }
    out
}

/// The run-length coded lengths.
fn rle(lens: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        let run = lens[i..].iter().take(256).take_while(|&&l| l == lens[i]).count();
        if run == 1 {
            out.push(lens[i]);
        } else {
            out.extend_from_slice(&[0x80 | lens[i], (run - 1) as u8]);
        }
        i += run;
    }
    out
}

struct Stream {
    format: Format,
    bits: u32,
    code: u8,
    slice_height: u32,
    interlaced: bool,
    pred: u8,
    /// Per (slice, plane): store raw.
    raw: fn(usize, usize) -> bool,
    pattern: u32,
}

fn encode(s: &Stream, pic: &Picture) -> Vec<u8> {
    let (w, h) = (pic.width, pic.height);
    let planes = pic.planes.len();
    let max = 1u32 << s.bits;
    let mask = (max - 1) as u16;
    // coded planes: B - G, G, R - G (, A) for RGB
    let coded: Vec<Vec<u16>> = match s.format {
        Format::Rgb | Format::Rgba => {
            let (r, g, b) = (&pic.planes[0], &pic.planes[1], &pic.planes[2]);
            let sub = |a: &Vec<u16>| a.iter().zip(g).map(|(x, y)| x.wrapping_sub(*y) & mask).collect::<Vec<u16>>();
            let mut v = vec![sub(b), g.clone(), sub(r)];
            v.extend(pic.planes.iter().skip(3).cloned());
            v
        }
        _ => pic.planes.clone(),
    };
    let nb_slices = h.div_ceil(s.slice_height) as usize;
    let lens: Vec<u8> = lengths(s.bits, s.pattern);
    let table = codes(&lens);
    let mut slices = vec![Vec::new(); planes];
    for j in 0..nb_slices {
        for (i, plane) in coded.iter().enumerate() {
            let (pw, _) = pic.plane_size(i);
            let vs = if (i == 1 || i == 2) && s.format == Format::Yuv420 { 1 } else { 0 };
            let rows = (s.slice_height as usize).min(h as usize - j * s.slice_height as usize);
            let ph = rows.div_ceil(1 << vs);
            let first = j * (s.slice_height as usize).div_ceil(1 << vs);
            let part = &plane[first * pw as usize..(first + ph) * pw as usize];
            let res = residuals(part, pw as usize, s.pred, s.interlaced, s.bits);
            let raw = (s.raw)(j, i);
            let mut out = vec![u8::from(raw), s.pred];
            let mut bw = BitWriter::default();
            for &v in &res {
                if raw {
                    if s.bits == 8 {
                        out.push(v as u8);
                    } else {
                        bw.put(s.bits, u64::from(v));
                    }
                } else {
                    let (c, l) = table[usize::from(v)];
                    bw.put(u32::from(l), c);
                }
            }
            out.extend_from_slice(&bw.out);
            slices[i].push(out);
        }
    }
    let mut tables = Vec::new();
    for _ in 0..planes {
        tables.extend_from_slice(&rle(&lens));
    }
    let header_size = 32u32;
    let mut head = b"MAGY".to_vec();
    head.extend_from_slice(&header_size.to_le_bytes());
    head.extend_from_slice(&[7, s.code, 0, 2, if s.interlaced { 2 } else { 0 } | 4, 0, 0, 0]);
    for v in [w, h, w, s.slice_height, 0] {
        head.extend_from_slice(&v.to_le_bytes());
    }
    let table_pos = head.len() + planes * nb_slices * 4 + 1 + nb_slices * planes;
    let data_start = table_pos + tables.len();
    let mut offset = data_start as u32 - header_size;
    let mut body = Vec::new();
    for plane in &slices {
        for sl in plane {
            head.extend_from_slice(&offset.to_le_bytes());
            offset += sl.len() as u32;
            body.extend_from_slice(sl);
        }
    }
    head.push(planes as u8);
    head.extend(std::iter::repeat_n(0u8, nb_slices * planes));
    head.extend_from_slice(&tables);
    head.extend_from_slice(&body);
    head
}

fn image(rng: &mut Rng, format: Format, bits: u32, w: u32, h: u32, t: u32) -> Picture {
    let n = match format {
        Format::Gray => 1,
        Format::Rgba | Format::Yuva444 => 4,
        _ => 3,
    };
    let mut p = Picture { width: w, height: h, format, bits, planes: Vec::new(), matrix: None, full_range: false, interlaced: false };
    let max = (1u32 << bits) - 1;
    for i in 0..n {
        let (pw, ph) = p.plane_size(i);
        let mut v = Vec::new();
        for y in 0..ph {
            for x in 0..pw {
                let base = if (x / 6 + y / 5 + t).is_multiple_of(3) { (i as u32 + 1) * 77 } else { x * 13 + y * 9 + t * 31 + i as u32 * 101 };
                let noise = if (x * 3 + y + t).is_multiple_of(4) { rng.below(1 << (bits - 3)) } else { 0 };
                v.push(((base + noise) & max) as u16);
            }
        }
        p.planes.push(v);
    }
    p
}

/// A minimal AVI with one `MAGY` video stream.
fn avi(w: u32, h: u32, frames: &[Vec<u8>]) -> Vec<u8> {
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
    let words = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let n = frames.len() as u32;
    let avih = words(&[40_000, 0, 0, 0x10, n, 0, 1, 0, w, h, 0, 0, 0, 0]);
    let strh = [b"vidsMAGY".as_slice(), &words(&[0, 0, 0, 1, 25, 0, n, 0, u32::MAX, 0]), &[0u8; 8]].concat();
    let strf = [words(&[40, w, h]), [1u16.to_le_bytes(), 24u16.to_le_bytes()].concat(), b"MAGY".to_vec(), words(&[0; 5])].concat();
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat())].concat());
    let (mut movi, mut idx) = (Vec::new(), Vec::new());
    for f in frames {
        idx.extend_from_slice(&[b"00dc".as_slice(), &words(&[0x10, 4 + movi.len() as u32, f.len() as u32])].concat());
        movi.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi), chunk(b"idx1", &idx)].concat())
}

#[test]
fn encoded_streams_match_ffmpeg_and_the_source() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("magicyuv oracle") else { return };
    let mut rng = Rng(0x3a61_c0de);
    let formats = [
        (Format::Rgb, 8, 0x65),
        (Format::Rgba, 8, 0x66),
        (Format::Yuv444, 8, 0x67),
        (Format::Yuv422, 8, 0x68),
        (Format::Yuv420, 8, 0x69),
        (Format::Yuva444, 8, 0x6a),
        (Format::Gray, 8, 0x6b),
        (Format::Yuv422, 10, 0x6c),
        (Format::Yuv444, 10, 0x76),
        (Format::Rgb, 10, 0x6d),
        (Format::Rgba, 10, 0x6e),
        (Format::Rgb, 12, 0x6f),
        (Format::Rgba, 12, 0x70),
        (Format::Rgb, 14, 0x71),
        (Format::Rgba, 14, 0x72),
        (Format::Gray, 10, 0x73),
        (Format::Yuv420, 10, 0x7b),
    ];
    let raws: [fn(usize, usize) -> bool; 3] = [|_, _| false, |j, i| (j + i) % 3 == 1, |_, _| true];
    for (k, &(format, bits, code)) in formats.iter().enumerate() {
        for interlaced in [false, true] {
            let (w, h) = if k % 2 == 0 { (40, 28) } else { (27, 21) };
            let mut frames = Vec::new();
            let mut pics = Vec::new();
            for t in 0..6u32 {
                let s = Stream {
                    format,
                    bits,
                    code,
                    // odd slice heights only where chroma is not halved vertically (4:2:0 streams use even ones)
                    slice_height: if interlaced {
                        [8, 12, 28][t as usize % 3]
                    } else if format == Format::Yuv420 {
                        [6, 28, 10][t as usize % 3]
                    } else {
                        [5, 28, 7][t as usize % 3]
                    },
                    interlaced,
                    pred: 1 + (t % 3) as u8,
                    raw: raws[(t as usize + k) % 3],
                    pattern: t % 3,
                };
                let pic = image(&mut rng, format, bits, w, h, t);
                let coded = encode(&s, &pic);
                let got = Decoder::new().decode(&coded).unwrap_or_else(|e| panic!("{format:?} {bits} frame {t}: {e}"));
                assert!(got.planes == pic.planes, "{format:?} {bits}-bit interlaced {interlaced} frame {t}: not the source");
                frames.push(coded);
                pics.push(pic);
            }
            let tag = format!("enc_{format:?}_{bits}_{}", if interlaced { "i" } else { "p" });
            let path = dir().join(format!("{tag}.avi"));
            std::fs::write(&path, avi(w, h, &frames)).expect("write avi");
            compare(&ff, &path, &tag);
        }
    }
}

#[test]
fn damaged_frames_never_panic() {
    let mut rng = Rng(0xbad5_eed5);
    for (format, bits, code) in [(Format::Rgba, 8, 0x66), (Format::Yuv420, 8, 0x69), (Format::Rgb, 12, 0x6f), (Format::Yuv422, 10, 0x6c)] {
        for t in 0..3u32 {
            let s = Stream { format, bits, code, slice_height: 6, interlaced: t == 1, pred: 1 + t as u8, raw: |_, i| i == 1, pattern: t };
            let good = encode(&s, &image(&mut rng, format, bits, 18, 14, t));
            for _ in 0..400 {
                let mut bad = good.clone();
                if rng.below(3) == 0 {
                    bad.truncate(rng.below(bad.len() as u32) as usize);
                } else {
                    for _ in 0..1 + rng.below(4) {
                        let i = if rng.below(2) == 0 { rng.below(80) as usize } else { rng.below(bad.len() as u32) as usize };
                        if let Some(b) = bad.get_mut(i) {
                            *b = rng.below(256) as u8;
                        }
                    }
                }
                let _ = Decoder::new().decode(&bad);
            }
        }
    }
}
