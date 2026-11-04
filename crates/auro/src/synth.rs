// SPDX-License-Identifier: Apache-2.0
// Adapted for CFA4 from the block writer in the tests of the `auro` crate 0.9.0 of
// harletty-bridge (https://github.com/harletty/harletty-bridge, commit 74cf6ed, Apache-2.0:
// crates/auro/LICENSE.harletty-bridge; crates/auro/tests/synthetic.rs here). Test support only,
// behind the `test-util` feature.
//
//! Synthetic Auro-Codec carriers for tests: blocks written with the reader's own bit map, so
//! detection and the plumbing above it can be exercised without a real carrier. The blocks fold
//! one stream each (the carrier's own), so a decode gives the carrier back with its borrowed bits
//! cleared, and silence on the heights.

use crate::block::{SYNC_SAMPLES, bit_location, block_crc};

/// Deterministic 24-bit "audio": a linear congruential generator.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.0 >> 8) as i32) << 8 >> 8
    }
}

fn set_bit(samples: &mut [i32], idx: usize, bit: u32, value: u32) {
    let Some(s) = samples.get_mut(idx) else { return };
    let mask = 1i32 << bit;
    if value & 1 != 0 {
        *s |= mask;
    } else {
        *s &= !mask;
    }
}

struct Writer<'a> {
    samples: &'a mut [i32],
    m: usize,
    pos: usize,
}

impl Writer<'_> {
    fn put(&mut self, bits: u32, value: u32) {
        for i in (0..bits).rev() {
            let (idx, b) = bit_location(self.pos, self.m, true);
            set_bit(self.samples, idx, b, (value >> i) & 1);
            self.pos += 1;
        }
    }
}

/// One block of `block_size` samples (16 < size ≤ 4096, a multiple of 16, or 1000) borrowing `m`
/// bits (3..=14) of noise at `level` (0.0..=1.0 of full scale), announcing channel-input
/// configuration `config` and folding the single stream `stream`.
pub fn block(seed: u32, block_size: usize, m: usize, level: f32, config: u8, stream: u8) -> Vec<i32> {
    let mut lcg = Lcg(seed);
    let mut s: Vec<i32> = (0..block_size).map(|_| ((lcg.next() as f32) * level) as i32).collect();
    for v in s.iter_mut().take(SYNC_SAMPLES) {
        *v &= !0b111;
        *v |= 1;
    }
    let size_code = if block_size == 1000 { 0x3Du32 } else { (block_size.saturating_sub(SYNC_SAMPLES) >> 4) as u32 };
    for i in 0..8 {
        set_bit(&mut s, i, 2, (size_code >> (7 - i)) & 1);
    }
    let width_code = 14u32.saturating_sub(m as u32);
    for i in 12..16 {
        set_bit(&mut s, i, 2, (width_code >> (15 - i)) & 1);
    }
    let mut w = Writer { samples: &mut s, m, pos: 48 };
    // The stream layer's view of the fixed fields: header 0x010A, then the word that says one
    // ADOL block, a 14-entry codebook of 5-bit entries and Rice parameter 0.
    w.put(8, 1);
    w.put(8, 10);
    w.put(8, 0);
    w.put(8, 3);
    w.put(8, 1);
    w.put(8, 5);
    w.put(8, u32::from(stream));
    w.put(8, 0xFF);
    w.put(8, 0xFF);
    w.put(8, 0xFF);
    for _ in 0..4 {
        w.put(8, 0);
    }
    w.put(8, 1);
    w.put(8, 0x1E);
    w.put(8, u32::from(config));
    w.put(8, 0);
    let mut pos = 17 * m - 1;
    while pos < m * block_size {
        let (idx, b) = bit_location(pos, m, false);
        set_bit(&mut s, idx, b, 0);
        pos += SYNC_SAMPLES * m;
    }
    let crc = block_crc(&s);
    for i in 0..16 {
        set_bit(&mut s, i, 1, u32::from((crc >> (15 - i)) & 1));
    }
    s
}

/// A carrier channel of `blocks` blocks of 1024 samples (`m` = 4) folding `stream` under
/// `config`.
pub fn channel(seed: u32, blocks: usize, level: f32, config: u8, stream: u8) -> Vec<i32> {
    (0..blocks).flat_map(|b| block(seed.wrapping_mul(7919).wrapping_add(b as u32), 1024, 4, level, config, stream)).collect()
}
