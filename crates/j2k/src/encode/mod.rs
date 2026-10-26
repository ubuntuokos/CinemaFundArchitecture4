//! JPEG 2000 Part 1 encoder (clean-room, ITU-T T.800): one tile, one quality layer.
//!
//! The pipeline is the decoder's run backwards: DC level shift and the RCT / ICT (Annex G), the
//! forward DWT ([`dwt`], Annex F), scalar dead-zone quantization with expounded step sizes
//! (Annex E), EBCOT tier-1 ([`t1`], Annexes C and D), post-compression rate-distortion
//! optimisation (each code-block truncated on its convex hull, one slope threshold for the
//! image) and tier-2 ([`t2`], Annex B). The geometry, packet order and step sizes come from the
//! decoder's own code ([`crate::decode::build`], [`crate::decode::packets`]), parsed from the
//! header this writes, so both sides agree by construction.
//!
//! [`EncodeOptions::cinema_2k`] and [`EncodeOptions::cinema_4k`] give the Digital Cinema
//! profiles (T.800 Amd. 1, SMPTE ST 429-4 / DCI): 12-bit X'Y'Z', 9/7, 32×32 code-blocks, CPRL,
//! 128 / 256 precincts, one tile-part per component (and per progression for 4K), TLM, and the
//! DCI byte limits per frame and per component.

mod dwt;
mod t1;
mod t2;

use crate::codestream::{self, Quant};
use crate::decode::{self, TileComp, map, quant};
use crate::dwt::{Rect, Sample, level as synthesis};
use crate::{Error, Image, MAX_SAMPLES, Result};
use t2::Contribution;

/// The forward lifting of a coefficient type ([`dwt`]).
pub(crate) trait Coeff: Copy + Default + Send + Sync {
    /// One-dimensional analysis of `n` rows of `w` samples, along the rows' index.
    fn analysis_rows(data: &mut [Self], w: usize, n: usize);
}

/// Packet progression order (A.6.1, B.12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progression {
    Lrcp,
    Rlcp,
    Rpcl,
    Pcrl,
    Cprl,
}

/// Codestream profile (Rsiz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Part 1 without restrictions (Rsiz 0).
    Part1,
    /// Digital Cinema 2K (Rsiz 3): at most 2048 × 1080, at most 5 decomposition levels.
    Cinema2k,
    /// Digital Cinema 4K (Rsiz 4): at most 4096 × 2160, 1-6 levels, the 4K resolution in its own
    /// progression (POC).
    Cinema4k,
}

/// Encoder settings. Start from [`EncodeOptions::lossless`], [`EncodeOptions::lossy`] or a
/// cinema preset.
#[derive(Clone, Debug, PartialEq)]
pub struct EncodeOptions {
    /// Reversible 5/3 wavelet (and RCT): lossless when nothing is truncated. Otherwise the
    /// irreversible 9/7 (and ICT).
    pub reversible: bool,
    /// Decomposition levels, 0-32.
    pub levels: u8,
    pub progression: Progression,
    /// Code-block size exponents (xcb, ycb): 2-10 each, sum at most 12.
    pub code_block: (u8, u8),
    /// Precinct size exponents (PPx, PPy) per resolution, lowest first; the last repeats. Empty
    /// for the maximal precincts (2^15).
    pub precincts: Vec<(u8, u8)>,
    /// The multiple-component transform on the first three components (same size needed).
    pub mct: bool,
    /// Irreversible only: the quantization step of the image in sample units before rate control
    /// (each subband gets it divided by its synthesis norm). Clamped to 2^-8 ..= 2^16.
    pub step: f32,
    /// Byte limit of the whole codestream.
    pub max_bytes: Option<usize>,
    /// Byte limit of each component (its packets and tile-part headers).
    pub max_component_bytes: Option<usize>,
    /// One tile-part per component (and per progression change) instead of one for the tile.
    pub tile_part_per_component: bool,
    /// A TLM marker (A.7.1) with the tile-part lengths.
    pub tlm: bool,
    pub profile: Profile,
}

impl EncodeOptions {
    /// Reversible 5/3, 5 levels, 64×64 code-blocks, LRCP: decodes to the input exactly.
    pub fn lossless() -> Self {
        EncodeOptions {
            reversible: true,
            levels: 5,
            progression: Progression::Lrcp,
            code_block: (6, 6),
            precincts: Vec::new(),
            mct: true,
            step: 1.0,
            max_bytes: None,
            max_component_bytes: None,
            tile_part_per_component: false,
            tlm: false,
            profile: Profile::Part1,
        }
    }

