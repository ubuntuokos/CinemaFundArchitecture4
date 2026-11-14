//! The structure of one picture frame against the DCI JPEG 2000 profiles (plan F07): the main
//! header (SIZ and COD through `cfa4-j2k`, the COD / QCD details and the optional markers from a
//! marker walk here) and the tile-parts. Nothing is decoded.
//!
//! Source: ISO/IEC 15444-1 Annex A (codestream syntax) and its Amendment 1 (the 2K and 4K digital
//! cinema profiles, `Rsiz` 3 and 4), as SMPTE ST 429-4 requires them for D-Cinema pictures:
//!
//! - one tile, image and tile origin at 0, three components of 12 bits, unsigned, not sub-sampled;
//! - 2K at most 2048 × 1080 with 1-5 decomposition levels, 4K at most 4096 × 2160 with 1-6;
//! - CPRL progression, one quality layer, the irreversible 9/7 wavelet, 32 × 32 code-blocks with
//!   no code-block style flags, precincts of 128 × 128 at the lowest resolution and 256 × 256 above;
//! - scalar expounded quantization with 1 guard bit (2K) or 2 (4K);
//! - one tile-part per component (2K: 3), and for 4K one more per component for the 4K resolution
//!   (6), which the progression order change (POC) describes.
//!
//! The multiple component transform (MCT, the ICT on X'Y'Z') is not a profile failure. A frame
//! with it passes; a frame without it is a remark (`WARNING`), because libdcp's `dcpverify` rejects
//! such frames and the DCI DCSS (§4.3.2) has every decoder apply the inverse ICT. The ISO/IEC
//! 15444-1 profile table itself was not checked (plan CFA4-DCPVAL-LIBDCP-001, D2 and D4).

use cfa4_j2k::Header;

const SOC: u16 = 0xFF4F;
const SIZ: u16 = 0xFF51;
const COD: u16 = 0xFF52;
const TLM: u16 = 0xFF55;
const QCD: u16 = 0xFF5C;
const POC: u16 = 0xFF5F;
const SOT: u16 = 0xFF90;
const EOC: u16 = 0xFFD9;

/// Bytes of a frame read first for its main header; a longer header is read again whole.
pub(crate) const HEAD: usize = 4096;
/// The longest main header accepted.
pub(crate) const MAX_HEAD: usize = 64 << 10;

/// The COD fields `cfa4-j2k` keeps to itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Cod {
    /// Code-block width and height exponents as written (exponent - 2).
    xcb: u8,
    ycb: u8,
    style: u8,
    /// Precinct sizes (PPx | PPy << 4) per resolution, lowest first; empty for the default.
    precincts: Vec<u8>,
}

/// What the marker walk of the main header found.
#[derive(Debug, Default)]
pub(crate) struct Main {
    /// Offset of the first SOT (the end of the main header).
    pub end: usize,
    cod: Option<Cod>,
    /// Sqcd: guard bits << 5 | quantization style.
    sqcd: Option<u8>,
    tlm: bool,
    poc: bool,
}

