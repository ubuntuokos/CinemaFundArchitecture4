//! Tile decoding: the geometry of tile-components, resolutions, subbands, precincts and
//! code-blocks (Annex B), packet parsing in progression order, tier-1, dequantization (Annex E),
//! the inverse wavelet transform, the inverse multiple-component transform (Annex G) and the DC
//! level shift.

use crate::codestream::{self, CodingStyle, Header, Params, Quant, Tile};
use crate::dwt::{self, Rect};
use crate::t2::{Bits, TagTree};
use crate::{Error, Image, Plane, Result};
use crate::{ht, t1};

/// Most code-blocks a tile may have (bounds the bookkeeping a hostile header can ask for).
const MAX_CODEBLOCKS: usize = 1 << 22;
/// Precincts and packets a tile may have (layers × precincts); larger is refused before the
/// packet order is built.
const MAX_PACKETS: usize = 1 << 22;

fn ceil_div(a: u64, b: u64) -> u64 {
    a.div_ceil(b.max(1))
}

/// ceil(a / 2^n) for a possibly negative `a`.
fn ceil_shift(a: i64, n: u32) -> i64 {
    let d = 1i64 << n.min(62);
    (a + d - 1).div_euclid(d)
}

pub(crate) struct CodeBlock {
    /// Rectangle in subband coordinates.
    pub(crate) x0: u32,
    pub(crate) y0: u32,
    pub(crate) x1: u32,
    pub(crate) y1: u32,
    pub(crate) included: bool,
    pub(crate) lblock: u32,
    pub(crate) zero: u32,
    pub(crate) segments: Vec<(Vec<u8>, u32)>,
}

/// The code-blocks of one subband inside one precinct.
pub(crate) struct PrecinctBand {
    pub(crate) cw: usize,
    pub(crate) blocks: Vec<CodeBlock>,
    pub(crate) incl: TagTree,
    pub(crate) zero: TagTree,
}

pub(crate) struct Band {
    /// 0 LL, 1 HL, 2 LH, 3 HH.
    pub(crate) orient: u8,
    pub(crate) rect: Rect,
    /// Bit-planes (Mb).
    pub(crate) mb: u32,
    /// Quantization step (irreversible).
    pub(crate) step: f32,
}

pub(crate) struct Resolution {
    pub(crate) rect: Rect,
    pub(crate) ppx: u32,
    pub(crate) ppy: u32,
    pub(crate) pw: usize,
    pub(crate) ph: usize,
    pub(crate) bands: Vec<Band>,
    /// Per precinct, per band.
    pub(crate) precincts: Vec<Vec<PrecinctBand>>,
}

pub(crate) struct TileComp {
    pub(crate) rect: Rect,
    pub(crate) style: CodingStyle,
    pub(crate) res: Vec<Resolution>,
    /// Region of interest shift (RGN Maxshift, Annex H): 0 for none.
    pub(crate) roi: u8,
}

/// A packet: layer, resolution, component, precinct.
pub(crate) type Packet = (u16, usize, usize, usize);

fn band_rect(tc: Rect, n: u32, orient: u8) -> Rect {
    if orient == 0 {
        return Rect {
            u0: ceil_shift(i64::from(tc.u0), n) as u32,
            v0: ceil_shift(i64::from(tc.v0), n) as u32,
            u1: ceil_shift(i64::from(tc.u1), n) as u32,
            v1: ceil_shift(i64::from(tc.v1), n) as u32,
        };
    }
    let (xo, yo) = (i64::from(orient & 1), i64::from(orient >> 1));
    let half = 1i64 << (n - 1).min(62);
    let f = |c: u32, o: i64| ceil_shift(i64::from(c) - half * o, n).max(0) as u32;
    Rect { u0: f(tc.u0, xo), v0: f(tc.v0, yo), u1: f(tc.u1, xo), v1: f(tc.v1, yo) }
}

