//! DVD sub-picture units (SPU, CFA4 #1812, D1/D2): a 2-bit bitmap in run-length coding plus the
//! display control commands. Menu buttons are drawn with it (the button highlight only remaps
//! the four colours through the HLI colour table).
//!
//! Layout as the public DVD notes and FFmpeg's `dvdsubdec.c` (LGPL-2.1+) decode it: `u16` size,
//! `u16` offset of the command sequence, the two run-length fields (even lines, then odd lines),
//! then the command sequence `date, next sequence offset, commands…, 0xFF`. A run of `n` pixels
//! of colour `c` is `n << 2 | c` in 4 (n 1..3), 8 (4..15), 12 (16..63) or 16 bits (64..255); the
//! last run of a line is `0000 00cc` ("rest of the line"), and every line ends on a byte boundary.

use crate::{Error, Result};

/// Most pixels of one SPU row we encode.
const MAX_W: usize = 1023;

struct Nibbles(Vec<u8>, bool);

impl Nibbles {
    fn new() -> Self {
        Self(Vec::new(), false)
    }
    fn push(&mut self, n: u8) {
        if self.1 {
            if let Some(last) = self.0.last_mut() {
                *last |= n & 0xF;
            }
        } else {
            self.0.push((n & 0xF) << 4);
        }
        self.1 = !self.1;
    }
    /// Pad to a byte boundary.
    fn align(&mut self) {
        self.1 = false;
    }
    fn run(&mut self, len: usize, color: u8) {
        let code = (len as u32) << 2 | u32::from(color & 3);
        let nibbles = match len {
            1..=3 => 1,
            4..=15 => 2,
            16..=63 => 3,
            _ => 4,
        };
        for k in (0..nibbles).rev() {
            self.push((code >> (4 * k)) as u8);
        }
    }
    fn fill_rest(&mut self, color: u8) {
        for k in [0, 0, 0, color & 3] {
            self.push(k);
        }
    }
}

/// Run-length code the lines `first, first + 2, …` of the bitmap.
fn field(bitmap: &[u8], w: usize, h: usize, first: usize) -> Vec<u8> {
    let mut out = Nibbles::new();
    for y in (first..h).step_by(2) {
        let row = &bitmap[y * w..(y + 1) * w];
        let mut x = 0;
        while x < w {
            let c = row[x] & 3;
            let mut n = 1;
            while x + n < w && row[x + n] & 3 == c {
                n += 1;
            }
            x += n;
            if x >= w {
                out.fill_rest(c);
            } else {
                // runs of 255 and more are cut: a run code holds at most 255 pixels
                let mut left = n;
                while left > 0 {
                    let take = left.min(255);
                    out.run(take, c);
                    left -= take;
                }
            }
        }
        out.align();
    }
    out.0
}

/// A sub-picture: the bitmap (`w * h` values 0..=3, row by row), its top-left position on the
/// 720-wide picture and the palette entries and contrasts of the four colours.
pub(crate) struct Spu<'a> {
    pub bitmap: &'a [u8],
    pub w: usize,
    pub h: usize,
    pub x: u16,
    pub y: u16,
    /// Palette index (0..15, into the PGC colour table) of colours 0..3.
    pub color: [u8; 4],
    /// Contrast (0 transparent .. 15 opaque) of colours 0..3.
    pub alpha: [u8; 4],
}

