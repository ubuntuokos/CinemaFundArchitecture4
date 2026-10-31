// SPDX-License-Identifier: LGPL-2.1-or-later
// The three-pass distribution of overflowing AC bits follows libdv 1.0.0 (libdv/parse.c,
// Copyright (C) Charles 'Buck' Krasic, Erik Walthinsen - April 2000; LGPL-2.1-or-later, "either
// version 2.1, or (at your option) any later version"), re-implemented for CFA4 with bit pools
// instead of lender marks; see crates/dv/README.md and docs/licensing.md §5.

//! Decoding of one video segment: five compressed macroblocks in five DIF blocks.
//!
//! Each DIF video block holds a status/QNO byte and six DCT blocks (Y0-Y3 in 14-byte areas, Cr
//! and Cb in 10-byte areas). A DCT block starts with the DC value (9 bits, two's complement),
//! the DCT mode bit and the 2-bit class, then the AC codes. Codes that do not fit into their
//! block's area continue in the unused space after the end-of-block of other blocks: first of
//! the same macroblock (pass 2), then of the whole video segment (pass 3), in block and
//! macroblock order (IEC 61834-2 / SMPTE 314M, the "deterministic distribution").

use crate::dif::{BLOCK_SIZE, sct};
use crate::vlc::{Code, Entry};

/// Byte offset of each DCT block's area in a DIF block, and the end of each area.
const AREA_START: [usize; 6] = [4, 18, 32, 46, 60, 70];
const AREA_END: [usize; 6] = [18, 32, 46, 60, 70, 80];

/// One DCT block after VLC decoding.
#[derive(Clone)]
pub(crate) struct Block {
    pub dc: i32,
    pub mode248: bool,
    pub class: u8,
    /// AC amplitudes in zigzag order (index 0 unused).
    pub amps: [i16; 64],
    /// Next zigzag index.
    pos: usize,
    /// End of block reached (or decoding stopped on an error).
    done: bool,
    /// Bits that did not form a whole code yet.
    partial: Vec<u8>,
}

/// One macroblock after VLC decoding.
pub(crate) struct Macroblock {
    pub qno: u8,
    pub blocks: [Block; 6],
    /// The DIF block was missing or of the wrong type: the macroblock is concealed.
    pub concealed: bool,
}

/// Result of a segment: the five macroblocks and the number of coding errors found.
pub(crate) struct Segment {
    pub mbs: Vec<Macroblock>,
    pub errors: u32,
}

/// Bits of one DIF block, one byte (0/1) per bit.
fn bits_of(block: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(block.len() * 8);
    for &b in block {
        for i in (0..8).rev() {
            v.push((b >> i) & 1);
        }
    }
    v
}

fn read(bits: &[u8], from: usize, n: usize) -> u32 {
    bits.iter().skip(from).take(n).fold(0, |acc, &b| (acc << 1) | u32::from(b))
}

enum Outcome {
    /// End of block after this many bits of the stream.
    Eob(usize),
    /// The stream ran out; this many bits were consumed (the rest is a partial code).
    Exhausted(usize),
    /// Invalid code or too many coefficients.
    Error,
}

/// Decode codes into `blk` from the bit stream `a` followed by `b`.
fn run(blk: &mut Block, table: &[Entry], a: &[u8], b: &[u8]) -> Outcome {
    let total = a.len() + b.len();
    let bit = |i: usize| -> u32 { u32::from(if i < a.len() { a.get(i).copied().unwrap_or(0) } else { b.get(i - a.len()).copied().unwrap_or(0) }) };
    let mut pos = 0usize;
    loop {
        let avail = total - pos;
        if avail == 0 {
            return Outcome::Exhausted(pos);
        }
        let mut window = 0u32;
        for i in 0..16 {
            window = (window << 1) | if i < avail { bit(pos + i) } else { 0 };
        }
        let Some(e) = table.get(window as usize) else { return Outcome::Error };
        if e.len == 0 {
            // with zero padding the code may only look invalid
            return if avail >= 16 { Outcome::Error } else { Outcome::Exhausted(pos) };
        }
        let len = usize::from(e.len);
        if len > avail {
            return Outcome::Exhausted(pos);
        }
        pos += len;
        match e.code {
            Code::Eob => return Outcome::Eob(pos),
            Code::Coef { run, amp } => {
                blk.pos += usize::from(run);
                let Some(slot) = blk.amps.get_mut(blk.pos) else { return Outcome::Error };
                *slot = amp;
                blk.pos += 1;
            }
        }
    }
}

/// Continue every unfinished block of `blocks` (in order) from `pool`; returns the unused rest
/// of the pool and counts errors.
fn spill<'a>(blocks: impl Iterator<Item = &'a mut Block>, table: &[Entry], pool: &[u8], errors: &mut u32) -> Vec<u8> {
    let mut at = 0usize;
    for blk in blocks {
        if blk.done {
            continue;
        }
        let rest = pool.get(at..).unwrap_or(&[]);
        let partial = std::mem::take(&mut blk.partial);
        match run(blk, table, &partial, rest) {
            Outcome::Eob(n) => {
                blk.done = true;
                at += n.saturating_sub(partial.len());
            }
            Outcome::Exhausted(n) => {
                let mut left: Vec<u8> = partial.iter().chain(rest).copied().skip(n).collect();
                blk.partial.append(&mut left);
                at = pool.len();
            }
            Outcome::Error => {
                blk.done = true;
                *errors += 1;
                at = pool.len();
            }
        }
    }
    pool.get(at..).unwrap_or(&[]).to_vec()
}

