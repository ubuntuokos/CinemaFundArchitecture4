// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! Pure-Rust MPEG-5 EVC (ISO/IEC 23094-1) Baseline profile decoder.
//!
//! A port of the Baseline part of Samsung's xevd v0.8.0 (`src_base`, BSD-3-Clause,
//! see `COPYING.BSD-3`): 64×64 coding tree units with quad-tree splits down to
//! 4×4, the five Baseline intra modes, run-length coefficient coding, quarter-
//! sample motion compensation with the Baseline motion vector prediction and
//! direct mode, the Baseline deblocking filter and the reference structure
//! derived from the temporal id (no reference picture lists). The output is
//! bit-exact with xevd.
//!
//! The elementary stream is a sequence of NAL units, each preceded by its
//! length as a 4-byte big-endian number (the raw `.evc` format of xeve/xevd
//! and FFmpeg; the same layout as MP4 `evc1` samples with 4-byte lengths).

mod bits;
mod dpb;
mod dsp;
mod params;
mod sbac;
mod slice;
mod tables;

use dpb::{Dpb, MAX_REFS, Refp};
use params::{Pps, Sh, Sps};

/// Errors of the EVC decoder.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("EVC stream is truncated")]
    Truncated,
    #[error("malformed EVC stream: {0}")]
    Malformed(&'static str),
    #[error("unsupported EVC stream: {0}")]
    Unsupported(&'static str),
}

/// Chroma sampling of the decoded pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    /// 4:0:0 (luma only).
    Mono,
    /// 4:2:0.
    Yuv420,
}

/// Properties of the stream from its sequence parameter set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    /// Size of the output pictures (after cropping).
    pub width: u32,
    pub height: u32,
    pub chroma: Chroma,
    pub bit_depth: u32,
    /// Sample aspect ratio, when the VUI gives one.
    pub sar: Option<(u32, u32)>,
    /// `(num_units_in_tick, time_scale)` from the VUI timing information.
    pub timing: Option<(u32, u32)>,
    pub profile_idc: u32,
    pub level_idc: u32,
}

/// One decoded picture in output order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub chroma: Chroma,
    pub bit_depth: u32,
    /// Picture order count (relative to the last IDR picture).
    pub poc: i32,
    /// The tag given to [`Decoder::decode_tagged`] with the picture's NAL
    /// unit (0 for [`Decoder::decode_nal`]); carries timestamps through
    /// reordering.
    pub tag: i64,
    /// Y, then Cb and Cr for 4:2:0. Each plane is tightly packed; chroma
    /// planes are `(width + 1) / 2 × (height + 1) / 2`.
    pub planes: Vec<Vec<u16>>,
}

impl Frame {
    /// Width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        if i == 0 { (self.width as usize, self.height as usize) } else { ((self.width as usize).div_ceil(2), (self.height as usize).div_ceil(2)) }
    }
}

fn stream_info(sps: &Sps) -> StreamInfo {
    let (w, h) = output_size(sps);
    StreamInfo {
        width: w as u32,
        height: h as u32,
        chroma: if sps.chroma_format_idc == 0 { Chroma::Mono } else { Chroma::Yuv420 },
        bit_depth: sps.bit_depth_luma,
        sar: sps.sar,
        timing: sps.timing,
        profile_idc: sps.profile_idc,
        level_idc: sps.level_idc,
    }
}

/// Output (cropped) luma size. Like xevd, cropping is applied to the size
/// aligned to the 4-sample grid.
fn output_size(sps: &Sps) -> (usize, usize) {
    let (w, h) = (sps.width as usize, sps.height as usize);
    match sps.crop {
        Some(c) => {
            let aw = w.div_ceil(4) * 4;
            let ah = h.div_ceil(4) * 4;
            let cs = 2;
            (aw.saturating_sub((c[0] as usize + c[1] as usize) * cs), ah.saturating_sub((c[2] as usize + c[3] as usize) * cs))
        }
        None => (w, h),
    }
}

/// Splits a raw `.evc` stream (4-byte big-endian length before every NAL
/// unit) into NAL units. Stops at the first length that runs past the data.
pub fn split_nal_units(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let len_bytes: [u8; 4] = rest.get(..4)?.try_into().ok()?;
        let len = u32::from_be_bytes(len_bytes) as usize;
        let nal = rest.get(4..4usize.checked_add(len)?)?;
        rest = rest.get(4 + len..).unwrap_or(&[]);
        Some(nal)
    })
}