/// Step size and bit-planes of a subband (E.1.1.1, E.1.1.2).
pub(crate) fn quant(q: &Quant, band: usize, levels: u8, precision: u8, orient: u8) -> Result<(u32, f32)> {
    let (e, mu) = q.step(band, levels)?;
    let mb = (u32::from(q.guard) + u32::from(e)).saturating_sub(1);
    let gain = match orient {
        0 => 0,
        3 => 2,
        _ => 1,
    };
    let rb = i32::from(precision) + gain;
    let step = 2f32.powi(rb - i32::from(e)) * (1.0 + f32::from(mu) / 2048.0);
    Ok((mb, step))
}

pub(crate) fn build(header: &Header, params: &Params, tile: u32) -> Result<(Rect, Vec<TileComp>)> {
    let (tw, _) = header.tiles();
    let (p, q) = (tile % tw.max(1), tile / tw.max(1));
    let tx0 = (u64::from(header.tile_x0) + u64::from(p) * u64::from(header.tile_w)).max(u64::from(header.x0));
    let ty0 = (u64::from(header.tile_y0) + u64::from(q) * u64::from(header.tile_h)).max(u64::from(header.y0));
    let tx1 = (u64::from(header.tile_x0) + u64::from(p + 1) * u64::from(header.tile_w)).min(u64::from(header.x1));
    let ty1 = (u64::from(header.tile_y0) + u64::from(q + 1) * u64::from(header.tile_h)).min(u64::from(header.y1));
    if tx0 >= tx1 || ty0 >= ty1 {
        return Err(Error::Invalid("empty tile"));
    }
    let trect = Rect { u0: tx0 as u32, v0: ty0 as u32, u1: tx1 as u32, v1: ty1 as u32 };
    let mut blocks_total = 0usize;
    let mut precincts_total = 0usize;
    let mut comps = Vec::with_capacity(header.components.len());
    for (c, info) in header.components.iter().enumerate() {
        let style = params.cod.get(c).ok_or(Error::Invalid("component without coding style"))?.clone();
        let qz = params.quant.get(c).ok_or(Error::Invalid("component without quantization"))?;
        let roi = params.roi.get(c).copied().unwrap_or(0);
        let (dx, dy) = (u64::from(info.dx), u64::from(info.dy));
        let tc = Rect { u0: ceil_div(tx0, dx) as u32, v0: ceil_div(ty0, dy) as u32, u1: ceil_div(tx1, dx) as u32, v1: ceil_div(ty1, dy) as u32 };
        let levels = u32::from(style.levels);
        let mut res = Vec::with_capacity(levels as usize + 1);
        for r in 0..=levels {
            let nb = levels - r;
            let rect = band_rect(tc, nb, 0);
            let &(ppx, ppy) = style.precincts.get(r as usize).ok_or(Error::Invalid("precinct sizes"))?;
            let (ppx, ppy) = (u32::from(ppx), u32::from(ppy));
            if r > 0 && (ppx == 0 || ppy == 0) {
                return Err(Error::Invalid("precinct size 1 above resolution 0"));
            }
            let count = |a0: u32, a1: u32, pp: u32| -> usize { if a1 <= a0 { 0 } else { (u64::from(a1).div_ceil(1 << pp) - (u64::from(a0) >> pp)) as usize } };
            let (pw, ph) = (count(rect.u0, rect.u1, ppx), count(rect.v0, rect.v1, ppy));
            precincts_total = precincts_total.saturating_add(pw.saturating_mul(ph));
            if precincts_total > MAX_PACKETS {
                return Err(Error::TooLarge);
            }
            let mut bands = Vec::new();
            let orients: &[u8] = if r == 0 { &[0] } else { &[1, 2, 3] };
            for &o in orients {
                let n = if r == 0 { levels } else { levels - r + 1 };
                let brect = band_rect(tc, n, o);
                let index = if r == 0 { 0 } else { 1 + 3 * (r as usize - 1) + (o as usize - 1) };
                let (mb, step) = quant(qz, index, style.levels, info.precision, o)?;
                // the region of interest is coded above the background (H.1)
                let mb = mb + u32::from(roi);
                bands.push(Band { orient: o, rect: brect, mb, step });
            }
            // code-block size within this resolution (B.7)
            let shrink = u32::from(r > 0);
            let xcb = u32::from(style.cbw).min(ppx - shrink.min(ppx));
            let ycb = u32::from(style.cbh).min(ppy - shrink.min(ppy));
            // precinct grid in subband coordinates
            let (gx, gy) = ((rect.u0 >> ppx) << ppx, (rect.v0 >> ppy) << ppy);
            let (gx, gy, sx, sy) = if r == 0 { (gx, gy, ppx, ppy) } else { (gx >> 1, gy >> 1, ppx - 1, ppy - 1) };
            let mut precincts = Vec::with_capacity((pw * ph).min(1 << 16));
            for k in 0..pw * ph {
                let (i, j) = ((k % pw.max(1)) as u64, (k / pw.max(1)) as u64);
                let mut pbands = Vec::with_capacity(bands.len());
                for b in &bands {
                    let cx0 = u64::from(gx) + (i << sx);
                    let cy0 = u64::from(gy) + (j << sy);
                    let px0 = cx0.max(u64::from(b.rect.u0));
                    let py0 = cy0.max(u64::from(b.rect.v0));
                    let px1 = (cx0 + (1 << sx)).min(u64::from(b.rect.u1));
                    let py1 = (cy0 + (1 << sy)).min(u64::from(b.rect.v1));
                    let (mut cw, mut chh) = (0usize, 0usize);
                    let mut blocks = Vec::new();
                    if px0 < px1 && py0 < py1 {
                        let (bx0, by0) = ((px0 >> xcb) << xcb, (py0 >> ycb) << ycb);
                        cw = (((px1.div_ceil(1 << xcb) << xcb) - bx0) >> xcb) as usize;
                        chh = (((py1.div_ceil(1 << ycb) << ycb) - by0) >> ycb) as usize;
                        blocks_total = blocks_total.saturating_add(cw * chh);
                        if blocks_total > MAX_CODEBLOCKS {
                            return Err(Error::TooLarge);
                        }
                        blocks.reserve(cw * chh);
                        for n in 0..cw * chh {
                            let x = bx0 + ((n % cw) as u64) * (1 << xcb);
                            let y = by0 + ((n / cw) as u64) * (1 << ycb);
                            blocks.push(CodeBlock {
                                x0: x.max(px0) as u32,
                                y0: y.max(py0) as u32,
                                x1: (x + (1 << xcb)).min(px1) as u32,
                                y1: (y + (1 << ycb)).min(py1) as u32,
                                included: false,
                                lblock: 3,
                                zero: 0,
                                segments: Vec::new(),
                            });
                        }
                    }
                    pbands.push(PrecinctBand { cw, blocks, incl: TagTree::new(cw, chh), zero: TagTree::new(cw, chh) });
                }
                precincts.push(pbands);
            }
            res.push(Resolution { rect, ppx, ppy, pw, ph, bands, precincts });
        }
        comps.push(TileComp { rect: tc, style, res, roi });
    }
    Ok((trect, comps))
}