    /// Irreversible 9/7 with quantization step `step` (sample units), no byte limit.
    pub fn lossy(step: f32) -> Self {
        EncodeOptions { reversible: false, step, ..Self::lossless() }
    }

    fn cinema(profile: Profile, levels: u8, fps: u32) -> Self {
        // DCI: 250 Mbit/s for the frame, 200 Mbit/s for each component
        let fps = fps.max(1) as usize;
        EncodeOptions {
            reversible: false,
            levels,
            progression: Progression::Cprl,
            code_block: (5, 5),
            precincts: vec![(7, 7), (8, 8)],
            mct: false,
            step: 1.0,
            max_bytes: Some(250_000_000 / 8 / fps),
            max_component_bytes: Some(200_000_000 / 8 / fps),
            tile_part_per_component: true,
            tlm: true,
            profile,
        }
    }

    /// Digital Cinema 2K at `fps` frames per second: 1 302 083 bytes per frame and 1 041 666 per
    /// component at 24.
    pub fn cinema_2k(fps: u32) -> Self {
        Self::cinema(Profile::Cinema2k, 5, fps)
    }

    /// Digital Cinema 4K at `fps` frames per second.
    pub fn cinema_4k(fps: u32) -> Self {
        Self::cinema(Profile::Cinema4k, 6, fps)
    }
}

/// Coefficients of one tile-component, per resolution and band (decoder order).
enum Coeffs {
    Int(Vec<Vec<Vec<i32>>>),
    Float(Vec<Vec<Vec<f32>>>),
}

/// A code-block to code: component, resolution, band, rectangle in band coordinates.
struct Job {
    c: usize,
    r: usize,
    bi: usize,
    rect: Rect,
    /// Width of the band (its buffer's rows).
    band_w: usize,
    orient: u8,
    step: f32,
    /// Largest index magnitude the header can carry (2^Mb - 1, at most 2^30 - 1).
    limit: u32,
}

/// A coded code-block and its rate-distortion hull: (passes, bytes, slope), slopes falling.
struct Coded {
    c: usize,
    zero: u32,
    enc: t1::Encoded,
    hull: Vec<(u32, usize, f64)>,
}

