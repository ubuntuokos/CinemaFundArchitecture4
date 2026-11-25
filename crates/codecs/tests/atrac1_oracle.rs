//! ATRAC1 in `.aea` files and Matroska (`A_ATRAC/AT1`), read through the media sources, against
//! ffmpeg's decode of the same units (ffmpeg has no ATRAC1 encoder and its Matroska muxer does not
//! know the codec id, so the Matroska file is written here): within 10^-5 of the peak, read front to
//! back and from the middle (a seek).

mod common;

use common::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

struct BitWriter {
    out: Vec<u8>,
    n: usize,
}
impl BitWriter {
    fn put(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            if self.n.is_multiple_of(8) {
                self.out.push(0);
            }
            if v >> i & 1 != 0 {
                let last = self.out.len() - 1;
                self.out[last] |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }
}

const TAB1: [usize; 8] = [20, 28, 32, 36, 40, 44, 48, 52];
const TAB2: [usize; 4] = [0, 112, 176, 208];
const TAB3: [usize; 8] = [0, 24, 36, 48, 72, 108, 132, 156];
const SPECS: [usize; 52] = [
    8, 8, 8, 8, 4, 4, 4, 4, 8, 8, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 9, 9, 9, 9, 10, 10, 10, 10, 12, 12, 12, 12, 12, 12, 12, 12, 20, 20, 20,
    20, 20, 20, 20, 20,
];

/// One random sound unit of 212 bytes that decodes.
fn unit(r: &mut Rng, max_wl: u64) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::new(), n: 0 };
    w.put([0, 2][r.below(2) as usize], 2);
    w.put([0, 2][r.below(2) as usize], 2);
    w.put([0, 3][r.below(2) as usize], 2);
    w.put(r.below(4) as u32, 2);
    let (i1, i2, i3) = (r.below(8) as usize, r.below(4) as usize, r.below(8) as usize);
    w.put(i1 as u32, 3);
    w.put(i2 as u32, 2);
    w.put(i3 as u32, 3);
    let nb = TAB1[i1];
    let mut bits = nb * 10 + 32 + TAB2[i2] + (TAB3[i3] << 1);
    let mut idwl = vec![0u32; 52];
    for (b, v) in idwl.iter_mut().enumerate().take(nb) {
        let mut x = if r.below(4) == 0 { 0 } else { r.below(max_wl) as u32 + 1 };
        let wl = if x == 0 { 0 } else { x as usize + 1 };
        if bits + wl * SPECS[b] > 1696 {
            x = 0;
        } else {
            bits += wl * SPECS[b];
        }
        *v = x;
    }
    for v in idwl.iter().take(nb) {
        w.put(*v, 4);
    }
    for _ in 0..nb {
        w.put(r.below(64) as u32, 6);
    }
    for (b, &v) in idwl.iter().enumerate() {
        if v != 0 {
            for _ in 0..SPECS[b] {
                w.put(r.below(1 << (v + 1)) as u32, v + 1);
            }
        }
    }
    w.out.resize(212, 0);
    w.out
}

fn aea(channels: usize, units: &[Vec<u8>]) -> Vec<u8> {
    let mut f = vec![0u8; 2048];
    f[..4].copy_from_slice(&0x800u32.to_le_bytes());
    f[4..9].copy_from_slice(b"test\0");
    f[264] = channels as u8;
    for u in units {
        f.extend_from_slice(u);
    }
    f
}

fn id(v: u32) -> Vec<u8> {
    let b = v.to_be_bytes();
    let skip = b.iter().take_while(|&&x| x == 0).count();
    b[skip..].to_vec()
}

fn el(i: u32, body: &[u8]) -> Vec<u8> {
    let mut o = id(i);
    o.push(0x01);
    o.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    o.extend_from_slice(body);
    o
}

fn uint(i: u32, v: u64) -> Vec<u8> {
    el(i, &v.to_be_bytes())
}

