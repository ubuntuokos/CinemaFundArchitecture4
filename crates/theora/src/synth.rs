//! Synthetic Theora streams for tests (and the oracle test in `cfa4-codecs`): the three headers
//! with flat Huffman tables and random quantizers, and frame payloads of random bytes, which parse
//! as valid frames because every coding flag, mode, vector and token is a bit field.
#![allow(clippy::indexing_slicing, clippy::cast_possible_truncation)]

pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x1234_5679) | 1)
    }
    pub fn draw(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.draw() % n
    }
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    n: usize,
}
impl BitWriter {
    fn put(&mut self, bits: u32, v: u64) {
        for i in (0..bits).rev() {
            if self.n.is_multiple_of(8) {
                self.out.push(0);
            }
            if v >> i & 1 != 0
                && let Some(b) = self.out.last_mut()
            {
                *b |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }
}

pub fn ident(w: u32, h: u32, pf: u64) -> Vec<u8> {
    let mut b = BitWriter::default();
    b.put(8, 0x80);
    for c in b"theora" {
        b.put(8, u64::from(*c));
    }
    b.put(8, 3);
    b.put(8, 2);
    b.put(8, 1);
    b.put(16, u64::from(w / 16));
    b.put(16, u64::from(h / 16));
    b.put(24, u64::from(w));
    b.put(24, u64::from(h));
    b.put(8, 0);
    b.put(8, 0);
    b.put(32, 25);
    b.put(32, 1);
    b.put(24, 1);
    b.put(24, 1);
    b.put(8, 1);
    b.put(24, 0);
    b.put(6, 32);
    b.put(5, 6);
    b.put(2, pf);
    b.put(3, 0);
    b.out
}

pub fn comment() -> Vec<u8> {
    let mut v = vec![0x81];
    v.extend_from_slice(b"theora");
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend_from_slice(b"test");
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

/// The tokens every Huffman table of the stream holds, 4 bits each: end-of-block runs, zero runs
/// and coefficients of small magnitude only, so no 16-bit overflow occurs (where ffmpeg's SIMD
/// transforms may differ from its C ones).
const TOKENS: [u64; 16] = [0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 23, 24];

/// A balanced tree of `1 << bits` leaves: the small tokens (`bits` 4) or all 32 (`bits` 5).
fn tree(b: &mut BitWriter, bits: u32, depth: u32, next: &mut usize) {
    if depth == bits {
        b.put(1, 1);
        b.put(5, if bits == 4 { TOKENS[*next] } else { *next as u64 });
        *next += 1;
    } else {
        b.put(1, 0);
        tree(b, bits, depth + 1, next);
        tree(b, bits, depth + 1, next);
    }
}

/// A setup header with small quantizers and small tokens only.
pub fn setup(rng: &mut Rng) -> Vec<u8> {
    setup_with(rng, false)
}

/// A setup header with large quantizers and every token (coefficients overflow 16 bits).
pub fn setup_wide(rng: &mut Rng) -> Vec<u8> {
    setup_with(rng, true)
}

fn setup_with(rng: &mut Rng, wide: bool) -> Vec<u8> {
    let mut b = BitWriter::default();
    b.put(8, 0x82);
    for c in b"theora" {
        b.put(8, u64::from(*c));
    }
    b.put(3, 6);
    for _ in 0..64 {
        b.put(6, rng.below(40));
    }
    for _ in 0..2 {
        b.put(4, 9); // 10-bit scale values
        for _ in 0..64 {
            b.put(10, 4 + rng.below(if wide { 1000 } else { 60 }));
        }
    }
    b.put(9, 2); // three base matrices
    for _ in 0..3 {
        for _ in 0..64 {
            b.put(8, if wide { rng.below(256) } else { 8 + rng.below(40) });
        }
    }
    for inter in 0..2 {
        for plane in 0..3 {
            if inter == 1 || plane > 0 {
                b.put(1, 1);
            }
            b.put(2, rng.below(3));
            b.put(6, 62);
            b.put(2, rng.below(3));
        }
    }
    for _ in 0..80 {
        let mut next = 0;
        tree(&mut b, if wide { 5 } else { 4 }, 0, &mut next);
    }
    b.out
}

pub fn laced(h: [&[u8]; 3]) -> Vec<u8> {
    let mut v = vec![2];
    for part in &h[..2] {
        let mut n = part.len();
        while n >= 255 {
            v.push(255);
            n -= 255;
        }
        v.push(n as u8);
    }
    for part in h {
        v.extend_from_slice(part);
    }
    v
}

pub fn packet(rng: &mut Rng, key: bool) -> Vec<u8> {
    let mut b = BitWriter::default();
    b.put(1, 0);
    b.put(1, u64::from(!key));
    b.put(6, 8 + rng.below(50));
    b.put(1, 0);
    if key {
        b.put(3, 0);
    }
    let mut v = b.out;
    // random bits after the header: a valid frame's every field is a bit field
    let skew = rng.below(3);
    v.extend((0..24_000).map(|_| match skew {
        // biased towards zeros: long end-of-block runs, mostly small coefficients
        0 => (rng.draw() & rng.draw() & rng.draw()) as u8,
        _ => rng.draw() as u8,
    }));
    v
}