/// Encodes `image` as a JPEG 2000 codestream (`.j2c`).
pub fn encode(image: &Image, opts: &EncodeOptions) -> Result<Vec<u8>> {
    validate(image, opts)?;
    let ncomp = image.components.len();
    let levels = opts.levels;
    let mct = opts.mct && mct_possible(image);
    // samples: level shifted, component transformed
    let coeffs = if opts.reversible {
        let mut planes: Vec<Vec<i32>> = image.components.iter().map(shifted).collect();
        if mct && let [r, g, b, ..] = &mut planes[..] {
            // RCT (G.2): Y = (R + 2G + B) >> 2, U = B - G, V = R - G
            for ((r, g), b) in r.iter_mut().zip(g.iter_mut()).zip(b.iter_mut()) {
                let (rr, gg, bb) = (*r, *g, *b);
                (*r, *g, *b) = ((rr + 2 * gg + bb) >> 2, bb - gg, rr - gg);
            }
        }
        let jobs: Vec<_> = image.components.iter().zip(planes).collect();
        map(&jobs, |(p, plane)| Ok(Coeffs::Int(analysis(plane, p.width as usize, p.height as usize, levels))))?
    } else {
        let mut planes: Vec<Vec<f32>> = image.components.iter().map(|p| shifted(p).into_iter().map(|v| v as f32).collect()).collect();
        if mct && let [r, g, b, ..] = &mut planes[..] {
            // ICT (G.3)
            for ((r, g), b) in r.iter_mut().zip(g.iter_mut()).zip(b.iter_mut()) {
                let (rr, gg, bb) = (*r, *g, *b);
                *r = 0.299 * rr + 0.587 * gg + 0.114 * bb;
                *g = -0.168_736 * rr - 0.331_264 * gg + 0.5 * bb;
                *b = 0.5 * rr - 0.418_688 * gg - 0.081_312 * bb;
            }
        }
        let jobs: Vec<_> = image.components.iter().zip(planes).collect();
        map(&jobs, |(p, plane)| Ok(Coeffs::Float(analysis(plane, p.width as usize, p.height as usize, levels))))?
    };
    // step sizes and synthesis norms per band (decoder band order)
    let norms = band_norms(levels, opts.reversible);
    let base = opts.step.clamp(1.0 / 256.0, 65536.0);
    let mut quants: Vec<Quant> = image
        .components
        .iter()
        .map(|p| {
            let steps = norms
                .iter()
                .zip(gains(levels))
                .map(|(&n, gain)| {
                    let rb = i32::from(p.precision) + gain;
                    if opts.reversible { (rb.clamp(0, 31) as u8, 0) } else { expounded(f64::from(base) / n.max(1e-9), rb) }
                })
                .collect();
            Quant { style: if opts.reversible { 0 } else { 2 }, guard: 1, steps }
        })
        .collect();
    // guard bits: enough for the largest index of every band
    let mut guard = 2u8;
    for ((p, q), cf) in image.components.iter().zip(&quants).zip(&coeffs) {
        for band in 0..q.steps.len() {
            let (r, bi) = band_position(band);
            let (e, _) = q.step(band, levels)?;
            let orient = if band == 0 { 0 } else { (bi + 1) as u8 };
            let (_, step) = quant(q, band, levels, p.precision, orient)?;
            let max = match cf {
                Coeffs::Int(b) => b.get(r).and_then(|x| x.get(bi)).map_or(0, |v| v.iter().map(|c| c.unsigned_abs()).max().unwrap_or(0)),
                Coeffs::Float(b) => b.get(r).and_then(|x| x.get(bi)).map_or(0, |v| v.iter().map(|c| (c.abs() / step) as u32).max().unwrap_or(0)),
            };
            let planes = 32 - max.leading_zeros();
            let need = (i64::from(planes) + 1 - i64::from(e)).clamp(0, 7) as u8;
            guard = guard.max(need);
        }
    }
    for q in &mut quants {
        q.guard = guard;
    }
    // the header, read back by the decoder for the geometry
    let main = main_header(image, opts, &quants, mct)?;
    let mut probe = main.clone();
    probe.extend_from_slice(&[0xFF, 0x90, 0, 10, 0, 0, 0, 0, 0, 14, 0, 1, 0xFF, 0x93, 0xFF, 0xD9]);
    let cs = codestream::parse(&probe, true)?;
    let tile = cs.tiles.first().ok_or(Error::Invalid("encoder header"))?;
    let (trect, comps) = decode::build(&cs.header, &tile.params, 0)?;
    let order = decode::packets(&cs.header, trect, &comps, &tile.params)?;
    // code-blocks, in component / resolution / precinct / band / block order
    let mut jobs = Vec::new();
    for (c, tc) in comps.iter().enumerate() {
        for (r, res) in tc.res.iter().enumerate() {
            for prec in &res.precincts {
                for (bi, pb) in prec.iter().enumerate() {
                    let band = res.bands.get(bi).ok_or(Error::Invalid("subband"))?;
                    for cb in &pb.blocks {
                        let rect = Rect { u0: cb.x0 - band.rect.u0, v0: cb.y0 - band.rect.v0, u1: cb.x1 - band.rect.u0, v1: cb.y1 - band.rect.v0 };
                        let limit = if band.mb >= 30 { (1 << 30) - 1 } else { (1u32 << band.mb) - 1 };
                        jobs.push(Job { c, r, bi, rect, band_w: band.rect.w(), orient: band.orient, step: band.step, limit });
                    }
                }
            }
        }
    }
    let weights = component_weights(ncomp, mct, opts.reversible);
    let coded = map(&jobs, |j| {
        let cf = coeffs.get(j.c).ok_or(Error::Invalid("component"))?;
        let mb = comps.get(j.c).and_then(|tc| tc.res.get(j.r)).and_then(|res| res.bands.get(j.bi)).map_or(0, |b| b.mb);
        let enc = code_block(cf, j)?;
        let band = band_index(j.r, j.bi);
        let norm = norms.get(band).copied().unwrap_or(1.0);
        let step = if opts.reversible { 1.0 } else { f64::from(j.step) };
        let w = (norm * step).powi(2) * weights.get(j.c).copied().unwrap_or(1.0);
        Ok(Coded { c: j.c, zero: mb.saturating_sub(enc.planes), hull: hull(&enc.passes, w), enc })
    })?;
    // rate control
    let all: Vec<u32> = coded.iter().map(|b| b.enc.passes.len() as u32).collect();
    let measure = |sel: &[u32]| assemble(&main, opts, &comps, &order, &coded, sel, false).map(|(_, total, sizes)| (total, sizes));
    let fits =
        |total: usize, sizes: &[usize]| opts.max_bytes.is_none_or(|m| total <= m) && opts.max_component_bytes.is_none_or(|m| sizes.iter().all(|&s| s <= m));
    let (total, sizes) = measure(&all)?;
    let sel = if fits(total, &sizes) {
        all
    } else {
        // per component: the smallest slope threshold within its own limit
        let mut floor = vec![f64::NEG_INFINITY; ncomp];
        if let Some(limit) = opts.max_component_bytes {
            for (c, f) in floor.iter_mut().enumerate() {
                if sizes.get(c).is_none_or(|&s| s <= limit) {
                    continue;
                }
                let slopes = slopes(coded.iter().filter(|b| b.c == c));
                let k = search(slopes.len(), |k| {
                    let mut th = vec![f64::INFINITY; ncomp];
                    th[c] = threshold(&slopes, k);
                    let (_, s) = measure(&select(&coded, &th))?;
                    Ok(s.get(c).is_some_and(|&s| s <= limit))
                })?;
                *f = threshold(&slopes, k);
            }
        }
        let slopes = slopes(coded.iter());
        let at = |k: Option<usize>| -> Vec<f64> { floor.iter().map(|&f| f.max(threshold(&slopes, k))).collect() };
        let k = search(slopes.len(), |k| {
            let (total, sizes) = measure(&select(&coded, &at(k)))?;
            Ok(fits(total, &sizes))
        })?;
        select(&coded, &at(k))
    };
    let (out, total, sizes) = assemble(&main, opts, &comps, &order, &coded, &sel, true)?;
    if out.len() != total || !fits(total, &sizes) {
        return Err(Error::Invalid("byte limit below the size of the headers"));
    }
    Ok(out)
}