/// Walks the main header in `b` (which starts with the frame). `Ok(None)`: `b` ends before the
/// first SOT.
pub(crate) fn main_header(b: &[u8]) -> Result<Option<Main>, String> {
    let u16_at = |at: usize| b.get(at..at + 2).map(|v| u16::from_be_bytes([v[0], v[1]]));
    if b.len() < 4 {
        return Ok(None);
    }
    if u16_at(0) != Some(SOC) {
        return Err("the frame is not a JPEG 2000 codestream (no SOC marker)".into());
    }
    if u16_at(2) != Some(SIZ) {
        return Err("SIZ does not follow SOC".into());
    }
    let mut m = Main::default();
    let mut at = 2;
    loop {
        let Some(marker) = u16_at(at) else { return Ok(None) };
        if marker == SOT {
            m.end = at;
            return Ok(Some(m));
        }
        if marker == EOC {
            return Err("the codestream ends (EOC) before any tile-part".into());
        }
        if marker >> 8 != 0xFF || marker < 0xFF30 {
            return Err(format!("expected a marker in the main header at byte {at}, found {marker:04X}"));
        }
        let Some(len) = u16_at(at + 2) else { return Ok(None) };
        let len = usize::from(len);
        if len < 2 {
            return Err(format!("marker {marker:04X} has a segment length of {len}"));
        }
        let Some(seg) = b.get(at + 4..at + 2 + len) else { return Ok(None) };
        match marker {
            COD => {
                // Scod, progression, layers (2), MCT, levels, xcb, ycb, style, transform, precincts
                let Some(&[scod, _, _, _, _, levels, xcb, ycb, style, _]) = seg.get(..10).and_then(|s| <&[u8; 10]>::try_from(s).ok()) else {
                    return Err("COD segment is too short".into());
                };
                let precincts = if scod & 1 == 1 { seg.get(10..10 + usize::from(levels) + 1).map(<[u8]>::to_vec).unwrap_or_default() } else { Vec::new() };
                m.cod = Some(Cod { xcb, ycb, style, precincts });
            }
            QCD => m.sqcd = seg.first().copied(),
            TLM => m.tlm = true,
            POC => m.poc = true,
            _ => {}
        }
        at += 2 + len;
        if at > MAX_HEAD {
            return Err(format!("the main header is longer than {MAX_HEAD} bytes"));
        }
    }
}

/// A 4K frame (the 4K profile): larger than the 2K container.
pub(crate) fn four_k(h: &Header) -> bool {
    h.width() > 2048 || h.height() > 1080
}

/// What the main header breaks of the DCI profile; empty when it follows it.
pub(crate) fn profile_problems(h: &Header, m: &Main) -> Vec<String> {
    let mut out = Vec::new();
    let k4 = four_k(h);
    let (want_rsiz, max_w, max_h, max_levels, guard, name) = if k4 { (4, 4096, 2160, 6, 2, "4K") } else { (3, 2048, 1080, 5, 1, "2K") };
    if h.capabilities != want_rsiz {
        out.push(format!("Rsiz is {}, the {name} digital cinema profile is {want_rsiz}", h.capabilities));
    }
    if h.width() > max_w || h.height() > max_h {
        out.push(format!("the image is {} × {}, larger than the 4K profile's {max_w} × {max_h}", h.width(), h.height()));
    }
    if h.x0 != 0 || h.y0 != 0 || h.tile_x0 != 0 || h.tile_y0 != 0 {
        out.push("the image or tile origin is not 0".into());
    }
    if h.tiles() != (1, 1) {
        let (x, y) = h.tiles();
        out.push(format!("{x} × {y} tiles; the profile has one tile"));
    }
    let dci_component = |c: &cfa4_j2k::ComponentInfo| c.precision == 12 && !c.signed && c.dx == 1 && c.dy == 1;
    if h.components.len() != 3 || !h.components.iter().all(dci_component) {
        out.push(format!("{} components; the profile has three of 12 bits, unsigned and not sub-sampled", h.components.len()));
    }
    if !(1..=max_levels).contains(&h.levels) {
        out.push(format!("{} decomposition levels; the {name} profile has 1 to {max_levels}", h.levels));
    }
    if h.progression != 4 {
        out.push(format!(
            "progression order {}; the profile is CPRL",
            ["LRCP", "RLCP", "RPCL", "PCRL", "CPRL"].get(usize::from(h.progression)).unwrap_or(&"unknown")
        ));
    }
    if h.layers != 1 {
        out.push(format!("{} quality layers; the profile has one", h.layers));
    }
    if h.reversible {
        out.push("the reversible 5/3 wavelet; the profile uses the irreversible 9/7".into());
    }
    match &m.cod {
        None => out.push("no COD marker in the main header".into()),
        Some(c) => {
            if (c.xcb, c.ycb) != (3, 3) {
                out.push(format!("code-blocks of {} × {}; the profile uses 32 × 32", 1u32 << (c.xcb + 2).min(31), 1u32 << (c.ycb + 2).min(31)));
            }
            if c.style != 0 {
                out.push(format!("code-block style {:#04x}; the profile uses none", c.style));
            }
            let wanted = |r: usize| if r == 0 { 0x77 } else { 0x88 };
            if c.precincts.is_empty() || c.precincts.iter().enumerate().any(|(r, &p)| p != wanted(r)) {
                out.push("precincts are not 128 × 128 at the lowest resolution and 256 × 256 above".into());
            }
        }
    }
    match m.sqcd {
        None => out.push("no QCD marker in the main header".into()),
        Some(s) => {
            if s & 0x1F != 2 {
                out.push(format!("quantization style {}; the profile uses scalar expounded (2)", s & 0x1F));
            }
            if s >> 5 != guard {
                out.push(format!("{} guard bits; the {name} profile has {guard}", s >> 5));
            }
        }
    }
    out
}

