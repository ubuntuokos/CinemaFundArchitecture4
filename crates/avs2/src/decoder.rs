// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/decoder.cc`, `header.cc`, `davs2.cc`), Copyright (C) 2018~ VCL, NELVT, Peking University
// (Falei Luo et al.), GPL-2.0-or-later.
//! Picture-level decoding: slices and LCUs, reference pictures, POC and output order.

use std::sync::Arc;

use crate::Error;
use crate::aec::{Aec, CTX_ALF_LCU};
use crate::bits::{SC_INTER_PICTURE, SC_INTRA_PICTURE, SC_SEQUENCE_HEADER, SC_SLICE_CODE_MAX, SC_USER_DATA, dispose_pseudo_code};
use crate::cu::{ColInfo, Mv, PicState, SaoParam};
use crate::filter::{Deblock, alf, sao};
use crate::header::{AlfParam, FrameType, Picture, Sequence, Slice};
use crate::recon::{Frame, Recon};

/// The largest coded picture (8192×4608 samples), against hostile headers.
const MAX_PIXELS: u64 = 8192 * 4608;
/// `MAX_POC_DISTANCE`: older reference pictures are dropped.
const MAX_POC_DISTANCE: i32 = 128;
/// davs2 stops waiting for a missing POC once this many pictures are queued.
const MAX_DELAYED: usize = 8;
const MULTI: i32 = 16384;
/// The most reference pictures kept, against streams that never remove any.
const MAX_DPB: usize = 32;

/// A decoded picture kept for reference.
struct RefPic {
    coi: i32,
    poc: i32,
    frame: Arc<Frame>,
    col: Arc<ColInfo>,
}

/// A decoded picture waiting for output.
pub(crate) struct OutPic {
    pub poc: i32,
    pub frame: Arc<Frame>,
    pub tag: i64,
    pub bit_depth: u32,
}

/// The decoder state between pictures (`davs2_mgr_t` + `davs2_t`).
pub(crate) struct Ctx {
    pub seq: Option<Sequence>,
    st: PicState,
    alf_params: [AlfParam; 3],
    refs: Vec<RefPic>,
    remove_coi: Vec<i32>,
    prev_coi: i32,
    wrap: i32,
    /// The next POC to output (`outpics.output`); `None` until the first I picture.
    pub output: Option<i32>,
    pub pending: Vec<OutPic>,
}

impl Default for Ctx {
    fn default() -> Self {
        Ctx {
            seq: None,
            st: PicState::new(),
            alf_params: Default::default(),
            refs: Vec::new(),
            remove_coi: Vec::new(),
            prev_coi: 0,
            wrap: 0,
            output: None,
            pending: Vec::new(),
        }
    }
}

/// The coded size: the picture size rounded up to whole 8×8 blocks (`i_enc_width`).
fn enc_size(seq: &Sequence) -> (i32, i32) {
    (((seq.width + 7) & !7) as i32, ((seq.height + 7) & !7) as i32)
}

/// `found_slice_header`: looks for a slice start code in the 4 bytes from the byte-aligned
/// position after `bit_pos`, advancing `bit_pos` past the bytes that are not one.
fn found_slice_header(data: &[u8], bit_pos: &mut usize) -> bool {
    for _ in 0..4 {
        let at = (*bit_pos + 7) >> 3;
        if let Some(b) = data.get(at..at + 4)
            && b[0] == 0
            && b[1] == 0
            && b[2] == 1
            && b[3] <= SC_SLICE_CODE_MAX
        {
            return true;
        }
        *bit_pos += 8;
    }
    false
}

impl Ctx {
    /// Forgets references and the output position (a seek): decoding resumes at an I picture.
    pub(crate) fn reset(&mut self) {
        self.refs.clear();
        self.remove_coi.clear();
        self.output = None;
        self.pending.clear();
        self.prev_coi = 0;
        self.wrap = 0;
    }

    fn set_sequence(&mut self, seq: Sequence) -> Result<(), Error> {
        if seq.field_coding {
            return Err(Error::Unsupported("field coding"));
        }
        let (w, h) = enc_size(&seq);
        if u64::from(w.unsigned_abs()) * u64::from(h.unsigned_abs()) > MAX_PIXELS {
            return Err(Error::Unsupported("picture size"));
        }
        if self.seq.as_ref().is_some_and(|s| enc_size(s) != (w, h) || s.log2_lcu_size != seq.log2_lcu_size) {
            self.refs.clear();
        }
        self.st.resize(w, h, seq.log2_lcu_size as i32);
        self.seq = Some(seq);
        Ok(())
    }