/// Checks the image and the options before any work.
fn validate(image: &Image, opts: &EncodeOptions) -> Result<()> {
    let (w, h) = (image.width, image.height);
    if w == 0 || h == 0 || image.components.is_empty() {
        return Err(Error::Invalid("empty image"));
    }
    if image.components.len() > 16384 {
        return Err(Error::Invalid("more than 16384 components"));
    }
    let mut samples = 0u64;
    for p in &image.components {
        if p.precision == 0 || p.precision > 16 {
            return Err(Error::Unsupported("component precision outside 1-16 bits"));
        }
        if p.dx == 0 || p.dy == 0 {
            return Err(Error::Invalid("component sub-sampling of zero"));
        }
        if p.width != w.div_ceil(u32::from(p.dx)) || p.height != h.div_ceil(u32::from(p.dy)) {
            return Err(Error::Invalid("component size does not match the image and its sub-sampling"));
        }
        let n = u64::from(p.width) * u64::from(p.height);
        if p.data.len() as u64 != n {
            return Err(Error::Invalid("component sample count"));
        }
        samples = samples.saturating_add(n);
    }
    if samples > MAX_SAMPLES {
        return Err(Error::TooLarge);
    }
    let (xcb, ycb) = opts.code_block;
    if !(2..=10).contains(&xcb) || !(2..=10).contains(&ycb) || xcb + ycb > 12 {
        return Err(Error::Invalid("code-block size"));
    }
    if opts.levels > 32 {
        return Err(Error::Invalid("more than 32 decomposition levels"));
    }
    // precincts as COD gives them: up to 2^15, at least 2 × 2 above resolution 0
    let precinct = |r: usize| opts.precincts.get(r).or(opts.precincts.last()).copied().unwrap_or((15, 15));
    if (0..=usize::from(opts.levels)).any(|r| precinct(r).0 > 15 || precinct(r).1 > 15 || r > 0 && (precinct(r).0 == 0 || precinct(r).1 == 0)) {
        return Err(Error::Invalid("precinct size"));
    }
    let (max_w, max_h, levels) = match opts.profile {
        Profile::Part1 => return Ok(()),
        Profile::Cinema2k => (2048, 1080, 1..=5),
        Profile::Cinema4k => (4096, 2160, 1..=6),
    };
    let dci = image.components.len() == 3 && image.components.iter().all(|p| p.precision == 12 && !p.signed && p.dx == 1 && p.dy == 1);
    if !dci
        || w > max_w
        || h > max_h
        || !levels.contains(&opts.levels)
        || opts.reversible
        || opts.mct
        || opts.code_block != (5, 5)
        || opts.progression != Progression::Cprl
    {
        return Err(Error::Invalid("image or options outside the Digital Cinema profile"));
    }
    Ok(())
}

