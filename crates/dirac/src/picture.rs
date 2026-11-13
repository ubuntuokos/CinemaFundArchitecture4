// Ported from the SMPTE ST 2042-1 pseudocode of the BBC VC-2 conformance software
// (vc2_conformance 2.0.0, Copyright British Broadcasting Corporation, GPL-3.0-only); see
// docs/licensing.md §5.

//! Picture decoding (ST 2042-1 §12, §13, §15): transform parameters, low-delay and high-quality
//! slices, inverse quantisation, DC prediction, the inverse wavelet transform, clipping and
//! offsetting.

use crate::bits::Bits;
use crate::header::Sequence;
use crate::tables::{FILTERS, Filter, QUANT_MATRICES};
use crate::{Error, Result};

/// Largest padded component accepted (samples): 8K UHD fits.
const MAX_SAMPLES: u64 = 1 << 25;
/// Deepest transform accepted (`dwt_depth + dwt_depth_ho`).
const MAX_DEPTH: u32 = 10;

/// One decoded picture (a frame, or a field when the sequence codes fields): three components
/// with the offset of §15.5 applied, as unsigned samples.
pub(crate) struct Components {
    pub picture_number: u32,
    pub planes: [Vec<u16>; 3],
}

struct Transform {
    wavelet: &'static Filter,
    wavelet_ho: &'static Filter,
    depth: u32,
    depth_ho: u32,
    slices_x: u64,
    slices_y: u64,
    /// Low delay: slice bytes numerator and denominator.
    ld: Option<(u64, u64)>,
    /// High quality: prefix bytes and size scaler.
    hq: (u64, u64),
    /// Quantisation matrix in subband coding order.
    quant_matrix: Vec<u64>,
}

/// A subband: its level and size in one component.
#[derive(Clone, Copy)]
struct Band {
    w: usize,
    h: usize,
}

fn filter(index: u64) -> Result<&'static Filter> {
    usize::try_from(index).ok().and_then(|i| FILTERS.get(i)).ok_or(Error::Invalid("unknown wavelet"))
}

fn transform_parameters(b: &mut Bits<'_>, seq: &Sequence, ld: bool) -> Result<Transform> {
    let wavelet_index = b.read_uint()?;
    let depth = b.read_uint()?;
    let mut wavelet_index_ho = wavelet_index;
    let mut depth_ho = 0;
    if seq.major_version >= 3 {
        if b.read_bool()? {
            wavelet_index_ho = b.read_uint()?;
        }
        if b.read_bool()? {
            depth_ho = b.read_uint()?;
        }
    }
    if depth.saturating_add(depth_ho) > u64::from(MAX_DEPTH) {
        return Err(Error::Unsupported("transform depth"));
    }
    let (depth, depth_ho) = (depth as u32, depth_ho as u32);
    let slices_x = b.read_uint()?;
    let slices_y = b.read_uint()?;
    if slices_x == 0 || slices_y == 0 {
        return Err(Error::Invalid("zero slices"));
    }
    let (mut ld_bytes, mut hq) = (None, (0, 1));
    if ld {
        let (n, d) = (b.read_uint()?, b.read_uint()?);
        if d == 0 || n < d {
            return Err(Error::Invalid("slice bytes below one"));
        }
        ld_bytes = Some((n, d));
    } else {
        hq = (b.read_uint()?, b.read_uint()?);
        if hq.1 == 0 {
            return Err(Error::Invalid("zero slice size scaler"));
        }
    }
    let nbands = 1 + depth_ho as usize + 3 * depth as usize;
    let quant_matrix = if b.read_bool()? {
        (0..nbands).map(|_| b.read_uint()).collect::<Result<Vec<_>>>()?
    } else {
        let key = (wavelet_index, wavelet_index_ho, u64::from(depth), u64::from(depth_ho));
        let m = QUANT_MATRICES
            .iter()
            .find(|e| (u64::from(e.0), u64::from(e.1), u64::from(e.2), u64::from(e.3)) == key)
            .ok_or(Error::Unsupported("no default quantisation matrix for this transform"))?;
        m.4.iter().map(|&v| u64::from(v)).collect()
    };
    Ok(Transform {
        wavelet: filter(wavelet_index)?,
        wavelet_ho: filter(wavelet_index_ho)?,
        depth,
        depth_ho,
        slices_x,
        slices_y,
        ld: ld_bytes,
        hq,
        quant_matrix,
    })
}