    /// Decodes one packet (from a picture start code, or a header-only packet).
    pub(crate) fn decode_packet(&mut self, packet: &[u8], tag: i64) -> Result<(), Error> {
        let data = dispose_pseudo_code(packet);
        let mut i = 0usize;
        while i + 3 < data.len() {
            if !(data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1) {
                i += 1;
                continue;
            }
            let code = data[i + 3];
            let body = i + 4;
            match code {
                SC_SEQUENCE_HEADER => {
                    // the stuffing removal skips sequence headers, so `data` holds it unchanged
                    let seq = Sequence::parse(data.get(body..).unwrap_or_default())?;
                    self.set_sequence(seq)?;
                    i = body;
                }
                SC_INTRA_PICTURE | SC_INTER_PICTURE => {
                    return self.decode_picture(&data, body, code == SC_INTRA_PICTURE, tag);
                }
                _ => i = body,
            }
        }
        Ok(())
    }

    fn decode_picture(&mut self, data: &[u8], body: usize, intra: bool, tag: i64) -> Result<(), Error> {
        let seq = self.seq.clone().ok_or(Error::Invalid("picture before the sequence header"))?;
        let (pic, hdr_len) = Picture::parse(data.get(body..).unwrap_or_default(), intra, &seq, &self.alf_params)?;
        if !intra && self.output.is_none() {
            // an I picture is expected first (and after a reset)
            return Ok(());
        }
        if !pic.frame {
            return Err(Error::Unsupported("field pictures"));
        }
        if matches!(pic.frame_type, FrameType::G | FrameType::Gb | FrameType::S) || pic.background_reference {
            return Err(Error::Unsupported("background pictures"));
        }
        self.alf_params = pic.alf;

        // COI and POC (`parse_picture_header`)
        let coi8 = pic.coi as i32;
        if coi8 < self.prev_coi {
            self.wrap = self.wrap.saturating_add(1);
        }
        self.prev_coi = coi8;
        let coi = coi8.saturating_add(self.wrap.saturating_mul(256));
        let poc = if seq.low_delay { coi } else { coi + pic.display_delay as i32 - seq.picture_reorder_delay as i32 };
        if self.output.is_none() && intra {
            self.output = Some(poc);
        }

        // reference pictures (`task_get_references`)
        let remove = std::mem::take(&mut self.remove_coi);
        self.refs.retain(|r| !remove.contains(&r.coi));
        let mut fref: Vec<(Arc<Frame>, Arc<ColInfo>, i32)> = Vec::new();
        for k in 0..pic.rps.num_of_ref.min(4) {
            let want = coi - pic.rps.ref_pic[k] as i32;
            // A picture whose reference is gone (a leading picture after a seek to an I picture,
            // or one after a lost picture) is skipped instead of failing the stream.
            let Some(r) = self.refs.iter().find(|r| r.coi == want) else {
                return Ok(());
            };
            fref.push((r.frame.clone(), r.col.clone(), r.poc));
        }
        if pic.frame_type == FrameType::B && (fref.len() != 2 || fref[0].2 <= poc || fref[1].2 >= poc) {
            return Err(Error::Invalid("B picture references"));
        }
        if !pic.frame_type.is_intra() && fref.is_empty() {
            return Err(Error::Invalid("inter picture without references"));
        }
        self.remove_coi = (0..pic.rps.num_to_remove.min(8)).map(|k| coi - pic.rps.remove_pic[k] as i32).collect();
        self.refs.retain(|r| (r.poc - poc).abs() < MAX_POC_DISTANCE);

        // init_fdec: reference distances
        let mut dist = [-1i32; 4];
        let mut scale = [-1i32; 4];
        let didx = |d: i32| (d + 512) & 511;
        if pic.frame_type == FrameType::B {
            dist[1] = didx(2 * (poc - fref[1].2));
            dist[0] = didx(2 * (fref[0].2 - poc));
            for k in 0..2 {
                if dist[k] <= 0 {
                    dist[k] = 1;
                }
                scale[k] = MULTI / dist[k];
            }
        } else {
            for (k, r) in fref.iter().enumerate() {
                dist[k] = didx(2 * (poc - r.2));
                if dist[k] <= 0 {
                    dist[k] = 1;
                }
                scale[k] = MULTI / dist[k];
            }
        }

        let st = &mut self.st;
        st.frame_type = pic.frame_type;
        st.num_refs = fref.len();
        st.dist_refs = dist;
        st.dist_scale_refs = scale;
        st.chroma_delta_u = pic.chroma_quant_delta_u;
        st.chroma_delta_v = pic.chroma_quant_delta_v;
        st.bit_depth = seq.bit_depth as i32;
        st.mono = seq.chroma_format == 0;
        st.amp = seq.amp;
        st.mhp_skip = seq.mhp_skip;
        st.wsm = seq.wsm;
        st.dhp = seq.dhp;
        st.nsqt = seq.nsqt;
        st.sdip = seq.sdip;
        st.pmvr = seq.pmvr;
        st.init_frame();

        let (w, h) = (st.width, st.height);
        let lv = st.lcu_level;
        let w_lcu = (w + (1 << lv) - 1) >> lv;
        let h_lcu = (h + (1 << lv) - 1) >> lv;
        let n_lcu = (w_lcu * h_lcu).max(0) as usize;
        let mut frame = Frame::new(w, h, st.bit_depth);
        let ref_frames: Vec<&Frame> = fref.iter().map(|r| r.0.as_ref()).collect();
        let col = fref.first().map(|r| r.1.as_ref());
        let refered = pic.rps.refered_by_others;
        let mut col_out = ColInfo {
            mvbuf: vec![Mv::default(); (st.w_spu * st.h_spu).max(0) as usize],
            refbuf: vec![-1; (st.w_spu * st.h_spu).max(0) as usize],
            dist_scale_refs: scale,
        };

        // the first slice (`decode_user_data`)
        let mut pos = body + hdr_len;
        let mut found = false;
        while pos + 3 < data.len() {
            if data[pos] == 0 && data[pos + 1] == 0 && data[pos + 2] == 1 {
                let c = data[pos + 3];
                if c != SC_USER_DATA && c <= SC_SLICE_CODE_MAX {
                    found = true;
                    break;
                }
                pos += 4;
            } else {
                pos += 1;
            }
        }
        if !found {
            return Ok(());
        }

        let mut bit_pos = pos * 8;
        let mut slice_checked = true;
        let mut slice_sao_on = [false; 3];
        let mut aec: Option<Aec<'_>> = None;
        let mut sao_params: Vec<SaoParam> = vec![SaoParam::default(); n_lcu];
        let mut alf_on: Vec<[bool; 3]> = vec![[false; 3]; n_lcu];
        let mut row_sao_on: Vec<[bool; 3]> = vec![[false; 3]; h_lcu.max(0) as usize];
        for ly in 0..h_lcu {
            for lx in 0..w_lcu {
                let lcu_xy = (ly * w_lcu + lx) as usize;
                if slice_checked && found_slice_header(data, &mut bit_pos) {
                    bit_pos = (bit_pos + 7) & !7;
                    let at = bit_pos / 8 + 3;
                    let slice = Slice::parse(data.get(at..).unwrap_or_default(), &seq, &pic, seq.sao)?;
                    let start = at + slice.data_bit_pos.div_ceil(8);
                    let mut a = Aec::new(data, start);
                    a.init_contexts();
                    aec = Some(a);
                    st.new_slice(slice.qp, slice.dqp, ly);
                    slice_sao_on = slice.sao_on;
                }
                let a = aec.as_mut().ok_or(Error::Invalid("LCU outside a slice"))?;
                st.lcu_init(lx, ly);
                if seq.sao {
                    if slice_sao_on.iter().any(|&b| b) {
                        let left = if lx > 0 { sao_params.get(lcu_xy - 1).copied() } else { None };
                        let up = if ly > 0 { sao_params.get(lcu_xy - w_lcu as usize).copied() } else { None };
                        let p = st.read_sao(a, slice_sao_on, left.as_ref(), up.as_ref());
                        if let Some(s) = sao_params.get_mut(lcu_xy) {
                            *s = p;
                        }
                    } else if let Some(s) = sao_params.get_mut(lcu_xy) {
                        *s = SaoParam::default();
                    }
                }
                if seq.alf {
                    for c in 0..3 {
                        if pic.alf_on[c] {
                            let on = a.symbol(CTX_ALF_LCU) != 0;
                            if let Some(f) = alf_on.get_mut(lcu_xy) {
                                f[c] = on;
                            }
                        }
                    }
                }
                st.parse_lcu(a, col, lv, lx << lv, ly << lv)?;
                slice_checked = a.final_bit() != 0;
                bit_pos = usize::try_from(a.bits_read()).unwrap_or(0);
                if a.bit_error {
                    return Err(Error::Invalid("arithmetic decoding error"));
                }

                // reconstruct the LCU (its coefficients are overwritten by the next one)
                let mut cy = std::mem::take(&mut st.coeff_y);
                let mut cu0 = std::mem::take(&mut st.coeff_uv[0]);
                let mut cu1 = std::mem::take(&mut st.coeff_uv[1]);
                let res = {
                    let mut rc = Recon::new(st, &mut frame, &ref_frames, seq.second_transform, &mut cy, [&mut cu0, &mut cu1]);
                    rc.recon_lcu(lv, lx << lv, ly << lv)
                };
                st.coeff_y = cy;
                st.coeff_uv = [cu0, cu1];
                res?;
            }
            if let Some(r) = row_sao_on.get_mut(ly as usize) {
                *r = slice_sao_on;
            }
            if refered && !pic.frame_type.is_intra() {
                save_mv_ref_info(st, &mut col_out, ly);
            }
        }

        // in-loop filters
        let st = &self.st;
        if pic.loop_filter {
            let mut db = Deblock::new(st, seq.cross_loop_filter, seq.nsqt, pic.alpha_offset, pic.beta_offset);
            for ly in 0..h_lcu {
                for lx in 0..w_lcu {
                    db.lcu(&mut frame, lx, ly);
                }
            }
        }
        if seq.sao {
            sao(st, seq.cross_loop_filter, &mut frame, &sao_params, &row_sao_on);
        }
        if seq.alf {
            alf(st, seq.cross_loop_filter, &mut frame, &pic.alf, pic.alf_on, &alf_on);
        }

        let frame = Arc::new(frame);
        if refered {
            self.refs.push(RefPic { coi, poc, frame: frame.clone(), col: Arc::new(col_out) });
            if self.refs.len() > MAX_DPB {
                self.refs.remove(0);
            }
        }
        let at = self.pending.partition_point(|p| p.poc <= poc);
        self.pending.insert(at, OutPic { poc, frame, tag, bit_depth: seq.bit_depth });
        Ok(())
    }

