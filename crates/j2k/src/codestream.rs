//! Codestream syntax (T.800 Annex A): the main header (SIZ, COD, COC, QCD, QCC), tile-part
//! headers and the tile data, gathered per tile.

use crate::{Error, MAX_SAMPLES, Result};

const SOC: u16 = 0xFF4F;
const SIZ: u16 = 0xFF51;
const COD: u16 = 0xFF52;
const COC: u16 = 0xFF53;
const QCD: u16 = 0xFF5C;
const QCC: u16 = 0xFF5D;
const RGN: u16 = 0xFF5E;
const POC: u16 = 0xFF5F;
const PPM: u16 = 0xFF60;
const PPT: u16 = 0xFF61;
const SOT: u16 = 0xFF90;
const SOD: u16 = 0xFF93;
const EOC: u16 = 0xFFD9;

/// Largest number of decomposition levels (A.6.1: 0-32).
const MAX_LEVELS: u8 = 32;

/// One component as SIZ describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentInfo {
    /// Bits per sample, 1-38 by the syntax; this decoder takes 1-16.
    pub precision: u8,
    pub signed: bool,
    /// Sub-sampling on the reference grid (XRsiz, YRsiz), 1-255.
    pub dx: u8,
    pub dy: u8,
}

/// What the main header says about the image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Image area on the reference grid: [x0, x1) × [y0, y1) (XOsiz, Xsiz, YOsiz, Ysiz).
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    /// Tile grid: origin (XTOsiz, YTOsiz) and size (XTsiz, YTsiz).
    pub tile_x0: u32,
    pub tile_y0: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    pub components: Vec<ComponentInfo>,
    /// Decomposition levels of the first component (main COD / COC).
    pub levels: u8,
    /// Quality layers.
    pub layers: u16,
    /// Reversible 5/3 wavelet (lossless) rather than the irreversible 9/7.
    pub reversible: bool,
    /// Progression order: 0 LRCP, 1 RLCP, 2 RPCL, 3 PCRL, 4 CPRL.
    pub progression: u8,
}

impl Header {
    pub fn width(&self) -> u32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> u32 {
        self.y1 - self.y0
    }
    /// Tiles across and down.
    pub fn tiles(&self) -> (u32, u32) {
        ((self.x1 - self.tile_x0).div_ceil(self.tile_w), (self.y1 - self.tile_y0).div_ceil(self.tile_h))
    }
}

/// Coding style of one component (COD / COC SPcod).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodingStyle {
    pub levels: u8,
    /// Code-block size exponents (xcb, ycb), 2-10 each, sum ≤ 12.
    pub cbw: u8,
    pub cbh: u8,
    pub cblk_style: u8,
    pub reversible: bool,
    /// Precinct size exponents (PPx, PPy) per resolution level 0 ..= levels.
    pub precincts: Vec<(u8, u8)>,
}

/// The parts of COD that apply to every component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Global {
    pub progression: u8,
    pub layers: u16,
    pub mct: bool,
    pub sop: bool,
    pub eph: bool,
}

/// Quantization of one component (QCD / QCC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Quant {
    /// 0 none (reversible), 1 scalar derived, 2 scalar expounded.
    pub style: u8,
    pub guard: u8,
    /// (exponent, mantissa) per subband in codestream order (LL, then HL, LH, HH per level from the
    /// lowest resolution up); one entry for the derived style.
    pub steps: Vec<(u8, u16)>,
}

impl Quant {
    /// (exponent, mantissa) of subband `band` (0 = LL, 1 + 3·(r-1) + orientation-1 for resolution
    /// r ≥ 1), with `levels` decomposition levels.
    pub fn step(&self, band: usize, levels: u8) -> Result<(u8, u16)> {
        if self.style == 1 {
            // derived: the LL values, the exponent decreasing with each level (A.6.4)
            let &(e0, mu) = self.steps.first().ok_or(Error::Invalid("QCD without step sizes"))?;
            let level = if band == 0 { levels as usize } else { levels as usize - (band - 1) / 3 };
            let e = i32::from(e0) - i32::from(levels) + level as i32;
            return u8::try_from(e).map(|e| (e, mu)).map_err(|_| Error::Invalid("derived step exponent below zero"));
        }
        self.steps.get(band).copied().ok_or(Error::Invalid("QCD has fewer step sizes than subbands"))
    }
}

/// One progression of a POC marker (A.6.6): resolutions [rs, re), components [cs, ce) and layers
/// below `layer_end`, in progression order `progression`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Poc {
    pub rs: u8,
    pub cs: u16,
    pub layer_end: u16,
    pub re: u8,
    pub ce: u16,
    pub progression: u8,
}

