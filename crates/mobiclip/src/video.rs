// Ported from FFmpeg n7.1.1 libavcodec/mobiclip.c, LGPL-2.1-or-later, Copyright (c) 2022
// Paul B Mahol.

//! The MobiClip video decoder: 16 × 16 macroblocks, intra prediction (nine modes, 4 × 4 and
//! 8 × 8 blocks), a small integer DCT and half-pel motion from the last five pictures.

use crate::tables::*;
use crate::{Error, Result};

/// The JPEG zigzag, `ff_zigzag_direct`.
const ZIGZAG8X8: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const RL_BITS: u32 = 12;
const MV_BITS_LEN: u32 = 6;

/// A prefix code table whose codes `ff_vlc_init_from_lengths` assigns in the order listed.
struct Vlc {
    bits: u32,
    /// `(symbol, length)` per `bits`-bit prefix; length 0 marks a prefix no code starts with.
    table: Vec<(u16, u8)>,
}

impl Vlc {
    fn new(bits: u32, lens: &[u8], syms: impl Fn(usize) -> u16) -> Self {
        let mut table = vec![(0u16, 0u8); 1 << bits];
        let mut code: u64 = 0;
        for (i, &len) in lens.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let len = u32::from(len);
            let first = (code >> (32 - len)) as usize;
            let span = 1usize << (bits - len);
            for e in &mut table[first << (bits - len)..(first << (bits - len)) + span] {
                *e = (syms(i), len as u8);
            }
            code += 1u64 << (32 - len);
        }
        Vlc { bits, table }
    }
}

struct Tables {
    rl: [Vlc; 2],
    mv: Vec<Vec<Vlc>>,
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| Tables {
        rl: [Vlc::new(RL_BITS, &BITS0, |i| SYMS0[i]), Vlc::new(RL_BITS, &BITS0, |i| SYMS1[i])],
        mv: (0..2)
            .map(|i| (0..16).map(|j| Vlc::new(MV_BITS_LEN, &MV_BITS[i][j][..usize::from(MV_LEN[j])], |k| u16::from(MV_SYMS[i][j][k]))).collect())
            .collect(),
    })
}

#[derive(Clone)]
struct Bits {
    data: Vec<u8>,
    pos: usize,
}

impl Bits {
    fn left(&self) -> i64 {
        self.data.len() as i64 * 8 - self.pos as i64
    }
    /// The next `n` (at most 25) bits, zeros past the end.
    fn show(&self, n: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut v = 0u32;
        for i in 0..4 {
            v = (v << 8) | u32::from(self.data.get(byte + i).copied().unwrap_or(0));
        }
        (v << (self.pos & 7)) >> (32 - n)
    }
    fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.show(n);
        self.pos += n as usize;
        v
    }
    fn bit(&mut self) -> bool {
        self.get(1) == 1
    }
    fn sbits(&mut self, n: u32) -> i32 {
        ((self.get(n) << (32 - n)) as i32) >> (32 - n)
    }
    fn vlc(&mut self, v: &Vlc) -> Result<u16> {
        let (sym, len) = v.table[self.show(v.bits) as usize];
        if len == 0 {
            return Err(Error::Invalid("bad prefix code"));
        }
        self.pos += usize::from(len);
        Ok(sym)
    }
    /// `get_ue_golomb_long`: up to 32 leading zeros.
    fn ue(&mut self) -> u32 {
        let mut zeros = 0u32;
        while zeros < 32 && self.get(1) == 0 {
            zeros += 1;
            if self.left() < -64 {
                break;
            }
        }
        if zeros == 0 {
            return 0;
        }
        let rest = if zeros >= 25 { (self.get(zeros - 16) << 16) | self.get(16) } else { self.get(zeros) };
        ((1u64 << zeros) - 1 + u64::from(rest)) as u32
    }
    /// `get_ue_golomb_31` (a code of at most 31 leading zeros; more are cut at 31).
    fn ue31(&mut self) -> u32 {
        self.ue()
    }
    fn se(&mut self) -> i32 {
        let v = i64::from(self.ue());
        (if v & 1 == 1 { (v + 1) >> 1 } else { -(v >> 1) }) as i32
    }
}

#[derive(Clone)]
struct Pic {
    planes: [Vec<u8>; 3],
}

