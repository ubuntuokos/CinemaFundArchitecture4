//! Interplay ACM streams from the test's own writer against ffmpeg's `interplayacm` decoder
//! (the only ACM oracle: ffmpeg has no ACM encoder).
//!
//! The writer draws every syntax element at random within the bitstream's rules: the amplitude
//! table (its size and step), the filler of each column (all 21 valid kinds, every symbol), and
//! block shapes from 1 to 32768 columns. Damaged streams must not panic.

use cfa4_acm::{Decoder, decode_file, parse_header};
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
    fn bit(&mut self) -> u64 {
        self.next() >> 63
    }
}

/// Little-endian bit writer (the first bit written is the lowest of a value).
#[derive(Default)]
struct Bw {
    bytes: Vec<u8>,
    nbits: usize,
}

impl Bw {
    fn put(&mut self, v: u64, n: u32) {
        for i in 0..n {
            if self.nbits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << (self.nbits % 8);
            }
            self.nbits += 1;
        }
    }
}

const KINDS: [u32; 21] = [0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22];
const MORE: [u32; 6] = [23, 24, 26, 27, 29, 3];

/// One column of `rows` values with filler `ind`, every symbol random.
fn column(w: &mut Bw, ind: u32, rows: usize, rng: &mut Rng) {
    let mut i = 0;
    match ind {
        0 => {}
        3..=16 => (0..rows).for_each(|_| w.put(rng.below(1 << ind), ind)),
        // 17 (k13), 20 (k24), 23 (k35), 26 (k45): zero pairs, a zero, or a symbol
        17 | 20 | 23 | 26 => {
            while i < rows {
                let b = rng.bit();
                w.put(b, 1);
                if b == 0 {
                    i += 2;
                    continue;
                }
                let b = rng.bit();
                w.put(b, 1);
                if b == 1 {
                    match ind {
                        17 => w.put(rng.bit(), 1),
                        20 => w.put(rng.below(4), 2),
                        23 => {
                            let c = rng.bit();
                            w.put(c, 1);
                            if c == 0 { w.put(rng.bit(), 1) } else { w.put(rng.below(4), 2) }
                        }
                        _ => w.put(rng.below(8), 3),
                    }
                }
                i += 1;
            }
        }
        18 => (0..rows).for_each(|_| {
            let b = rng.bit();
            w.put(b, 1);
            if b == 1 {
                w.put(rng.bit(), 1);
            }
        }),
        21 => (0..rows).for_each(|_| {
            let b = rng.bit();
            w.put(b, 1);
            if b == 1 {
                w.put(rng.below(4), 2);
            }
        }),
        24 => (0..rows).for_each(|_| {
            let b = rng.bit();
            w.put(b, 1);
            if b == 1 {
                let c = rng.bit();
                w.put(c, 1);
                if c == 0 { w.put(rng.bit(), 1) } else { w.put(rng.below(4), 2) }
            }
        }),
        27 => (0..rows).for_each(|_| {
            let b = rng.bit();
            w.put(b, 1);
            if b == 1 {
                w.put(rng.below(8), 3);
            }
        }),
        19 => (0..rows.div_ceil(3)).for_each(|_| w.put(rng.below(27), 5)),
        22 => (0..rows.div_ceil(3)).for_each(|_| w.put(rng.below(125), 7)),
        29 => (0..rows.div_ceil(2)).for_each(|_| w.put(rng.below(121), 7)),
        _ => unreachable!("filler {ind}"),
    }
}

/// One block. ffmpeg decodes a block from exactly `rows << level` bytes, so a block must fit in
/// them: columns are drawn again (cheaper ones) until it does.
fn block(w: &mut Bw, level: u32, rows: usize, rng: &mut Rng) {
    let budget = 8 * (rows << level) - 8;
    for attempt in 0..40 {
        let mut t = Bw::default();
        t.put(rng.below(16), 4);
        // mostly small steps, sometimes the whole 16 bits
        let val = if rng.bit() == 0 { rng.below(64) } else { rng.below(1 << 16) };
        t.put(val, 16);
        for _ in 0..1usize << level {
            let ind = if attempt >= 39 {
                0
            } else if rng.below(4) == 0 {
                MORE[rng.below(MORE.len() as u64) as usize]
            } else {
                KINDS[rng.below(KINDS.len() as u64) as usize]
            };
            t.put(u64::from(ind), 5);
            column(&mut t, ind, rows, rng);
        }
        if t.nbits <= budget {
            for i in 0..t.nbits {
                w.put(u64::from((t.bytes[i / 8] >> (i % 8)) & 1), 1);
            }
            return;
        }
    }
    unreachable!("a block of zero columns fits");
}