/// Remarks on the main header that do not break the profile.
pub(crate) fn header_remarks(h: &Header, m: &Main) -> Vec<String> {
    let mut out = Vec::new();
    if !m.tlm {
        out.push("no TLM marker (tile-part lengths); servers find tile-parts without it, but more slowly".into());
    }
    if !h.mct {
        out.push(
            "no multiple component transform (ICT): libdcp's dcpverify rejects such a frame, and DCI decoders all apply \
             the inverse ICT (DCSS §4.3.2)"
                .into(),
        );
    }
    if four_k(h) && !m.poc {
        out.push("a 4K frame without a POC marker: a 2K projector cannot take the 2K resolutions from it".into());
    }
    out
}

/// The tile-parts of one frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TileParts {
    /// Length of each tile-part (Psot), in order.
    pub lengths: Vec<u64>,
    pub problems: Vec<String>,
    /// The codestream does not end with EOC right after the last tile-part.
    pub no_eoc: bool,
}

/// Walks the tile-parts of a frame of `size` bytes from `start` (the first SOT). `read(at, n)`
/// gives `n` bytes of the frame at `at`.
pub(crate) fn tile_parts(size: u64, start: u64, mut read: impl FnMut(u64, usize) -> std::io::Result<Vec<u8>>) -> std::io::Result<TileParts> {
    let mut t = TileParts::default();
    let mut at = start;
    // a tile-part is at least SOT (12 bytes) and SOD (2), so this bounds the walk
    for _ in 0..=size / 14 {
        if at + 2 > size {
            t.no_eoc = true;
            return Ok(t);
        }
        let m = read(at, 2)?;
        let marker = u16::from_be_bytes([m[0], m[1]]);
        if marker == EOC {
            t.no_eoc = at + 2 != size;
            return Ok(t);
        }
        if marker != SOT {
            t.problems.push(format!("expected SOT or EOC at byte {at}, found {marker:04X}"));
            return Ok(t);
        }
        if at + 12 > size {
            t.problems.push(format!("the SOT at byte {at} is cut short"));
            return Ok(t);
        }
        let s = read(at, 12)?;
        let (isot, psot, tpsot) = (u16::from_be_bytes([s[4], s[5]]), u32::from_be_bytes([s[6], s[7], s[8], s[9]]), s[10]);
        if isot != 0 {
            t.problems.push(format!("tile-part {} is of tile {isot}; the profile has one tile", t.lengths.len()));
        }
        if usize::from(tpsot) != t.lengths.len() {
            t.problems.push(format!("tile-part {} has the index {tpsot}", t.lengths.len()));
        }
        // Psot 0: the last tile-part, up to EOC
        let len = if psot == 0 { size.saturating_sub(at + 2) } else { u64::from(psot) };
        if len < 14 || at + len > size {
            t.problems.push(format!("tile-part {} has the length {psot}, outside the frame", t.lengths.len()));
            return Ok(t);
        }
        t.lengths.push(len);
        at += len;
    }
    t.problems.push("too many tile-parts".into());
    Ok(t)
}

/// The bytes of each component, when the tile-parts follow the profile's layout (one per component,
/// and for 4K a second one per component); `None` otherwise.
pub(crate) fn component_bytes(lengths: &[u64], four_k: bool) -> Option<[u64; 3]> {
    match (lengths, four_k) {
        (&[a, b, c], false) => Some([a, b, c]),
        (&[a, b, c, d, e, f], true) => Some([a + d, b + e, c + f]),
        _ => None,
    }
}