fn mct_possible(image: &Image) -> bool {
    let c = &image.components;
    c.len() >= 3 && c.iter().take(3).all(|p| p.dx == c[0].dx && p.dy == c[0].dy)
}

/// A plane's samples with the DC level shift (G.1.2), clamped into the component's range.
fn shifted(p: &crate::Plane) -> Vec<i32> {
    let bits = u32::from(p.precision);
    let (lo, hi, shift) = if p.signed { (-(1i32 << (bits - 1)), (1i32 << (bits - 1)) - 1, 0) } else { (0, ((1i64 << bits) - 1) as i32, 1i32 << (bits - 1)) };
    p.data.iter().map(|&v| v.clamp(lo, hi) - shift).collect()
}

/// The forward DWT of a `w` × `h` tile-component: per resolution (0 = LL), its bands.
fn analysis<T: Coeff>(plane: &[T], w: usize, h: usize, levels: u8) -> Vec<Vec<Vec<T>>> {
    let mut res: Vec<Vec<Vec<T>>> = vec![Vec::new(); usize::from(levels) + 1];
    let (mut cur, mut cw, mut ch) = (plane.to_vec(), w, h);
    for d in 1..=usize::from(levels) {
        let [ll, hl, lh, hh] = dwt::level(&cur, cw, ch);
        if let Some(r) = res.get_mut(usize::from(levels) + 1 - d) {
            *r = vec![hl, lh, hh];
        }
        (cur, cw, ch) = (ll, cw.div_ceil(2), ch.div_ceil(2));
    }
    if let Some(r) = res.first_mut() {
        *r = vec![cur];
    }
    res
}

/// Band index in QCD order (0 LL, then HL, LH, HH per resolution) → (resolution, band).
fn band_position(band: usize) -> (usize, usize) {
    if band == 0 { (0, 0) } else { (1 + (band - 1) / 3, (band - 1) % 3) }
}

fn band_index(r: usize, bi: usize) -> usize {
    if r == 0 { 0 } else { 1 + 3 * (r - 1) + bi }
}

/// Nominal gain (log2) of each band in QCD order (E.1.1.1).
fn gains(levels: u8) -> impl Iterator<Item = i32> {
    std::iter::once(0).chain((0..levels).flat_map(|_| [1, 1, 2]))
}

/// (ε, μ) of step `delta` against the nominal range 2^rb (E.1.1.1): Δ = 2^(rb-ε)·(1 + μ/2^11).
fn expounded(delta: f64, rb: i32) -> (u8, u16) {
    let x = delta / 2f64.powi(rb);
    let mut e = -(x.log2().floor() as i32);
    let mut mu = ((x * 2f64.powi(e) - 1.0) * 2048.0).round() as i32;
    if mu >= 2048 {
        e -= 1;
        mu = 0;
    }
    if e < 0 {
        return (0, 2047);
    }
    if e > 31 {
        return (31, 0);
    }
    (e as u8, mu.clamp(0, 2047) as u16)
}

/// The L2 norm of each band's synthesis basis function, in QCD order: a unit coefficient's
/// energy in the image, so that a quantization error of Δ there costs (Δ·norm)².
fn band_norms(levels: u8, reversible: bool) -> Vec<f64> {
    let one_d = |d: u32, high: bool| -> f64 {
        if reversible {
            const A: i32 = 1 << 16;
            norm_1d(d, high, A, |v: i32| f64::from(v) / f64::from(A))
        } else {
            norm_1d(d, high, 1.0f32, f64::from)
        }
    };
    let l = u32::from(levels);
    let mut out = vec![one_d(l, false).powi(2)];
    for r in 1..=l {
        let d = l - r + 1;
        let (lo, hi) = (one_d(d, false), one_d(d, true));
        out.extend([hi * lo, lo * hi, hi * hi]);
    }
    out
}

/// The 1-D synthesis norm of an impulse in the low or high band of decomposition level `d`.
fn norm_1d<T: Sample>(d: u32, high: bool, one: T, value: impl Fn(T) -> f64) -> f64 {
    if d == 0 {
        return 1.0;
    }
    let d = d.min(12);
    let n = 64usize << d;
    let len = n >> d;
    let (mut lo, mut hi) = (vec![T::default(); len], vec![T::default(); len]);
    if let Some(v) = if high { hi.get_mut(len / 2) } else { lo.get_mut(len / 2) } {
        *v = one;
    }
    for k in (1..=d).rev() {
        let size = (n >> (k - 1)) as u32;
        let out = synthesis(&lo, &hi, &[], &[], Rect { u0: 0, v0: 0, u1: size, v1: 1 });
        hi = vec![T::default(); out.len()];
        lo = out;
    }
    lo.iter().map(|&v| value(v).powi(2)).sum::<f64>().sqrt()
}

