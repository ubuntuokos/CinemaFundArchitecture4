//! The Lagarith decoder against ffmpeg's `lagarith` decoder (the external oracle), on streams
//! written by a small encoder here: every frame type, the range coder with and without the
//! zero-run escape, uncompressed and solid planes, and odd sizes. The encoder is also checked by
//! decoding back to the source. Zero-run-only planes (which the reference encoder no longer
//! writes) are compared on random data, and damaged frames must never panic.

use std::path::Path;
use std::process::Command;

use cfa4_lagarith::{Decoder, Format, Picture};

// ---------------------------------------------------------------- helpers

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

/// The probability header's prefix code: (length, code, value), as in the format.
const PROB_CODES: [(u8, u8, u8); 53] = [
    (7, 0x01, 20),
    (7, 0x02, 12),
    (2, 0x03, 0),
    (7, 0x03, 12),
    (3, 0x03, 1),
    (4, 0x03, 2),
    (5, 0x03, 4),
    (6, 0x03, 7),
    (7, 0x04, 7),
    (7, 0x05, 28),
    (7, 0x08, 4),
    (7, 0x09, 25),
    (7, 0x0A, 17),
    (6, 0x0B, 10),
    (7, 0x0B, 17),
    (4, 0x0B, 3),
    (5, 0x0B, 6),
    (7, 0x10, 2),
    (7, 0x11, 23),
    (7, 0x12, 15),
    (7, 0x13, 15),
    (5, 0x13, 5),
    (6, 0x13, 9),
    (7, 0x14, 10),
    (7, 0x15, 31),
    (7, 0x20, 1),
    (7, 0x21, 22),
    (7, 0x22, 14),
    (7, 0x23, 14),
    (6, 0x23, 8),
    (7, 0x24, 9),
    (7, 0x25, 30),
    (7, 0x28, 6),
    (7, 0x29, 27),
    (7, 0x2A, 19),
    (6, 0x2B, 11),
    (7, 0x2B, 19),
    (7, 0x40, 0),
    (7, 0x41, 21),
    (7, 0x42, 13),
    (7, 0x43, 13),
    (7, 0x44, 8),
    (7, 0x45, 29),
    (7, 0x48, 5),
    (7, 0x49, 26),
    (7, 0x4A, 18),
    (7, 0x4B, 18),
    (7, 0x50, 3),
    (7, 0x51, 24),
    (7, 0x52, 16),
    (7, 0x53, 16),
    (7, 0x54, 11),
    (7, 0x55, 32),
];

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    fn put(&mut self, n: u32, v: u32) {
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
    /// One probability-header value.
    fn value(&mut self, v: u32) {
        let bits = if v == 0 { 0 } else { 31 - (v + 1).leading_zeros() };
        let &(len, code, _) = PROB_CODES.iter().find(|c| u32::from(c.2) == bits).expect("code");
        self.put(u32::from(len), u32::from(code));
        if bits > 0 {
            self.put(bits, (v + 1) - (1 << bits));
        }
    }
}

fn log2(v: u32) -> u32 {
    if v == 0 { 0 } else { 31 - v.leading_zeros() }
}

fn softfloat_reciprocal(denom: u32) -> u64 {
    let denom = u64::from(denom);
    let shift = log2((denom - 1) as u32) + 1;
    let mut ret = (1u64 << 52) / denom;
    let mut err = (1u64 << 52) - ret * denom;
    ret <<= shift;
    err <<= shift;
    err += denom / 2;
    ret + err / denom
}

fn softfloat_mul(x: u32, mantissa: u64) -> u32 {
    let x = u64::from(x);
    let mut l = x * (mantissa & 0xffff_ffff);
    let mut h = x * (mantissa >> 32);
    h += l >> 32;
    l &= 0xffff_ffff;
    l += 1u64 << log2((h >> 21) as u32);
    h += l >> 32;
    (h >> 20) as u32
}