/// Coding parameters of a tile: COD, COC, QCD and QCC after the precedence rules (A.6), the
/// progression changes (POC, main then tile) and the region of interest shift per component (RGN).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Params {
    pub global: Global,
    pub cod: Vec<CodingStyle>,
    pub quant: Vec<Quant>,
    pub poc: Vec<Poc>,
    pub roi: Vec<u8>,
}

/// One tile's coding parameters and data (its tile-parts' SOD..end bodies, concatenated), and
/// its packet headers when they are packed into PPM or PPT markers (A.7.4, A.7.5).
pub(crate) struct Tile {
    pub index: u32,
    pub params: Params,
    pub data: Vec<u8>,
    pub headers: Option<Vec<u8>>,
}

pub(crate) struct Codestream {
    pub header: Header,
    pub tiles: Vec<Tile>,
}

struct Reader<'a> {
    d: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.d.get(self.at).ok_or(Error::Truncated)?;
        self.at += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from(self.u8()?) << 8 | u16::from(self.u8()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from(self.u16()?) << 16 | u32::from(self.u16()?))
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or(Error::Truncated)?;
        let b = self.d.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(b)
    }
    /// A marker segment's parameters (after its 16-bit length).
    fn segment(&mut self) -> Result<&'a [u8]> {
        let len = self.u16()? as usize;
        if len < 2 {
            return Err(Error::Invalid("marker segment length below 2"));
        }
        self.bytes(len - 2)
    }
}

fn parse_siz(s: &[u8]) -> Result<Header> {
    let mut r = Reader { d: s, at: 0 };
    let _rsiz = r.u16()?;
    let (x1, y1, x0, y0) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let (tile_w, tile_h, tile_x0, tile_y0) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let csiz = r.u16()?;
    if x0 >= x1 || y0 >= y1 {
        return Err(Error::Invalid("empty image area"));
    }
    if tile_w == 0 || tile_h == 0 || tile_x0 > x0 || tile_y0 > y0 {
        return Err(Error::Invalid("tile grid"));
    }
    if u64::from(tile_x0) + u64::from(tile_w) <= u64::from(x0) || u64::from(tile_y0) + u64::from(tile_h) <= u64::from(y0) {
        return Err(Error::Invalid("first tile outside the image"));
    }
    if csiz == 0 {
        return Err(Error::Invalid("no components"));
    }
    let mut components = Vec::with_capacity(usize::from(csiz).min(64));
    for _ in 0..csiz {
        let ssiz = r.u8()?;
        let (dx, dy) = (r.u8()?, r.u8()?);
        let precision = (ssiz & 0x7F) + 1;
        if dx == 0 || dy == 0 {
            return Err(Error::Invalid("component sub-sampling of zero"));
        }
        if precision > 16 {
            return Err(Error::Unsupported("components deeper than 16 bits"));
        }
        components.push(ComponentInfo { precision, signed: ssiz & 0x80 != 0, dx, dy });
    }
    // each component's own size (B.2), as the decoder allocates it
    let span = |a0: u32, a1: u32, d: u8| u64::from(a1).div_ceil(u64::from(d)) - u64::from(a0).div_ceil(u64::from(d));
    let samples = components.iter().try_fold(0u64, |sum, c| sum.checked_add(span(x0, x1, c.dx).checked_mul(span(y0, y1, c.dy))?));
    if samples.is_none_or(|n| n > MAX_SAMPLES) {
        return Err(Error::TooLarge);
    }
    Ok(Header { x0, y0, x1, y1, tile_x0, tile_y0, tile_w, tile_h, components, levels: 0, layers: 1, reversible: true, progression: 0 })
}

/// SPcod / SPcoc, with Scod / Scoc's "precincts defined" flag.
fn parse_sp(r: &mut Reader, precincts_defined: bool) -> Result<CodingStyle> {
    let levels = r.u8()?;
    let (cbw, cbh) = (r.u8()?.saturating_add(2), r.u8()?.saturating_add(2));
    let cblk_style = r.u8()?;
    let transform = r.u8()?;
    if levels > MAX_LEVELS {
        return Err(Error::Invalid("more than 32 decomposition levels"));
    }
    if cbw > 10 || cbh > 10 || cbw + cbh > 12 {
        return Err(Error::Invalid("code-block size"));
    }
    if transform > 1 {
        return Err(Error::Unsupported("Part 2 wavelet transform"));
    }
    if cblk_style & crate::ht::STYLE_HT_MIXED != 0 {
        return Err(Error::Unsupported("mixed HT and Part 1 code-blocks"));
    }
    let mut precincts = Vec::with_capacity(usize::from(levels) + 1);
    for _ in 0..=levels {
        if precincts_defined {
            let b = r.u8()?;
            precincts.push((b & 15, b >> 4));
        } else {
            precincts.push((15, 15));
        }
    }
    Ok(CodingStyle { levels, cbw, cbh, cblk_style, reversible: transform == 1, precincts })
}