    /// `output_list_get_one_output_picture`: the next picture in POC order, if it is due.
    pub(crate) fn next_output(&mut self, flushing: bool) -> Option<OutPic> {
        loop {
            let first = self.pending.first()?;
            let out = self.output.unwrap_or(first.poc);
            if first.poc == out {
                self.output = Some(out.saturating_add(1));
                return Some(self.pending.remove(0));
            }
            if first.poc > out && !flushing && self.pending.len() < MAX_DELAYED {
                return None;
            }
            // a late picture, a gap at the end of the stream or too many queued pictures
            self.output = Some(first.poc);
        }
    }
}

/// `save_mv_ref_info`: the motion of LCU row `row`, subsampled to one vector per 16×16 block,
/// kept for temporal prediction from this picture.
fn save_mv_ref_info(st: &PicState, col: &mut ColInfo, row: i32) {
    let w = st.w_spu;
    let h = st.h_spu;
    let spu_y = row << (st.lcu_level - 2);
    let lcu_h = 1 << (st.lcu_level - 2);
    for j in spu_y..(spu_y + lcu_h).min(h) {
        let mut y = ((j >> 2) << 2) + 2;
        if y >= h {
            y = (((j >> 2) << 2) + h) >> 1;
        }
        for i in 0..w {
            let mut x = ((i >> 2) << 2) + 2;
            if x >= w {
                x = (((i >> 2) << 2) + w) >> 1;
            }
            let src = (y * w + x).max(0) as usize;
            let dst = (j * w + i).max(0) as usize;
            let mv = st.tmv[0].get(src).copied().unwrap_or_default();
            let r = st.ref_idx.get(src).map_or(-1, |r| r.r[0]);
            if let Some(d) = col.mvbuf.get_mut(dst) {
                *d = mv;
            }
            if let Some(d) = col.refbuf.get_mut(dst) {
                *d = r;
            }
        }
    }
}