/// The decoder's scaled cumulative probabilities and scale for the symbol `counts`, or `None`
/// where the format cannot code them (then the plane is stored uncompressed).
fn model(counts: &[u32; 256]) -> Option<([u32; 257], u32)> {
    let mut prob = [0u32; 257];
    prob[1..].copy_from_slice(counts);
    let cumul: u32 = counts.iter().sum();
    if counts.iter().filter(|&&c| c > 0).count() < 2 {
        return None;
    }
    let mut scale = log2(cumul);
    if cumul & (cumul - 1) != 0 {
        let mul = softfloat_reciprocal(cumul);
        let mut scaled = 0u32;
        for p in &mut prob[1..=128] {
            *p = softfloat_mul(*p, mul);
            scaled += *p;
        }
        if scaled == 0 {
            return None;
        }
        for p in &mut prob[129..] {
            *p = softfloat_mul(*p, mul);
            scaled += *p;
        }
        scale += 1;
        let target = 1u32 << scale;
        if scaled > target {
            return None;
        }
        let mut rest = target - scaled;
        let mut k = 1usize;
        while rest > 0 {
            if prob[k] != 0 {
                prob[k] += 1;
                rest -= 1;
            }
            k = (k & 0x7f) + 1;
        }
    }
    if scale > 23 {
        return None;
    }
    // a symbol that was counted but scaled to nothing cannot be coded
    if counts.iter().zip(&prob[1..]).any(|(&c, &p)| c > 0 && p == 0) {
        return None;
    }
    for k in 1..257 {
        prob[k] += prob[k - 1];
    }
    Some((prob, scale))
}

/// The range coder (the inverse of the decoder's): the code is a big number whose first digit
/// has 7 bits and every later one 8; it is written shifted by one bit.
fn range_encode(symbols: &[u8], cum: &[u32; 257], scale: u32) -> Vec<u8> {
    let mut digits: Vec<u32> = vec![0];
    let mut range: u32 = 0x80;
    for &s in symbols {
        while range <= 0x80_0000 {
            digits.push(0);
            range <<= 8;
        }
        let rs = range >> scale;
        let s = usize::from(s);
        let (add, next) = if s < 255 {
            (u64::from(rs) * u64::from(cum[s]), rs * (cum[s + 1] - cum[s]))
        } else {
            (u64::from(rs) * u64::from(cum[255]), range - rs * cum[255])
        };
        let mut carry = add;
        let mut k = digits.len() - 1;
        while carry > 0 {
            let base = if k == 0 { 128 } else { 256 };
            let v = u64::from(digits[k]) + carry;
            digits[k] = (v % base) as u32;
            carry = v / base;
            if carry > 0 {
                k = k.checked_sub(1).expect("range coder carry out of the code");
            }
        }
        range = if next == 0 { 0x80 } else { next };
    }
    let mut w = BitWriter::default();
    w.put(7, digits[0]);
    for &d in &digits[1..] {
        w.put(8, d);
    }
    w.put(1, 0);
    w.out
}

/// How a plane is stored.
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Range-coded, with the zero-run escape after this many zeros (0: none).
    Arith(u8),
    /// Uncompressed residuals.
    Raw,
}