/// The packets of a tile in its progression order (B.12).
pub(crate) fn packets(header: &Header, trect: Rect, comps: &[TileComp], params: &Params) -> Result<Vec<Packet>> {
    let layers = params.global.layers;
    let max_r = comps.iter().map(|c| c.res.len()).max().unwrap_or(0);
    let precincts: usize = comps.iter().flat_map(|c| &c.res).map(|r| r.pw.saturating_mul(r.ph)).fold(0, usize::saturating_add);
    if precincts.saturating_mul(usize::from(layers)) > MAX_PACKETS {
        return Err(Error::TooLarge);
    }
    // reference-grid corner of a precinct (where the position loops of B.12.1.3 meet it)
    let corner = |c: usize, r: usize, p: usize| -> (u64, u64) {
        let (Some(tc), Some(info)) = (comps.get(c), header.components.get(c)) else { return (0, 0) };
        let Some(res) = tc.res.get(r) else { return (0, 0) };
        let level = (tc.res.len() - 1 - r) as u32;
        let (i, j) = ((p % res.pw.max(1)) as u64, (p / res.pw.max(1)) as u64);
        let px = ((u64::from(res.rect.u0) >> res.ppx) + i) << res.ppx;
        let py = ((u64::from(res.rect.v0) >> res.ppy) + j) << res.ppy;
        let x = (px << level) * u64::from(info.dx);
        let y = (py << level) * u64::from(info.dy);
        (y.max(u64::from(trect.v0)), x.max(u64::from(trect.u0)))
    };
    let np = |c: usize, r: usize| comps.get(c).and_then(|tc| tc.res.get(r)).map_or(0, |res| res.pw * res.ph);
    // the packets of one progression over resolutions rs..re, components cs..ce, layers below le
    let order = |progression: u8, (rs, re): (usize, usize), (cs, ce): (usize, usize), le: u16, out: &mut Vec<Packet>| match progression {
        0 => {
            for l in 0..le {
                for r in rs..re {
                    for c in cs..ce {
                        for p in 0..np(c, r) {
                            out.push((l, r, c, p));
                        }
                    }
                }
            }
        }
        1 => {
            for r in rs..re {
                for l in 0..le {
                    for c in cs..ce {
                        for p in 0..np(c, r) {
                            out.push((l, r, c, p));
                        }
                    }
                }
            }
        }
        order => {
            // (sort key, r, c, p)
            let mut items: Vec<((u64, u64, u64, u64), usize, usize, usize)> = Vec::new();
            for c in cs..ce {
                for r in rs..re {
                    for p in 0..np(c, r) {
                        let (y, x) = corner(c, r, p);
                        let key = match order {
                            2 => (r as u64, y, x, c as u64),
                            3 => (y, x, c as u64, r as u64),
                            _ => (c as u64, y, x, r as u64),
                        };
                        items.push((key, r, c, p));
                    }
                }
            }
            items.sort_by_key(|i| i.0);
            for (_, r, c, p) in items {
                for l in 0..le {
                    out.push((l, r, c, p));
                }
            }
        }
    };
    let all = ((0, max_r), (0, comps.len()));
    let mut out = Vec::new();
    if params.poc.is_empty() {
        order(params.global.progression, all.0, all.1, layers, &mut out);
        return Ok(out);
    }
    // progression changes (B.12.2): each packet once, the layers of a precinct in order; what
    // they leave out follows in the COD progression
    let mut candidates = Vec::new();
    for poc in &params.poc {
        let r = (usize::from(poc.rs), usize::from(poc.re).min(max_r));
        let c = (usize::from(poc.cs), usize::from(poc.ce).min(comps.len()));
        order(poc.progression, r, c, poc.layer_end.min(layers), &mut candidates);
        if candidates.len() > MAX_PACKETS {
            return Err(Error::TooLarge);
        }
    }
    order(params.global.progression, all.0, all.1, layers, &mut candidates);
    // next layer per (component, resolution, precinct)
    let mut next: Vec<Vec<Vec<u16>>> = comps.iter().map(|tc| tc.res.iter().map(|res| vec![0; res.pw * res.ph]).collect()).collect();
    for (l, r, c, p) in candidates {
        if let Some(n) = next.get_mut(c).and_then(|v| v.get_mut(r)).and_then(|v| v.get_mut(p))
            && *n == l
        {
            *n += 1;
            out.push((l, r, c, p));
        }
    }
    Ok(out)
}