/// The subbands of a `w` x `h` component in coding order (§13.2.2 `subband_width/height`).
fn bands(t: &Transform, w: u32, h: u32) -> Result<Vec<Band>> {
    let all = t.depth + t.depth_ho;
    let (sw, sh) = (1u64 << all, 1u64 << t.depth);
    let pw = u64::from(w).div_ceil(sw) * sw;
    let ph = u64::from(h).div_ceil(sh) * sh;
    if pw.saturating_mul(ph) > MAX_SAMPLES {
        return Err(Error::Unsupported("picture too large"));
    }
    let mut out = vec![Band { w: (pw >> all) as usize, h: (ph >> t.depth) as usize }];
    for level in 1..=all {
        let bw = (pw >> (all - level + 1)) as usize;
        let bh = if level <= t.depth_ho { (ph >> t.depth) as usize } else { (ph >> (all - level + 1)) as usize };
        let n = if level <= t.depth_ho { 1 } else { 3 };
        out.extend(std::iter::repeat_n(Band { w: bw, h: bh }, n));
    }
    Ok(out)
}

/// `quant_factor` and `quant_offset` (§13.3.2) for indices 0..=255; `inverse_quant` saturates.
struct Quant {
    factor: [u128; 256],
    offset: [u128; 256],
}

impl Quant {
    fn new() -> Self {
        let mut factor = [0u128; 256];
        let mut offset = [0u128; 256];
        for (i, (f, o)) in factor.iter_mut().zip(offset.iter_mut()).enumerate() {
            let base = 1u128 << (i / 4);
            *f = match i % 4 {
                0 => 4 * base,
                1 => (503829 * base + 52958) / 105917,
                2 => (665857 * base + 58854) / 117708,
                _ => (440253 * base + 32722) / 65444,
            };
            *o = match i {
                0 => 1,
                1 => 2,
                _ => f.div_ceil(2),
            };
        }
        Self { factor, offset }
    }

    #[inline]
    fn inverse(&self, v: i64, q: usize) -> i32 {
        if v == 0 {
            return 0;
        }
        let (f, o) = (self.factor.get(q).copied().unwrap_or(0), self.offset.get(q).copied().unwrap_or(0));
        let m = (u128::from(v.unsigned_abs()).saturating_mul(f).saturating_add(o).saturating_add(2) / 4).min(i32::MAX as u128) as i32;
        if v < 0 { -m } else { m }
    }
}

/// The coefficients of one component, one vector per subband.
type Coeffs = Vec<Vec<i32>>;

struct Slices<'a> {
    t: &'a Transform,
    bands: [Vec<Band>; 3],
    coeffs: [Coeffs; 3],
    quant: Quant,
    qi: Vec<usize>,
}

