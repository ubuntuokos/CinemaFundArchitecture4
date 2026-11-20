// SPDX-License-Identifier: BSD-3-Clause-Clear
// Copyright (c) V-Nova International Limited 2014-2026. All rights reserved.
// Ported to Rust for CFA4 from LCEVCdec 4.3.1 (https://github.com/v-novaltd/LCEVCdec, commit
// e295d843). As LCEVCdec requires, this file stays under the BSD-3-Clause-Clear licence and the
// additional licensing information in `crates/lcevc/LICENSE.LCEVCdec`: NO PATENT LICENCES ARE
// GRANTED by it.
//! Residual decoding of one plane at one level of quality (LCEVCdec `decode.c`, `dequant.c`,
//! `transform.c`, `transform_unit.c`) and the application of the resulting commands
//! (`apply_cmdbuffer_scalar.c`).

use crate::Error;
use crate::config::{Frame, Global, Scaling};
use crate::entropy::{self, NO_DATA};

/// A plane of signed 16-bit internal samples.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plane {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) data: Vec<i16>,
}

impl Plane {
    pub(crate) fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0; width * height] }
    }

    pub(crate) fn get(&self, x: usize, y: usize) -> i16 {
        self.data.get(y * self.width + x).copied().unwrap_or(0)
    }

    pub(crate) fn set(&mut self, x: usize, y: usize, v: i16) {
        if x < self.width
            && let Some(p) = self.data.get_mut(y * self.width + x)
        {
            *p = v;
        }
    }
}