fn parse_cod(s: &[u8]) -> Result<(Global, CodingStyle)> {
    let mut r = Reader { d: s, at: 0 };
    let scod = r.u8()?;
    let progression = r.u8()?;
    let layers = r.u16()?;
    let mct = r.u8()?;
    if progression > 4 {
        return Err(Error::Invalid("progression order"));
    }
    if layers == 0 {
        return Err(Error::Invalid("zero quality layers"));
    }
    let style = parse_sp(&mut r, scod & 1 != 0)?;
    Ok((Global { progression, layers, mct: mct == 1, sop: scod & 2 != 0, eph: scod & 4 != 0 }, style))
}

/// The component index of a COC / QCC / RGN segment (8 bits below 257 components, else 16).
fn component_index(r: &mut Reader, ncomp: usize) -> Result<usize> {
    let c = if ncomp < 257 { usize::from(r.u8()?) } else { usize::from(r.u16()?) };
    if c >= ncomp {
        return Err(Error::Invalid("marker for a component that does not exist"));
    }
    Ok(c)
}

fn parse_coc(s: &[u8], ncomp: usize) -> Result<(usize, CodingStyle)> {
    let mut r = Reader { d: s, at: 0 };
    let c = component_index(&mut r, ncomp)?;
    let scoc = r.u8()?;
    Ok((c, parse_sp(&mut r, scoc & 1 != 0)?))
}

fn parse_quant(r: &mut Reader) -> Result<Quant> {
    let s = r.u8()?;
    let (style, guard) = (s & 0x1F, s >> 5);
    let mut steps = Vec::new();
    match style {
        0 => {
            while r.at < r.d.len() {
                steps.push((r.u8()? >> 3, 0));
            }
        }
        1 | 2 => {
            while r.at + 1 < r.d.len() {
                let v = r.u16()?;
                steps.push(((v >> 11) as u8, v & 0x7FF));
            }
        }
        _ => return Err(Error::Invalid("quantization style")),
    }
    if steps.is_empty() {
        return Err(Error::Invalid("quantization without step sizes"));
    }
    Ok(Quant { style, guard, steps })
}

/// Coding parameters as markers set them, with what each was set by (for the precedence rules).
#[derive(Clone)]
struct Defaults {
    global: Option<Global>,
    cod: Option<CodingStyle>,
    coc: Vec<Option<CodingStyle>>,
    qcd: Option<Quant>,
    qcc: Vec<Option<Quant>>,
    poc: Vec<Poc>,
    rgn: Vec<Option<u8>>,
    /// PPM (main header) or PPT (tile-part header) segments: (Zppm / Zppt, data). Zppt restarts
    /// in every tile-part, so a tile-part's segments are taken out ([`Defaults::take_packed`])
    /// before the next one's header is read.
    packed: Vec<(u8, Vec<u8>)>,
    /// Bytes ever added to `packed` (kept as a running total so each marker costs O(1)).
    packed_bytes: usize,
}

/// Most bytes of packed packet headers a codestream may carry (bounds what hostile PPM / PPT
/// segments can make the decoder hold).
const MAX_PACKED: usize = 1 << 28;

impl Defaults {
    fn new(n: usize) -> Defaults {
        Defaults {
            global: None,
            cod: None,
            coc: vec![None; n],
            qcd: None,
            qcc: vec![None; n],
            poc: Vec::new(),
            rgn: vec![None; n],
            packed: Vec::new(),
            packed_bytes: 0,
        }
    }

    /// The packed segments in index order, concatenated.
    fn packed(&self) -> Vec<u8> {
        let mut segs: Vec<&(u8, Vec<u8>)> = self.packed.iter().collect();
        segs.sort_by_key(|s| s.0);
        segs.into_iter().flat_map(|s| s.1.iter().copied()).collect()
    }

    /// [`Defaults::packed`] of the segments read since the last call, which are removed (one
    /// tile-part's PPT segments); `None` when there were none.
    fn take_packed(&mut self) -> Option<Vec<u8>> {
        if self.packed.is_empty() {
            return None;
        }
        let out = self.packed();
        self.packed.clear();
        Some(out)
    }