fn empty_block() -> Block {
    Block { dc: 0, mode248: false, class: 0, amps: [0; 64], pos: 1, done: true, partial: Vec::new() }
}

/// Decode the five DIF video blocks of a segment (`data` holds 5 × 80 bytes; a short slice
/// conceals the missing macroblocks).
pub(crate) fn decode(data: &[u8], seq: usize, table: &[Entry]) -> Segment {
    let mut errors = 0u32;
    let mut mbs = Vec::with_capacity(5);
    // pass 1: each block from its own area
    let mut mb_pools: Vec<Vec<u8>> = Vec::with_capacity(5);
    for m in 0..5 {
        let block = data.get(m * BLOCK_SIZE..(m + 1) * BLOCK_SIZE);
        let valid = block.filter(|b| b.first().map(|x| x >> 5) == Some(sct::VIDEO) && b.get(1).map(|x| usize::from(x >> 4)) == Some(seq));
        let Some(block) = valid else {
            errors += 1;
            mbs.push(Macroblock { qno: 0, blocks: std::array::from_fn(|_| empty_block()), concealed: true });
            mb_pools.push(Vec::new());
            continue;
        };
        let bits = bits_of(block);
        let qno = (read(&bits, 24, 8) & 0x0f) as u8;
        let mut pool = Vec::new();
        let blocks: [Block; 6] = std::array::from_fn(|b| {
            let start = AREA_START[b] * 8;
            let end = AREA_END[b] * 8;
            let mut dc = read(&bits, start, 9) as i32;
            if dc > 255 {
                dc -= 512;
            }
            let mode248 = read(&bits, start + 9, 1) == 1;
            let class = read(&bits, start + 10, 2) as u8;
            let mut blk = Block { dc, mode248, class, amps: [0; 64], pos: 1, done: false, partial: Vec::new() };
            let area = bits.get(start + 12..end).unwrap_or(&[]);
            match run(&mut blk, table, area, &[]) {
                Outcome::Eob(n) => {
                    blk.done = true;
                    pool.extend_from_slice(area.get(n..).unwrap_or(&[]));
                }
                Outcome::Exhausted(n) => blk.partial = area.get(n..).unwrap_or(&[]).to_vec(),
                Outcome::Error => {
                    blk.done = true;
                    errors += 1;
                }
            }
            blk
        });
        mbs.push(Macroblock { qno, blocks, concealed: false });
        mb_pools.push(pool);
    }
    // pass 2: within each macroblock
    let mut seg_pool = Vec::new();
    for (mb, pool) in mbs.iter_mut().zip(&mb_pools) {
        let mut rest = spill(mb.blocks.iter_mut(), table, pool, &mut errors);
        seg_pool.append(&mut rest);
    }
    // pass 3: across the video segment
    let _ = spill(mbs.iter_mut().flat_map(|mb| mb.blocks.iter_mut()), table, &seg_pool, &mut errors);
    // blocks that never reached their end of block
    for mb in &mut mbs {
        for blk in &mut mb.blocks {
            if !blk.done {
                errors += 1;
                blk.done = true;
            }
        }
    }
    Segment { mbs, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(s: &str) -> Vec<u8> {
        s.bytes().filter(|b| *b != b' ').map(|b| u8::from(b == b'1')).collect()
    }

    #[test]
    fn codes_split_across_streams() {
        let t = crate::vlc::table();
        let mut blk = empty_block();
        blk.done = false;
        // (0, +1) (1, -1) EOB, split in the middle of the second code
        let all = bits("000 01111 0110");
        let (a, b) = all.split_at(5);
        match run(&mut blk, t, a, &[]) {
            Outcome::Exhausted(n) => assert_eq!(n, 3),
            _ => panic!("expected exhaustion"),
        }
        let partial = a[3..].to_vec();
        match run(&mut blk, t, &partial, b) {
            Outcome::Eob(n) => assert_eq!(n, 2 + b.len()),
            _ => panic!("expected EOB"),
        }
        assert_eq!(&blk.amps[1..4], &[1, 0, -1]);
    }

    #[test]
    fn too_many_coefficients_is_an_error() {
        let t = crate::vlc::table();
        let mut blk = empty_block();
        // run 61 escape (13 bits) twice overflows the block
        let s = bits("1111110 111101 1111110 111101");
        assert!(matches!(run(&mut blk, t, &s, &[]), Outcome::Error));
    }

    #[test]
    fn garbage_never_panics() {
        let t = crate::vlc::table();
        let mut x = 0x1234_5678u32;
        for _ in 0..200 {
            let data: Vec<u8> = (0..400)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let _ = decode(&data, 0, t);
            let _ = decode(&data[..123], 3, t);
        }
    }
}