impl Slices<'_> {
    fn quantizers(&mut self, qindex: u64) {
        self.qi = self.t.quant_matrix.iter().map(|&m| qindex.saturating_sub(m).min(255) as usize).collect();
    }

    /// The `[x1, x2) x [y1, y2)` region of slice (`sx`, `sy`) in band `bi` of component `c`.
    fn region(&self, c: usize, bi: usize, sx: u64, sy: u64) -> (usize, usize, usize, usize, usize) {
        let band = self.bands[c].get(bi).copied().unwrap_or(Band { w: 0, h: 0 });
        let (w, h) = (band.w as u64, band.h as u64);
        let (sxn, syn) = (self.t.slices_x, self.t.slices_y);
        let x1 = (w * sx / sxn) as usize;
        let x2 = (w * (sx + 1) / sxn) as usize;
        let y1 = (h * sy / syn) as usize;
        let y2 = (h * (sy + 1) / syn) as usize;
        (x1, x2, y1, y2, band.w)
    }

    /// `slice_band` for every subband of component `c`.
    fn component(&mut self, b: &mut Bits<'_>, c: usize, sx: u64, sy: u64) -> Result<()> {
        for bi in 0..self.bands[c].len() {
            let q = self.qi.get(bi).copied().unwrap_or(0);
            let (x1, x2, y1, y2, w) = self.region(c, bi, sx, sy);
            let band = self.coeffs[c].get_mut(bi).ok_or(Error::Invalid("subband"))?;
            for y in y1..y2 {
                for x in x1..x2 {
                    let v = self.quant.inverse(b.read_sintb()?, q);
                    if let Some(s) = band.get_mut(y * w + x) {
                        *s = v;
                    }
                }
            }
        }
        Ok(())
    }

    /// `color_diff_slice_band` for every subband: the two colour difference components interleaved.
    fn color_diff(&mut self, b: &mut Bits<'_>, sx: u64, sy: u64) -> Result<()> {
        for bi in 0..self.bands[1].len() {
            let q = self.qi.get(bi).copied().unwrap_or(0);
            let (x1, x2, y1, y2, w) = self.region(1, bi, sx, sy);
            let [_, c1, c2] = &mut self.coeffs;
            let (Some(b1), Some(b2)) = (c1.get_mut(bi), c2.get_mut(bi)) else { return Err(Error::Invalid("subband")) };
            for y in y1..y2 {
                for x in x1..x2 {
                    let v1 = self.quant.inverse(b.read_sintb()?, q);
                    let v2 = self.quant.inverse(b.read_sintb()?, q);
                    if let (Some(s1), Some(s2)) = (b1.get_mut(y * w + x), b2.get_mut(y * w + x)) {
                        (*s1, *s2) = (v1, v2);
                    }
                }
            }
        }
        Ok(())
    }

    fn ld_slice(&mut self, b: &mut Bits<'_>, sx: u64, sy: u64, (num, den): (u64, u64)) -> Result<()> {
        let n = sy.saturating_mul(self.t.slices_x).saturating_add(sx);
        let bytes = (n + 1).checked_mul(num).ok_or(Error::Invalid("slice bytes"))? / den - n.checked_mul(num).ok_or(Error::Invalid("slice bytes"))? / den;
        let total = bytes.checked_mul(8).filter(|&t| t <= b.remaining()).ok_or(Error::Truncated)?;
        let mut left = total.checked_sub(7).ok_or(Error::Invalid("slice too small"))?;
        let qindex = b.read_nbits(7)?;
        self.quantizers(qindex);
        // intlog2(total - 7); a slice has at least 8 bits
        let length_bits = 64 - total.saturating_sub(8).leading_zeros();
        let y_len = b.read_nbits(length_bits)?;
        left = left.checked_sub(u64::from(length_bits)).ok_or(Error::Invalid("slice too small"))?;
        if y_len > left {
            return Err(Error::Invalid("slice luma length"));
        }
        b.bits_left = y_len;
        self.component(b, 0, sx, sy)?;
        b.flush_inputb()?;
        b.bits_left = left - y_len;
        self.color_diff(b, sx, sy)?;
        b.flush_inputb()
    }

    fn hq_slice(&mut self, b: &mut Bits<'_>, sx: u64, sy: u64) -> Result<()> {
        let (prefix, scaler) = self.t.hq;
        b.skip_bytes(prefix)?;
        let qindex = b.read_byte()?;
        self.quantizers(qindex);
        for c in 0..3 {
            let length = scaler.saturating_mul(b.read_byte()?);
            b.bits_left = length.saturating_mul(8);
            if b.bits_left > b.remaining() {
                return Err(Error::Truncated);
            }
            self.component(b, c, sx, sy)?;
            b.flush_inputb()?;
        }
        Ok(())
    }
}

