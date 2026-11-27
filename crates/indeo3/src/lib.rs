//! Intel Indeo 3 (`IV31`, `IV32`) video decoder, a Rust re-expression of FFmpeg n7.1.1
//! `libavcodec/indeo3.c` and `indeo3data.h` (LGPL-2.1-or-later, Copyright (c) 2009 - 2011 Maxim
//! Poliakovski). See `README.md` for the licence notes.
//!
//! Indeo 3 is vector quantisation with run-length coding and motion compensation, on 7-bit
//! pixels held in two alternating frame buffers. The decoder works on byte offsets into the two
//! buffers and reproduces ffmpeg's word-wide (SWAR) arithmetic on little-endian words, including
//! its in-place requantisation of reference pixels.
//!
//! Safe Rust only; damaged data is an error, never a panic.

use thiserror::Error;

mod data;
use data::VQ;

/// Indeo 3 decoding errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("Indeo 3: invalid picture size {0}x{1}")]
    Size(u32, u32),
    #[error("Indeo 3: frame too short or damaged")]
    Invalid,
    #[error("Indeo 3: unsupported feature: {0}")]
    Unsupported(&'static str),
}

/// A decoded picture: planar YUV 4:1:0, tightly packed (`Y`, `U`, `V`; chroma at a quarter of
/// the width and height).
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub planes: Vec<Vec<u8>>,
    /// Whether this was a key (intra) frame.
    pub key: bool,
}

impl Picture {
    /// Width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if i == 0 { (w, h) } else { (w / 4, h / 4) }
    }
}

const RLE_ESC_F9: u8 = 249;
const RLE_ESC_FA: u8 = 250;
const RLE_ESC_FB: u8 = 251;
const RLE_ESC_FC: u8 = 252;
const RLE_ESC_FD: u8 = 253;
const RLE_ESC_FE: u8 = 254;
const RLE_ESC_FF: u8 = 255;

const BS_8BIT_PEL: u16 = 1 << 1;
const BS_KEYFRAME: u16 = 1 << 2;
const BS_MV_Y_HALF: u16 = 1 << 4;
const BS_MV_X_HALF: u16 = 1 << 5;
const BS_BUFFER: u32 = 9;

const CELL_STACK_MAX: i32 = 20;
const OS_HDR_ID: u32 = u32::from_be_bytes(*b"FRMH");

const H_SPLIT: u32 = 0;
const V_SPLIT: u32 = 1;
const INTRA_NULL: u32 = 2;
const INTER_DATA: u32 = 3;

/// One VQ table in its expanded form.
struct Vq {
    deltas: Vec<i16>,
    deltas_m10: Vec<i32>,
    num_dyads: usize,
    quad_exp: usize,
}

/// The decoder's static tables: the VQ delta tables and the requantisation table.
struct Tables {
    vq: Vec<Vq>,
    requant: [[u8; 128]; 8],
}

fn pd16(a: i32, b: i32) -> i16 {
    (b * 256 + a) as i16
}

fn pd32(a: i32, b: i32) -> i32 {
    b.wrapping_mul(1 << 24).wrapping_add(b.wrapping_mul(1 << 16)).wrapping_add(a.wrapping_mul(1 << 8)).wrapping_add(a)
}

impl Tables {
    fn new() -> Self {
        let mut vq = Vec::with_capacity(24);
        for &(comp, size, num_dyads, quad_exp) in VQ.iter() {
            let (mut deltas, mut m10) = (Vec::with_capacity(size), Vec::with_capacity(size));
            let mut push = |a: i32, b: i32| {
                deltas.push(pd16(a, b));
                m10.push(pd32(a, b));
            };
            for &(kind, a, b) in comp {
                let (a, b) = (i32::from(a), i32::from(b));
                push(a, b);
                if kind >= 2 {
                    push(-a, -b);
                }
                if kind == 4 {
                    push(b, a);
                    push(-b, -a);
                }
            }
            deltas.resize(size, 0);
            m10.resize(size, 0);
            vq.push(Vq { deltas, deltas_m10: m10, num_dyads: usize::from(num_dyads), quad_exp: usize::from(quad_exp) });
        }
        let offsets: [i32; 8] = [1, 1, 2, -3, -3, 3, 4, 4];
        let deltas: [i32; 8] = [0, 1, 0, 4, 4, 1, 0, 1];
        let mut requant = [[0u8; 128]; 8];
        for i in 0..8 {
            let step = i as i32 + 2;
            for j in 0..128i32 {
                requant[i][j as usize] = ((j + offsets[i]) / step * step + deltas[i]) as u8;
            }
        }
        requant[0][127] = 126;
        requant[1][119] = 118;
        requant[1][120] = 118;
        requant[2][126] = 124;
        requant[2][127] = 124;
        requant[6][124] = 120;
        requant[6][125] = 120;
        requant[6][126] = 120;
        requant[6][127] = 120;
        requant[1][7] = 10;
        requant[4][8] = 10;
        Self { vq, requant }
    }
}