/// One coded plane from its residuals (`plane` is the pixels, for the solid shortcut).
fn encode_plane(res: &[u8], pixels: &[u8], mode: Mode) -> Vec<u8> {
    if pixels.iter().all(|&p| p == pixels[0]) && matches!(mode, Mode::Arith(1)) {
        return vec![0xff, pixels[0]];
    }
    let raw = || {
        let mut v = vec![4u8];
        v.extend_from_slice(res);
        v
    };
    let Mode::Arith(esc) = mode else { return raw() };
    // the symbols, with the zero-run escape
    let mut syms = Vec::with_capacity(res.len());
    let mut zeros = 0u32;
    let mut i = 0;
    while i < res.len() {
        let v = res[i];
        syms.push(v);
        zeros = if v == 0 { zeros + 1 } else { 0 };
        i += 1;
        if esc != 0 && zeros == u32::from(esc) {
            let run = res[i..].iter().take(255).take_while(|&&r| r == 0).count();
            let x: i32 = if run % 2 == 0 { run as i32 / 2 } else { -(run as i32 + 1) / 2 };
            syms.push(x as i8 as u8);
            i += run;
            zeros = 0;
        }
    }
    let mut counts = [0u32; 256];
    for &s in &syms {
        counts[usize::from(s)] += 1;
    }
    let Some((cum, scale)) = model(&counts) else { return raw() };
    let mut w = BitWriter::default();
    let mut k = 0usize;
    while k < 256 {
        w.value(counts[k]);
        if counts[k] == 0 {
            let cap = 256 - (k + 1);
            let run = counts[k + 1..].iter().take_while(|&&c| c == 0).count().min(cap);
            w.value(run as u32);
            k += run;
        }
        k += 1;
    }
    let mut out = vec![esc];
    if esc != 0 {
        out.extend_from_slice(&((syms.len() as u32).min(res.len() as u32 - 1)).to_le_bytes());
    }
    out.extend_from_slice(&w.out);
    out.extend_from_slice(&range_encode(&syms, &cum, scale));
    out
}

fn mid(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

#[derive(Clone, Copy, PartialEq)]
enum Pred {
    Rgb,
    Yv12,
    Yuy2(bool),
}

/// The residuals of a plane (rows in coding order).
fn residuals(p: &[u8], w: usize, h: usize, pred: Pred) -> Vec<u8> {
    let mut r = vec![0u8; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let cur = p[row + x];
            let prev_px = if x > 0 { p[row + x - 1] } else { 0 };
            let pv: u8 = match (pred, y) {
                (Pred::Yuy2(true), 0) if x == 0 => 0,
                (Pred::Yuy2(true), 0) if x == 1 => 0,
                (_, 0) => prev_px,
                (Pred::Yuy2(luma), 1) => {
                    let head = if luma { 4 } else { 2 };
                    let left = if x > 0 { prev_px } else { p[row - 1] };
                    if x < head {
                        left
                    } else {
                        let t = i32::from(p[row - w + x]);
                        let tl = i32::from(p[row - w + x - 1]);
                        mid(i32::from(left), t, (i32::from(left) + t - tl) & 0xff) as u8
                    }
                }
                (Pred::Yuy2(_), _) => {
                    let left = if x > 0 { prev_px } else { p[row - 1] };
                    let t = i32::from(p[row - w + x]);
                    let tl = if x > 0 { i32::from(p[row - w + x - 1]) } else { i32::from(p[row - w - 1]) };
                    mid(i32::from(left), t, (i32::from(left) + t - tl) & 0xff) as u8
                }
                _ => {
                    let left = if x > 0 { prev_px } else { p[row - 1] };
                    let t = i32::from(p[row - w + x]);
                    let tl = if x > 0 {
                        i32::from(p[row - w + x - 1])
                    } else if y == 1 {
                        if pred == Pred::Yv12 { i32::from(p[row - w]) } else { i32::from(left) }
                    } else {
                        i32::from(p[row - w - 1])
                    };
                    (mid(i32::from(left), t, i32::from(left) + t - tl) & 0xff) as u8
                }
            };
            r[row + x] = cur.wrapping_sub(pv);
        }
    }
    r
}

fn flip(p: &[u8], w: usize) -> Vec<u8> {
    p.chunks(w).rev().flatten().copied().collect()
}

/// A test frame and how to code it.
struct Frame {
    pic: Picture,
    /// One mode per coded plane.
    modes: Vec<Mode>,
    /// A frame type to use instead (solid frames, `U_RGB24`).
    kind: Option<u8>,
}

