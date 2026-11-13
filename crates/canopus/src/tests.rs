//! Synthetic HQ, HQA and HQX frames (random codes of every table, random headers) decoded by us
//! and by ffmpeg's `hq_hqa` and `hqx` decoders (an external oracle only): every sample equal.
//! Damaged frames never panic. Skips without ffmpeg unless `CFA4_REQUIRE_ORACLES=1`.

use std::path::PathBuf;
use std::process::Command;

use super::*;
use crate::bits::Bits;
use crate::tables::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

#[derive(Default)]
struct W {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl W {
    fn put(&mut self, v: u32, n: u32) {
        if n == 0 {
            return;
        }
        self.acc = (self.acc << n) | u64::from(v & ((1u64 << n) - 1) as u32);
        self.n += n;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
    }
    fn bytes(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put(0, pad);
        }
        self.out
    }
}

/// A random code read by `read` from random bits; the consumed bits are written to `w`.
fn code<R>(rng: &mut Rng, w: &mut W, read: impl Fn(&mut Bits<'_>) -> Option<R>) -> R {
    loop {
        let v = rng.next();
        let bytes = v.to_be_bytes();
        let mut b = Bits::new(&bytes);
        if let Some(r) = read(&mut b) {
            let used = (32 - b.left()) as u32;
            w.put(v >> (32 - used), used);
            return r;
        }
    }
}

fn hq_block(t: &hq::Tables, rng: &mut Rng, w: &mut W, hqa: bool) {
    let (dc, q) = (rng.below(512), rng.below(4));
    if hqa {
        w.put(q, 2);
        w.put(dc, 9);
    } else {
        w.put(dc, 9);
        w.put(q, 2);
    }
    let mut pos = 1usize;
    loop {
        let val = code(rng, w, |b| t.ac_read(b));
        pos += usize::from(HQ_AC_SKIPS[val]);
        if pos >= 64 {
            break;
        }
        pos += 1;
    }
}

fn be(v: u32, n: usize) -> Vec<u8> {
    v.to_be_bytes()[4 - n..].to_vec()
}

/// Slices after a header of `head` bytes (counted from `base`), with offsets of `n` bytes plus `add`.
fn assemble(mut out: Vec<u8>, slices: &[Vec<u8>], n: usize, base: usize, add: u32) -> Vec<u8> {
    let mut at = (base + n * (slices.len() + 1)) as u32;
    let mut offs = Vec::new();
    for s in slices {
        offs.extend(be(at + add, n));
        at += s.len() as u32;
    }
    offs.extend(be(at + add, n));
    out.extend(offs);
    for s in slices {
        out.extend_from_slice(s);
    }
    out
}

fn hq_frame(t: &hq::Tables, rng: &mut Rng, profile: u8) -> Vec<u8> {
    let (_, _, _, ns, tab_w, tab_h) = *HQ_PROFILES.get(usize::from(profile)).unwrap_or(&HQ_PROFILES[0]);
    let ns = ns as usize;
    let mut slices = Vec::new();
    let mut next = 0usize;
    for s in 0..ns {
        let start = next;
        next = tab_h as usize * (s + 1) / ns;
        let mut w = W::default();
        for _ in 0..(next - start) * tab_w as usize {
            w.put(rng.below(16), 4);
            w.put(rng.below(2), 1);
            for _ in 0..8 {
                hq_block(t, rng, &mut w, false);
            }
        }
        slices.push(w.bytes());
    }
    assemble(vec![b'U', b'V', b'C', profile], &slices, 3, 0, 4)
}

fn hqa_frame(t: &hq::Tables, rng: &mut Rng, width: u32, height: u32, quant: u8) -> Vec<u8> {
    let mut slices = Vec::new();
    for s in 0..8usize {
        let mut w = W::default();
        for i in (0..height as usize).step_by(16) {
            for _ in (((s * 16 + i * 3) & 0x70)..width as usize).step_by(128) {
                let cbp = code(rng, &mut w, |b| t.cbp_read(b)) as u32;
                if cbp != 0 {
                    w.put(rng.below(2), 1);
                    let mut cbp = cbp | cbp << 4;
                    if cbp & 3 != 0 {
                        cbp |= 0x500;
                    }
                    if cbp & 0xC != 0 {
                        cbp |= 0xA00;
                    }
                    for i in 0..12 {
                        if cbp & (1 << i) != 0 {
                            hq_block(t, rng, &mut w, true);
                        }
                    }
                }
            }
        }
        slices.push(w.bytes());
    }
    let mut head = b"HQA1".to_vec();
    head.extend(be(width, 2));
    head.extend(be(height, 2));
    head.extend([quant, 0, 0, 0]);
    assemble(head, &slices, 4, 8, 4)
}

fn hqx_block(t: &hqx::Tables, rng: &mut Rng, w: &mut W, dcb: u32, quants: &[i32; 4]) {
    code(rng, w, |b| t.dc_read(dcb, b));
    let qi = rng.below(4);
    w.put(qi, 2);
    let q = quants[qi as usize];
    let mut pos = 1usize;
    while pos < 64 {
        let (run, _) = code(rng, w, |b| Some(t.ac_read(q, b)));
        pos += run;
        if pos >= 64 {
            break;
        }
        pos += 1;
    }
}

fn hqx_frame(t: &hqx::Tables, rng: &mut Rng, width: u32, height: u32, format: u8, dcb: u32, interlaced: bool) -> Vec<u8> {
    let alpha = format & 2 != 0;
    let full = format & 1 != 0;
    let n = match (full, alpha) {
        (false, false) => 8,
        (true, true) => 16,
        _ => 12,
    };
    let num_mbs = (width.div_ceil(16) * height.div_ceil(16)) as usize;
    let num_tiles = num_mbs.div_ceil(480);
    let std = num_mbs / (16 * num_tiles);
    let mut slices = Vec::new();
    for s in 0..16 {
        let mut w = W::default();
        let mut count = 0;
        for tile in 0..num_tiles {
            let g_tile = s * num_tiles + tile;
            count += std + usize::from(g_tile < num_mbs - std * 16 * num_tiles);
        }
        for _ in 0..count {
            let mut cbp = 0x7FFF_FFFF;
            if alpha {
                cbp = code(rng, &mut w, |b| t.cbp_read(b)) as u32;
            }
            if cbp != 0 {
                if interlaced {
                    w.put(rng.below(2), 1);
                }
                let qi = rng.below(16);
                w.put(qi, 4);
                let quants = hqx::QUANTS[qi as usize];
                if alpha {
                    cbp |= cbp << 4;
                    if full {
                        cbp |= cbp << 8;
                    } else {
                        if cbp & 3 != 0 {
                            cbp |= 0x500;
                        }
                        if cbp & 0xC != 0 {
                            cbp |= 0xA00;
                        }
                    }
                }
                for i in 0..n {
                    if cbp & (1 << i) != 0 {
                        hqx_block(t, rng, &mut w, dcb, &quants);
                    }
                }
            }
        }
        slices.push(w.bytes());
    }
    let mut head = vec![b'H', b'Q', if interlaced { 0 } else { 0x80 } | format, (dcb - 8) as u8];
    head.extend(be(width, 2));
    head.extend(be(height, 2));
    assemble(head, &slices, 3, 8, 0)
}

fn ffmpeg() -> Option<PathBuf> {
    let f = cfa4_testkit::oracle::ffmpeg();
    if f.is_none() {
        cfa4_testkit::oracle::skip("canopus oracle", "ffmpeg not found");
    }
    f
}

/// ffmpeg's decode of one frame, in its native pixel format.
fn ffmpeg_decode(ff: &PathBuf, name: &str, decoder: &str, frame: &[u8]) -> Vec<u8> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("canopus");
    let file = dir.join(name);
    std::fs::write(&file, frame).expect("write frame");
    let out = Command::new(ff)
        .args(["-v", "error", "-f", "image2", "-c:v", decoder, "-i"])
        .arg(&file)
        .args(["-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "{name}: ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// Our picture in ffmpeg's layout: 8-bit planes as they are, 12-bit ones as ffmpeg's 16-bit LE.
fn ours(p: &Picture) -> Vec<u8> {
    match &p.planes {
        Planes::U8(planes) => planes.concat(),
        Planes::U16(planes) => planes.iter().flatten().flat_map(|&v| ((v << 4) | (v >> 8)).to_le_bytes()).collect(),
    }
}

fn check(ff: &PathBuf, name: &str, decoder: &str, codec: Codec, frame: &[u8]) {
    let want = ffmpeg_decode(ff, name, decoder, frame);
    let p = Decoder::new(codec).decode(frame).unwrap_or_else(|e| panic!("{name}: {e}"));
    let got = ours(&p);
    assert_eq!(got.len(), want.len(), "{name}: size ({}x{} {:?})", p.width, p.height, p.layout);
    if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
        panic!("{name}: first difference at byte {i} of {}: ours {}, ffmpeg {}", got.len(), got[i], want[i]);
    }
}

#[test]
fn hq_and_hqa_match_ffmpeg() {
    let Some(ff) = ffmpeg() else { return };
    let t = hq::Tables::new();
    let mut rng = Rng(0x4851_0001);
    for profile in [0u8, 16, 10, 14, 9, 2, 30] {
        let f = hq_frame(&t, &mut rng, profile);
        check(&ff, &format!("hq_{profile}.bin"), "hq_hqa", Codec::Hq, &f);
    }
    // with an INFO block in front
    let mut f = b"INFO".to_vec();
    f.extend(16u32.to_le_bytes());
    f.extend([0u8; 16]);
    f.extend(hq_frame(&t, &mut rng, 11));
    check(&ff, "hq_info.bin", "hq_hqa", Codec::Hq, &f);
    for (i, (w, h, q)) in [(64, 48, 0u8), (200, 120, 7), (333, 250, 15), (720, 486, 3), (16, 16, 9)].into_iter().enumerate() {
        let f = hqa_frame(&t, &mut rng, w, h, q);
        check(&ff, &format!("hqa_{i}.bin"), "hq_hqa", Codec::Hq, &f);
    }
}

#[test]
fn hqx_matches_ffmpeg() {
    let Some(ff) = ffmpeg() else { return };
    let t = hqx::Tables::new();
    let mut rng = Rng(0x4851_5801);
    let cases: &[(u32, u32, u8, u32, bool)] = &[
        (64, 48, 0, 9, false),
        (176, 144, 0, 10, true),
        (200, 120, 1, 11, false),
        (333, 250, 2, 9, true),
        (320, 240, 3, 10, false),
        (720, 486, 0, 11, true),
        (1280, 720, 2, 10, false),
        (16, 16, 1, 9, true),
        (1920, 1080, 0, 10, true),
    ];
    for (i, &(w, h, format, dcb, ilace)) in cases.iter().enumerate() {
        let f = hqx_frame(&t, &mut rng, w, h, format, dcb, ilace);
        check(&ff, &format!("hqx_{i}.bin"), "hqx", Codec::Hqx, &f);
    }
}

#[test]
fn damaged_frames_never_panic() {
    let (t, tx) = (hq::Tables::new(), hqx::Tables::new());
    let mut rng = Rng(0xDA3A_6E01);
    let sources = [
        hq_frame(&t, &mut rng, 11),
        hqa_frame(&t, &mut rng, 96, 64, 2),
        hqx_frame(&tx, &mut rng, 96, 64, 3, 11, true),
        hqx_frame(&tx, &mut rng, 80, 48, 0, 9, false),
    ];
    let (dq, dx) = (Decoder::new(Codec::Hq), Decoder::new(Codec::Hqx));
    for round in 0..2000 {
        let mut d = sources[round % sources.len()].clone();
        match round / 4 % 4 {
            0 => {
                let at = rng.below(d.len() as u32) as usize;
                d[at] ^= 1 << rng.below(8);
            }
            1 => d.truncate(rng.below(d.len() as u32) as usize),
            2 => {
                let start = rng.below(d.len() as u32) as usize;
                for b in d.iter_mut().skip(start).take(1 + rng.below(100) as usize) {
                    *b = rng.next() as u8;
                }
            }
            _ => {
                // the header and the slice offsets
                let at = rng.below(64.min(d.len() as u32)) as usize;
                d[at] = rng.next() as u8;
            }
        }
        let _ = dq.decode(&d);
        let _ = dx.decode(&d);
    }
    assert!(dq.decode(b"UVC").is_err());
    assert!(dx.decode(&[0; 7]).is_err());
}

#[test]
fn fourccs() {
    assert_eq!(Codec::from_fourcc(b"CUVC"), Some(Codec::Hq));
    assert_eq!(Codec::from_fourcc(b"chqx"), Some(Codec::Hqx));
    assert_eq!(Codec::from_fourcc(b"MJPG"), None);
}