fn file(channels: u16, rate: u16, level: u32, rows: usize, blocks: usize, frames: Option<u32>, rng: &mut Rng) -> Vec<u8> {
    let per_block = (rows << level) as u32;
    let total = frames.map_or(per_block * blocks as u32, |f| f * u32::from(channels));
    let mut out = vec![0x97, 0x28, 0x03, 0x01];
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(level as u16 | (rows as u16) << 4).to_le_bytes());
    let mut w = Bw::default();
    for _ in 0..blocks {
        block(&mut w, level, rows, rng);
    }
    out.extend_from_slice(&w.bytes);
    out
}

fn ffmpeg_decode(ff: &Path, data: &[u8], tag: &str) -> Option<Vec<i16>> {
    let dir = std::env::temp_dir().join(format!("cfa4-acm-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.acm");
    std::fs::write(&input, data).ok()?;
    let out = Command::new(ff).args(["-v", "error", "-f", "acm", "-i"]).arg(&input).args(["-f", "s16le", "-c:a", "pcm_s16le", "-"]).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    out.status.success().then(|| out.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

/// (channels, level, rows, blocks, samples per channel when it is not the whole data)
const CASES: [(u16, u32, usize, usize, Option<u32>); 10] = [
    (1, 0, 40, 5, None),
    (2, 1, 30, 6, None),
    (2, 2, 16, 8, None),
    (1, 4, 6, 6, None),
    (2, 6, 8, 5, None),
    (1, 8, 4, 4, None),
    (2, 8, 8, 3, Some(1000)),
    (2, 9, 4, 3, None),
    (1, 10, 3, 2, None),
    (4, 5, 12, 4, Some(300)),
];

#[test]
fn random_syntax_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("acm oracle") else { return };
    for (i, &(ch, level, rows, blocks, frames)) in CASES.iter().enumerate() {
        for round in 0..6u64 {
            let tag = format!("c{i}-r{round}");
            let mut rng = Rng(0xAC31_0000 + i as u64 * 1000 + round);
            let data = file(ch, 22050, level, rows, blocks, frames, &mut rng);
            let (h, ours) = decode_file(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
            assert_eq!(ours.len() as u64, h.frames * u64::from(ch), "{tag}");
            let theirs = ffmpeg_decode(&ff, &data, &tag).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
            assert_eq!(ours.len(), theirs.len(), "{tag}");
            if let Some(f) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
                let n = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
                panic!("{tag}: first difference at {f}, {n} differ ({} vs {})", ours[f], theirs[f]);
            }
        }
    }
}

#[test]
fn bytes_in_any_pieces_decode_the_same() {
    let mut rng = Rng(0xAC31_9999);
    let data = file(2, 22050, 6, 8, 5, None, &mut rng);
    let (h, whole) = decode_file(&data).unwrap();
    let mut dec = Decoder::new(&h);
    let mut out = Vec::new();
    for piece in data[14..].chunks(37) {
        dec.push(piece, &mut out).unwrap();
    }
    dec.finish(&mut out).unwrap();
    assert_eq!(out, whole);
}

#[test]
fn headers() {
    assert!(parse_header(b"RIFF").is_err());
    assert!(parse_header(&[0x97, 0x28, 0x03, 0x01, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]).is_err()); // no channels
    assert!(parse_header(&[0x97, 0x28, 0x03, 0x01, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0]).is_err()); // no rows
    assert!(parse_header(&[0x97, 0x28, 0x03, 0x01, 0, 0, 0, 0, 1, 0, 1, 0, 0xFF, 0xFF]).is_err()); // 4095 rows × 32768 columns
}

#[test]
fn damaged_streams_do_not_panic() {
    let mut rng = Rng(0xAC31_BEEF);
    for round in 0..300 {
        let level = (round % 9) as u32;
        let mut data = file(1 + (round % 2) as u16, 8000, level, 12 + (round % 5) as usize, 3, None, &mut rng);
        for _ in 0..1 + rng.below(8) {
            let at = 14 + rng.below((data.len() - 14) as u64) as usize;
            data[at] ^= 1 << rng.below(8);
        }
        if round % 7 == 0 {
            data.truncate(14 + rng.below((data.len() - 14) as u64) as usize);
        }
        let _ = decode_file(&data);
    }
}