pub(crate) fn floor_log2(n: u32) -> u32 {
    31 - n.max(1).leading_zeros()
}

/// Reads one packet at `pos`, its header there too or at `packed` (the header stream of PPM /
/// PPT, which this moves past it); returns the position after the packet body.
fn read_packet(data: &[u8], pos: usize, packed: Option<(&[u8], &mut usize)>, comps: &mut [TileComp], packet: Packet, params: &Params) -> Result<usize> {
    read_packet_spans(data, pos, packed, comps, packet, params).map(|s| s.1)
}

/// [`read_packet`], also giving where the header ends: (header end, body end).
fn read_packet_spans(
    data: &[u8],
    mut pos: usize,
    packed: Option<(&[u8], &mut usize)>,
    comps: &mut [TileComp],
    (layer, r, c, p): Packet,
    params: &Params,
) -> Result<(usize, usize)> {
    if params.global.sop && data.get(pos..pos + 2) == Some(&[0xFF, 0x91]) {
        pos += 6;
    }
    let tc = comps.get_mut(c).ok_or(Error::Invalid("packet component"))?;
    let style = tc.style.cblk_style;
    let res = tc.res.get_mut(r).ok_or(Error::Invalid("packet resolution"))?;
    let prec = res.precincts.get_mut(p).ok_or(Error::Invalid("packet precinct"))?;
    let (hdr, hpos) = match &packed {
        Some((h, at)) => (*h, **at),
        None => (data, pos),
    };
    let mut bits = Bits::new(hdr, hpos);
    // (band, block, passes before this packet, (length, passes) per segment piece)
    let mut included: Vec<(usize, usize, u32, Vec<(u32, u32)>)> = Vec::new();
    if bits.bit()? == 1 {
        for (bi, pb) in prec.iter_mut().enumerate() {
            for k in 0..pb.blocks.len() {
                let (x, y) = (k % pb.cw.max(1), k / pb.cw.max(1));
                let first = !pb.blocks[k].included;
                let inc = if first { pb.incl.decode(&mut bits, x, y, u32::from(layer) + 1)? } else { bits.bit()? == 1 };
                if !inc {
                    continue;
                }
                if first {
                    let mut i = 1;
                    while !pb.zero.decode(&mut bits, x, y, i)? {
                        i += 1;
                        if i > 64 {
                            return Err(Error::Invalid("zero bit-plane count"));
                        }
                    }
                    let b = &mut pb.blocks[k];
                    b.zero = i - 1;
                    b.included = true;
                }
                let n = bits.passes()?;
                let b = &mut pb.blocks[k];
                while bits.bit()? == 1 {
                    b.lblock += 1;
                    if b.lblock > 32 {
                        return Err(Error::Invalid("Lblock"));
                    }
                }
                // the new passes, split where codeword segments end (B.10.7.2): a length per piece
                let prior: u32 = b.segments.iter().map(|s| s.1).sum();
                let mut lens = Vec::new();
                if style & ht::STYLE_HT != 0 {
                    // HT (T.814 B.3): placeholder passes, the cleanup pass, then at most two
                    // refinement passes in a second segment; one HT set per code-block
                    if prior > 0 {
                        return Err(Error::Unsupported("HT code-block contributions in more than one layer"));
                    }
                    let cup = 3 * ((n - 1) / 3) + 1;
                    lens.push((bits.bits(b.lblock + floor_log2(cup))?, cup));
                    if n > cup {
                        lens.push((bits.bits(b.lblock + floor_log2(n - cup))?, n - cup));
                    }
                    included.push((bi, k, prior, lens));
                    continue;
                }
                let mut start = 0;
                for i in 0..n {
                    if i + 1 == n || t1::ends_segment(prior + i, style) {
                        let passes = i + 1 - start;
                        lens.push((bits.bits(b.lblock + floor_log2(passes))?, passes));
                        start = i + 1;
                    }
                }
                included.push((bi, k, prior, lens));
            }
        }
    }
    bits.align();
    let mut end = bits.pos;
    if params.global.eph && hdr.get(end..end + 2) == Some(&[0xFF, 0x92]) {
        end += 2;
    }
    let header_end = end;
    match packed {
        Some((_, at)) => *at = end,
        None => pos = end,
    }
    for (bi, k, prior, lens) in included {
        let Some(b) = prec.get_mut(bi).and_then(|pb| pb.blocks.get_mut(k)) else { continue };
        for (i, &(len, passes)) in lens.iter().enumerate() {
            let end = pos.saturating_add(len as usize).min(data.len());
            let bytes = data.get(pos..end).unwrap_or_default();
            pos = end;
            // the first piece continues the last segment unless that one was terminated
            let open = i == 0 && prior > 0 && style & ht::STYLE_HT == 0 && !t1::ends_segment(prior - 1, style);
            match b.segments.last_mut() {
                Some(last) if open => {
                    last.0.extend_from_slice(bytes);
                    last.1 += passes;
                }
                _ => b.segments.push((bytes.to_vec(), passes)),
            }
        }
    }
    Ok((header_end, pos))
}