fn encode_frame(f: &Frame) -> Vec<u8> {
    let p = &f.pic;
    let (w, h) = (p.width as usize, p.height as usize);
    if let Some(k @ (5 | 6 | 9)) = f.kind {
        let pl = |i: usize| p.planes[i][0];
        return match k {
            5 => vec![5, pl(0)],
            6 => vec![6, pl(2), pl(1), pl(0)],
            _ => vec![9, pl(2), pl(1), pl(0), pl(3)],
        };
    }
    let mode = |i: usize| f.modes[i % f.modes.len()];
    let (frametype, coded): (u8, Vec<Vec<u8>>) = match p.format {
        Format::Rgb | Format::Rgba => {
            let (r, g, b) = (&p.planes[0], &p.planes[1], &p.planes[2]);
            let sub = |a: &Vec<u8>| a.iter().zip(g).map(|(x, y)| x.wrapping_sub(*y)).collect::<Vec<u8>>();
            // coding order: R - G, G, B - G (, A), each bottom-up
            let mut planes = vec![sub(r), g.clone(), sub(b)];
            if p.format == Format::Rgba {
                planes.push(p.planes[3].clone());
            }
            let coded = planes
                .iter()
                .enumerate()
                .map(|(i, pl)| {
                    let up = flip(pl, w);
                    encode_plane(&residuals(&up, w, h, Pred::Rgb), &up, mode(i))
                })
                .collect();
            (if p.format == Format::Rgba { 8 } else { f.kind.unwrap_or(4) }, coded)
        }
        Format::Yuv422 | Format::Yuv420 => {
            let yv12 = p.format == Format::Yuv420;
            let (cw, ch) = p.plane_size(1);
            let (cw, ch) = (cw as usize, ch as usize);
            let (py, pc) = if yv12 { (Pred::Yv12, Pred::Yv12) } else { (Pred::Yuy2(true), Pred::Yuy2(cw == w)) };
            // coding order: Y, then U, V (YUY2) or V, U (YV12)
            let order = if yv12 { [0, 2, 1] } else { [0, 1, 2] };
            let coded = order
                .iter()
                .enumerate()
                .map(|(n, &i)| {
                    let (pw, ph, pr) = if i == 0 { (w, h, py) } else { (cw, ch, pc) };
                    encode_plane(&residuals(&p.planes[i], pw, ph, pr), &p.planes[i], mode(n))
                })
                .collect();
            (if yv12 { 10 } else { 3 }, coded)
        }
    };
    // header: type, offsets of the second and first coded planes, (alpha offset); planes follow
    let header = if frametype == 8 { 13 } else { 9 };
    let mut offs = Vec::new();
    let mut at = header;
    for c in &coded {
        offs.push(at as u32);
        at += c.len();
    }
    let mut out = vec![frametype];
    match frametype {
        // RGB: 9 (or 13) holds B - G; offset_gu G; offset_bv R - G; [9..13] the alpha offset
        3 | 10 => {
            out.extend_from_slice(&offs[1].to_le_bytes());
            out.extend_from_slice(&offs[2].to_le_bytes());
            let body: Vec<u8> = coded.concat();
            out.extend_from_slice(&body);
        }
        _ => {
            // reorder so that B - G comes first in the file
            let order = if frametype == 8 { [2, 1, 0, 3].as_slice() } else { [2, 1, 0].as_slice() };
            let mut pos = header as u32;
            let mut place = [0u32; 4];
            let mut body = Vec::new();
            for &i in order {
                place[i] = pos;
                pos += coded[i].len() as u32;
                body.extend_from_slice(&coded[i]);
            }
            out.extend_from_slice(&place[1].to_le_bytes());
            out.extend_from_slice(&place[0].to_le_bytes());
            if frametype == 8 {
                out.extend_from_slice(&place[3].to_le_bytes());
            }
            out.extend_from_slice(&body);
        }
    }
    out
}