/// Squared-error weight of each component in the output: through the inverse ICT / RCT for the
/// first three when the MCT is on.
fn component_weights(n: usize, mct: bool, reversible: bool) -> Vec<f64> {
    let mut w = vec![1.0; n];
    if mct && n >= 3 {
        let three: [f64; 3] =
            if reversible { [3.0, 11.0 / 16.0, 11.0 / 16.0] } else { [3.0, 0.344_136f64.powi(2) + 1.772f64.powi(2), 1.402f64.powi(2) + 0.714_136f64.powi(2)] };
        w[..3].copy_from_slice(&three);
    }
    w
}

/// Quantizes and codes one code-block.
fn code_block(cf: &Coeffs, j: &Job) -> Result<t1::Encoded> {
    let (w, h) = (j.rect.w(), j.rect.h());
    let mut q = Vec::with_capacity(w * h);
    let mut exact = Vec::with_capacity(w * h);
    let limit = j.limit as i64;
    match cf {
        Coeffs::Int(bands) => {
            let band = bands.get(j.r).and_then(|b| b.get(j.bi)).ok_or(Error::Invalid("subband"))?;
            let bw = j.band_w;
            for y in j.rect.v0..j.rect.v1 {
                let row = band.get(y as usize * bw + j.rect.u0 as usize..y as usize * bw + j.rect.u1 as usize).ok_or(Error::Invalid("code-block"))?;
                for &c in row {
                    let v = i64::from(c).clamp(-limit, limit) as i32;
                    q.push(v);
                    exact.push(c as f32);
                }
            }
        }
        Coeffs::Float(bands) => {
            let band = bands.get(j.r).and_then(|b| b.get(j.bi)).ok_or(Error::Invalid("subband"))?;
            let bw = j.band_w;
            let inv = 1.0 / j.step;
            for y in j.rect.v0..j.rect.v1 {
                let row = band.get(y as usize * bw + j.rect.u0 as usize..y as usize * bw + j.rect.u1 as usize).ok_or(Error::Invalid("code-block"))?;
                for &c in row {
                    let x = c * inv;
                    let m = (x.abs() as i64).min(limit) as i32;
                    q.push(if x < 0.0 { -m } else { m });
                    exact.push(x);
                }
            }
        }
    }
    Ok(t1::encode(&q, &exact, w, h, j.orient))
}

/// The convex hull of a block's truncation points (0, 0), (len_k, w·dist_k): (passes, bytes,
/// slope), slopes strictly falling.
fn hull(passes: &[t1::Pass], w: f64) -> Vec<(u32, usize, f64)> {
    let mut h: Vec<(u32, usize, f64, f64)> = Vec::new(); // passes, bytes, distortion, slope
    for (k, p) in passes.iter().enumerate() {
        let (len, dist) = (p.len, p.dist * w);
        loop {
            let (l0, d0) = h.last().map_or((0, 0.0), |x| (x.1, x.2));
            if len <= l0 {
                // free passes: no bytes, take them with the last point
                if dist >= d0
                    && let Some(last) = h.last_mut()
                {
                    *last = (k as u32 + 1, last.1, dist, last.3);
                }
                break;
            }
            if dist <= d0 {
                break;
            }
            let s = (dist - d0) / (len - l0) as f64;
            if h.last().is_some_and(|x| s >= x.3) {
                h.pop();
                continue;
            }
            h.push((k as u32 + 1, len, dist, s));
            break;
        }
    }
    h.into_iter().map(|(p, l, _, s)| (p, l, s)).collect()
}

/// The distinct hull slopes of `blocks`, falling.
fn slopes<'a>(blocks: impl Iterator<Item = &'a Coded>) -> Vec<f64> {
    let mut s: Vec<f64> = blocks.flat_map(|b| b.hull.iter().map(|h| h.2)).collect();
    s.sort_by(|a, b| b.total_cmp(a));
    s.dedup();
    s
}

/// The threshold of search step `k`: the `k`-th slope, or nothing included for `None`.
fn threshold(slopes: &[f64], k: Option<usize>) -> f64 {
    k.and_then(|k| slopes.get(k)).copied().unwrap_or(f64::INFINITY)
}

