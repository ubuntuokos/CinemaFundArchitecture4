// SPDX-License-Identifier: BSD-3-Clause-Clear
// Copyright (c) V-Nova International Limited 2014-2026. All rights reserved.
// Ported to Rust for CFA4 from LCEVCdec 4.3.1 (https://github.com/v-novaltd/LCEVCdec, commit
// e295d843). As LCEVCdec requires, this file stays under the BSD-3-Clause-Clear licence and the
// additional licensing information in `crates/lcevc/LICENSE.LCEVCdec`: NO PATENT LICENCES ARE
// GRANTED by it.
//! The LCEVC reconstruction pipeline (LCEVCdec `pipeline_cpu/src/tasks_cpu.cpp`, the scalar
//! paths of `pixel_processing`): base conversion, upscaling with predicted average, residuals
//! of both levels of quality and the temporal buffer.

use crate::Error;
use crate::bits::unencapsulate;
use crate::config::{self, Chroma, Global, Persistent, Scaling, Sequence};
use crate::residual::{self, Plane};

/// A planar picture: 8-bit samples or 10/12/14-bit samples, one `u16` per sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub chroma: Chroma,
    /// Y, then U and V unless `chroma` is `Mono`; rows tightly packed.
    pub planes: Vec<Vec<u16>>,
}

impl Picture {
    /// Size of `plane`.
    pub fn plane_size(&self, plane: usize) -> (u32, u32) {
        if plane == 0 {
            return (self.width, self.height);
        }
        let (sx, sy) = self.chroma.shifts();
        (self.width >> sx, self.height >> sy)
    }
}

/// Enhanced picture with its conformance window (left, right, top, bottom, in luma samples).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub picture: Picture,
    pub crop: [u32; 4],
}

/// LCEVC decoder: one call per picture, in decoding order.
#[derive(Debug, Default)]
pub struct Decoder {
    sequence: Sequence,
    global: Option<Global>,
    persistent: Persistent,
    temporal: Vec<Option<Plane>>,
}

/// `fpU16ToS16`: unsigned sample of `bits` bits to the signed internal range.
fn to_internal(v: u16, bits: u32) -> i16 {
    let shift = 15 - bits;
    ((i32::from(v) << shift) - 0x4000) as i16
}

/// `fpS16ToU16`: internal value (a sum of two internal values at most) back to `bits` bits.
fn from_internal(v: i32, bits: u32) -> u16 {
    let shift = 15 - bits;
    let round = 1i32 << (shift - 1);
    let max = (1i32 << bits) - 1;
    let r = (((v + round) >> shift) + (1 << (bits - 1))) as i16;
    i32::from(r).clamp(0, max) as u16
}

fn shift_round(v: i64) -> i16 {
    ((v + 0x2000) >> 14).clamp(-0x4000, 0x3FFF) as i16
}