/// `dc_prediction` (§13.4) on a `w`-wide band.
fn dc_prediction(band: &mut [i32], w: usize) {
    if w == 0 {
        return;
    }
    let h = band.len() / w;
    let at = |band: &[i32], x: usize, y: usize| i64::from(band.get(y * w + x).copied().unwrap_or(0));
    for y in 0..h {
        for x in 0..w {
            let p = if x > 0 && y > 0 {
                (at(band, x - 1, y) + at(band, x - 1, y - 1) + at(band, x, y - 1) + 1).div_euclid(3)
            } else if x > 0 {
                at(band, x - 1, 0)
            } else if y > 0 {
                at(band, 0, y - 1)
            } else {
                0
            };
            if let Some(v) = band.get_mut(y * w + x) {
                *v = (i64::from(*v) + p).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            }
        }
    }
}

#[inline]
fn sat(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `oned_synthesis` (§15.4.4.2): the lifting stages of `f` on `a` in place.
fn oned_synthesis(a: &mut [i32], f: &Filter) {
    let len = a.len() as i64;
    if len < 2 {
        return;
    }
    for st in f.stages {
        let odd_src = st.kind <= 2;
        let round = if st.s > 0 { 1i64 << (st.s - 1) } else { 0 };
        for n in 0..len / 2 {
            let mut sum = 0i64;
            for (k, &tap) in st.taps.iter().enumerate() {
                let i = st.d + k as i64;
                let pos = if odd_src { (2 * (n + i) - 1).clamp(1, len - 1) } else { (2 * (n + i)).clamp(0, len - 2) };
                sum += tap * i64::from(a.get(pos as usize).copied().unwrap_or(0));
            }
            let delta = (sum + round) >> st.s;
            let target = if odd_src { 2 * n } else { 2 * n + 1 } as usize;
            if let Some(v) = a.get_mut(target) {
                *v = sat(match st.kind {
                    1 | 3 => i64::from(*v) + delta,
                    _ => i64::from(*v) - delta,
                });
            }
        }
    }
}

fn bit_shift(a: &mut [i32], shift: u32) {
    if shift > 0 {
        let r = 1i64 << (shift - 1);
        for v in a {
            *v = ((i64::from(*v) + r) >> shift) as i32;
        }
    }
}

/// `idwt` (§15.4.1) of one component; returns the synthesised `w` x `h` array.
fn idwt(t: &Transform, bands: &[Band], mut coeffs: Coeffs) -> Result<(Vec<i32>, usize)> {
    let mut iter = coeffs.drain(..).zip(bands.iter().copied());
    let (mut dc, b0) = iter.next().ok_or(Error::Invalid("no subbands"))?;
    let (mut w, mut h) = (b0.w, b0.h);
    let shift = t.wavelet_ho.shift;
    for _ in 0..t.depth_ho {
        let (hb, _) = iter.next().ok_or(Error::Invalid("subband"))?;
        let mut synth = vec![0i32; 2 * w * h];
        for y in 0..h {
            for x in 0..w {
                if let (Some(s), Some(&l), Some(&hv)) = (synth.get_mut(2 * (y * w + x)..2 * (y * w + x) + 2), dc.get(y * w + x), hb.get(y * w + x)) {
                    s.copy_from_slice(&[l, hv]);
                }
            }
        }
        w *= 2;
        for row in synth.chunks_exact_mut(w) {
            oned_synthesis(row, t.wavelet_ho);
        }
        bit_shift(&mut synth, shift);
        dc = synth;
    }
    for _ in 0..t.depth {
        let (hl, _) = iter.next().ok_or(Error::Invalid("subband"))?;
        let (lh, _) = iter.next().ok_or(Error::Invalid("subband"))?;
        let (hh, _) = iter.next().ok_or(Error::Invalid("subband"))?;
        let (w2, h2) = (2 * w, 2 * h);
        let mut synth = vec![0i32; w2 * h2];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let get = |b: &Vec<i32>| b.get(i).copied().unwrap_or(0);
                let top = 2 * y * w2 + 2 * x;
                if let Some(s) = synth.get_mut(top..top + 2) {
                    s.copy_from_slice(&[get(&dc), get(&hl)]);
                }
                let bottom = top + w2;
                if let Some(s) = synth.get_mut(bottom..bottom + 2) {
                    s.copy_from_slice(&[get(&lh), get(&hh)]);
                }
            }
        }
        let mut col = vec![0i32; h2];
        for x in 0..w2 {
            for (y, c) in col.iter_mut().enumerate() {
                *c = synth.get(y * w2 + x).copied().unwrap_or(0);
            }
            oned_synthesis(&mut col, t.wavelet);
            for (y, &c) in col.iter().enumerate() {
                if let Some(s) = synth.get_mut(y * w2 + x) {
                    *s = c;
                }
            }
        }
        for row in synth.chunks_exact_mut(w2) {
            oned_synthesis(row, t.wavelet_ho);
        }
        bit_shift(&mut synth, shift);
        (dc, w, h) = (synth, w2, h2);
    }
    Ok((dc, w))
}