/// The largest `k` below `n` for which `ok(Some(k))` holds (it holds for a prefix), or `None`.
fn search(n: usize, mut ok: impl FnMut(Option<usize>) -> Result<bool>) -> Result<Option<usize>> {
    // invariant: ok(lo) holds (lo = None is assumed to), ok(hi) fails or hi = n
    let (mut lo, mut hi): (Option<usize>, usize) = (None, n);
    loop {
        let start = lo.map_or(0, |l| l + 1);
        if start >= hi {
            return Ok(lo);
        }
        let mid = start + (hi - start) / 2;
        if ok(Some(mid))? {
            lo = Some(mid);
        } else {
            hi = mid;
        }
    }
}

/// Passes per block at the thresholds (per component): the hull points at or above them.
fn select(coded: &[Coded], th: &[f64]) -> Vec<u32> {
    coded
        .iter()
        .map(|b| {
            let t = th.get(b.c).copied().unwrap_or(f64::INFINITY);
            b.hull.iter().take_while(|h| h.2 >= t).last().map_or(0, |h| h.0)
        })
        .collect()
}

/// SOC and the main header: SIZ, COD, QCD (QCC where a component differs), POC for the 4K
/// profile; the TLM marker is added by [`assemble`].
fn main_header(image: &Image, opts: &EncodeOptions, quants: &[Quant], mct: bool) -> Result<Vec<u8>> {
    let mut o = vec![0xFF, 0x4F];
    let n = image.components.len();
    let wide = n >= 257;
    // SIZ
    let rsiz: u16 = match opts.profile {
        Profile::Part1 => 0,
        Profile::Cinema2k => 3,
        Profile::Cinema4k => 4,
    };
    marker(&mut o, 0xFF51, |s| {
        s.extend(rsiz.to_be_bytes());
        for v in [image.width, image.height, 0, 0, image.width, image.height, 0, 0] {
            s.extend(v.to_be_bytes());
        }
        s.extend((n as u16).to_be_bytes());
        for p in &image.components {
            s.extend([(p.precision - 1) | if p.signed { 0x80 } else { 0 }, p.dx, p.dy]);
        }
    })?;
    // COD
    let defined = !opts.precincts.is_empty();
    marker(&mut o, 0xFF52, |s| {
        s.push(u8::from(defined));
        s.push(opts.progression as u8);
        s.extend(1u16.to_be_bytes());
        s.push(u8::from(mct));
        s.extend([opts.levels, opts.code_block.0 - 2, opts.code_block.1 - 2, 0, u8::from(opts.reversible)]);
        if defined {
            for r in 0..=usize::from(opts.levels) {
                let (x, y) = opts.precincts.get(r).or(opts.precincts.last()).copied().unwrap_or((15, 15));
                s.push(x | y << 4);
            }
        }
    })?;
    // QCD, QCC
    let qbody = |s: &mut Vec<u8>, q: &Quant| {
        s.push(q.guard << 5 | q.style);
        for &(e, mu) in &q.steps {
            if q.style == 0 {
                s.push(e << 3);
            } else {
                s.extend((u16::from(e) << 11 | mu).to_be_bytes());
            }
        }
    };
    let first = quants.first().ok_or(Error::Invalid("no components"))?;
    marker(&mut o, 0xFF5C, |s| qbody(s, first))?;
    for (c, q) in quants.iter().enumerate().skip(1) {
        if q != first {
            marker(&mut o, 0xFF5D, |s| {
                if wide {
                    s.extend((c as u16).to_be_bytes());
                } else {
                    s.push(c as u8);
                }
                qbody(s, q);
            })?;
        }
    }
    // POC: the 4K resolution after all the 2K ones
    if opts.profile == Profile::Cinema4k {
        let l = opts.levels;
        marker(&mut o, 0xFF5F, |s| {
            for (rs, re) in [(0, l), (l, l + 1)] {
                s.extend([rs, 0]);
                s.extend(1u16.to_be_bytes());
                s.extend([re, n as u8, opts.progression as u8]);
            }
        })?;
    }
    Ok(o)
}

/// Writes marker `m` with the segment `body` builds (A.1.4: the length counts itself).
fn marker(o: &mut Vec<u8>, m: u16, body: impl FnOnce(&mut Vec<u8>)) -> Result<()> {
    let mut s = Vec::new();
    body(&mut s);
    let len = u16::try_from(s.len() + 2).map_err(|_| Error::TooLarge)?;
    o.extend(m.to_be_bytes());
    o.extend(len.to_be_bytes());
    o.extend(s);
    Ok(())
}