/// A Matroska file with `A_ATRAC/AT1` packets of 512 samples at 44.1 kHz, 11.6 ms apart.
fn mkv(channels: usize, packets: &[Vec<u8>]) -> Vec<u8> {
    let ebml = el(
        0x1A45_DFA3,
        &[uint(0x4286, 1), uint(0x42F7, 1), uint(0x42F2, 4), uint(0x42F3, 8), el(0x4282, b"matroska"), uint(0x4287, 4), uint(0x4285, 2)].concat(),
    );
    let info = el(0x1549_A966, &uint(0x2A_D7B1, 1_000_000));
    let audio = el(0xE1, &[el(0xB5, &44_100f64.to_be_bytes()), uint(0x9F, channels as u64)].concat());
    let track = el(0xAE, &[uint(0xD7, 1), uint(0x73C5, 1), uint(0x83, 2), el(0x86, b"A_ATRAC/AT1"), audio].concat());
    let tracks = el(0x1654_AE6B, &track);
    let mut cluster = uint(0xE7, 0);
    for (i, p) in packets.iter().enumerate() {
        let ms = (i as u64 * 512 * 1000 / 44_100) as u16;
        let mut b = vec![0x81];
        b.extend_from_slice(&ms.to_be_bytes());
        b.push(0x80);
        b.extend_from_slice(p);
        cluster.extend(el(0xA3, &b));
    }
    [ebml, el(0x1853_8067, &[info, tracks, el(0x1F43_B675, &cluster)].concat())].concat()
}

fn check(name: &str, channels: usize, max_wl: u64, as_mkv: bool) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut r = Rng(0xABCD_1234 ^ (channels as u64 * 977 + max_wl));
    let frames = 120;
    let units: Vec<Vec<u8>> = (0..frames * channels).map(|_| unit(&mut r, max_wl)).collect();
    let packets: Vec<Vec<u8>> = units.chunks(channels).map(|c| c.concat()).collect();
    let dir = dir();
    std::fs::create_dir_all(&dir).unwrap();
    let aea_path = dir.join(format!("atrac1_{name}.aea"));
    std::fs::write(&aea_path, aea(channels, &units)).unwrap();
    let theirs = ffmpeg_audio_f32(&ff, &aea_path, &["-f", "aea"]);
    let (file, data) =
        if as_mkv { (format!("atrac1_{name}.mka"), mkv(channels, &packets)) } else { (format!("atrac1_{name}.aea"), std::fs::read(&aea_path).unwrap()) };
    let s = cfa4_codecs::open_bytes(&file, data.into()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    assert_eq!((info.sample_rate, info.channels as usize), (44_100, channels), "{name}");
    assert_eq!(info.codec, "ATRAC1", "{name}: codec label");
    let read = |from: usize, n: usize| -> Vec<f32> {
        let mut out = Vec::with_capacity(n * channels);
        let mut at = from;
        while at < from + n {
            let k = 1000.min(from + n - at);
            let b = s.audio(at as i64, k, 44_100).unwrap_or_else(|e| panic!("{name}: {e}"));
            for i in 0..k {
                out.extend(b.channels.iter().map(|p| p[i]));
            }
            at += k;
        }
        out
    };
    let total = theirs.len() / channels;
    assert_eq!(total, frames * 512, "{name}: ffmpeg length");
    let peak = theirs.iter().fold(0f32, |a, v| a.max(v.abs())).max(1e-3);
    let close = |a: &[f32], b: &[f32], what: &str| {
        assert_eq!(a.len(), b.len(), "{name}: {what} length");
        let worst = a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(worst <= 1e-5 * peak, "{name}: {what}: worst {worst:e} of peak {peak:e}");
    };
    close(&read(0, total), &theirs, "whole file");
    // jumps: backwards and into the middle, with the overlap and filter memories rebuilt
    let mid = total / 2 + 123;
    close(&read(mid, 3000), &theirs[mid * channels..(mid + 3000) * channels], "from the middle");
    close(&read(700, 2000), &theirs[700 * channels..2700 * channels], "backwards");
}

#[test]
fn atrac1_in_aea_matches_ffmpeg() {
    check("mono", 1, 5, false);
    check("stereo", 2, 7, false);
}

#[test]
fn atrac1_in_matroska_matches_ffmpeg() {
    check("mono_mka", 1, 5, true);
    check("stereo_mka", 2, 7, true);
}