/// For tests: the packets of each tile of a codestream (no SOP / EPH markers, headers in the tile
/// data) as (tile index, tile data, (header start, header end, body end) per packet).
#[cfg(test)]
pub(crate) fn packet_spans(data: &[u8]) -> Result<Vec<(u32, Vec<u8>, Vec<(usize, usize, usize)>)>> {
    let cs = codestream::parse(data, true)?;
    let mut out = Vec::new();
    for tile in &cs.tiles {
        let (trect, mut comps) = build(&cs.header, &tile.params, tile.index)?;
        let mut spans = Vec::new();
        let mut pos = 0;
        for p in packets(&cs.header, trect, &comps, &tile.params)? {
            if pos >= tile.data.len() {
                break;
            }
            let (he, be) = read_packet_spans(&tile.data, pos, None, &mut comps, p, &tile.params)?;
            spans.push((pos, he, be));
            pos = be;
        }
        out.push((tile.index, tile.data.clone(), spans));
    }
    Ok(out)
}

/// Nearest integer, halves away from zero (as `f32::round`, without the library call).
fn round(x: f32) -> i32 {
    (x + if x >= 0.0 { 0.5 } else { -0.5 }) as i32
}

enum Samples {
    Int(Vec<i32>),
    Float(Vec<f32>),
}