    fn marker(&mut self, marker: u16, seg: &[u8], ncomp: usize) -> Result<()> {
        match marker {
            COD => {
                let (g, s) = parse_cod(seg)?;
                self.global = Some(g);
                self.cod = Some(s);
            }
            COC => {
                let (c, s) = parse_coc(seg, ncomp)?;
                if let Some(slot) = self.coc.get_mut(c) {
                    *slot = Some(s);
                }
            }
            QCD => self.qcd = Some(parse_quant(&mut Reader { d: seg, at: 0 })?),
            QCC => {
                let mut r = Reader { d: seg, at: 0 };
                let c = component_index(&mut r, ncomp)?;
                let q = parse_quant(&mut r)?;
                if let Some(slot) = self.qcc.get_mut(c) {
                    *slot = Some(q);
                }
            }
            RGN => {
                let mut r = Reader { d: seg, at: 0 };
                let c = component_index(&mut r, ncomp)?;
                if r.u8()? != 0 {
                    return Err(Error::Unsupported("region of interest style other than Maxshift"));
                }
                let shift = r.u8()?;
                if shift > 37 {
                    return Err(Error::Invalid("region of interest shift"));
                }
                if let Some(slot) = self.rgn.get_mut(c) {
                    *slot = Some(shift);
                }
            }
            POC => {
                let mut r = Reader { d: seg, at: 0 };
                while r.at < seg.len() {
                    let comp = |r: &mut Reader| if ncomp < 257 { r.u8().map(u16::from) } else { r.u16() };
                    let (rs, cs, layer_end, re) = (r.u8()?, comp(&mut r)?, r.u16()?, r.u8()?);
                    let ce = comp(&mut r)?;
                    let progression = r.u8()?;
                    if progression > 4 {
                        return Err(Error::Invalid("POC progression order"));
                    }
                    // CEpoc 0 means 256 (A.6.6)
                    let ce = if ce == 0 && ncomp < 257 { 256 } else { ce };
                    if self.poc.len() >= 1024 {
                        return Err(Error::Invalid("too many progression changes"));
                    }
                    self.poc.push(Poc { rs, cs, layer_end, re, ce, progression });
                }
            }
            PPM | PPT => {
                let (&z, data) = seg.split_first().ok_or(Error::Truncated)?;
                // Zppm / Zppt numbers the segments of one scope (the main header or one
                // tile-part), 0..=255, each once: at most 256 entries are ever held
                if self.packed.iter().any(|p| p.0 == z) {
                    return Err(Error::Invalid("repeated Zppm / Zppt index"));
                }
                self.packed_bytes = self.packed_bytes.saturating_add(data.len());
                if self.packed_bytes > MAX_PACKED {
                    return Err(Error::TooLarge);
                }
                self.packed.push((z, data.to_vec()));
            }
            _ => {}
        }
        Ok(())
    }

    /// Tile parameters: tile-part COC > tile-part COD > main COC > main COD (and the same for
    /// QCC / QCD).
    fn resolve(main: &Defaults, tile: &Defaults) -> Result<Params> {
        let global = tile.global.or(main.global).ok_or(Error::Invalid("no COD marker"))?;
        let n = main.coc.len();
        let mut cod = Vec::with_capacity(n);
        let mut quant = Vec::with_capacity(n);
        for c in 0..n {
            let style = tile.coc[c]
                .clone()
                .or_else(|| tile.cod.clone())
                .or_else(|| main.coc[c].clone())
                .or_else(|| main.cod.clone())
                .ok_or(Error::Invalid("no COD marker"))?;
            let q = tile.qcc[c]
                .clone()
                .or_else(|| tile.qcd.clone())
                .or_else(|| main.qcc[c].clone())
                .or_else(|| main.qcd.clone())
                .ok_or(Error::Invalid("no QCD marker"))?;
            cod.push(style);
            quant.push(q);
        }
        let roi = (0..n).map(|c| tile.rgn[c].or(main.rgn[c]).unwrap_or(0)).collect();
        let poc = if tile.poc.is_empty() { main.poc.clone() } else { tile.poc.clone() };
        Ok(Params { global, cod, quant, poc, roi })
    }
}

/// Parse the main header of codestream `cs` (not a JP2 file: see [`crate::codestream`]).
pub fn parse_header(data: &[u8]) -> Result<Header> {
    Ok(parse(data, false)?.header)
}

