//! Hap (`cfa4-hap`) through the containers against ffmpeg: QuickTime and AVI files that ffmpeg
//! writes (Hap, Hap Alpha, Hap Q; uncompressed, Snappy, chunks), and files whose textures were
//! replaced with random blocks (every DXT1 / DXT5 mode, YCoCg scales) or rewritten as Hap
//! Alpha-Only and Hap Q Alpha, decode to ffmpeg's pictures frame by frame, also out of order.

mod common;

use std::path::{Path, PathBuf};

use common::*;

const W: u32 = 128;
const H: u32 = 64;
const SRC: &str = "testsrc2=size=128x64:rate=10:duration=1,noise=alls=40:allf=t,format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='mod(X*5+Y*3+N*17,256)'";

/// Compare every frame of `path` with ffmpeg's RGBA, backwards and then forwards.
fn check(ff: &Path, name: &str, path: &Path) {
    let src = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height), (W, H), "{name}: reported size");
    let raw = ffmpeg_frames(ff, path, "rgba");
    let want: Vec<&[u8]> = raw.chunks((W * H * 4) as usize).collect();
    assert_eq!(want.len(), 10, "{name}");
    for i in (0..want.len()).rev().chain(0..want.len()) {
        let f = src.video_frame(cfa4_media::FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
        let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("{name}: RGBA8 frame") };
        assert!(px.as_slice() == want[i], "{name}: frame {i} differs from ffmpeg's");
    }
}

#[test]
fn hap_written_by_ffmpeg_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for format in ["hap", "hap_alpha", "hap_q"] {
        for comp in ["none", "snappy"] {
            let name = format!("hap_{format}_{comp}.mov");
            let Some(path) = fixture(&ff, &name, &["-f", "lavfi", "-i", SRC, "-c:v", "hap", "-format", format, "-compressor", comp, "-chunks", "3"]) else {
                eprintln!("SKIPPED ({name}): this ffmpeg can't write it");
                continue;
            };
            check(&ff, &name, &path);
        }
    }
    let name = "hap_q_snappy.avi";
    if let Some(path) = fixture(&ff, name, &["-f", "lavfi", "-i", SRC, "-c:v", "hap", "-format", "hap_q", "-chunks", "4"]) {
        check(&ff, name, &path);
    }
}

/// A section header (the 4-byte form).
fn sec(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_le_bytes()[..3].to_vec();
    v.push(kind);
    v.extend_from_slice(body);
    v
}

/// A Snappy stream of `total` bytes repeating `unit` (a literal, then 64-byte copies).
fn snappy_repeat(unit: &[u8], total: usize) -> Vec<u8> {
    let mut o = Vec::new();
    let mut n = total;
    while n >= 0x80 {
        o.push((n as u8 & 0x7F) | 0x80);
        n >>= 7;
    }
    o.push(n as u8);
    o.extend_from_slice(&[60 << 2, (unit.len() - 1) as u8]);
    o.extend_from_slice(unit);
    let mut left = total - unit.len();
    while left > 0 {
        let k = left.min(64);
        o.push((((k - 1) as u8) << 2) | 2);
        o.extend_from_slice(&(unit.len() as u16).to_le_bytes());
        left -= k;
    }
    o
}

/// Rewrite every uncompressed frame (8-byte header of type `kind`, `tex` bytes of texture) of
/// `src` with `make`, which must return a frame of the same size, and rename the FourCC.
fn rewrite(src: &Path, out: &str, kind: u8, tex: usize, from: &[u8; 4], to: &[u8; 4], mut make: impl FnMut(&mut Rng) -> Vec<u8>) -> PathBuf {
    let mut d = std::fs::read(src).unwrap();
    let mut hdr = vec![0, 0, 0, kind];
    hdr.extend_from_slice(&(tex as u32).to_le_bytes());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut p, mut n) = (0, 0);
    while let Some(at) = d[p..].windows(8).position(|w| w == hdr.as_slice()) {
        let at = p + at;
        let frame = make(&mut rng);
        assert_eq!(frame.len(), 8 + tex, "{out}: rewritten frame size");
        d[at..at + 8 + tex].copy_from_slice(&frame);
        p = at + 8 + tex;
        n += 1;
    }
    assert_eq!(n, 10, "{out}: frames found");
    let q = d.windows(4).position(|w| w == from).unwrap();
    d[q..q + 4].copy_from_slice(to);
    let path = dir().join(out);
    std::fs::write(&path, d).unwrap();
    path
}