/// Reads the stream properties from the first sequence parameter set in the
/// beginning of a raw `.evc` stream. `None` when `head` does not start with a
/// length-prefixed EVC NAL unit or holds no valid SPS.
pub fn probe(head: &[u8]) -> Option<StreamInfo> {
    for (i, nal) in split_nal_units(head).enumerate() {
        if i >= 16 {
            break;
        }
        let mut bs = bits::Bits::new(nal);
        let hdr = params::nal_header(&mut bs).ok()?;
        if hdr.nal_type == params::NUT_SPS {
            return params::sps(&mut bs).ok().map(|s| stream_info(&s));
        }
    }
    None
}

/// The NAL unit type (`nal_unit_type_plus1 - 1`) and temporal id of a NAL
/// unit, `None` when its header is invalid.
pub fn nal_type(nal: &[u8]) -> Option<(i32, u8)> {
    let mut bs = bits::Bits::new(nal);
    params::nal_header(&mut bs).ok().map(|h| (h.nal_type, h.temporal_id))
}

/// NAL unit type of a sequence parameter set.
pub const NUT_SPS: i32 = params::NUT_SPS;
/// NAL unit type of an IDR picture.
pub const NUT_IDR: i32 = params::NUT_IDR;

/// What the headers of a picture's slice tell, without decoding it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PicInfo {
    /// An IDR picture: decoding can start here.
    pub idr: bool,
    /// An intra slice (an IDR or a clean random access picture).
    pub intra: bool,
    pub temporal_id: u8,
    /// Picture order count, from 0 at the last IDR picture.
    pub poc: i32,
}

/// Follows the parameter sets, slice headers and picture order counts of a
/// stream without decoding pictures (for indexing a file).
#[derive(Default)]
pub struct Scanner {
    sps: Option<Sps>,
    pps: Option<Pps>,
    sh: Sh,
    poc: Poc,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the headers of one NAL unit; a picture's slice gives its
    /// [`PicInfo`]. Only the start of a slice NAL unit is needed (its header
    /// is short: a few bytes with a single tile).
    pub fn push(&mut self, nal: &[u8]) -> Result<Option<PicInfo>, Error> {
        let mut bs = bits::Bits::new(nal);
        let hdr = params::nal_header(&mut bs)?;
        match hdr.nal_type {
            params::NUT_SPS => {
                self.sps = Some(params::sps(&mut bs)?);
                Ok(None)
            }
            params::NUT_PPS => {
                self.pps = Some(params::pps(&mut bs)?);
                Ok(None)
            }
            t if t < params::NUT_SPS => {
                let (Some(sps), Some(pps)) = (&self.sps, &self.pps) else {
                    return Err(Error::Malformed("slice before the parameter sets"));
                };
                params::slice_header(&mut bs, sps, pps, &mut self.sh, t)?;
                let idr = t == params::NUT_IDR;
                if idr {
                    self.poc = Poc { poc_val: 0, prev_poc_val: 0, prev_doc_offset: -1 };
                } else {
                    self.poc.derive(hdr.temporal_id, sps.log2_sub_gop_length)?;
                }
                Ok(Some(PicInfo { idr, intra: self.sh.slice_type == params::SLICE_I, temporal_id: hdr.temporal_id, poc: self.poc.poc_val }))
            }
            _ => Ok(None),
        }
    }
}

/// Picture order count state (`XEVD_POC`).
#[derive(Clone, Copy, Default)]
struct Poc {
    poc_val: i32,
    prev_poc_val: i32,
    prev_doc_offset: i32,
}

impl Poc {
    /// `xevd_poc_derivation`.
    fn derive(&mut self, tid: u8, log2_sub_gop: u32) -> Result<(), Error> {
        let sub_gop = 1i32 << log2_sub_gop;
        let tid = i32::from(tid);
        let p = self;
        if tid == 0 {
            p.poc_val = p.prev_poc_val.wrapping_add(sub_gop);
            p.prev_doc_offset = 0;
            p.prev_poc_val = p.poc_val;
            return Ok(());
        }
        let ilog2 = |v: i32| 31 - (v.max(1) as u32).leading_zeros() as i32;
        let mut expected = 0;
        let mut doc_offset = (p.prev_doc_offset + 1).rem_euclid(sub_gop.max(1));
        if doc_offset == 0 {
            p.prev_poc_val = p.prev_poc_val.wrapping_add(sub_gop);
        } else {
            expected = 1 + ilog2(doc_offset);
        }
        let mut guard = 0;
        while tid != expected {
            doc_offset = (doc_offset + 1) % sub_gop.max(1);
            expected = if doc_offset == 0 { 0 } else { 1 + ilog2(doc_offset) };
            guard += 1;
            if guard > sub_gop + 1 {
                return Err(Error::Malformed("temporal id does not fit the sub-GOP"));
            }
        }
        let offset = (f64::from(sub_gop) * ((2.0 * f64::from(doc_offset) + 1.0) / f64::from(1i32 << tid.min(30)) - 2.0)) as i32;
        p.poc_val = p.prev_poc_val.wrapping_add(offset);
        p.prev_doc_offset = doc_offset;
        Ok(())
    }
}