/// Maps `f` over `items`, on all cores with the `threads` feature.
pub(crate) fn map<T: Sync, U: Send>(items: &[T], f: impl Fn(&T) -> Result<U> + Sync + Send) -> Result<Vec<U>> {
    #[cfg(feature = "threads")]
    {
        use rayon::prelude::*;
        items.par_iter().map(f).collect()
    }
    #[cfg(not(feature = "threads"))]
    {
        items.iter().map(f).collect()
    }
}

fn reconstruct(tc: &TileComp) -> Result<Samples> {
    let reversible = tc.style.reversible;
    // every code-block with data: (resolution, band, block)
    let mut jobs: Vec<(usize, usize, &CodeBlock)> = Vec::new();
    for (r, res) in tc.res.iter().enumerate() {
        for prec in &res.precincts {
            for (bi, pb) in prec.iter().enumerate() {
                jobs.extend(pb.blocks.iter().filter(|cb| !cb.segments.is_empty()).map(|cb| (r, bi, cb)));
            }
        }
    }
    let decoded = map(&jobs, |&(r, bi, cb)| {
        let band = tc.res.get(r).and_then(|res| res.bands.get(bi)).ok_or(Error::Invalid("subband"))?;
        let (w, h) = ((cb.x1 - cb.x0) as usize, (cb.y1 - cb.y0) as usize);
        let block = t1::Block { w, h, orient: band.orient, style: tc.style.cblk_style, mb: band.mb, zero: cb.zero, segments: &cb.segments };
        let mut coeffs = if block.style & ht::STYLE_HT != 0 { ht::decode(&block)? } else { t1::decode(&block)? };
        if tc.roi > 0 {
            // Maxshift (H.1): coefficients at or above 2^s are the region, scaled back down
            // (the values carry one fractional bit)
            let s = u32::from(tc.roi);
            let threshold = 1i64 << (s + 1);
            for v in &mut coeffs {
                if i64::from(*v).abs() >= threshold {
                    *v = v.signum() * (v.unsigned_abs() >> s.min(31)) as i32;
                }
            }
        }
        Ok(coeffs)
    })?;
    // into their subbands
    let mut bands: Vec<Vec<Vec<i32>>> = tc.res.iter().map(|res| res.bands.iter().map(|b| vec![0; b.rect.w() * b.rect.h()]).collect()).collect();
    for (&(r, bi, cb), coeffs) in jobs.iter().zip(decoded) {
        let Some(band) = tc.res.get(r).and_then(|res| res.bands.get(bi)) else { continue };
        let Some(out) = bands.get_mut(r).and_then(|b| b.get_mut(bi)) else { continue };
        let w = (cb.x1 - cb.x0) as usize;
        let bw = band.rect.w();
        let (ox, oy) = ((cb.x0 - band.rect.u0) as usize, (cb.y0 - band.rect.v0) as usize);
        for (j, row) in coeffs.chunks(w.max(1)).enumerate() {
            let start = (oy + j) * bw + ox;
            if let Some(dst) = out.get_mut(start..start + row.len()) {
                dst.copy_from_slice(row);
            }
        }
    }
    let empty: Vec<i32> = Vec::new();
    let get = |r: usize, b: usize| bands.get(r).and_then(|x| x.get(b)).unwrap_or(&empty);
    if reversible {
        let half = |v: &Vec<i32>| v.iter().map(|&c| if c < 0 { -((-c) >> 1) } else { c >> 1 }).collect::<Vec<i32>>();
        let mut ll = half(get(0, 0));
        for (r, res) in tc.res.iter().enumerate().skip(1) {
            ll = dwt::level(&ll, &half(get(r, 0)), &half(get(r, 1)), &half(get(r, 2)), res.rect);
        }
        Ok(Samples::Int(ll))
    } else {
        let scale = |r: usize, b: usize| -> Vec<f32> {
            let step = tc.res.get(r).and_then(|x| x.bands.get(b)).map_or(1.0, |b| b.step) * 0.5;
            get(r, b).iter().map(|&c| c as f32 * step).collect()
        };
        let mut ll = scale(0, 0);
        for (r, res) in tc.res.iter().enumerate().skip(1) {
            ll = dwt::level(&ll, &scale(r, 0), &scale(r, 1), &scale(r, 2), res.rect);
        }
        Ok(Samples::Float(ll))
    }
}