/// A minimal AVI with one `LAGS` video stream.
fn avi(w: u32, h: u32, bit_count: u16, frames: &[Vec<u8>]) -> Vec<u8> {
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
        let mut b = kind.to_vec();
        b.extend_from_slice(body);
        chunk(b"LIST", &b)
    }
    let le = |v: u32| v.to_le_bytes();
    let n = frames.len() as u32;
    let maxsize = frames.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let mut avih = Vec::new();
    for v in [40_000, 0, 0, 0x10, n, 0, 1, maxsize, w, h, 0, 0, 0, 0] {
        avih.extend_from_slice(&le(v));
    }
    let mut strh = b"vidsLAGS".to_vec();
    for v in [0, 0, 0, 1, 25, 0, n, maxsize, u32::MAX, 0] {
        strh.extend_from_slice(&le(v));
    }
    strh.extend_from_slice(&[0u8; 8]);
    let mut strf = Vec::new();
    strf.extend_from_slice(&le(40));
    strf.extend_from_slice(&le(w));
    strf.extend_from_slice(&le(h));
    strf.extend_from_slice(&1u16.to_le_bytes());
    strf.extend_from_slice(&bit_count.to_le_bytes());
    strf.extend_from_slice(b"LAGS");
    for v in [w * h * 4, 0, 0, 0, 0] {
        strf.extend_from_slice(&le(v));
    }
    let strl = list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat());
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), strl].concat());
    let mut movi_body = Vec::new();
    let mut idx = Vec::new();
    for f in frames {
        let off = 4 + movi_body.len() as u32;
        idx.extend_from_slice(b"00dc");
        idx.extend_from_slice(&le(0x10));
        idx.extend_from_slice(&le(off));
        idx.extend_from_slice(&le(f.len() as u32));
        movi_body.extend_from_slice(&chunk(b"00dc", f));
    }
    let movi = list(b"movi", &movi_body);
    let body = [b"AVI ".to_vec(), hdrl, movi, chunk(b"idx1", &idx)].concat();
    chunk(b"RIFF", &body)
}

fn pix_fmt(f: Format) -> &'static str {
    match f {
        Format::Rgb => "gbrp",
        Format::Rgba => "gbrap",
        Format::Yuv422 => "yuv422p",
        Format::Yuv420 => "yuv420p",
    }
}

/// A picture as ffmpeg's raw output of its native format (planes G, B, R, A for RGB).
fn raw(p: &Picture) -> Vec<u8> {
    let order: &[usize] = match p.format {
        Format::Rgb => &[1, 2, 0],
        Format::Rgba => &[1, 2, 0, 3],
        _ => &[0, 1, 2],
    };
    order.iter().flat_map(|&i| p.planes[i].iter().copied()).collect()
}

/// ffmpeg's decoding of an AVI file, as raw frames of `fmt`.
fn ffmpeg_decode(ff: &Path, file: &[u8], tag: &str, fmt: &str) -> Vec<u8> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("lagarith");
    let path = dir.join(format!("{tag}.avi"));
    std::fs::write(&path, file).expect("write avi");
    let out =
        Command::new(ff).args(["-v", "error", "-threads", "1", "-i"]).arg(&path).args(["-f", "rawvideo", "-pix_fmt", fmt, "-"]).output().expect("run ffmpeg");
    out.stdout
}

// ---------------------------------------------------------------- pictures

fn image(rng: &mut Rng, format: Format, w: u32, h: u32, t: u32) -> Picture {
    let mut p = Picture { width: w, height: h, format, planes: Vec::new() };
    let n = match format {
        Format::Rgb | Format::Yuv422 | Format::Yuv420 => 3,
        Format::Rgba => 4,
    };
    for i in 0..n {
        let (pw, ph) = p.plane_size(i);
        let iu = i as u32;
        let mut v = Vec::with_capacity((pw * ph) as usize);
        for y in 0..ph {
            for x in 0..pw {
                // flat areas (zero residuals), gradients, edges and noise
                let base = if (x / 8 + y / 8 + t).is_multiple_of(3) { 40 + 30 * iu } else { (x * 3 + y * 2 + t * 5 + iu * 50) & 0xff };
                let noise = if (x + y * 7 + t).is_multiple_of(5) { rng.below(40) } else { 0 };
                v.push(((base + noise) & 0xff) as u8);
            }
        }
        p.planes.push(v);
    }
    p
}