/// The codestream with `sel` passes of each block (with `write`; else only its size), its
/// length and the bytes of each component.
fn assemble(
    main: &[u8],
    opts: &EncodeOptions,
    comps: &[TileComp],
    order: &[decode::Packet],
    coded: &[Coded],
    sel: &[u32],
    write: bool,
) -> Result<(Vec<u8>, usize, Vec<usize>)> {
    // index of the first block of each (component, resolution, precinct, band)
    let mut first = Vec::with_capacity(comps.len());
    let mut at = 0usize;
    for tc in comps {
        let mut rs = Vec::with_capacity(tc.res.len());
        for res in &tc.res {
            let mut ps = Vec::with_capacity(res.precincts.len());
            for prec in &res.precincts {
                let mut bs = Vec::with_capacity(prec.len());
                for pb in prec {
                    bs.push(at);
                    at += pb.blocks.len();
                }
                ps.push(bs);
            }
            rs.push(ps);
        }
        first.push(rs);
    }
    // the packets (a single layer: each stands alone): component, header and body, body length
    let packets = map(order, |&(_, r, c, p)| {
        let prec = comps.get(c).and_then(|tc| tc.res.get(r)).and_then(|res| res.precincts.get(p)).ok_or(Error::Invalid("packet"))?;
        let starts = first.get(c).and_then(|x| x.get(r)).and_then(|x| x.get(p)).ok_or(Error::Invalid("packet"))?;
        let mut bands = Vec::with_capacity(prec.len());
        for (pb, &start) in prec.iter().zip(starts) {
            let blocks = (start..start + pb.blocks.len())
                .map(|i| {
                    let (b, &n) = (coded.get(i)?, sel.get(i)?);
                    let len = b.enc.passes.get((n as usize).checked_sub(1)?)?.len;
                    Some(Contribution { zero: b.zero, passes: n, data: b.enc.data.get(..len)? })
                })
                .collect();
            bands.push((pb.cw, blocks));
        }
        let (bytes, body) = t2::packet(&bands, write);
        Ok((c, bytes, body))
    })?;
    // tile-parts: component, length, packets
    let mut parts: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    let mut sizes = vec![0usize; comps.len()];
    for (c, bytes, body) in packets {
        let len = if write { bytes.len() } else { bytes.len() + body };
        if let Some(s) = sizes.get_mut(c) {
            *s += len;
        }
        match parts.last_mut() {
            Some((pc, n, data)) if !opts.tile_part_per_component || *pc == c => {
                *n += len;
                data.extend(bytes);
            }
            _ => parts.push((c, len, bytes)),
        }
    }
    if parts.is_empty() {
        parts.push((0, 0, Vec::new()));
    }
    let count = u8::try_from(parts.len()).map_err(|_| Error::Invalid("more than 255 tile-parts"))?;
    let psot = |len: usize| u32::try_from(len + 14).map_err(|_| Error::TooLarge);
    let tlm = if opts.tlm { 6 + 5 * parts.len() } else { 0 };
    let total = main.len() + tlm + parts.iter().map(|p| p.1 + 14).sum::<usize>() + 2;
    for (c, _, _) in &parts {
        if opts.tile_part_per_component
            && let Some(s) = sizes.get_mut(*c)
        {
            *s += 14;
        }
    }
    if !write {
        return Ok((Vec::new(), total, sizes));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(main);
    if opts.tlm {
        // TLM: Stlm 0x50, an 8-bit tile index and a 32-bit length per tile-part
        let mut lens = Vec::with_capacity(parts.len());
        for (_, len, _) in &parts {
            lens.push(psot(*len)?);
        }
        marker(&mut out, 0xFF55, |s| {
            s.extend([0, 0x50]);
            for l in lens {
                s.push(0);
                s.extend(l.to_be_bytes());
            }
        })?;
    }
    for (k, (_, len, data)) in parts.iter().enumerate() {
        out.extend([0xFF, 0x90, 0, 10, 0, 0]);
        out.extend(psot(*len)?.to_be_bytes());
        out.extend([k as u8, count, 0xFF, 0x93]);
        out.extend_from_slice(data);
    }
    out.extend([0xFF, 0xD9]);
    Ok((out, total, sizes))
}

#[cfg(test)]
mod tests;