fn sat16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// The scalar upscaler (`upscale_macros/scalar_1d_loop.h`, `scalar_2d_loop.h`).
fn upscale(src: &Plane, mode: Scaling, kernel: [u16; 4], pa: bool) -> Plane {
    let k = [-i64::from(kernel[0]), i64::from(kernel[1]), i64::from(kernel[2]), -i64::from(kernel[3])];
    let (w, h) = (src.width as i64, src.height as i64);
    let at = |x: i64, y: i64| i64::from(src.get(x.clamp(0, w - 1) as usize, y.clamp(0, h - 1) as usize));
    // Even output sample: s[-2]k3 + s[-1]k2 + s[0]k1 + s[1]k0; odd: s[-1]k0 + s[0]k1 + s[1]k2 + s[2]k3.
    let filt = |f: &dyn Fn(i64) -> i64, c: i64, odd: bool| -> i16 {
        let acc = if odd {
            f(c - 1) * k[0] + f(c) * k[1] + f(c + 1) * k[2] + f(c + 2) * k[3]
        } else {
            f(c - 2) * k[3] + f(c - 1) * k[2] + f(c) * k[1] + f(c + 1) * k[0]
        };
        shift_round(acc)
    };
    match mode {
        Scaling::None => src.clone(),
        Scaling::Horizontal => {
            let mut dst = Plane::new(src.width * 2, src.height);
            for y in 0..h {
                let row = |x: i64| at(x, y);
                for x in 0..w {
                    let mut o = [filt(&row, x, false), filt(&row, x, true)];
                    if pa {
                        let mean = (i32::from(o[0]) + i32::from(o[1]) + 1) >> 1;
                        let adjust = row(x) as i32 - mean;
                        o = o.map(|v| sat16(i32::from(v) + adjust));
                    }
                    for (dx, v) in o.into_iter().enumerate() {
                        dst.set(2 * x as usize + dx, y as usize, v);
                    }
                }
            }
            dst
        }
        Scaling::Both => {
            let mut dst = Plane::new(src.width * 2, src.height * 2);
            let mut vert = [vec![0i16; src.width], vec![0i16; src.width]];
            for y in 0..h {
                for (dy, line) in vert.iter_mut().enumerate() {
                    for (x, v) in line.iter_mut().enumerate() {
                        let col = |yy: i64| at(x as i64, yy);
                        *v = filt(&col, y, dy == 1);
                    }
                }
                for x in 0..w {
                    let mut o = [[0i16; 2]; 2];
                    for (dy, line) in vert.iter().enumerate() {
                        let row = |xx: i64| i64::from(line.get(xx.clamp(0, w - 1) as usize).copied().unwrap_or(0));
                        o[dy] = [filt(&row, x, false), filt(&row, x, true)];
                    }
                    if pa {
                        let mean = (i32::from(o[0][0]) + i32::from(o[0][1]) + i32::from(o[1][0]) + i32::from(o[1][1]) + 2) >> 2;
                        let adjust = at(x, y) as i32 - mean;
                        o = o.map(|r| r.map(|v| sat16(i32::from(v) + adjust)));
                    }
                    for (dy, r) in o.iter().enumerate() {
                        for (dx, &v) in r.iter().enumerate() {
                            dst.set(2 * x as usize + dx, 2 * y as usize + dy, v);
                        }
                    }
                }
            }
            dst
        }
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Size and format the base picture must have for the current configuration.
    pub fn base_format(&self) -> Option<(u32, u32, u32, Chroma)> {
        let g = self.global.as_ref()?;
        let (w, h) = g.plane_size(2, 0);
        Some((w, h, g.base_depth, g.chroma))
    }

    /// Size of the enhanced picture after its conformance window, from an LCEVC access unit that
    /// carries a global configuration (an IDR one); `None` otherwise.
    pub fn probe(nal: &[u8]) -> Option<(u32, u32)> {
        let (data, idr) = unencapsulate(nal).ok()?;
        let (mut seq, mut global, mut persistent) = (Sequence::default(), None, Persistent::default());
        config::parse(&data, idr, &mut seq, &mut global, &mut persistent).ok()?;
        let g = global?;
        let [l, r, t, b] = g.crop;
        Some((g.width.checked_sub(l.saturating_add(r))?, g.height.checked_sub(t.saturating_add(b))?))
    }

    /// Converts the base planes the configuration uses to the internal format. A monochrome
    /// enhancement uses only the luma plane, whatever the base chroma.
    fn base_planes(g: &Global, base: &Picture) -> Result<Vec<Plane>, Error> {
        let image_planes = if g.chroma == Chroma::Mono { 1 } else { 3 };
        let (bw, bh) = g.plane_size(2, 0);
        let chroma_ok = g.chroma == Chroma::Mono || base.chroma == g.chroma;
        if base.width != bw || base.height != bh || !chroma_ok || base.bit_depth != g.base_depth || base.planes.len() < image_planes {
            return Err(Error::Invalid("base picture does not match the enhancement configuration"));
        }
        (0..image_planes)
            .map(|p| {
                let (w2, h2) = base.plane_size(p);
                let src = base.planes.get(p).ok_or(Error::Invalid("base plane"))?;
                if src.len() != (w2 as usize) * (h2 as usize) {
                    return Err(Error::Invalid("base plane size"));
                }
                Ok(Plane { width: w2 as usize, height: h2 as usize, data: src.iter().map(|&v| to_internal(v, g.base_depth)).collect() })
            })
            .collect()
    }

    fn output(g: &Global, planes: Vec<Vec<u16>>) -> Decoded {
        let chroma = g.chroma;
        Decoded { picture: Picture { width: g.width, height: g.height, bit_depth: g.enhanced_depth, chroma, planes }, crop: g.crop }
    }

    /// The base picture upscaled with the last configuration, without residuals: what a picture
    /// whose enhancement data is missing or damaged shows, at the enhanced size.
    pub fn upscale_only(&self, base: &Picture) -> Result<Decoded, Error> {
        let g = self.global.as_ref().ok_or(Error::Invalid("no global configuration"))?;
        let planes = Self::base_planes(g, base)?
            .into_iter()
            .map(|cur| {
                let cur = upscale(&upscale(&cur, g.scaling[1], g.kernel, g.predicted_average), g.scaling[0], g.kernel, g.predicted_average);
                cur.data.iter().map(|&a| from_internal(i32::from(a), g.enhanced_depth)).collect()
            })
            .collect();
        Ok(Self::output(g, planes))
    }

    /// Forgets the temporal buffer and the configuration (after a seek).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Decodes one LCEVC access unit (an Annex B NAL unit, start code included) on top of the
    /// decoded `base` picture.
    pub fn decode(&mut self, nal: &[u8], base: &Picture) -> Result<Decoded, Error> {
        let (data, idr) = unencapsulate(nal)?;
        let frame = config::parse(&data, idr, &mut self.sequence, &mut self.global, &mut self.persistent)?;
        let g = self.global.as_ref().ok_or(Error::Invalid("no global configuration"))?;
        if !frame.set {
            return Err(Error::Invalid("no picture configuration"));
        }
        let planes = Self::base_planes(g, base)?;
        if self.temporal.len() != planes.len() {
            self.temporal = vec![None; planes.len()];
        }
        let mut out_planes = Vec::with_capacity(planes.len());
        for (p, cur) in planes.into_iter().enumerate() {
            let enhanced = p < g.num_planes;
            let mut cur = upscale(&cur, g.scaling[1], g.kernel, g.predicted_average);
            if enhanced && frame.loq_enabled[1] {
                residual::decode(g, &frame, &data, &self.persistent.qm, p, 1, &mut cur)?;
            }
            cur = upscale(&cur, g.scaling[0], g.kernel, g.predicted_average);
            let out: Vec<u16> = if g.temporal_enabled && enhanced {
                let slot = self.temporal.get_mut(p).ok_or(Error::Invalid("temporal buffer"))?;
                let fresh = frame.idr || frame.temporal_refresh;
                let tb = match slot {
                    Some(t) if !fresh && t.width == cur.width && t.height == cur.height => t,
                    _ => slot.insert(Plane::new(cur.width, cur.height)),
                };
                if frame.loq_enabled[0] {
                    residual::decode(g, &frame, &data, &self.persistent.qm, p, 0, tb)?;
                }
                cur.data.iter().zip(&tb.data).map(|(&a, &b)| from_internal(i32::from(a) + i32::from(b), g.enhanced_depth)).collect()
            } else {
                if enhanced && frame.loq_enabled[0] {
                    residual::decode(g, &frame, &data, &self.persistent.qm, p, 0, &mut cur)?;
                }
                cur.data.iter().map(|&a| from_internal(i32::from(a), g.enhanced_depth)).collect()
            };
            out_planes.push(out);
        }
        Ok(Self::output(g, out_planes))
    }
}