fn solid(format: Format, w: u32, h: u32, vals: &[u8]) -> Picture {
    let n = (w * h) as usize;
    Picture { width: w, height: h, format, planes: vals.iter().map(|&v| vec![v; n]).collect() }
}

fn check(ff: &Path, tag: &str, bit_count: u16, frames: &[Frame]) {
    let (w, h) = (frames[0].pic.width, frames[0].pic.height);
    let fmt = frames[0].pic.format;
    let coded: Vec<Vec<u8>> = frames.iter().map(encode_frame).collect();
    let mut dec = Decoder::new(w, h, bit_count).expect("decoder");
    let mut ours = Vec::new();
    for (k, (f, c)) in frames.iter().zip(&coded).enumerate() {
        let pic = dec.decode(c).unwrap_or_else(|e| panic!("{tag} frame {k}: {e}"));
        let want_format = match (f.kind, bit_count) {
            (Some(5), 32) | (Some(6), 32) | (Some(9), _) => Format::Rgba,
            _ => f.pic.format,
        };
        assert_eq!(pic.format, want_format, "{tag} frame {k}");
        if f.kind != Some(5) || bit_count == 24 {
            for (i, (a, b)) in pic.planes.iter().zip(&f.pic.planes).enumerate() {
                assert!(a == b, "{tag} frame {k}: plane {i} does not decode back to the source");
            }
        }
        ours.extend(raw(&pic));
    }
    let theirs = ffmpeg_decode(ff, &avi(w, h, bit_count, &coded), tag, pix_fmt(fmt));
    assert_eq!(theirs.len(), ours.len(), "{tag}: ffmpeg decoded {} bytes, we {}", theirs.len(), ours.len());
    if let Some(i) = theirs.iter().zip(&ours).position(|(a, b)| a != b) {
        panic!("{tag}: differs from ffmpeg at byte {i}");
    }
}

#[test]
fn encoded_streams_match_ffmpeg_and_the_source() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("lagarith oracle") else { return };
    let mut rng = Rng(0x1a6a_417e);
    let modes = [Mode::Arith(0), Mode::Arith(1), Mode::Arith(2), Mode::Arith(3), Mode::Raw];
    let frames = |rng: &mut Rng, format, w, h| -> Vec<Frame> {
        (0..6u32).map(|t| Frame { pic: image(rng, format, w, h, t), modes: vec![modes[t as usize % 5], modes[(t as usize + 2) % 5]], kind: None }).collect()
    };

    // RGB24, with an unaligned-RGB24 frame and solid frames
    for (w, h) in [(64, 48), (37, 23)] {
        let mut f = frames(&mut rng, Format::Rgb, w, h);
        f.push(Frame { pic: image(&mut rng, Format::Rgb, w, h, 9), modes: vec![Mode::Arith(1)], kind: Some(2) });
        f.push(Frame { pic: solid(Format::Rgb, w, h, &[200, 100, 50]), modes: vec![], kind: Some(6) });
        f.push(Frame { pic: solid(Format::Rgb, w, h, &[77, 77, 77]), modes: vec![], kind: Some(5) });
        check(&ff, &format!("rgb24_{w}x{h}"), 24, &f);
    }
    // RGBA, with solid frames (grey and colour have an opaque or grey alpha at 32 bits)
    for (w, h) in [(48, 32), (21, 15)] {
        let mut f = frames(&mut rng, Format::Rgba, w, h);
        f.push(Frame { pic: solid(Format::Rgba, w, h, &[1, 2, 3, 4]), modes: vec![], kind: Some(9) });
        f.push(Frame { pic: solid(Format::Rgba, w, h, &[9, 8, 7, 255]), modes: vec![], kind: Some(6) });
        f.push(Frame { pic: solid(Format::Rgba, w, h, &[33, 33, 33, 33]), modes: vec![], kind: Some(5) });
        check(&ff, &format!("rgba_{w}x{h}"), 32, &f);
    }
    // YUY2 and YV12, even and odd sizes
    for (w, h) in [(64, 40), (38, 17)] {
        check(&ff, &format!("yuy2_{w}x{h}"), 16, &frames(&mut rng, Format::Yuv422, w, h));
    }
    for (w, h) in [(50, 30), (31, 19), (8, 6)] {
        check(&ff, &format!("yv12_{w}x{h}"), 12, &frames(&mut rng, Format::Yuv420, w, h));
    }
}