fn sat16(v: i32) -> i16 {
    i16::try_from(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}

/// `lnFpScaled`: `ln(x)` in fixed point (12 fractional bits) times `scale`, shifted back.
fn ln_fp_scaled(input: i32, scale: i32, round_to_zero: bool) -> i32 {
    const Q: u32 = 33;
    const LN2_Q33: i64 = 5_954_088_943;
    let one: i64 = 1 << Q;
    let x = i64::from(input.max(1));
    let n = 63 - x.leading_zeros() as i64;
    let mut k = n;
    let mut pow2: i64 = 1 << n;
    if x * x >= 2 * pow2 * pow2 {
        k += 1;
        pow2 <<= 1;
    }
    let y = ((x - pow2) * one) / (x + pow2);
    let y2 = (y * y) / one;
    let mut term = y;
    let mut sum = term;
    for i in 1..5 {
        term = (term * y2) / one;
        sum += term / (2 * i + 1);
    }
    let ln_q33 = k * LN2_Q33 + 2 * sum;
    let ln_q12 = ln_q33 >> (Q - 12);
    let prod = ln_q12 * i64::from(scale);
    let mut res = (prod >> 12) as i32;
    if round_to_zero {
        let exact = i64::from(res) * 4096 == prod;
        if res < 0 && !exact {
            res += 1;
        }
    }
    res
}

/// Step widths and offsets per temporal type (0 inter, 1 intra) and layer.
struct Dequant {
    step: [[i16; 16]; 2],
    offset: [[i16; 16]; 2],
}

fn dequant(g: &Global, f: &Frame, qm: &[u8; 16], plane: usize, loq: usize) -> Dequant {
    const A: i64 = 39;
    const B: i64 = 126_484;
    const C: i32 = 5242;
    const D: i64 = 99_614;
    let sw = f.step_widths.get(loq).copied().unwrap_or(32767);
    let loq_sw = if plane > 0 && loq == 0 { ((sw * i32::from(g.chroma_sw_multiplier)) >> 6).clamp(1, 32767) } else { sw.clamp(1, 32767) };
    let mut d = Dequant { step: [[0; 16]; 2], offset: [[0; 16]; 2] };
    for t in 0..2 {
        let mut temporal_sw = loq_sw;
        if t == 0 && loq == 0 && g.temporal_enabled && !f.temporal_refresh {
            // calculateFixedPointTemporalSW
            let modifier = (u32::from(g.temporal_sw_modifier) * 257).min(1 << 15) as u16;
            let mult = (1u32 << 16) - u32::from(modifier);
            let floored = (mult * temporal_sw as u32) >> 16;
            temporal_sw = i32::from(floored as i16).clamp(1, 32767);
        }
        for layer in 0..g.num_layers() {
            let mut layer_qm = i64::from(qm.get(layer).copied().unwrap_or(0)) * i64::from(temporal_sw);
            layer_qm += 1 << 16;
            layer_qm = layer_qm.clamp(0, 196_608);
            layer_qm *= i64::from(temporal_sw);
            layer_qm >>= 16;
            let mut layer_sw = layer_qm.clamp(1, 32767) as i32;
            // calculateDequantOffsetActual
            let offset_actual = if f.dequant_offset == -1 || f.dequant_offset == 0 {
                0
            } else {
                let log_layer = ln_fp_scaled(layer_sw, -C, true);
                let log_master = ln_fp_scaled(temporal_sw, C, true);
                let base = if f.dequant_offset_const { i64::from(f.dequant_offset) << 9 } else { i64::from(f.dequant_offset) << 11 };
                (((i64::from(log_layer) + base + i64::from(log_master)) * i64::from(layer_sw)) >> 16) as i32
            };
            // calculateStepWidthModifier
            let modifier = if f.dequant_offset == -1 {
                let l = D + i64::from(ln_fp_scaled(layer_sw, -C, false));
                ((l * i64::from(layer_sw) * i64::from(layer_sw)) / 2_147_483_648) as i32
            } else if !f.dequant_offset_const {
                ((i64::from(offset_actual) * i64::from(layer_sw)) / 32768) as i32
            } else {
                0
            };
            layer_sw = (layer_sw + modifier).clamp(1, 32767);
            // calculateDeadzoneWidth
            let deadzone = if temporal_sw <= 16 {
                temporal_sw >> 1
            } else {
                let scaled = (1i64 << 16) - ((A * i64::from(layer_sw) + B) >> 1);
                ((scaled * i64::from(layer_sw)) >> 16) as i32
            };
            let applied = if f.dequant_offset == -1 || !f.dequant_offset_const { -i64::from(deadzone) } else { i64::from(offset_actual) - i64::from(deadzone) };
            if let (Some(s), Some(o)) = (d.step.get_mut(t).and_then(|r| r.get_mut(layer)), d.offset.get_mut(t).and_then(|r| r.get_mut(layer))) {
                *s = layer_sw as i16;
                *o = applied.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
            }
        }
    }
    d
}

fn transform(dds: bool, one_d: bool, c: &[i16; 16], r: &mut [i16; 16]) {
    let c: [i32; 16] = c.map(i32::from);
    if !dds {
        let out = if one_d {
            [c[0] + c[1] + c[2], c[0] - c[1] - c[2], c[3] + c[1] - c[2], c[3] - c[1] + c[2]]
        } else {
            [c[0] + c[1] + c[2] + c[3], c[0] - c[1] + c[2] - c[3], c[0] + c[1] - c[2] - c[3], c[0] - c[1] - c[2] + c[3]]
        };
        for (o, v) in r.iter_mut().zip(out) {
            *o = sat16(v);
        }
        return;
    }
    let g = |i: usize| -> [i32; 4] {
        let b = 4 * i;
        let (p, q, s, t) = (c[b], c[b + 1], c[b + 2], c[b + 3]);
        [p + q + s + t, p - q + s - t, p + q - s - t, p - q - s + t]
    };
    // a, h, v, d over the four groups
    let groups = [g(0), g(1), g(2), g(3)];
    for k in 0..4 {
        let x = [groups[0][k], groups[1][k], groups[2][k], groups[3][k]];
        let out = if one_d {
            [x[0] + x[1] + x[3], x[0] - x[1] - x[3], x[1] + x[2] - x[3], x[2] - x[1] + x[3]]
        } else {
            [x[0] + x[1] + x[2] + x[3], x[0] - x[1] + x[2] - x[3], x[0] + x[1] - x[2] - x[3], x[0] - x[1] - x[2] + x[3]]
        };
        for (j, v) in out.into_iter().enumerate() {
            r[4 * k + j] = sat16(v);
        }
    }
}

/// Transform unit layout (`TUState`), without tiles.
struct Tu {
    shift: u32,
    across: u32,
    total: u32,
    dims_shift: u32,
    dims: u32,
    per_block_shift: u32,
    per_block: u32,
    right_edge: u32,
    bottom_edge_col: u32,
    bottom_edge: u32,
    per_row: u32,
    whole_per_row: u32,
    whole_per_col: u32,
    max_whole_tu: u32,
}

impl Tu {
    fn new(width: u32, height: u32, shift: u32) -> Option<Self> {
        let size = 1 << shift;
        if !width.is_multiple_of(size) || !height.is_multiple_of(size) {
            return None;
        }
        let across = width >> shift;
        let dims_shift = if shift == 1 { 4 } else { 3 };
        let per_row = across << dims_shift;
        Some(Self {
            shift,
            across,
            total: across * (height >> shift),
            dims_shift,
            dims: 1 << dims_shift,
            per_block_shift: dims_shift * 2,
            per_block: 1 << (dims_shift * 2),
            right_edge: (width & 31) >> shift,
            bottom_edge_col: (height & 31) >> shift,
            bottom_edge: ((height & 31) >> shift) << dims_shift,
            per_row,
            whole_per_row: width >> 5,
            whole_per_col: height >> 5,
            max_whole_tu: (height >> 5) * per_row,
        })
    }

    fn raster(&self, i: u32) -> (u32, u32) {
        ((i % self.across) << self.shift, (i / self.across) << self.shift)
    }

    /// `ldeTuCoordsBlockRaster`.
    fn block_raster(&self, i: u32) -> (u32, u32) {
        let row = i / self.per_row;
        let row_tu = i - row * self.per_row;
        let (col, in_block) = if row >= self.whole_per_col {
            let e = self.bottom_edge.max(1);
            (row_tu / e, row_tu % e)
        } else {
            let col = row_tu >> self.per_block_shift;
            (col, row_tu - (col << self.per_block_shift))
        };
        let (mut tx, mut ty) = if col >= self.whole_per_row {
            let e = self.right_edge.max(1);
            (in_block % e, in_block / e)
        } else {
            let ty = in_block >> self.dims_shift;
            (in_block - (ty << self.dims_shift), ty)
        };
        tx += col << self.dims_shift;
        ty += row << self.dims_shift;
        (tx << self.shift, ty << self.shift)
    }

    /// `ldeTuCoordsBlockTuCount`.
    fn block_tu_count(&self, i: u32) -> u32 {
        let right_limit = self.whole_per_row << self.per_block_shift;
        let wide = if i % self.per_row >= right_limit { self.right_edge } else { self.dims };
        let high = if i >= self.max_whole_tu { self.bottom_edge_col } else { self.dims };
        wide * high
    }

    /// `ldeTuIsBlockStart`.
    fn is_block_start(&self, i: u32) -> bool {
        if i >= self.max_whole_tu {
            return (i - self.max_whole_tu).is_multiple_of(self.bottom_edge.max(1));
        }
        (i % self.per_row).is_multiple_of(self.per_block)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmd {
    Add,
    Set,
    SetZero,
}

/// Applies one transform unit's residuals (raster order) at `(x, y)`.
fn apply(target: &mut Plane, cmd: Cmd, x: u32, y: u32, size: u32, res: &[i16; 16]) {
    for row in 0..size {
        for col in 0..size {
            let r = res.get((row * size + col) as usize).copied().unwrap_or(0);
            let (px, py) = ((x + col) as usize, (y + row) as usize);
            let v = match cmd {
                Cmd::Add => sat16(i32::from(target.get(px, py)) + i32::from(r)),
                Cmd::Set => r,
                Cmd::SetZero => 0,
            };
            target.set(px, py, v);
        }
    }
}

fn clear_block(target: &mut Plane, x: u32, y: u32) {
    let (x, y) = (x as usize, y as usize);
    let h = 32.min(target.height.saturating_sub(y));
    let w = 32.min(target.width.saturating_sub(x));
    for row in y..y + h {
        for col in x..x + w {
            target.set(col, row, 0);
        }
    }
}

/// Decodes the residuals of `plane` at `loq` (0 or 1) and applies them to `target` (the
/// temporal buffer for LOQ0 with temporal prediction, otherwise the picture itself).
pub(crate) fn decode(g: &Global, f: &Frame, data: &[u8], qm: &[[u8; 16]; 2], plane: usize, loq: usize, target: &mut Plane) -> Result<(), Error> {
    let chunks = f.residual.get(plane).and_then(|p| p.get(loq));
    let qm_loq = qm.get(loq).ok_or(Error::Invalid("quant matrix"))?;
    let dq = dequant(g, f, qm_loq, plane, loq);
    let temporal_enabled = g.temporal_enabled;
    let temporal_refresh = temporal_enabled && f.temporal_refresh;
    let num_layers = g.num_layers();
    let dds = g.dds;
    let tu_shift = if dds { 2 } else { 1 };
    let tu_size = 1u32 << tu_shift;
    let raster_order = !temporal_enabled;
    let one_d = g.scaling.get(loq).copied() == Some(Scaling::Horizontal);
    let (w, h) = g.plane_size(loq, plane);
    let tu = Tu::new(w, h, tu_shift).ok_or(Error::Invalid("plane is not a whole number of transforms"))?;
    if tu.total == 0 {
        return Ok(());
    }
    let total = i32::try_from(tu.total).map_err(|_| Error::Unsupported("picture too large"))?;
    let chunk_bytes = |c: &crate::config::Chunk| data.get(c.start..c.start.saturating_add(c.size)).unwrap_or(&[]);

    let has_entropy = f.entropy_enabled;
    let mut layers: Vec<entropy::Residual<'_>> = Vec::with_capacity(num_layers);
    if has_entropy {
        let chunks = chunks.ok_or(Error::Invalid("missing residual chunks"))?;
        for c in chunks.iter().take(num_layers) {
            layers.push(entropy::Residual::new(c.enabled, c.rle_only, chunk_bytes(c))?);
        }
    }
    let mut temporal_dec = if loq == 0 && f.temporal_signalling_present {
        let c = f.temporal.get(plane).ok_or(Error::Invalid("missing temporal chunk"))?;
        Some(entropy::Temporal::new(c.enabled, c.rle_only, chunk_bytes(c))?)
    } else {
        None
    };
    let has_temporal = temporal_dec.is_some();

    let mut coeffs = [0i16; 16];
    let mut residuals = [0i16; 16];
    let mut next_event = [0i32; 16];
    let mut min_next: i32 = 0;
    let mut expired: u32 = (1u32 << num_layers) - 1;
    let mut temporal_run: i32 = 0;
    let mut tu_index: u32 = 0;
    let mut intra = false;
    let mut clear_queue: i32 = 0;
    let mut clear_remainder = false;

    loop {
        let ti = i32::try_from(tu_index).unwrap_or(i32::MAX);
        // entropyDecodeAllLayers
        let mut nonzero: u32 = 0;
        if ti == min_next {
            coeffs = [0; 16];
            let mut mask = expired;
            while mask != 0 {
                let layer = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                let (Some(slot), Some(next)) = (coeffs.get_mut(layer), next_event.get_mut(layer)) else {
                    break;
                };
                if has_entropy {
                    let dec = layers.get_mut(layer).ok_or(Error::Invalid("layer"))?;
                    let mut zeros = dec.decode(slot)?;
                    if zeros == NO_DATA {
                        zeros = total - 1;
                    }
                    if zeros < 0 {
                        return Err(Error::Invalid("residual decode"));
                    }
                    *next = ti + zeros.min(total - 1) + 1;
                    if *slot != 0 {
                        nonzero |= 1 << layer;
                    }
                } else {
                    *next = ti + total;
                }
            }
            let mut m = next_event[0];
            let mut e = 1u32;
            for layer in 1..num_layers {
                let n = next_event.get(layer).copied().unwrap_or(i32::MAX);
                if n < m {
                    m = n;
                    e = 1 << layer;
                } else if n == m {
                    e |= 1 << layer;
                }
            }
            min_next = m;
            expired = e;
        }
        let mut min_zero = min_next - ti - 1;

        if let Some((index, shift)) = g.user_data
            && loq == 1
            && let Some(c) = coeffs.get_mut(index)
        {
            let packed = *c as u16;
            let signed_mag = packed >> shift;
            let sign = if signed_mag & 1 != 0 { -1 } else { 1 };
            *c = (i32::from(signed_mag >> 1) * sign) as i16;
        }

        let block_start = has_temporal && tu.is_block_start(tu_index);
        if clear_queue == 0
            && temporal_enabled
            && let Some(td) = temporal_dec.as_mut()
        {
            if temporal_run <= 0 {
                clear_remainder = false;
                loop {
                    let (i, run) = td.decode()?;
                    intra = i;
                    temporal_run = if run == NO_DATA || run > total { total } else { run };
                    if temporal_run != 0 {
                        break;
                    }
                }
                if temporal_run < 0 {
                    return Err(Error::Invalid("temporal decode"));
                }
            }
            temporal_run -= 1;
            if block_start && intra && g.temporal_reduced_signalling {
                clear_queue = temporal_run + 1;
                temporal_run = 0;
                let mut block = clear_queue;
                while block > 0 {
                    let idx = tu_index.saturating_add(u32::try_from(temporal_run).unwrap_or(0));
                    temporal_run += i32::try_from(tu.block_tu_count(idx)).unwrap_or(0);
                    block -= 1;
                }
            }
        }

        let block_tu_count = if has_temporal { i32::try_from(tu.block_tu_count(tu_index)).unwrap_or(0) } else { 0 };
        let (x, y) = if raster_order { tu.raster(tu_index) } else { tu.block_raster(tu_index) };
        let mut cleared = false;
        if block_start && clear_queue > 0 {
            clear_block(target, x, y);
            cleared = true;
            clear_queue -= 1;
            if clear_queue == 0 {
                clear_remainder = true;
            }
        }

        if nonzero != 0 || (!cleared && (!temporal_enabled || intra)) {
            let t = usize::from(intra);
            if nonzero != 0 {
                let (Some(steps), Some(offs)) = (dq.step.get(t), dq.offset.get(t)) else {
                    return Err(Error::Invalid("dequant"));
                };
                for ((c, &s), &o) in coeffs.iter_mut().zip(steps).zip(offs).take(num_layers) {
                    let v = i32::from(*c) * i32::from(s);
                    if *c > 0 {
                        *c = sat16(v + i32::from(o));
                    } else if *c < 0 {
                        *c = sat16(v - i32::from(o));
                    }
                }
                transform(dds, one_d, &coeffs, &mut residuals);
                if loq == 1 && dds && f.deblock_enabled {
                    let (corner, side) = (g.deblock_corner as i32, g.deblock_side as i32);
                    for (i, k) in [
                        (0, corner),
                        (1, side),
                        (4, side),
                        (5, corner),
                        (2, side),
                        (7, side),
                        (8, side),
                        (13, side),
                        (10, corner),
                        (11, side),
                        (14, side),
                        (15, corner),
                    ] {
                        if let Some(r) = residuals.get_mut(i) {
                            *r = ((k * i32::from(*r)) >> 4) as i16;
                        }
                    }
                }
            } else {
                residuals = [0; 16];
            }
            let cmd = if nonzero == 0 && intra {
                Cmd::SetZero
            } else if loq == 0 && (temporal_refresh || intra || clear_queue > 0 || clear_remainder) {
                Cmd::Set
            } else {
                Cmd::Add
            };
            // Residual raster order: DD r0 r1 / r2 r3; DDS rows r0 r1 r4 r5, r2 r3 r6 r7,
            // r8 r9 r12 r13, r10 r11 r14 r15.
            let raster: [i16; 16] = if dds {
                let r = residuals;
                [r[0], r[1], r[4], r[5], r[2], r[3], r[6], r[7], r[8], r[9], r[12], r[13], r[10], r[11], r[14], r[15]]
            } else {
                residuals
            };
            apply(target, cmd, x, y, tu_size, &raster);
        }

        if has_temporal {
            if cleared {
                min_zero = min_zero.min(block_tu_count - 1);
                temporal_run -= min_zero + 1;
            } else if clear_queue > 0 {
                let mut next_block_start = block_tu_count - 1;
                if tu_index >= tu.max_whole_tu {
                    next_block_start -= i32::try_from((tu_index - tu.max_whole_tu) % (block_tu_count.max(1) as u32)).unwrap_or(0);
                } else {
                    next_block_start -= i32::try_from((tu_index % tu.per_row) % tu.per_block).unwrap_or(0);
                }
                min_zero = next_block_start.min(min_zero);
                temporal_run -= min_zero + 1;
            } else if !intra || (clear_remainder && min_zero > temporal_run) {
                min_zero = min_zero.min(temporal_run);
                temporal_run -= min_zero;
            } else if !clear_remainder {
                min_zero = 0;
            } else {
                temporal_run -= min_zero;
            }
        }
        let step = u32::try_from(min_zero.max(0)).unwrap_or(0).saturating_add(1);
        tu_index = tu_index.saturating_add(step);
        if tu_index >= tu.total {
            break;
        }
    }
    Ok(())
}