/// Build a menu SPU (start-display "menu" command, a single sequence at date 0).
pub(crate) fn encode_menu(s: &Spu<'_>) -> Result<Vec<u8>> {
    if s.w == 0 || s.h < 2 || s.w > MAX_W || s.w.checked_mul(s.h) != Some(s.bitmap.len()) {
        return Err(Error::Vob("sub-picture: bad bitmap size".into()));
    }
    let top = field(s.bitmap, s.w, s.h, 0);
    let bottom = field(s.bitmap, s.w, s.h, 1);
    let (x1, x2) = (usize::from(s.x), usize::from(s.x) + s.w - 1);
    let (y1, y2) = (usize::from(s.y), usize::from(s.y) + s.h - 1);
    if x2 > 0xFFF || y2 > 0xFFF {
        return Err(Error::Vob("sub-picture: outside the picture".into()));
    }
    let cmd_at = 4 + top.len() + bottom.len();
    let mut cmds = vec![0u8; 4]; // date 0, next sequence = itself (patched below)
    cmds.push(0x00); // forced/menu display
    cmds.push(0x03);
    cmds.push(s.color[3] << 4 | s.color[2] & 0xF);
    cmds.push(s.color[1] << 4 | s.color[0] & 0xF);
    cmds.push(0x04);
    cmds.push(s.alpha[3] << 4 | s.alpha[2] & 0xF);
    cmds.push(s.alpha[1] << 4 | s.alpha[0] & 0xF);
    cmds.push(0x05);
    cmds.extend_from_slice(&[(x1 >> 4) as u8, ((x1 & 0xF) << 4 | x2 >> 8) as u8, x2 as u8]);
    cmds.extend_from_slice(&[(y1 >> 4) as u8, ((y1 & 0xF) << 4 | y2 >> 8) as u8, y2 as u8]);
    cmds.push(0x06);
    cmds.extend_from_slice(&4u16.to_be_bytes());
    cmds.extend_from_slice(&((4 + top.len()) as u16).to_be_bytes());
    cmds.push(0xFF);
    let total = cmd_at + cmds.len();
    if total > 0xFFFF {
        return Err(Error::Vob("sub-picture too large".into()));
    }
    cmds[2..4].copy_from_slice(&(cmd_at as u16).to_be_bytes());
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u16).to_be_bytes());
    out.extend_from_slice(&(cmd_at as u16).to_be_bytes());
    out.extend_from_slice(&top);
    out.extend_from_slice(&bottom);
    out.extend_from_slice(&cmds);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FFmpeg's `decode_run_2bit` / `decode_rle` for the 2-bit case, as a reference decoder.
    fn decode(spu: &[u8]) -> (Vec<u8>, usize, usize) {
        let be16 = |at: usize| usize::from(u16::from_be_bytes([spu[at], spu[at + 1]]));
        assert_eq!(be16(0), spu.len(), "size field");
        let mut pos = be16(2) + 4;
        let (mut x1, mut x2, mut y1, mut y2) = (0, 0, 0, 0);
        let (mut o1, mut o2) = (0, 0);
        loop {
            let cmd = spu[pos];
            pos += 1;
            match cmd {
                0x00..=0x02 => {}
                0x03 | 0x04 => pos += 2,
                0x05 => {
                    x1 = usize::from(spu[pos]) << 4 | usize::from(spu[pos + 1]) >> 4;
                    x2 = (usize::from(spu[pos + 1]) & 0xF) << 8 | usize::from(spu[pos + 2]);
                    y1 = usize::from(spu[pos + 3]) << 4 | usize::from(spu[pos + 4]) >> 4;
                    y2 = (usize::from(spu[pos + 4]) & 0xF) << 8 | usize::from(spu[pos + 5]);
                    pos += 6;
                }
                0x06 => {
                    o1 = be16(pos);
                    o2 = be16(pos + 2);
                    pos += 4;
                }
                0xFF => break,
                other => panic!("unknown command {other:#x}"),
            }
        }
        let (w, h) = (x2 - x1 + 1, y2 - y1 + 1);
        let mut bitmap = vec![0u8; w * h];
        for (start, first, lines) in [(o1, 0, h.div_ceil(2)), (o2, 1, h / 2)] {
            let bit = std::cell::Cell::new(start * 8);
            let nib = || {
                let b = bit.get();
                let v = (spu[b / 8] >> (4 - b % 8)) & 0xF;
                bit.set(b + 4);
                u32::from(v)
            };
            for l in 0..lines {
                let y = first + 2 * l;
                let mut x = 0;
                while x < w {
                    let (mut v, mut t) = (0u32, 1u32);
                    while v < t && t <= 0x40 {
                        v = v << 4 | nib();
                        t <<= 2;
                    }
                    let color = (v & 3) as u8;
                    let len = if v < 4 { w - x } else { (v >> 2) as usize };
                    assert!(len <= w - x, "run too long");
                    bitmap[y * w + x..y * w + x + len].fill(color);
                    x += len;
                }
                // byte align
                bit.set(bit.get().div_ceil(8) * 8);
            }
        }
        (bitmap, w, h)
    }

    #[test]
    fn a_tiny_bitmap_has_the_expected_bytes() {
        // line 0: 2 x colour 0, rest colour 1; line 1: all colour 2
        let bitmap = [0, 0, 1, 1, 2, 2, 2, 2];
        let spu = encode_menu(&Spu { bitmap: &bitmap, w: 4, h: 2, x: 0, y: 0, color: [0, 1, 2, 3], alpha: [0, 15, 15, 15] }).expect("encode");
        // field 1: run 2 c0 (0x8), fill c1 (0 0 0 1), pad -> 80 00 10; field 2: fill c2 -> 00 02
        assert_eq!(&spu[4..9], &[0x80, 0x00, 0x10, 0x00, 0x02]);
        assert_eq!(&spu[0..2], &(spu.len() as u16).to_be_bytes());
        let (bm, w, h) = decode(&spu);
        assert_eq!((w, h), (4, 2));
        assert_eq!(bm, bitmap);
    }

    #[test]
    fn every_run_length_round_trips() {
        // runs of 1, 3, 4, 15, 16, 63, 64, 255, 300 in alternating colours, then a closing run
        let lens = [1usize, 3, 4, 15, 16, 63, 64, 255, 300];
        let w: usize = lens.iter().sum::<usize>() + 5;
        let mut row = Vec::new();
        for (i, l) in lens.iter().enumerate() {
            row.extend(std::iter::repeat_n((i % 3) as u8 + 1, *l));
        }
        row.extend(std::iter::repeat_n(0u8, 5));
        let bitmap: Vec<u8> = row.iter().chain(row.iter().rev()).copied().collect();
        let spu = encode_menu(&Spu { bitmap: &bitmap, w, h: 2, x: 100, y: 200, color: [4, 5, 6, 7], alpha: [0, 15, 15, 15] }).expect("encode");
        let (bm, dw, dh) = decode(&spu);
        assert_eq!((dw, dh), (w, 2));
        assert_eq!(bm, bitmap);
    }

    #[test]
    fn odd_line_counts_and_position() {
        let (w, h) = (9usize, 5usize);
        let bitmap: Vec<u8> = (0..w * h).map(|i| ((i * 7 + i / w) % 4) as u8).collect();
        let spu = encode_menu(&Spu { bitmap: &bitmap, w, h, x: 0x123, y: 0x0F0, color: [1, 2, 3, 4], alpha: [0, 8, 15, 15] }).expect("encode");
        let (bm, dw, dh) = decode(&spu);
        assert_eq!((dw, dh), (w, h));
        assert_eq!(bm, bitmap);
        // colours and contrasts are nibble-packed 3 2 | 1 0
        let cmd = u16::from_be_bytes([spu[2], spu[3]]) as usize;
        assert_eq!(&spu[cmd + 4..cmd + 14], &[0x00, 0x03, 0x43, 0x21, 0x04, 0xFF, 0x80, 0x05, 0x12, 0x31]);
    }

    #[test]
    fn bad_input_is_refused() {
        let b = [0u8; 4];
        assert!(encode_menu(&Spu { bitmap: &b, w: 4, h: 1, x: 0, y: 0, color: [0; 4], alpha: [0; 4] }).is_err());
        assert!(encode_menu(&Spu { bitmap: &b, w: 3, h: 2, x: 0, y: 0, color: [0; 4], alpha: [0; 4] }).is_err());
    }
}
