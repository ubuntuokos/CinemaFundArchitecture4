//! ATRAC1: streams of random sound units that the format allows (every block size mode, 20 to
//! 52 coded BFUs, word lengths up to 7 bits, every scale factor), written as MiniDisc `.aea`
//! files and decoded by us and by ffmpeg's `atrac1` decoder (ffmpeg has no ATRAC1 encoder).

use std::process::Command;

use cfa4_realaudio::atrac1::{Decoder, SU_SIZE};

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
    w.out.resize(SU_SIZE, 0);
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

#[test]
fn matches_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("atrac1") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-atrac1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (case, (channels, max_wl)) in [(1usize, 3u64), (2, 3), (1, 7), (2, 7), (2, 14)].into_iter().enumerate() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15 ^ ((case as u64 + 1) * 0x1234_5679));
        let frames = 150;
        let units: Vec<Vec<u8>> = (0..frames * channels).map(|_| unit(&mut r, max_wl)).collect();
        let file = dir.join(format!("c{case}.aea"));
        std::fs::write(&file, aea(channels, &units)).unwrap();
        let mut dec = Decoder::new(channels, SU_SIZE * channels).unwrap();
        let mut ours = Vec::new();
        for f in 0..frames {
            let mut pkt = Vec::new();
            for c in 0..channels {
                pkt.extend_from_slice(&units[f * channels + c]);
            }
            let p = dec.decode(&pkt).unwrap();
            for i in 0..512 {
                for ch in &p {
                    ours.push(ch[i]);
                }
            }
        }
        let out = Command::new(&ff).args(["-v", "error", "-f", "aea", "-i"]).arg(&file).args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]).output().unwrap();
        let theirs: Vec<f32> = out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        assert_eq!(ours.len(), theirs.len(), "case {case}: {}", String::from_utf8_lossy(&out.stderr));
        let peak = ours.iter().fold(0f32, |a, v| a.max(v.abs())).max(1e-9);
        let worst = ours.iter().zip(&theirs).fold(0f32, |a, (x, y)| a.max((x - y).abs()));
        eprintln!("case {case}: {} samples, peak {peak:e}, worst {worst:e}", ours.len());
        assert!(worst <= 1e-5 * peak.max(1e-3), "case {case}: worst {worst:e} of peak {peak:e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn damaged_units_are_errors_not_panics() {
    let mut r = Rng(77);
    let mut d = Decoder::new(2, 424).unwrap();
    for _ in 0..500 {
        let pkt: Vec<u8> = (0..424).map(|_| r.next() as u8).collect();
        let _ = d.decode(&pkt);
    }
    assert!(d.decode(&[0u8; 100]).is_err());
    assert!(Decoder::new(3, 636).is_err());
    assert!(Decoder::new(0, 212).is_err());
    assert!(Decoder::new(1, 0).is_err());
}