/// Zero-run-only planes (escape codes 5–7) on random data: wherever ffmpeg decodes a frame, the
/// result is the same, and where it refuses one, so do we.
#[test]
fn zero_run_planes_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("lagarith zero-run oracle") else { return };
    let mut rng = Rng(0x2e60_5eed);
    let (w, h) = (24u32, 8u32);
    let mut agreed = 0;
    for k in 0..150 {
        let mut planes = Vec::new();
        for _ in 0..3 {
            let esc = 5 + rng.below(3) as u8;
            let len = (w * h) as usize + rng.below(200) as usize;
            let mut p = vec![esc];
            for _ in 0..len {
                // rare zero groups, short runs (small run codes)
                let z = rng.below(14) == 0;
                p.push(if z {
                    0
                } else if p.last() == Some(&0) && rng.below(2) == 0 {
                    rng.below(6) as u8
                } else {
                    1 + rng.below(255) as u8
                });
            }
            planes.push(p);
        }
        let o1 = 9 + planes[0].len() as u32;
        let o2 = o1 + planes[1].len() as u32;
        let mut frame = vec![4u8];
        frame.extend_from_slice(&o1.to_le_bytes());
        frame.extend_from_slice(&o2.to_le_bytes());
        frame.extend_from_slice(&planes.concat());
        let ours = Decoder::new(w, h, 24).unwrap().decode(&frame);
        let theirs = ffmpeg_decode(&ff, &avi(w, h, 24, std::slice::from_ref(&frame)), &format!("zero_run_{k}"), "gbrp");
        match ours {
            Ok(p) => {
                assert_eq!(raw(&p), theirs, "zero-run frame {k}");
                agreed += 1;
            }
            Err(e) => assert!(theirs.is_empty(), "zero-run frame {k}: we refused ({e}), ffmpeg decoded it"),
        }
    }
    assert!(agreed >= 10, "only {agreed} zero-run frames decoded");
}

#[test]
fn damaged_frames_never_panic() {
    let mut rng = Rng(0xdead_beef);
    for (format, bits) in [(Format::Rgb, 24), (Format::Rgba, 32), (Format::Yuv422, 16), (Format::Yuv420, 12)] {
        for t in 0..4 {
            let f = Frame { pic: image(&mut rng, format, 20, 12, t), modes: vec![Mode::Arith(t as u8 % 4)], kind: None };
            let good = encode_frame(&f);
            for _ in 0..300 {
                let mut bad = good.clone();
                match rng.below(3) {
                    0 => bad.truncate(rng.below(bad.len() as u32) as usize),
                    1 => {
                        for _ in 0..1 + rng.below(4) {
                            let i = rng.below(bad.len() as u32) as usize;
                            bad[i] = rng.below(256) as u8;
                        }
                    }
                    _ => {
                        let i = 1 + rng.below(12) as usize;
                        if i < bad.len() {
                            bad[i] = rng.below(256) as u8;
                        }
                    }
                }
                let mut dec = Decoder::new(20, 12, bits).unwrap();
                let _ = dec.decode(&bad);
                let _ = dec.decode(&good);
            }
        }
    }
}