/// One decoded picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Y, U, V (4:2:0), without padding.
    pub planes: [Vec<u8>; 3],
    /// An intra picture.
    pub key: bool,
    /// The stream is not a MOFLEX one: its pictures are YCgCo (with Cg and Co as the second and
    /// third plane, in that order as ffmpeg stores them).
    pub ycgco: bool,
}

/// The decoder: six pictures kept, the last quantiser and the motion of the macroblock row above.
#[derive(Clone)]
pub struct Decoder {
    w: usize,
    h: usize,
    pics: Vec<Pic>,
    have: [bool; 6],
    current: usize,
    moflex: bool,
    dct_tab_idx: usize,
    quantizer: i64,
    qtab: [[i32; 64]; 2],
    pre: [u8; 32],
    motion: Vec<(i32, i32)>,
    gb: Bits,
}

/// Where a block sits in its plane, for the neighbour lookups of the prediction modes.
struct Blk<'a> {
    plane: &'a [u8],
    w: i32,
    h: i32,
    ax: i32,
    ay: i32,
    size: i32,
}

impl Blk<'_> {
    fn pget(&self, x: i32, y: i32) -> i32 {
        let (mut rx, mut ry) = (x, y);
        if x == -1 && y >= self.size {
            rx = -1;
            ry = self.size - 1;
        } else if x >= -1 && y >= -1 {
        } else if x == -1 && y == -2 {
            rx = 0;
            ry = -1;
        } else if x == -2 && y == -1 {
            rx = -1;
            ry = 0;
        }
        let yy = (self.ay + ry).clamp(0, self.h - 1);
        let xx = (self.ax + rx).clamp(0, self.w - 1);
        i32::from(self.plane[(yy * self.w + xx) as usize])
    }
    fn half_horz(&self, x: i32, y: i32) -> i32 {
        half3(self.pget(x - 1, y), self.pget(x, y), self.pget(x + 1, y))
    }
    fn half_vert(&self, x: i32, y: i32) -> i32 {
        half3(self.pget(x, y - 1), self.pget(x, y), self.pget(x, y + 1))
    }
}

fn half(a: i32, b: i32) -> i32 {
    (a + b + 1) / 2
}

fn half3(a: i32, b: i32, c: i32) -> i32 {
    ((a + b + b + c) * 2 / 4 + 1) / 2
}