#[test]
fn hap_with_random_and_rewritten_frames_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let blocks = ((W / 4) * (H / 4)) as usize;
    let random = |rng: &mut Rng, n: usize| -> Vec<u8> { (0..n).map(|_| rng.below(256) as u8).collect() };
    let raw_frame = |kind: u8, tex: Vec<u8>| -> Vec<u8> {
        let mut v = vec![0, 0, 0, kind];
        v.extend_from_slice(&(tex.len() as u32).to_le_bytes());
        v.extend(tex);
        v
    };
    let src = |format: &str| {
        let name = format!("hap_{format}_plain.mov");
        fixture(&ff, &name, &["-f", "lavfi", "-i", SRC, "-c:v", "hap", "-format", format, "-compressor", "none"])
    };
    let (Some(hap), Some(alpha), Some(q)) = (src("hap"), src("hap_alpha"), src("hap_q")) else {
        eprintln!("SKIPPED: this ffmpeg can't write Hap");
        return;
    };
    // random blocks: every DXT1 mode (including transparent black), DXT5 alpha modes, YCoCg scales
    let p = rewrite(&hap, "hap_random1.mov", 0xAB, blocks * 8, b"Hap1", b"Hap1", |r| raw_frame(0xAB, random(r, blocks * 8)));
    check(&ff, "hap_random1.mov", &p);
    let p = rewrite(&alpha, "hap_random5.mov", 0xAE, blocks * 16, b"Hap5", b"Hap5", |r| raw_frame(0xAE, random(r, blocks * 16)));
    check(&ff, "hap_random5.mov", &p);
    let p = rewrite(&q, "hap_randomy.mov", 0xAF, blocks * 16, b"HapY", b"HapY", |r| raw_frame(0xAF, random(r, blocks * 16)));
    check(&ff, "hap_randomy.mov", &p);
    // Hap Alpha-Only: random RGTC1 blocks in place of the DXT1 ones (same size)
    let p = rewrite(&hap, "hap_alpha_only.mov", 0xAB, blocks * 8, b"Hap1", b"HapA", |r| raw_frame(0xA1, random(r, blocks * 8)));
    check(&ff, "hap_alpha_only.mov", &p);
    // Hap Q Alpha: Snappy-compressed YCoCg, then chunked RGTC1 alpha padded to the frame size
    let tex = blocks * 16;
    let p = rewrite(&alpha, "hap_q_alpha.mov", 0xAE, tex, b"Hap5", b"HapM", |r| {
        let ycocg = sec(0xBF, &snappy_repeat(&random(r, 256), tex));
        let a = snappy_repeat(&random(r, 200), blocks * 8);
        let mut instr = sec(0x02, &[0x0B]);
        instr.extend(sec(0x03, &(a.len() as u32).to_le_bytes()));
        instr.extend(sec(0x04, &0u32.to_le_bytes()));
        let head = sec(0x01, &instr);
        let pad = 8 + tex - 4 - ycocg.len() - 4 - head.len() - a.len();
        let mut alpha = head;
        alpha.extend(a);
        alpha.extend(vec![0; pad]);
        let mut inner = ycocg;
        inner.extend(sec(0xC1, &alpha));
        sec(0x0D, &inner)
    });
    check(&ff, "hap_q_alpha.mov", &p);
}