/// One plane in two buffers, each with an extra line on top for intra prediction.
struct Plane {
    width: usize,
    height: usize,
    pitch: usize,
    bufs: [Vec<u8>; 2],
}

impl Plane {
    fn new(width: usize, height: usize) -> Self {
        let pitch = width.next_multiple_of(16);
        let mk = || {
            let mut b = vec![0u8; pitch * (height + 1)];
            b[..pitch].fill(0x40);
            b
        };
        Self { width, height, pitch, bufs: [mk(), mk()] }
    }
    fn rd(&self, buf: usize, i: isize) -> u8 {
        usize::try_from(i).ok().and_then(|i| self.bufs[buf].get(i)).copied().unwrap_or(0)
    }
    fn wr(&mut self, buf: usize, i: isize, v: u8) {
        if let Some(p) = usize::try_from(i).ok().and_then(|i| self.bufs[buf].get_mut(i)) {
            *p = v;
        }
    }
    fn rd_n(&self, buf: usize, i: isize, n: usize) -> u64 {
        (0..n).fold(0u64, |a, k| a | u64::from(self.rd(buf, i + k as isize)) << (8 * k))
    }
    fn wr_n(&mut self, buf: usize, i: isize, n: usize, v: u64) {
        for k in 0..n {
            self.wr(buf, i + k as isize, (v >> (8 * k)) as u8);
        }
    }
}

#[derive(Clone, Copy)]
struct Cell {
    xpos: i32,
    ypos: i32,
    width: i32,
    height: i32,
    /// 0: motion-compensation tree, 1: VQ tree.
    tree: u8,
    /// (mv_y, mv_x) of an inter cell.
    mv: Option<(i32, i32)>,
}

fn replicate64(a: u64) -> u64 {
    let a = a & 0x00FF_00FF_00FF_00FF;
    a | a << 8
}

fn replicate32(a: u32) -> u32 {
    let a = a & 0x00FF_00FF;
    a | a << 8
}

fn avg64(a: u64, b: u64) -> u64 {
    (a.wrapping_add(b) >> 1) & 0x7F7F_7F7F_7F7F_7F7F
}

fn avg32(a: u32, b: u32) -> u32 {
    (a.wrapping_add(b) >> 1) & 0x7F7F_7F7F
}

/// Cell decoding errors, ffmpeg's `IV3_*`.
#[derive(Debug)]
enum Code {
    BadRle,
    BadData,
    BadCounter,
    Unsupported,
    OutOfData,
    Invalid,
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> usize {
        (self.d.len() * 8).saturating_sub(self.pos)
    }
    fn get(&mut self, n: usize) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            let bit = self.d.get(self.pos >> 3).map_or(0, |b| (b >> (7 - (self.pos & 7))) & 1);
            v = v << 1 | u32::from(bit);
            self.pos += 1;
        }
        v
    }
}

/// Everything needed to decode one plane of one frame.
struct PlaneDecoder<'a> {
    t: &'a Tables,
    plane: &'a mut Plane,
    buf_sel: usize,
    cb_offset: usize,
    alt_quant: [u8; 16],
    /// Plane data after the vector count: motion vectors, then the bit stream.
    pd: &'a [u8],
    num_vectors: usize,
    bits: Bits<'a>,
    skip_bits: usize,
    need_resync: bool,
    next_cell_data: usize,
}