/// EVC Baseline decoder. Feed NAL units in stream order with
/// [`Decoder::decode_nal`]; call [`Decoder::flush`] at the end of the stream.
pub struct Decoder {
    sps: Option<Sps>,
    pps: Option<Pps>,
    sh: Sh,
    seq: Option<slice::Seq>,
    dpb: Dpb,
    refp: [[Refp; 2]; MAX_REFS],
    poc: Poc,
    pic_cnt: u32,
    cra_start_poc: i32,
    last_intra_poc: i32,
    /// Chroma QP mapping (`xevd_tbl_qp_chroma_dynamic_ext`), indexed with
    /// `qp + chroma_qp_base`.
    chroma_qp: [[i32; 94]; 2],
    chroma_qp_base: i32,
    ref_pic_gap_length: i32,
    /// Tag of the NAL unit being decoded.
    tag: i64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            sps: None,
            pps: None,
            sh: Sh::default(),
            seq: None,
            dpb: Dpb::new(),
            refp: [[Refp::default(); 2]; MAX_REFS],
            poc: Poc::default(),
            pic_cnt: 0,
            cra_start_poc: 0,
            last_intra_poc: 0,
            chroma_qp: [[0; 94]; 2],
            chroma_qp_base: 0,
            ref_pic_gap_length: 1,
            tag: 0,
        }
    }

    /// Properties from the active sequence parameter set.
    pub fn stream_info(&self) -> Option<StreamInfo> {
        self.sps.as_ref().map(stream_info)
    }

    /// Decodes one NAL unit (without its length prefix). Returns the next
    /// picture in output order when one is ready.
    pub fn decode_nal(&mut self, nal: &[u8]) -> Result<Option<Frame>, Error> {
        self.decode_tagged(nal, 0)
    }

    /// [`Decoder::decode_nal`] with a tag (a timestamp, say) that comes back
    /// in [`Frame::tag`] of the picture decoded from `nal`.
    pub fn decode_tagged(&mut self, nal: &[u8], tag: i64) -> Result<Option<Frame>, Error> {
        self.tag = tag;
        let mut bs = bits::Bits::new(nal);
        let hdr = params::nal_header(&mut bs)?;
        match hdr.nal_type {
            params::NUT_SPS => {
                let sps = params::sps(&mut bs)?;
                self.sequence_init(&sps)?;
                self.sps = Some(sps);
                Ok(None)
            }
            params::NUT_PPS => {
                if self.sps.is_none() {
                    return Err(Error::Malformed("PPS before any SPS"));
                }
                self.pps = Some(params::pps(&mut bs)?);
                Ok(None)
            }
            t if t < params::NUT_SPS => {
                if self.decode_picture(bs, hdr)? {
                    Ok(self.output(false))
                } else {
                    Ok(None)
                }
            }
            // SEI, APS and filler data carry nothing the Baseline decoder needs.
            params::NUT_SEI => Ok(None),
            _ => Ok(None),
        }
    }

    /// Returns the pictures still waiting for output, in output order.
    pub fn flush(&mut self) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(f) = self.output(true) {
            out.push(f);
        }
        out
    }

    /// Forgets all pictures (after a seek). Parameter sets are kept.
    pub fn reset(&mut self) {
        self.dpb.clear();
        self.refp = [[Refp::default(); 2]; MAX_REFS];
        self.poc = Poc::default();
        self.pic_cnt = 0;
        self.cra_start_poc = 0;
        self.last_intra_poc = 0;
    }

    fn sequence_init(&mut self, sps: &Sps) -> Result<(), Error> {
        if sps.bit_depth_luma != sps.bit_depth_chroma {
            return Err(Error::Unsupported("different luma and chroma bit depths"));
        }
        let same = self.seq.as_ref().is_some_and(|s| s.matches(sps));
        if !same {
            self.seq = Some(slice::Seq::new(sps));
            self.reset();
        }
        self.ref_pic_gap_length = 1i32 << sps.log2_ref_pic_gap_length.min(16);
        // xevd_set_chroma_qp_tbl_loc + the default or signalled mapping.
        let off = 6 * (sps.bit_depth_luma as i32 - 8);
        self.chroma_qp_base = off;
        for t in &mut self.chroma_qp {
            for i in 0..off {
                t[i as usize] = i - off;
            }
        }
        if sps.chroma_qp.present {
            self.derive_chroma_qp(sps)?;
        } else {
            let base = 6 * (sps.bit_depth_chroma as usize - 8);
            for t in &mut self.chroma_qp {
                for (i, v) in tables::QP_CHROMA_ADJUST_BASE.iter().enumerate() {
                    if let Some(d) = t.get_mut(base + i) {
                        *d = *v;
                    }
                }
            }
        }
        Ok(())
    }

    /// `xevd_derived_chroma_qp_mapping_tables`.
    fn derive_chroma_qp(&mut self, sps: &Sps) -> Result<(), Error> {
        let t = &sps.chroma_qp;
        let max_qp = 57i32;
        let off = 6 * (sps.bit_depth_chroma as i32 - 8);
        let start = if t.global_offset { 16 } else { -off };
        let base = self.chroma_qp_base;
        let bad = Error::Malformed("chroma QP mapping table out of range");
        let tables = if t.same_for_chroma { 1 } else { 2 };
        for i in 0..tables {
            let n = t.num_points_minus1[i] as usize;
            let mut qin = [0i32; 94];
            let mut qout = [0i32; 94];
            let din = |j: usize| t.delta_in_minus1[i].get(j).copied().unwrap_or(0);
            let dout = |j: usize| t.delta_out[i].get(j).copied().unwrap_or(0);
            qin[0] = start + din(0);
            qout[0] = qin[0].saturating_add(dout(0));
            for j in 1..=n {
                qin[j] = qin[j - 1] + din(j) + 1;
                qout[j] = qout[j - 1].saturating_add(din(j) + 1).saturating_add(dout(j));
            }
            for j in 0..=n {
                if !(-off..=max_qp).contains(&qin[j]) || !(-off..=max_qp).contains(&qout[j]) {
                    return Err(bad);
                }
            }
            let tab = &mut self.chroma_qp[i];
            let idx = |k: i32| -> Result<usize, Error> {
                let v = k + base;
                if (0..94).contains(&v) { Ok(v as usize) } else { Err(Error::Malformed("chroma QP mapping table out of range")) }
            };
            tab[idx(qin[0])?] = qout[0];
            let mut k = qin[0] - 1;
            while k >= -off {
                let next = tab[idx(k + 1)?];
                tab[idx(k)?] = (next - 1).clamp(-off, max_qp);
                k -= 1;
            }
            for j in 0..n {
                let d = din(j + 1) + 1;
                let sh = d >> 1;
                let base_v = tab[idx(qin[j])?];
                for (m, k) in (qin[j] + 1..=qin[j + 1]).enumerate() {
                    let m = m as i32 + 1;
                    tab[idx(k)?] = base_v + ((qout[j + 1] - qout[j]) * m + sh) / d;
                }
            }
            for k in qin[n] + 1..=max_qp {
                let prev = tab[idx(k - 1)?];
                tab[idx(k)?] = (prev + 1).clamp(-off, max_qp);
            }
        }
        if t.same_for_chroma {
            let first = self.chroma_qp[0];
            let lo = ((base - off).max(0) as usize).min(94);
            self.chroma_qp[1][lo..].copy_from_slice(&first[lo..]);
        }
        Ok(())
    }

    fn output(&mut self, bump: bool) -> Option<Frame> {
        let id = self.dpb.out(bump)?;
        let sps = self.sps.as_ref()?;
        let pic = self.dpb.arena.get(id)?;
        let (w, h) = output_size(sps);
        let crop = sps.crop.unwrap_or([0; 4]);
        let mut planes = Vec::new();
        for (i, plane) in pic.planes.iter().enumerate() {
            let (cs, pw, ph) = if i == 0 { (2, w, h) } else { (1, w.div_ceil(2), h.div_ceil(2)) };
            let (x0, y0) = (crop[0] as usize * cs, crop[2] as usize * cs);
            let mut out = vec![0u16; pw * ph];
            for y in 0..ph {
                let src = (y0 + y) * plane.stride + x0;
                if let (Some(s), Some(d)) = (plane.data.get(src..src + pw), out.get_mut(y * pw..(y + 1) * pw)) {
                    d.copy_from_slice(s);
                }
            }
            planes.push(out);
        }
        Some(Frame {
            width: w as u32,
            height: h as u32,
            chroma: if sps.chroma_format_idc == 0 { Chroma::Mono } else { Chroma::Yuv420 },
            bit_depth: sps.bit_depth_luma,
            poc: pic.poc,
            tag: pic.tag,
            planes,
        })
    }
}