/// Decodes a JPEG 2000 codestream or JP2 file.
pub fn decode(data: &[u8]) -> Result<Image> {
    let cs = codestream::parse(data, true)?;
    let h = &cs.header;
    let mut planes: Vec<Plane> = h
        .components
        .iter()
        .map(|c| {
            let (dx, dy) = (u64::from(c.dx), u64::from(c.dy));
            let w = (ceil_div(u64::from(h.x1), dx) - ceil_div(u64::from(h.x0), dx)) as u32;
            let hh = (ceil_div(u64::from(h.y1), dy) - ceil_div(u64::from(h.y0), dy)) as u32;
            Plane { width: w, height: hh, precision: c.precision, signed: c.signed, dx: c.dx, dy: c.dy, data: vec![0; w as usize * hh as usize] }
        })
        .collect();
    for tile in &cs.tiles {
        decode_tile(h, tile, &mut planes)?;
    }
    Ok(Image { width: h.width(), height: h.height(), components: planes })
}

fn decode_tile(h: &Header, tile: &Tile, planes: &mut [Plane]) -> Result<()> {
    let (trect, mut comps) = build(h, &tile.params, tile.index)?;
    let order = packets(h, trect, &comps, &tile.params)?;
    let (mut pos, mut hpos) = (0, 0);
    for p in order {
        let packed = tile.headers.as_deref().map(|h| (h, &mut hpos));
        let more = match &packed {
            Some((h, at)) => **at < h.len(),
            None => pos < tile.data.len(),
        };
        if !more {
            break;
        }
        match read_packet(&tile.data, pos, packed, &mut comps, p, &tile.params) {
            Ok(next) => pos = next,
            // a stream using a tool we lack must not decode to a silently incomplete picture
            Err(e @ Error::Unsupported(_)) => return Err(e),
            // a damaged or truncated packet ends the tile; what arrived still decodes
            Err(_) => break,
        }
    }
    let mut samples: Vec<Samples> = map(&comps, reconstruct)?;
    let mct = tile.params.global.mct && comps.len() >= 3 && comps.iter().take(3).all(|c| c.rect == comps[0].rect);
    if mct {
        let reversible = comps.iter().take(3).all(|c| c.style.reversible);
        if let [Samples::Int(y0), Samples::Int(y1), Samples::Int(y2), ..] = &mut samples[..]
            && reversible
        {
            // RCT (G.2), in place: Y0 Y1 Y2 → R G B
            for ((y, cb), cr) in y0.iter_mut().zip(y1.iter_mut()).zip(y2.iter_mut()) {
                let g = y.wrapping_sub(cb.wrapping_add(*cr) >> 2);
                (*y, *cb, *cr) = (cr.wrapping_add(g), g, cb.wrapping_add(g));
            }
        } else {
            for s in samples.iter_mut().take(3) {
                if let Samples::Int(v) = s {
                    *s = Samples::Float(v.iter().map(|&x| x as f32).collect());
                }
            }
            if let [Samples::Float(y0), Samples::Float(y1), Samples::Float(y2), ..] = &mut samples[..] {
                // ICT (G.3), in place
                for ((y, cb), cr) in y0.iter_mut().zip(y1.iter_mut()).zip(y2.iter_mut()) {
                    let (yy, b, r) = (*y, *cb, *cr);
                    (*y, *cb, *cr) = (yy + 1.402 * r, yy - 0.344_136 * b - 0.714_136 * r, yy + 1.772 * b);
                }
            }
        }
    }
    let put = |(plane, (s, tc)): (&mut Plane, (&Samples, &TileComp))| {
        let p = u32::from(plane.precision);
        let (lo, hi, shift) = if plane.signed { (-(1i64 << (p - 1)), (1i64 << (p - 1)) - 1, 0i64) } else { (0, (1i64 << p) - 1, 1i64 << (p - 1)) };
        let (lo, hi, shift) = (lo as i32, hi as i32, shift as i32);
        let px0 = ceil_div(u64::from(h.x0), u64::from(plane.dx)) as u32;
        let py0 = ceil_div(u64::from(h.y0), u64::from(plane.dy)) as u32;
        let (w, ox, oy) = (tc.rect.w(), (tc.rect.u0 - px0) as usize, (tc.rect.v0 - py0) as usize);
        let pw = plane.width as usize;
        if w == 0 || ox >= pw {
            return;
        }
        // the tile-component's rows into the plane, level shifted and clamped (G.1.2)
        let n = w.min(pw - ox);
        let rows = plane.data.chunks_exact_mut(pw).skip(oy);
        match s {
            Samples::Int(v) => {
                for (dst, src) in rows.zip(v.chunks_exact(w)) {
                    for (d, &x) in dst[ox..ox + n].iter_mut().zip(src) {
                        *d = x.saturating_add(shift).clamp(lo, hi);
                    }
                }
            }
            Samples::Float(v) => {
                for (dst, src) in rows.zip(v.chunks_exact(w)) {
                    for (d, &x) in dst[ox..ox + n].iter_mut().zip(src) {
                        *d = round(x).saturating_add(shift).clamp(lo, hi);
                    }
                }
            }
        }
    };
    #[cfg(feature = "threads")]
    {
        use rayon::prelude::*;
        planes.par_iter_mut().zip(samples.par_iter().zip(comps.par_iter())).for_each(put);
    }
    #[cfg(not(feature = "threads"))]
    planes.iter_mut().zip(samples.iter().zip(comps.iter())).for_each(put);
    Ok(())
}