impl PlaneDecoder<'_> {
    fn pitch(&self) -> isize {
        self.plane.pitch as isize
    }

    /// Copy the cell from the previous frame, displaced by its motion vector.
    fn copy_cell(&mut self, cell: &Cell) -> Result<(), Code> {
        let pitch = self.pitch();
        let off_dst = (cell.ypos << 2) as isize * pitch + (cell.xpos << 2) as isize;
        let (mv_y, mv_x) = cell.mv.unwrap_or((0, 0));
        if (cell.ypos << 2) + mv_y < -1
            || (cell.xpos << 2) + mv_x < 0
            || ((cell.ypos + cell.height) << 2) + mv_y > self.plane.height as i32
            || ((cell.xpos + cell.width) << 2) + mv_x > self.plane.width as i32
        {
            return Err(Code::Invalid);
        }
        let (dst, src) = (pitch + off_dst, pitch + off_dst + mv_y as isize * pitch + mv_x as isize);
        let (sel, other) = (self.buf_sel, self.buf_sel ^ 1);
        for r in 0..(cell.height << 2) as isize {
            for c in 0..(cell.width << 2) as isize {
                let v = self.plane.rd(other, src + r * pitch + c);
                self.plane.wr(sel, dst + r * pitch + c, v);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_cell_data(
        &mut self,
        cell: &Cell,
        mut block: isize,
        mut ref_block: Option<(usize, isize)>,
        h_zoom: i32,
        v_zoom: i32,
        mode: i32,
        delta: [usize; 2],
        swap_quads: [bool; 2],
        data: &[u8],
        dpos: &mut usize,
    ) -> Result<(), Code> {
        let row = self.pitch();
        let sel = self.buf_sel;
        let (mut rle_blocks, mut skip_flag, mut is_first_row) = (0i32, false, true);
        let blk_row_offset = (row << (2 + v_zoom)) - (cell.width << 2) as isize;
        let line_offset = if v_zoom != 0 { row } else { 0 };
        if cell.height & v_zoom != 0 || cell.width & h_zoom != 0 {
            return Err(Code::BadData);
        }
        let has_mv = cell.mv.is_some();
        let (rbuf0, _) = ref_block.unwrap_or((sel, 0));
        let mut y = 0;
        while y < cell.height {
            let mut x = 0;
            while x < cell.width {
                let mut rf = ref_block.map_or(0, |r| r.1);
                let mut dst = block;
                let rbuf = rbuf0;
                // copies `n` rows of 4 pixels from the reference to the destination
                macro_rules! copy_block4 {
                    ($n:expr) => {{
                        for r in 0..$n as isize {
                            let v = self.plane.rd_n(rbuf, rf + r * row, 4);
                            self.plane.wr_n(sel, dst + r * row, 4, v);
                        }
                    }};
                }
                macro_rules! fill64 {
                    ($at:expr, $pix:expr, $n:expr) => {{
                        for r in 0..$n as isize {
                            self.plane.wr_n(sel, $at + r * row, 8, $pix);
                        }
                    }};
                }
                if rle_blocks > 0 {
                    if mode <= 4 {
                        if has_mv || !skip_flag {
                            copy_block4!(4 << v_zoom);
                        }
                    } else if mode == 10 && !has_mv {
                        // RLE_BLOCK_COPY_8
                        let mut pix = self.plane.rd_n(rbuf, rf, 8);
                        if is_first_row {
                            pix = replicate64(pix);
                            fill64!(dst + row, pix, 7);
                            let v = avg64(self.plane.rd_n(rbuf, rf, 8), self.plane.rd_n(sel, dst + row, 8));
                            self.plane.wr_n(sel, dst, 8, v);
                        } else {
                            fill64!(dst, pix, 8);
                        }
                    }
                    rle_blocks -= 1;
                } else {
                    let mut line = 0i32;
                    while line < 4 {
                        let mut num_lines = 1i32;
                        let is_top = is_first_row && line == 0;
                        let dt = if mode <= 4 { delta[(line & 1) as usize] } else { delta[1] };
                        if *dpos >= data.len() {
                            return Err(Code::OutOfData);
                        }
                        let mut code = data[*dpos];
                        *dpos += 1;
                        if code < 248 {
                            let vq = &self.t.vq[dt];
                            let (dyad1, dyad2);
                            if (code as usize) < vq.num_dyads {
                                if *dpos >= data.len() {
                                    return Err(Code::OutOfData);
                                }
                                dyad1 = usize::from(data[*dpos]);
                                *dpos += 1;
                                dyad2 = usize::from(code);
                                if dyad1 >= vq.num_dyads || dyad1 >= 248 {
                                    return Err(Code::BadData);
                                }
                            } else {
                                let c = usize::from(code) - vq.num_dyads;
                                let (mut a, mut b) = (c / vq.quad_exp, c % vq.quad_exp);
                                if swap_quads[(line & 1) as usize] {
                                    std::mem::swap(&mut a, &mut b);
                                }
                                dyad1 = a;
                                dyad2 = b;
                            }
                            let d16 = |d: usize| i32::from(vq.deltas.get(d).copied().unwrap_or(0));
                            let d32 = |d: usize| vq.deltas_m10.get(d).copied().unwrap_or(0) as u32;
                            let (a16, b16) = (d16(dyad1), d16(dyad2));
                            let (a32, b32) = (d32(dyad1), d32(dyad2));
                            if mode <= 4 {
                                // APPLY_DELTA_4
                                let lo = dst + line_offset;
                                let v1 = (self.plane.rd_n(rbuf, rf, 2) as i32 + a16) & 0x7F7F;
                                self.plane.wr_n(sel, lo, 2, v1 as u64);
                                let v2 = (self.plane.rd_n(rbuf, rf + 2, 2) as i32 + b16) & 0x7F7F;
                                self.plane.wr_n(sel, lo + 2, 2, v2 as u64);
                                if mode >= 3 {
                                    if is_top && cell.ypos == 0 {
                                        let v = self.plane.rd_n(sel, dst + row, 4);
                                        self.plane.wr_n(sel, dst, 4, v);
                                    } else {
                                        let v = avg32(self.plane.rd_n(rbuf, rf, 4) as u32, self.plane.rd_n(sel, dst + row, 4) as u32);
                                        self.plane.wr_n(sel, dst, 4, u64::from(v));
                                    }
                                }
                            } else if mode == 10 && !has_mv {
                                // APPLY_DELTA_8
                                let rep = |p: &Plane, o: isize| if is_top { replicate32(p.rd_n(rbuf, o, 4) as u32) } else { p.rd_n(rbuf, o, 4) as u32 };
                                let v1 = rep(self.plane, rf).wrapping_add(a32) & 0x7F7F_7F7F;
                                self.plane.wr_n(sel, dst + row, 4, u64::from(v1));
                                let v2 = rep(self.plane, rf + 4).wrapping_add(b32) & 0x7F7F_7F7F;
                                self.plane.wr_n(sel, dst + row + 4, 4, u64::from(v2));
                                if is_top && cell.ypos == 0 {
                                    let v = self.plane.rd_n(sel, dst + row, 8);
                                    self.plane.wr_n(sel, dst, 8, v);
                                } else {
                                    let v = avg64(self.plane.rd_n(rbuf, rf, 8), self.plane.rd_n(sel, dst + row, 8));
                                    self.plane.wr_n(sel, dst, 8, v);
                                }
                            } else if mode == 10 {
                                // APPLY_DELTA_1011_INTER, mode 10
                                for (o, d) in [(0, a32), (4, b32), (row, a32), (row + 4, b32)] {
                                    let v = (self.plane.rd_n(sel, dst + o, 4) as u32).wrapping_add(d) & 0x7F7F_7F7F;
                                    self.plane.wr_n(sel, dst + o, 4, u64::from(v));
                                }
                            } else {
                                for (o, d) in [(0, a16), (2, b16), (row, a16), (row + 2, b16)] {
                                    let v = (self.plane.rd_n(sel, dst + o, 2) as i32 + d) & 0x7F7F;
                                    self.plane.wr_n(sel, dst + o, 2, v as u64);
                                }
                            }
                        } else {
                            match code {
                                RLE_ESC_FC | RLE_ESC_FF | RLE_ESC_FE | RLE_ESC_FD => {
                                    if code == RLE_ESC_FC {
                                        skip_flag = false;
                                        rle_blocks = 1;
                                        code = RLE_ESC_FD;
                                    }
                                    num_lines = 257 - i32::from(code) - line;
                                    if num_lines <= 0 {
                                        return Err(Code::BadRle);
                                    }
                                    if mode <= 4 {
                                        copy_block4!(num_lines << v_zoom);
                                    } else if mode == 10 && !has_mv {
                                        lines_copy_m10(self.plane, sel, rbuf, rf, dst, row, is_top, num_lines);
                                    }
                                }
                                RLE_ESC_FB => {
                                    if *dpos >= data.len() {
                                        return Err(Code::OutOfData);
                                    }
                                    let c = data[*dpos];
                                    *dpos += 1;
                                    rle_blocks = i32::from(c & 0x1F) - 1;
                                    if c >= 64 || rle_blocks < 0 {
                                        return Err(Code::BadCounter);
                                    }
                                    skip_flag = c & 0x20 != 0;
                                    num_lines = 4 - line;
                                    if mode >= 10 || (has_mv || !skip_flag) {
                                        if mode <= 4 {
                                            copy_block4!(num_lines << v_zoom);
                                        } else if mode == 10 && !has_mv {
                                            lines_copy_m10(self.plane, sel, rbuf, rf, dst, row, is_top, num_lines);
                                        }
                                    }
                                }
                                RLE_ESC_F9 | RLE_ESC_FA => {
                                    if code == RLE_ESC_F9 {
                                        skip_flag = true;
                                        rle_blocks = 1;
                                    }
                                    if line != 0 {
                                        return Err(Code::BadRle);
                                    }
                                    num_lines = 4;
                                    if has_mv {
                                        if mode <= 4 {
                                            copy_block4!(num_lines << v_zoom);
                                        } else if mode == 10 && !has_mv {
                                            lines_copy_m10(self.plane, sel, rbuf, rf, dst, row, is_top, num_lines);
                                        }
                                    }
                                }
                                _ => return Err(Code::Unsupported),
                            }
                        }
                        line += num_lines;
                        rf += row * (num_lines << v_zoom) as isize;
                        dst += row * (num_lines << v_zoom) as isize;
                    }
                }
                block += 4 << h_zoom;
                if let Some(r) = ref_block.as_mut() {
                    r.1 += 4 << h_zoom;
                }
                x += 1 + h_zoom;
            }
            is_first_row = false;
            if let Some(r) = ref_block.as_mut() {
                r.1 += blk_row_offset;
            }
            block += blk_row_offset;
            y += 1 + v_zoom;
        }
        Ok(())
    }

    /// Decode a vector-quantised cell; returns the bytes consumed.
    fn decode_cell(&mut self, cell: &Cell, data: &[u8]) -> Result<usize, Code> {
        let first = *data.first().ok_or(Code::OutOfData)?;
        let mut dpos = 1usize;
        let (mode, mut vq_index) = (i32::from(first >> 4), usize::from(first & 0xF));
        let pitch = self.pitch();
        let sel = self.buf_sel;
        let mut offset = (cell.ypos << 2) as isize * pitch + (cell.xpos << 2) as isize;
        let block = pitch + offset;
        let mut ref_block: Option<(usize, isize)> = None;
        if cell.mv.is_none() {
            ref_block = Some((sel, block - pitch));
        } else if mode >= 10 {
            self.copy_cell(cell)?;
        } else {
            let (mv_y, mv_x) = cell.mv.unwrap_or((0, 0));
            if (cell.ypos << 2) + mv_y < -1
                || (cell.xpos << 2) + mv_x < 0
                || ((cell.ypos + cell.height) << 2) + mv_y > self.plane.height as i32
                || ((cell.xpos + cell.width) << 2) + mv_x > self.plane.width as i32
            {
                return Err(Code::Invalid);
            }
            offset += mv_y as isize * pitch + mv_x as isize;
            ref_block = Some((sel ^ 1, pitch + offset));
        }
        let (prim, second);
        if mode == 1 || mode == 4 {
            let c = self.alt_quant[vq_index];
            prim = usize::from(c >> 4) + self.cb_offset;
            second = usize::from(c & 0xF) + self.cb_offset;
        } else {
            vq_index += self.cb_offset;
            prim = vq_index;
            second = vq_index;
        }
        if prim >= 24 || second >= 24 {
            return Err(Code::Invalid);
        }
        let delta = [second, prim];
        let swap = [second >= 16, prim >= 16];
        if vq_index >= 8
            && let Some((rb, ro)) = ref_block
        {
            for x in 0..(cell.width << 2) as isize {
                let v = self.plane.rd(rb, ro + x);
                let nv = self.t.requant[vq_index & 7][usize::from(v & 127)];
                self.plane.wr(rb, ro + x, nv);
            }
        }
        let has_mv = cell.mv.is_some();
        match mode {
            0 | 1 | 3 | 4 => {
                if mode >= 3 && has_mv {
                    return Err(Code::Invalid);
                }
                self.decode_cell_data(cell, block, ref_block, 0, i32::from(mode >= 3), mode, delta, swap, data, &mut dpos)?;
            }
            10 | 11 => {
                if mode == 10 && !has_mv {
                    self.decode_cell_data(cell, block, ref_block, 1, 1, mode, delta, swap, data, &mut dpos)?;
                } else {
                    if mode == 11 && !has_mv {
                        return Err(Code::Invalid);
                    }
                    self.decode_cell_data(cell, block, ref_block, i32::from(mode == 10), 1, mode, delta, swap, data, &mut dpos)?;
                }
            }
            _ => return Err(Code::Invalid),
        }
        Ok(dpos)
    }

    fn check_cell(&self, c: &Cell) -> Result<(), Code> {
        if c.xpos + c.width > (self.plane.width >> 2) as i32 || c.ypos + c.height > (self.plane.height >> 2) as i32 {
            return Err(Code::Invalid);
        }
        Ok(())
    }

    fn resync(&mut self) {
        if self.need_resync && self.bits.pos & 7 == 0 {
            self.bits.pos += self.skip_bits;
            self.skip_bits = 0;
            self.need_resync = false;
        }
    }

    fn parse_bintree(&mut self, mut code: u32, ref_cell: &mut Cell, depth: i32, strip_width: i32) -> Result<(), Code> {
        if depth <= 0 {
            return Err(Code::Invalid);
        }
        let split = |size: i32| if size > 2 { ((size + 2) >> 2) << 1 } else { 1 };
        let mut cur = *ref_cell;
        if code == H_SPLIT {
            cur.height = split(ref_cell.height);
            ref_cell.ypos += cur.height;
            ref_cell.height -= cur.height;
            if ref_cell.height <= 0 || cur.height <= 0 {
                return Err(Code::Invalid);
            }
        } else if code == V_SPLIT {
            if cur.width > strip_width {
                cur.width = (if cur.width <= (strip_width << 1) { 1 } else { 2 }) * strip_width;
            } else {
                cur.width = split(ref_cell.width);
            }
            ref_cell.xpos += cur.width;
            ref_cell.width -= cur.width;
            if ref_cell.width <= 0 || cur.width <= 0 {
                return Err(Code::Invalid);
            }
        }
        while self.bits.left() >= 2 {
            self.resync();
            code = self.bits.get(2);
            match code {
                H_SPLIT | V_SPLIT => self.parse_bintree(code, &mut cur, depth - 1, strip_width)?,
                INTRA_NULL => {
                    if cur.tree == 0 {
                        cur.mv = None;
                        cur.tree = 1;
                    } else {
                        self.resync();
                        let c = self.bits.get(2);
                        if c >= 2 {
                            return Err(Code::Invalid);
                        }
                        self.check_cell(&cur)?;
                        if cur.mv.is_none() {
                            return Err(Code::Invalid);
                        }
                        return self.copy_cell(&cur);
                    }
                }
                _ => {
                    // INTER_DATA
                    debug_assert_eq!(code, INTER_DATA);
                    if cur.tree == 0 {
                        if !self.need_resync {
                            self.next_cell_data = self.num_vectors * 2 + ((self.bits.pos + 7) >> 3);
                        }
                        if self.next_cell_data >= self.pd.len() {
                            return Err(Code::Invalid);
                        }
                        let idx = usize::from(self.pd[self.next_cell_data]);
                        self.next_cell_data += 1;
                        if idx >= self.num_vectors {
                            return Err(Code::Invalid);
                        }
                        cur.mv = Some((i32::from(self.pd[idx * 2] as i8), i32::from(self.pd[idx * 2 + 1] as i8)));
                        cur.tree = 1;
                        self.skip_bits += 8;
                        self.need_resync = true;
                    } else {
                        if !self.need_resync {
                            self.next_cell_data = self.num_vectors * 2 + ((self.bits.pos + 7) >> 3);
                        }
                        self.check_cell(&cur)?;
                        let start = self.next_cell_data.min(self.pd.len());
                        let used = self.decode_cell(&cur, &self.pd[start..])?;
                        self.skip_bits += used << 3;
                        self.need_resync = true;
                        self.next_cell_data += used;
                        return Ok(());
                    }
                }
            }
        }
        Err(Code::Invalid)
    }
}

#[allow(clippy::too_many_arguments)]
fn lines_copy_m10(p: &mut Plane, sel: usize, rbuf: usize, rf: isize, dst: isize, row: isize, is_top: bool, num_lines: i32) {
    let mut pix = p.rd_n(rbuf, rf, 8);
    if is_top {
        pix = replicate64(pix);
        for r in 0..((num_lines << 1) - 1) as isize {
            p.wr_n(sel, dst + row + r * row, 8, pix);
        }
        let v = avg64(p.rd_n(rbuf, rf, 8), p.rd_n(sel, dst + row, 8));
        p.wr_n(sel, dst, 8, v);
    } else {
        for r in 0..(num_lines << 1) as isize {
            p.wr_n(sel, dst + r * row, 8, pix);
        }
    }
}

fn le32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn le16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

/// An Indeo 3 decoder for one stream.
pub struct Decoder {
    t: Tables,
    planes: Vec<Plane>,
    width: usize,
    height: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self { t: Tables::new(), planes: Vec::new(), width: 0, height: 0 }
    }

    /// Reset to the state of a new stream (the next frame allocates the buffers again).
    pub fn reset(&mut self) {
        self.planes.clear();
        self.width = 0;
        self.height = 0;
    }

    fn allocate(&mut self, w: usize, h: usize) -> Result<(), Error> {
        let (lw, lh) = (w.next_multiple_of(2), h.next_multiple_of(2));
        if !(16..=640).contains(&lw) || !(16..=480).contains(&lh) {
            return Err(Error::Size(w as u32, h as u32));
        }
        let (cw, ch) = ((lw >> 2).next_multiple_of(4), (lh >> 2).next_multiple_of(4));
        self.planes = vec![Plane::new(lw, lh), Plane::new(cw, ch), Plane::new(cw, ch)];
        self.width = lw;
        self.height = lh;
        Ok(())
    }

    /// Decode one frame. `Ok(None)` for a sync (null) frame, which carries no picture.
    pub fn decode(&mut self, buf: &[u8]) -> Result<Option<Picture>, Error> {
        let inv = Error::Invalid;
        let (frame_num, word2, check_sum, data_size) =
            (le32(buf, 0).ok_or(inv)?, le32(buf, 4).ok_or(Error::Invalid)?, le32(buf, 8).ok_or(Error::Invalid)?, le32(buf, 12).ok_or(Error::Invalid)?);
        if frame_num ^ word2 ^ data_size ^ OS_HDR_ID != check_sum {
            return Err(Error::Invalid);
        }
        let bs = buf.get(16..).ok_or(Error::Invalid)?;
        if le16(bs, 0).ok_or(Error::Invalid)? != 32 {
            return Err(Error::Invalid);
        }
        let frame_flags = le16(bs, 2).ok_or(Error::Invalid)?;
        let mut dsize = ((u64::from(le32(bs, 4).ok_or(Error::Invalid)?) + 7) >> 3) as i64;
        let cb_offset = usize::from(*bs.get(8).ok_or(Error::Invalid)?);
        if dsize == 16 {
            return Ok(None);
        }
        dsize = dsize.min(buf.len() as i64 - 16);
        // bytes 9..12: a reserved byte and the checksum
        let height = usize::from(le16(bs, 12).ok_or(Error::Invalid)?);
        let width = usize::from(le16(bs, 14).ok_or(Error::Invalid)?);
        if width == 0 || height == 0 {
            return Err(Error::Invalid);
        }
        if width != self.width || height != self.height {
            if !(16..=640).contains(&width) || !(16..=480).contains(&height) || width & 3 != 0 || height & 3 != 0 {
                return Err(Error::Size(width as u32, height as u32));
            }
            self.allocate(width, height)?;
        }
        let rd = |o| le32(bs, o).map(|v| v as i32).ok_or(Error::Invalid);
        let (y_off, v_off, u_off) = (rd(16)?, rd(20)?, rd(24)?);
        let starts = [i64::from(y_off), i64::from(v_off), i64::from(u_off)];
        let mut ends = [0i64; 3];
        for j in 0..3 {
            ends[j] = dsize;
            for i in (0..3).rev() {
                if starts[i] < ends[j] && starts[i] > starts[j] {
                    ends[j] = starts[i];
                }
            }
        }
        let sizes = [ends[0] - starts[0], ends[1] - starts[1], ends[2] - starts[2]];
        let (min_off, max_off) = (starts.iter().copied().min().unwrap_or(0), starts.iter().copied().max().unwrap_or(0));
        if min_off < 0 || max_off >= dsize - 16 || min_off < 32 + 16 || sizes.iter().copied().min().unwrap_or(0) <= 0 {
            return Err(Error::Invalid);
        }
        let alt_quant: [u8; 16] = bs.get(32..48).and_then(|s| s.try_into().ok()).ok_or(Error::Invalid)?;
        if frame_flags & BS_8BIT_PEL != 0 {
            return Err(Error::Unsupported("8-bit pixel format"));
        }
        if frame_flags & (BS_MV_X_HALF | BS_MV_Y_HALF) != 0 {
            return Err(Error::Unsupported("halfpel motion vectors"));
        }
        let buf_sel = usize::from((frame_flags >> BS_BUFFER) & 1 != 0);
        // plane data: luma, then U (offset u), then V (offset v), strip widths 40, 10, 10
        let plane_data = [(starts[0], sizes[0], 40), (starts[2], sizes[2], 10), (starts[1], sizes[1], 10)];
        for (p, &(start, size, strip)) in plane_data.iter().enumerate() {
            let slice = bs.get(start as usize..(start + size) as usize).ok_or(Error::Invalid)?;
            decode_plane(&self.t, &mut self.planes[p], buf_sel, cb_offset, alt_quant, slice, strip).map_err(|_| Error::Invalid)?;
        }
        // output: (pixel & 0x7F) << 1, chroma cropped to a quarter
        let (w, h) = (self.width, self.height);
        let mut planes = Vec::with_capacity(3);
        for (p, pl) in self.planes.iter().enumerate() {
            let (ow, oh) = if p == 0 { (w, h) } else { (w / 4, h / 4) };
            let mut out = Vec::with_capacity(ow * oh);
            for y in 0..oh.min(pl.height) {
                let base = pl.pitch * (1 + y);
                out.extend(pl.bufs[buf_sel][base..base + ow].iter().map(|&v| (v & 0x7F) << 1));
            }
            planes.push(out);
        }
        Ok(Some(Picture { width: w as u32, height: h as u32, planes, key: frame_flags & BS_KEYFRAME != 0 }))
    }
}

fn decode_plane(t: &Tables, plane: &mut Plane, buf_sel: usize, cb_offset: usize, alt_quant: [u8; 16], data: &[u8], strip_width: i32) -> Result<(), Code> {
    let nv = le32(data, 0).ok_or(Code::Invalid)? as usize;
    let pd = &data[4..];
    if nv > 256 || nv * 2 > pd.len() {
        return Err(Code::Invalid);
    }
    let (pw, ph) = ((plane.width >> 2) as i32, (plane.height >> 2) as i32);
    let mut pdec = PlaneDecoder {
        t,
        plane,
        buf_sel,
        cb_offset,
        alt_quant,
        pd,
        num_vectors: nv,
        bits: Bits { d: &pd[nv * 2..], pos: 0 },
        skip_bits: 0,
        need_resync: false,
        next_cell_data: 0,
    };
    let mut cell = Cell { xpos: 0, ypos: 0, width: pw, height: ph, tree: 0, mv: None };
    pdec.parse_bintree(INTRA_NULL, &mut cell, CELL_STACK_MAX, strip_width)
}