/// Decode a picture (parse code `parse_code`) from the bytes after its parse info header.
/// Returns the picture and the bytes it used.
pub(crate) fn decode(seq: &Sequence, ld: bool, data: &[u8]) -> Result<(Components, usize)> {
    let mut b = Bits::new(data);
    let picture_number = b.read_nbits(32)? as u32;
    let t = transform_parameters(&mut b, seq, ld)?;
    b.byte_align();
    let dims = [(seq.luma_width, seq.luma_height), (seq.color_diff_width, seq.color_diff_height), (seq.color_diff_width, seq.color_diff_height)];
    let bands = [bands(&t, dims[0].0, dims[0].1)?, bands(&t, dims[1].0, dims[1].1)?, bands(&t, dims[2].0, dims[2].1)?];
    // Every slice reads at least one byte: reject slice counts the data can't hold before looping.
    let nslices = t.slices_x.checked_mul(t.slices_y).ok_or(Error::Invalid("slice count"))?;
    if nslices > b.remaining() / 8 {
        return Err(Error::Truncated);
    }
    let coeffs = bands.clone().map(|bs| bs.iter().map(|b| vec![0i32; b.w * b.h]).collect::<Coeffs>());
    let mut s = Slices { t: &t, bands, coeffs, quant: Quant::new(), qi: Vec::new() };
    for sy in 0..t.slices_y {
        for sx in 0..t.slices_x {
            match t.ld {
                Some(nd) => s.ld_slice(&mut b, sx, sy, nd)?,
                None => s.hq_slice(&mut b, sx, sy)?,
            }
        }
    }
    let Slices { bands, mut coeffs, .. } = s;
    if t.ld.is_some() {
        for (c, bs) in coeffs.iter_mut().zip(&bands) {
            if let (Some(band), Some(info)) = (c.get_mut(0), bs.first()) {
                dc_prediction(band, info.w);
            }
        }
    }
    let depths = [seq.luma_depth, seq.color_diff_depth, seq.color_diff_depth];
    let mut planes: [Vec<u16>; 3] = Default::default();
    for (c, coeffs) in coeffs.into_iter().enumerate() {
        let (pic, pw) = idwt(&t, &bands[c], coeffs)?;
        let (w, h) = (dims[c].0 as usize, dims[c].1 as usize);
        let d = depths[c];
        let (lo, hi) = (-(1i32 << (d - 1)), (1i32 << (d - 1)) - 1);
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let row = pic.get(y * pw..y * pw + w).ok_or(Error::Invalid("picture smaller than its dimensions"))?;
            out.extend(row.iter().map(|&v| (v.clamp(lo, hi) - lo) as u16));
        }
        planes[c] = out;
    }
    b.byte_align();
    Ok((Components { picture_number, planes }, b.bytes_used()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiser_tables() {
        let q = Quant::new();
        // index 0 is lossless; index 4 doubles
        assert_eq!(q.inverse(5, 0), 5);
        assert_eq!(q.inverse(-5, 0), -5);
        assert_eq!(q.factor[4], 8);
        assert_eq!(q.offset[4], 4);
        assert_eq!(q.inverse(3, 4), 7);
        assert_eq!(q.inverse(i64::MAX, 255), i32::MAX);
    }

    #[test]
    fn haar_synthesis_inverts_analysis() {
        // Haar analysis of (7, 12): H = 12 - 7, L = 7 + ((H + 1) >> 1)
        let mut s = [10, 5];
        oned_synthesis(&mut s, &FILTERS[3]);
        assert_eq!(s, [7, 12]);
    }
}