/// Parse a whole codestream; with `tiles` false, stop after the main header.
pub(crate) fn parse(data: &[u8], tiles: bool) -> Result<Codestream> {
    let cs = crate::codestream(data)?;
    let mut r = Reader { d: cs, at: 0 };
    if r.u16()? != SOC {
        return Err(Error::Invalid("no SOC marker"));
    }
    if r.u16()? != SIZ {
        return Err(Error::Invalid("SIZ does not follow SOC"));
    }
    let mut header = parse_siz(r.segment()?)?;
    let n = header.components.len();
    let mut main = Defaults::new(n);
    // main header
    loop {
        let at = r.at;
        let m = r.u16()?;
        if m == SOT {
            r.at = at;
            break;
        }
        if m == EOC {
            r.at = at;
            break;
        }
        if m >> 8 != 0xFF {
            return Err(Error::Invalid("expected a marker in the main header"));
        }
        let seg = r.segment()?;
        main.marker(m, seg, n)?;
    }
    let first = Defaults::resolve(&main, &Defaults::new(n))?;
    header.levels = first.cod.first().map_or(0, |c| c.levels);
    header.reversible = first.cod.first().is_some_and(|c| c.reversible);
    header.layers = first.global.layers;
    header.progression = first.global.progression;
    if !tiles {
        return Ok(Codestream { header, tiles: Vec::new() });
    }
    let (tw, th) = header.tiles();
    let ntiles = u64::from(tw) * u64::from(th);
    if ntiles > 65_535 {
        return Err(Error::Invalid("more than 65535 tiles"));
    }
    // tiles in order of their first tile-part: index, data, header defaults, packed headers (PPM,
    // or each tile-part's PPT in turn), whether any were given
    let mut parts: Vec<(u32, Vec<u8>, Defaults, Vec<u8>, bool)> = Vec::new();
    // PPM: Nppm and the packed headers of each tile-part in turn (A.7.4)
    let ppm = main.packed();
    let mut ppm_at = 0usize;
    // tile-parts
    while r.at + 2 <= cs.len() {
        let start = r.at;
        let m = r.u16()?;
        if m == EOC {
            break;
        }
        if m != SOT {
            return Err(Error::Invalid("expected SOT"));
        }
        let seg = r.segment()?;
        let mut sr = Reader { d: seg, at: 0 };
        let isot = u32::from(sr.u16()?);
        let psot = sr.u32()? as usize;
        let tpsot = sr.u8()?;
        if u64::from(isot) >= ntiles {
            return Err(Error::Invalid("tile index outside the tile grid"));
        }
        let end = if psot == 0 { cs.len() } else { start.checked_add(psot).ok_or(Error::Truncated)?.min(cs.len()) };
        // tile-part header
        let slot = match parts.iter().position(|p| p.0 == isot) {
            Some(i) => i,
            None => {
                parts.push((isot, Vec::new(), Defaults::new(n), Vec::new(), false));
                parts.len() - 1
            }
        };
        let Some((_, data, td, headers, has_headers)) = parts.get_mut(slot) else { return Err(Error::Invalid("tile")) };
        loop {
            let m = r.u16()?;
            if m == SOD {
                break;
            }
            if m >> 8 != 0xFF {
                return Err(Error::Invalid("expected a marker in a tile-part header"));
            }
            let seg = r.segment()?;
            // only the first tile-part may change the coding parameters (A.4.2)
            if tpsot == 0 || !matches!(m, COD | COC | QCD | QCC) {
                td.marker(m, seg, n)?;
            }
        }
        // this tile-part's PPT segments, in Zppt order, follow the previous tile-parts' (A.7.5)
        if let Some(ppt) = td.take_packed() {
            headers.extend_from_slice(&ppt);
            *has_headers = true;
        }
        let body = cs.get(r.at..end.max(r.at)).unwrap_or_default();
        // with Psot 0 the tile-part runs to EOC
        let body = if psot == 0 { body.strip_suffix(&[0xFF, 0xD9]).unwrap_or(body) } else { body };
        data.extend_from_slice(body);
        if !ppm.is_empty() {
            let mut pr = Reader { d: &ppm, at: ppm_at };
            let len = pr.u32()? as usize;
            headers.extend_from_slice(pr.bytes(len)?);
            *has_headers = true;
            ppm_at = pr.at;
        }
        r.at = end.max(r.at);
    }
    let mut out = Vec::with_capacity(parts.len());
    for (index, data, td, packed, has_headers) in parts {
        let params = Defaults::resolve(&main, &td)?;
        let headers = has_headers.then_some(packed);
        out.push(Tile { index, params, data, headers });
    }
    Ok(Codestream { header, tiles: out })
}