type Pick = fn(&Blk<'_>, i32, i32) -> i32;

fn pick_above(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    b.pget(x, y - 1)
}

fn pick_left(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    b.pget(x - 1, y)
}

fn pick_4(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    if x % 2 == 0 { half(b.pget(-1, y + x / 2), b.pget(-1, y + x / 2 + 1)) } else { b.half_vert(-1, y + x / 2 + 1) }
}

fn pick_5(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    if x == 0 {
        half(b.pget(-1, y - 1), b.pget(-1, y))
    } else if y == 0 {
        b.half_horz(x - 2, y - 1)
    } else if x == 1 {
        b.half_vert(x - 2, y - 1)
    } else {
        b.pget(x - 2, y - 1)
    }
}

fn pick_6(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    if y == 0 {
        half(b.pget(x - 1, -1), b.pget(x, -1))
    } else if x == 0 {
        b.half_vert(x - 1, y - 2)
    } else if y == 1 {
        b.half_horz(x - 1, y - 2)
    } else {
        b.pget(x - 1, y - 2)
    }
}

fn pick_7(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    let clr = b.pget(x - 1, y - 1);
    if x != 0 && y != 0 {
        return clr;
    }
    let acc1 = if x == 0 { b.pget(-1, y) } else { b.pget(x - 2, -1) };
    let acc2 = if y == 0 { b.pget(x, -1) } else { b.pget(-1, y - 2) };
    half3(acc1, clr, acc2)
}

fn pick_8(b: &Blk<'_>, x: i32, y: i32) -> i32 {
    let size = b.size;
    if y == 0 {
        half(b.pget(x, -1), b.pget(x + 1, -1))
    } else if y == 1 {
        b.half_horz(x + 1, y - 2)
    } else if x < size - 1 {
        b.pget(x + 1, y - 2)
    } else if y % 2 == 0 {
        half(b.pget(y / 2 + size - 1, -1), b.pget(y / 2 + size, -1))
    } else {
        b.half_horz(y / 2 + size, -1)
    }
}

fn inverse4(rs: &mut [i32]) {
    let a = rs[0].wrapping_add(rs[2]);
    let b = rs[0].wrapping_sub(rs[2]);
    let c = rs[1].wrapping_add(rs[3] >> 1);
    let d = (rs[1] >> 1).wrapping_sub(rs[3]);
    rs[0] = a.wrapping_add(c);
    rs[1] = b.wrapping_add(d);
    rs[2] = b.wrapping_sub(d);
    rs[3] = a.wrapping_sub(c);
}

fn idct(arr: &mut [i32], size: usize) {
    if size == 4 {
        inverse4(&mut arr[..4]);
        return;
    }
    let mut tmp = [arr[0], arr[2], arr[4], arr[6]];
    inverse4(&mut tmp);
    let w = i32::wrapping_add;
    let s = i32::wrapping_sub;
    let e = s(s(w(arr[7], arr[1]), arr[3]), arr[3] >> 1);
    let f = w(w(s(arr[7], arr[1]), arr[5]), arr[5] >> 1);
    let g = s(s(arr[5], arr[3]), w(arr[7], arr[7] >> 1));
    let h = w(w(w(arr[5], arr[3]), arr[1]), arr[1] >> 1);
    let x3 = w(g, h >> 2);
    let x2 = w(e, f >> 2);
    let x1 = s(e >> 2, f);
    let x0 = s(h, g >> 2);
    arr[0] = w(tmp[0], x0);
    arr[1] = w(tmp[1], x1);
    arr[2] = w(tmp[2], x2);
    arr[3] = w(tmp[3], x3);
    arr[4] = s(tmp[3], x3);
    arr[5] = s(tmp[2], x2);
    arr[6] = s(tmp[1], x1);
    arr[7] = s(tmp[0], x0);
}

fn adjust(x: i32, size: i32) -> i32 {
    if size == 16 { (x + 1) >> 1 } else { x }
}

fn get_index(x: i32) -> usize {
    match x {
        16 => 0,
        8 => 1,
        4 => 2,
        2 => 3,
        _ => 0,
    }
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

impl Decoder {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || w & 15 != 0 || h & 15 != 0 {
            return Err(Error::Invalid("width and height must be multiples of 16"));
        }
        if (w * h) as u64 > crate::MAX_PIXELS {
            return Err(Error::Invalid("picture size"));
        }
        let pic = Pic { planes: [vec![0; w * h], vec![0; w * h / 4], vec![0; w * h / 4]] };
        Ok(Decoder {
            w,
            h,
            pics: vec![pic; 6],
            have: [false; 6],
            current: 0,
            moflex: false,
            dct_tab_idx: 0,
            quantizer: 0,
            qtab: [[0; 64]; 2],
            pre: [0; 32],
            motion: vec![(0, 0); w / 16 + 3],
            gb: Bits { data: Vec::new(), pos: 0 },
        })
    }

    /// Bytes of the state a clone keeps.
    pub fn state_bytes(&self) -> usize {
        6 * self.w * self.h * 3 / 2 + 1024
    }

    fn setup_qtables(&mut self, quantizer: i64) -> Result<()> {
        if !(12..=161).contains(&quantizer) {
            return Err(Error::Invalid("quantiser"));
        }
        self.quantizer = quantizer;
        let (qx, qy) = ((quantizer % 6) as usize, (quantizer / 6) as u32);
        for i in 0..16 {
            self.qtab[0][i] = i32::from(QUANT4X4[qx][i]) << qy;
        }
        for i in 0..64 {
            self.qtab[1][i] = i32::from(QUANT8X8[qx][i]) << (qy - 2);
        }
        self.pre[..20].fill(9);
        Ok(())
    }

    fn read_run_encoding(&mut self) -> Result<(bool, i32, i32)> {
        let n = i32::from(self.gb.vlc(&tables().rl[self.dct_tab_idx])?);
        Ok(((n >> 11) == 1, (n >> 5) & 0x3F, n & 0x1F))
    }

    fn plane_dims(&self, plane: usize) -> (usize, usize) {
        if plane == 0 { (self.w, self.h) } else { (self.w / 2, self.h / 2) }
    }

    fn add_coefficients(&mut self, bx: i32, by: i32, size: usize, plane: usize) -> Result<()> {
        let mut mat = [0i32; 64];
        let q8 = usize::from(size == 8);
        let mut pos = 0i32;
        let idx = self.dct_tab_idx;
        while self.gb.left() > 0 {
            let (mut last, mut run, mut level) = self.read_run_encoding()?;
            if level != 0 {
                if self.gb.bit() {
                    level = -level;
                }
            } else if !self.gb.bit() {
                (last, run, level) = self.read_run_encoding()?;
                level += i32::from(RUN_RESIDUE[idx][(if last { 64 } else { 0 }) + run as usize]);
                if self.gb.bit() {
                    level = -level;
                }
            } else if !self.gb.bit() {
                (last, run, level) = self.read_run_encoding()?;
                run += i32::from(RUN_RESIDUE[idx][128 + (if last { 64 } else { 0 }) + level as usize]);
                if self.gb.bit() {
                    level = -level;
                }
            } else {
                last = self.gb.bit();
                run = self.gb.get(6) as i32;
                level = self.gb.sbits(12);
            }
            pos += run;
            if pos >= (size * size) as i32 {
                return Err(Error::Invalid("coefficient position"));
            }
            let z = if size == 8 { usize::from(ZIGZAG8X8[pos as usize]) } else { usize::from(ZIGZAG4X4[pos as usize]) };
            mat[z] = self.qtab[q8][pos as usize].wrapping_mul(level);
            if last {
                break;
            }
            pos += 1;
        }
        mat[0] = mat[0].wrapping_add(32);
        for y in 0..size {
            idct(&mut mat[y * size..], size);
        }
        let (pw, _) = self.plane_dims(plane);
        let pl = &mut self.pics[self.current].planes[plane];
        for y in 0..size {
            for x in y + 1..size {
                let a = mat[x * size + y];
                let b = mat[y * size + x];
                mat[y * size + x] = a;
                mat[x * size + y] = b;
            }
            idct(&mut mat[y * size..], size);
            for x in 0..size {
                let o = (by as usize + y) * pw + bx as usize + x;
                pl[o] = (i32::from(pl[o]) + (mat[y * size + x] >> 6)).clamp(0, 255) as u8;
            }
        }
        Ok(())
    }

    fn add_pframe_coefficients(&mut self, bx: i32, by: i32, size: usize, plane: usize) -> Result<()> {
        let idx = self.gb.ue31() as usize;
        if idx == 0 {
            self.add_coefficients(bx, by, size, plane)
        } else if idx < PFRAME_BLOCK4X4_COEFFICIENTS.len() {
            let mut flags = PFRAME_BLOCK4X4_COEFFICIENTS[idx];
            for y in (by..by + 8).step_by(4) {
                for x in (bx..bx + 8).step_by(4) {
                    if flags & 1 != 0 {
                        self.add_coefficients(x, y, 4, plane)?;
                    }
                    flags >>= 1;
                }
            }
            Ok(())
        } else {
            Err(Error::Invalid("coefficient pattern"))
        }
    }

    fn block_fill(&mut self, plane: usize, size: i32, ax: i32, ay: i32, pick: Pick) {
        let (pw, ph) = self.plane_dims(plane);
        let p = &mut self.pics[self.current].planes[plane];
        for y in 0..size {
            for x in 0..size {
                let b = Blk { plane: p, w: pw as i32, h: ph as i32, ax, ay, size };
                let v = pick(&b, x, y);
                p[(ay + y) as usize * pw + (ax + x) as usize] = v as u8;
            }
        }
    }

    fn predict_intra(&mut self, ax: i32, ay: i32, pmode: i32, add_coeffs: bool, size: i32, plane: usize) -> Result<()> {
        let (pw, _) = self.plane_dims(plane);
        let stride = pw as i32;
        match pmode {
            0 => self.block_fill(plane, size, ax, ay, pick_above),
            1 => self.block_fill(plane, size, ax, ay, pick_left),
            2 => {
                let se = self.gb.se().clamp(-(1 << 16), 1 << 16);
                let p = &mut self.pics[self.current].planes[plane];
                let top = ((ay - 1).max(0) * stride + ax) as usize;
                let left = (ay * stride + (ax - 1).max(0)) as usize;
                let bottommost = i32::from(p[((ay + size - 1) * stride + (ax - 1).max(0)) as usize]);
                let rightmost = i32::from(p[((ay - 1).max(0) * stride + ax + size - 1) as usize]);
                let avg = (bottommost + rightmost + 1) / 2 + 2 * se;
                let r6 = adjust(avg - bottommost, size);
                let r9 = adjust(avg - rightmost, size);
                let shift = if adjust(size, size) == 8 { 3 } else { 2 };
                let mut arr1 = [0i32; 16];
                let mut arr2 = [0i32; 16];
                for x in 0..size as usize {
                    let val = i32::from(p[top + x]);
                    arr1[x] = adjust(((bottommost - val) << shift) + r6 * (x as i32 + 1), size);
                }
                for y in 0..size as usize {
                    let val = i32::from(p[left + y * pw]);
                    arr2[y] = adjust(((rightmost - val) << shift) + r9 * (y as i32 + 1), size);
                }
                for y in 0..size as usize {
                    for x in 0..size as usize {
                        let t = i32::from(p[top + x]);
                        let l = i32::from(p[left + y * pw]);
                        let v = (t + l + ((arr1[x] * (y as i32 + 1) + arr2[y] * (x as i32 + 1)) >> (2 * shift)) + 1) / 2;
                        p[(ay as usize + y) * pw + ax as usize + x] = v as u8;
                    }
                }
            }
            3 => {
                let p = &mut self.pics[self.current].planes[plane];
                let sum_col = |p: &[u8]| -> i32 { (0..size).map(|y| i32::from(p[((ay + y) * stride + ax - 1) as usize])).sum() };
                let sum_row = |p: &[u8]| -> i32 { (0..size).map(|x| i32::from(p[((ay - 1) * stride + ax + x) as usize])).sum() };
                let fill = if ax == 0 && ay == 0 {
                    0x80
                } else if ax >= 1 && ay >= 1 {
                    ((sum_col(p) + sum_row(p)) * 2 / (2 * size) + 1) / 2
                } else if ax >= 1 {
                    (sum_col(p) * 2 / size + 1) / 2
                } else if ay >= 1 {
                    (sum_row(p) * 2 / size + 1) / 2
                } else {
                    return Err(Error::Invalid("DC prediction"));
                };
                for y in 0..size {
                    let o = ((ay + y) * stride + ax) as usize;
                    p[o..o + size as usize].fill(fill as u8);
                }
            }
            4 => self.block_fill(plane, size, ax, ay, pick_4),
            5 => self.block_fill(plane, size, ax, ay, pick_5),
            6 => self.block_fill(plane, size, ax, ay, pick_6),
            7 => self.block_fill(plane, size, ax, ay, pick_7),
            8 => self.block_fill(plane, size, ax, ay, pick_8),
            _ => {}
        }
        if add_coeffs {
            self.add_coefficients(ax, ay, size as usize, plane)?;
        }
        Ok(())
    }

    fn get_prediction(&mut self, x: i32, y: i32, size: i32) -> i32 {
        let index = ((y & 0xC) | (x / 4 % 4)) as usize;
        let mut val = self.pre[index].min(if index.is_multiple_of(4) { 9 } else { self.pre[index + 3] });
        if val == 9 {
            val = 3;
        }
        if !self.gb.bit() {
            let x = self.gb.get(3) as u8;
            val = x + u8::from(x >= val);
        }
        self.pre[index + 4] = val;
        if size == 8 {
            self.pre[index + 5] = val;
            self.pre[index + 8] = val;
            self.pre[index + 9] = val;
        }
        i32::from(val)
    }

    fn process_block(&mut self, x: i32, y: i32, mut pmode: i32, has_coeffs: bool, plane: usize) -> Result<()> {
        if !has_coeffs {
            if pmode < 0 {
                pmode = self.get_prediction(x, y, 8);
            }
            return self.predict_intra(x, y, pmode, false, 8, plane);
        }
        let tmp = self.gb.ue31() as usize;
        if tmp > BLOCK4X4_COEFFICIENTS.len() {
            return Err(Error::Invalid("coefficient pattern"));
        }
        if tmp == 0 {
            if pmode < 0 {
                pmode = self.get_prediction(x, y, 8);
            }
            self.predict_intra(x, y, pmode, true, 8, plane)
        } else {
            let mut flags = BLOCK4X4_COEFFICIENTS[tmp - 1];
            for by in (y..y + 8).step_by(4) {
                for bx in (x..x + 8).step_by(4) {
                    let mut new_pmode = pmode;
                    if new_pmode < 0 {
                        new_pmode = self.get_prediction(bx, by, 4);
                    }
                    self.predict_intra(bx, by, new_pmode, flags & 1 != 0, 4, plane)?;
                    flags >>= 1;
                }
            }
            Ok(())
        }
    }

    fn decode_macroblock(&mut self, x: i32, y: i32, predict: bool) -> Result<()> {
        let idx = self.gb.ue() as usize;
        if idx >= BLOCK8X8_COEFFICIENTS.len() {
            return Err(Error::Invalid("coefficient pattern"));
        }
        let mut flags = BLOCK8X8_COEFFICIENTS[idx];
        if predict {
            for (dx, dy) in [(0, 0), (8, 0), (0, 8), (8, 8)] {
                self.process_block(x + dx, y + dy, -1, flags & 1 != 0, 0)?;
                flags >>= 1;
            }
        } else {
            let mut pmode = self.gb.get(3) as i32;
            if pmode == 2 {
                self.predict_intra(x, y, pmode, false, 16, 0)?;
                pmode = 9;
            }
            for (dx, dy) in [(0, 0), (8, 0), (0, 8), (8, 8)] {
                self.process_block(x + dx, y + dy, pmode, flags & 1 != 0, 0)?;
                flags >>= 1;
            }
        }
        let (pa, pb) = if self.moflex { (1, 2) } else { (2, 1) };
        let mut pmode_uv = self.gb.get(3) as i32;
        if pmode_uv == 2 {
            self.predict_intra(x >> 1, y >> 1, pmode_uv, false, 8, pa)?;
            self.predict_intra(x >> 1, y >> 1, pmode_uv, false, 8, pb)?;
            pmode_uv = 9;
        }
        self.process_block(x >> 1, y >> 1, pmode_uv, flags & 1 != 0, pa)?;
        flags >>= 1;
        self.process_block(x >> 1, y >> 1, pmode_uv, flags & 1 != 0, pb)
    }

    #[allow(clippy::too_many_arguments)]
    fn predict_motion(&mut self, mut width: i32, mut height: i32, index: usize, offsetm: usize, mut offsetx: i32, mut offsety: i32) -> Result<()> {
        let (mut fwidth, mut fheight) = (self.w as i32, self.h as i32);
        if index <= 5 {
            let mut sidx = self.current as i32 - index.max(1) as i32;
            let mut mv = self.motion[0];
            if sidx < 0 {
                sidx += 6;
            }
            let sidx = sidx as usize;
            if index > 0 {
                mv.0 = mv.0.wrapping_add(self.gb.se());
                mv.1 = mv.1.wrapping_add(self.gb.se());
            }
            if mv.0 == i32::MAX || mv.1 == i32::MAX {
                return Err(Error::Invalid("motion vector"));
            }
            self.motion[offsetm] = mv;
            for i in 0..3 {
                if i == 1 {
                    offsetx >>= 1;
                    offsety >>= 1;
                    mv.0 >>= 1;
                    mv.1 >>= 1;
                    width >>= 1;
                    height >>= 1;
                    fwidth >>= 1;
                    fheight >>= 1;
                }
                if !self.have[sidx] {
                    return Err(Error::Invalid("reference picture missing"));
                }
                let method = (mv.0 & 1) | ((mv.1 & 1) << 1);
                if offsetx + (mv.0 >> 1) < 0
                    || offsety + (mv.1 >> 1) < 0
                    || offsetx + width + ((mv.0 + 1) >> 1) > fwidth
                    || offsety + height + ((mv.1 + 1) >> 1) > fheight
                {
                    return Err(Error::Invalid("motion vector out of the picture"));
                }
                let stride = fwidth as usize;
                let sx = (offsetx + (mv.0 >> 1)) as usize;
                let sy = (offsety + (mv.1 >> 1)) as usize;
                let (wd, ht) = (width as usize, height as usize);
                // the block read from the reference picture first (the planes of two pictures
                // cannot be borrowed together)
                let src = &self.pics[sidx].planes[i];
                let at = |x: usize, y: usize| -> u8 { src[(sy + y) * stride + sx + x] };
                let mut out = Vec::with_capacity(wd * ht);
                for y in 0..ht {
                    for x in 0..wd {
                        let v = match method {
                            0 => at(x, y),
                            1 => (at(x, y) >> 1).wrapping_add(at(x + 1, y) >> 1),
                            2 => (at(x, y) >> 1).wrapping_add(at(x, y + 1) >> 1),
                            _ => {
                                let a = (at(x, y) >> 1).wrapping_add(at(x + 1, y) >> 1) >> 1;
                                let b = (at(x, y + 1) >> 1).wrapping_add(at(x + 1, y + 1) >> 1) >> 1;
                                a.wrapping_add(b)
                            }
                        };
                        out.push(v);
                    }
                }
                let dst = &mut self.pics[self.current].planes[i];
                for y in 0..ht {
                    let o = (offsety as usize + y) * stride + offsetx as usize;
                    dst[o..o + wd].copy_from_slice(&out[y * wd..(y + 1) * wd]);
                }
            }
        } else {
            let adjx = if index == 8 { 0 } else { width / 2 };
            let adjy = if index == 8 { height / 2 } else { 0 };
            width -= adjx;
            height -= adjy;
            let tidx = get_index(height) * 4 + get_index(width);
            for i in 0..2 {
                let idx2 = usize::from(self.gb.vlc(&tables().mv[usize::from(self.moflex)][tidx])?);
                self.predict_motion(width, height, idx2, offsetm, offsetx + i * adjx, offsety + i * adjy)?;
            }
        }
        Ok(())
    }

    /// Decode one packet.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Frame> {
        let aligned = pkt.len().div_ceil(2) * 2;
        if self.h / 16 * (self.w / 16) * 2 > 8 * aligned {
            return Err(Error::Invalid("packet too small"));
        }
        let mut bs = vec![0u8; aligned];
        for (i, pair) in pkt.chunks(2).enumerate() {
            bs[2 * i] = pair.get(1).copied().unwrap_or(0);
            bs[2 * i + 1] = pair[0];
        }
        self.gb = Bits { data: bs, pos: 0 };
        let key = self.gb.bit();
        if key {
            self.moflex = self.gb.bit();
            self.dct_tab_idx = usize::from(self.gb.bit());
            let q = i64::from(self.gb.get(6));
            self.setup_qtables(q)?;
            for y in (0..self.h as i32).step_by(16) {
                for x in (0..self.w as i32).step_by(16) {
                    let predict = self.gb.bit();
                    self.decode_macroblock(x, y, predict)?;
                }
            }
        } else {
            self.motion.fill((0, 0));
            self.dct_tab_idx = 0;
            let q = self.quantizer + i64::from(self.gb.se());
            self.setup_qtables(q)?;
            for y in (0..self.h as i32).step_by(16) {
                for x in (0..self.w as i32).step_by(16) {
                    let m = (x / 16) as usize;
                    self.motion[0].0 = mid_pred(self.motion[m + 1].0, self.motion[m + 2].0, self.motion[m + 3].0);
                    self.motion[0].1 = mid_pred(self.motion[m + 1].1, self.motion[m + 2].1, self.motion[m + 3].1);
                    self.motion[m + 2] = (0, 0);
                    let idx = usize::from(self.gb.vlc(&tables().mv[usize::from(self.moflex)][0])?);
                    if idx == 6 || idx == 7 {
                        self.decode_macroblock(x, y, idx == 7)?;
                    } else {
                        self.predict_motion(16, 16, idx, m + 2, x, y)?;
                        let idx2 = self.gb.ue() as usize;
                        if idx2 >= PFRAME_BLOCK8X8_COEFFICIENTS.len() {
                            return Err(Error::Invalid("coefficient pattern"));
                        }
                        let mut flags = PFRAME_BLOCK8X8_COEFFICIENTS[idx2];
                        for sy in (y..y + 16).step_by(8) {
                            for sx in (x..x + 16).step_by(8) {
                                if flags & 1 != 0 {
                                    let _ = self.add_pframe_coefficients(sx, sy, 8, 0);
                                }
                                flags >>= 1;
                            }
                        }
                        let (pa, pb) = if self.moflex { (1, 2) } else { (2, 1) };
                        if flags & 1 != 0 {
                            let _ = self.add_pframe_coefficients(x >> 1, y >> 1, 8, pa);
                        }
                        flags >>= 1;
                        if flags & 1 != 0 {
                            let _ = self.add_pframe_coefficients(x >> 1, y >> 1, 8, pb);
                        }
                    }
                }
            }
        }
        let frame = Frame { width: self.w as u32, height: self.h as u32, planes: self.pics[self.current].planes.clone(), key, ycgco: !self.moflex };
        self.have[self.current] = true;
        self.current = (self.current + 1) % 6;
        Ok(frame)
    }
}
