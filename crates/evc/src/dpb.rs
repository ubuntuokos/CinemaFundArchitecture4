// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! Decoded picture buffer of the EVC Baseline profile (`xevd_picman.c`):
//! reference marking without reference picture lists, reference list
//! construction from POC and temporal id, and output order.

use crate::Error;

pub(crate) const MAX_PB: usize = 21 + 5 + 8;
pub(crate) const MAX_REFS: usize = 21;
const MAX_ACTIVE_REFS: usize = 5;

pub(crate) struct Plane {
    pub(crate) data: Vec<u16>,
    pub(crate) stride: usize,
    /// Coded size of the plane.
    pub(crate) w: usize,
    pub(crate) h: usize,
    /// Size aligned to the minimum CU (the buffer size).
    pub(crate) aw: usize,
    pub(crate) ah: usize,
}

impl Plane {
    pub(crate) fn new(w: usize, h: usize, align: usize) -> Self {
        let aw = w.div_ceil(align) * align;
        let ah = h.div_ceil(align) * align;
        Self { data: vec![0; aw * ah], stride: aw, w, h, aw, ah }
    }

    /// `picbuf_expand` as far as it reaches into the buffer: the columns and
    /// rows between the coded and the aligned size repeat the last coded one.
    pub(crate) fn expand(&mut self) {
        let (w, h, s) = (self.w, self.h, self.stride);
        if w == 0 || h == 0 {
            return;
        }
        for y in 0..h {
            let edge = self.data.get(y * s + w - 1).copied().unwrap_or(0);
            for x in w..self.aw {
                if let Some(p) = self.data.get_mut(y * s + x) {
                    *p = edge;
                }
            }
        }
        for y in h..self.ah {
            let (top, rest) = self.data.split_at_mut(y * s);
            if let (Some(src), Some(dst)) = (top.get((h - 1) * s..h * s), rest.get_mut(..s)) {
                dst.copy_from_slice(src);
            }
        }
    }
}

pub(crate) struct Pic {
    pub(crate) planes: Vec<Plane>,
    pub(crate) poc: i32,
    pub(crate) temporal_id: u8,
    pub(crate) is_ref: bool,
    pub(crate) need_for_out: bool,
    pub(crate) list_poc: [u32; MAX_REFS],
    pub(crate) map_mv: Vec<[[i16; 2]; 2]>,
    pub(crate) map_refi: Vec<[i8; 2]>,
    /// The caller's tag of the NAL unit the picture was decoded from.
    pub(crate) tag: i64,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Refp {
    pub(crate) pic: Option<usize>,
    pub(crate) poc: u32,
}

pub(crate) struct Dpb {
    /// Pictures, addressed by a stable id (the C pointers).
    pub(crate) arena: Vec<Pic>,
    pic: [Option<usize>; MAX_PB],
    pic_ref: [Option<usize>; MAX_REFS],
    cur_num_ref_pics: usize,
    pub(crate) num_refp: [usize; 2],
    poc_next_output: i32,
    /// Picture being decoded (taken out of the buffer).
    pub(crate) lease: Option<usize>,
}

impl Dpb {
    pub(crate) fn new() -> Self {
        Self { arena: Vec::new(), pic: [None; MAX_PB], pic_ref: [None; MAX_REFS], cur_num_ref_pics: 0, num_refp: [0; 2], poc_next_output: 0, lease: None }
    }

    fn p(&self, id: usize) -> Option<&Pic> {
        self.arena.get(id)
    }

    fn is_ref(&self, slot: usize) -> bool {
        self.pic.get(slot).copied().flatten().and_then(|id| self.p(id)).is_some_and(|p| p.is_ref)
    }

    fn set_ref(&mut self, id: usize, v: bool) {
        if let Some(p) = self.arena.get_mut(id) {
            p.is_ref = v;
        }
    }

    fn move_pic(&mut self, from: usize, to: usize) {
        let pic = self.pic[from];
        for i in from..to {
            self.pic[i] = self.pic[i + 1];
        }
        self.pic[to] = pic;
    }

    fn marking_no_rpl(&mut self, ref_pic_gap_length: i32) {
        let mut i = 0usize;
        while i < MAX_PB {
            let unmark = match self.pic[i].and_then(|id| self.p(id)) {
                Some(p) => p.is_ref && (p.temporal_id > 0 || (i > 0 && ref_pic_gap_length > 0 && p.poc % ref_pic_gap_length != 0)),
                None => false,
            };
            if unmark {
                if let Some(id) = self.pic[i] {
                    self.set_ref(id, false);
                }
                self.move_pic(i, MAX_PB - 1);
                self.cur_num_ref_pics = self.cur_num_ref_pics.saturating_sub(1);
                // Re-examine the slot: every pass unmarks one reference, so
                // the loop ends.
                continue;
            }
            i += 1;
        }
        while self.cur_num_ref_pics >= MAX_ACTIVE_REFS {
            let Some(i) = (0..MAX_PB).find(|&i| self.is_ref(i)) else {
                self.cur_num_ref_pics = 0;
                break;
            };
            if let Some(id) = self.pic[i] {
                self.set_ref(id, false);
            }
            self.move_pic(i, MAX_PB - 1);
            self.cur_num_ref_pics -= 1;
        }
    }

    fn flush_pb(&mut self) {
        let mut i = 0usize;
        while i < MAX_PB {
            if self.is_ref(i) {
                if let Some(id) = self.pic[i] {
                    self.set_ref(id, false);
                }
                self.move_pic(i, MAX_PB - 1);
                continue;
            }
            i += 1;
        }
        let mut max_poc = 0i32;
        for id in self.pic.iter().flatten() {
            if let Some(p) = self.p(*id)
                && p.need_for_out
                && p.poc != 0
                && p.poc > max_poc
            {
                max_poc = p.poc;
            }
        }
        if max_poc != 0 {
            max_poc = max_poc.wrapping_add(1);
        }
        let mut reordered_min = i32::MAX;
        for slot in 0..MAX_PB {
            let Some(id) = self.pic[slot] else { continue };
            if let Some(p) = self.arena.get_mut(id)
                && p.need_for_out
                && p.poc != 0
            {
                p.is_ref = false;
                p.poc = p.poc.wrapping_sub(max_poc);
                reordered_min = reordered_min.min(p.poc);
            }
        }
        self.poc_next_output = if max_poc == 0 { 0 } else { reordered_min };
        self.cur_num_ref_pics = 0;
    }

    fn update_pic_ref(&mut self) {
        let mut refs: Vec<usize> = Vec::new();
        for slot in 0..MAX_PB {
            if self.is_ref(slot)
                && let Some(id) = self.pic[slot]
            {
                refs.push(id);
            }
        }
        refs.truncate(MAX_REFS);
        let poc = |s: &Self, id: usize| s.p(id).map_or(0, |p| p.poc);
        let cnt = refs.len();
        for i in 0..cnt.saturating_sub(1) {
            for j in i + 1..cnt {
                if poc(self, refs[i]) < poc(self, refs[j]) {
                    refs.swap(i, j);
                }
            }
        }
        self.pic_ref = [None; MAX_REFS];
        for (d, s) in self.pic_ref.iter_mut().zip(refs) {
            *d = Some(s);
        }
    }

    /// `xevd_picman_refp_init`.
    pub(crate) fn refp_init(
        &mut self,
        refp: &mut [[Refp; 2]; MAX_REFS],
        max_num_ref_pics: usize,
        slice_type: u32,
        poc: i32,
        layer_id: u8,
        last_intra: i32,
    ) -> Result<(), Error> {
        use crate::params::{SLICE_B, SLICE_I, SLICE_P};
        if slice_type == SLICE_I {
            return Ok(());
        }
        self.update_pic_ref();
        if self.cur_num_ref_pics == 0 {
            return Err(Error::Malformed("inter picture without reference pictures"));
        }
        for r in refp.iter_mut() {
            r[0].pic = None;
            r[1].pic = None;
        }
        self.num_refp = [0, 0];
        let n = self.cur_num_ref_pics.min(MAX_REFS);
        let info = |s: &Self, i: usize| -> Option<(usize, i32, u8)> {
            let id = s.pic_ref.get(i).copied().flatten()?;
            let p = s.p(id)?;
            Some((id, p.poc, p.temporal_id))
        };
        let set = |r: &mut Refp, id: usize, poc: i32| {
            r.pic = Some(id);
            r.poc = poc as u32;
        };
        let mut cnt = 0usize;
        let layer = i32::from(layer_id);
        if slice_type == SLICE_P {
            if layer_id > 0 {
                for i in 0..n {
                    if cnt >= max_num_ref_pics {
                        break;
                    }
                    let Some((id, rpoc, tid)) = info(self, i) else { continue };
                    let take = if layer_id == 1 {
                        rpoc < poc && tid <= layer_id
                    } else if rpoc < poc && cnt == 0 {
                        true
                    } else {
                        cnt != 0 && rpoc < poc && tid <= 1
                    };
                    if take {
                        set(&mut refp[cnt][0], id, rpoc);
                        cnt += 1;
                    }
                }
            } else {
                for i in 0..n {
                    if cnt >= max_num_ref_pics {
                        break;
                    }
                    let Some((id, rpoc, _)) = info(self, i) else { continue };
                    if poc >= last_intra && rpoc < last_intra {
                        continue;
                    }
                    if rpoc < poc {
                        set(&mut refp[cnt][0], id, rpoc);
                        cnt += 1;
                    }
                }
            }
        } else {
            let mut next = (layer - 1).max(0);
            for i in 0..n {
                if cnt >= max_num_ref_pics {
                    break;
                }
                let Some((id, rpoc, tid)) = info(self, i) else { continue };
                if poc >= last_intra && rpoc < last_intra {
                    continue;
                }
                if rpoc < poc && i32::from(tid) <= next {
                    set(&mut refp[cnt][0], id, rpoc);
                    cnt += 1;
                    next = (i32::from(tid) - 1).max(0);
                }
            }
        }
        if cnt < max_num_ref_pics && slice_type == SLICE_B {
            let mut next = (layer - 1).max(0);
            for i in (0..n).rev() {
                if cnt >= max_num_ref_pics {
                    break;
                }
                let Some((id, rpoc, tid)) = info(self, i) else { continue };
                if poc >= last_intra && rpoc < last_intra {
                    continue;
                }
                if rpoc > poc && i32::from(tid) <= next {
                    set(&mut refp[cnt][0], id, rpoc);
                    cnt += 1;
                    next = (i32::from(tid) - 1).max(0);
                }
            }
        }
        if cnt == 0 {
            return Err(Error::Malformed("no usable reference picture"));
        }
        self.num_refp[0] = cnt;
        if slice_type == SLICE_B {
            let mut next = (layer - 1).max(0);
            cnt = 0;
            for i in (0..n).rev() {
                if cnt >= max_num_ref_pics {
                    break;
                }
                let Some((id, rpoc, tid)) = info(self, i) else { continue };
                if poc >= last_intra && rpoc < last_intra {
                    continue;
                }
                if rpoc > poc && i32::from(tid) <= next {
                    set(&mut refp[cnt][1], id, rpoc);
                    cnt += 1;
                    next = (i32::from(tid) - 1).max(0);
                }
            }
            if cnt < max_num_ref_pics {
                next = (layer - 1).max(0);
                for i in 0..n {
                    if cnt >= max_num_ref_pics {
                        break;
                    }
                    let Some((id, rpoc, tid)) = info(self, i) else { continue };
                    if poc >= last_intra && rpoc < last_intra {
                        continue;
                    }
                    if rpoc < poc && i32::from(tid) <= next {
                        set(&mut refp[cnt][1], id, rpoc);
                        cnt += 1;
                        next = (i32::from(tid) - 1).max(0);
                    }
                }
            }
            if cnt == 0 {
                return Err(Error::Malformed("no usable backward reference picture"));
            }
            self.num_refp[1] = cnt;
            self.num_refp[0] = self.num_refp[0].min(max_num_ref_pics);
            self.num_refp[1] = self.num_refp[1].min(max_num_ref_pics);
        }
        Ok(())
    }

    fn allocated(&self) -> usize {
        self.pic.iter().filter(|p| p.is_some()).count()
    }

    /// `xevd_picman_get_empty_pic`: reuses a picture that is neither a
    /// reference nor waiting for output, or allocates one with `alloc`.
    pub(crate) fn get_empty(&mut self, alloc: impl FnOnce() -> Pic) -> Result<usize, Error> {
        if let Some(id) = self.lease {
            return Ok(id);
        }
        let found = (0..MAX_PB).find(|&i| self.pic[i].and_then(|id| self.p(id)).is_some_and(|p| !p.is_ref && !p.need_for_out));
        if let Some(pos) = found {
            let id = self.pic[pos];
            for i in pos..MAX_PB - 1 {
                self.pic[i] = self.pic[i + 1];
            }
            self.pic[MAX_PB - 1] = None;
            if let Some(id) = id {
                self.lease = Some(id);
                return Ok(id);
            }
        }
        if self.allocated() < MAX_PB {
            self.arena.push(alloc());
            let id = self.arena.len() - 1;
            self.lease = Some(id);
            return Ok(id);
        }
        Err(Error::Malformed("decoded picture buffer is full"))
    }

    /// `xevd_picman_put_pic` (always with `need_for_output`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put(&mut self, id: usize, is_idr: bool, poc: i32, temporal_id: u8, refp: &[[Refp; 2]; MAX_REFS], ref_pic: bool, ref_pic_gap_length: i32) {
        if is_idr {
            self.flush_pb();
        } else if temporal_id == 0 {
            self.marking_no_rpl(ref_pic_gap_length);
        }
        let num_refp0 = self.num_refp[0].min(MAX_REFS);
        if let Some(p) = self.arena.get_mut(id) {
            p.is_ref = ref_pic;
            p.temporal_id = temporal_id;
            p.poc = poc;
            p.need_for_out = true;
            for i in 0..num_refp0 {
                p.list_poc[i] = refp[i][0].poc;
            }
        }
        let pos = if ref_pic { Some(self.cur_num_ref_pics) } else { None };
        match pos {
            Some(pos) if pos < MAX_PB && self.pic[pos].is_none() => self.pic[pos] = Some(id),
            _ => {
                if let Some(slot) = (0..MAX_PB).rev().find(|&i| self.pic[i].is_none()) {
                    self.pic[slot] = Some(id);
                }
            }
        }
        if ref_pic {
            self.cur_num_ref_pics += 1;
        }
        if self.lease == Some(id) {
            self.lease = None;
        }
    }

    /// `xevd_picman_out_pic`. With `bump`, a picture is returned even when
    /// its POC is past the next expected one (end of stream).
    pub(crate) fn out(&mut self, bump: bool) -> Option<usize> {
        let mut first_waiting = None;
        for slot in 0..MAX_PB {
            let Some(id) = self.pic[slot] else { continue };
            let Some(p) = self.arena.get(id) else { continue };
            if p.need_for_out {
                if p.poc <= self.poc_next_output {
                    return Some(self.take_out(id));
                }
                let better = match first_waiting {
                    Some((_, poc)) => p.poc < poc,
                    None => true,
                };
                if better {
                    first_waiting = Some((id, p.poc));
                }
            }
        }
        if bump {
            return first_waiting.map(|(id, _)| self.take_out(id));
        }
        None
    }

    fn take_out(&mut self, id: usize) -> usize {
        if let Some(p) = self.arena.get_mut(id) {
            p.need_for_out = false;
            self.poc_next_output = p.poc.wrapping_add(1);
        }
        id
    }

    /// After a CRA picture the output restarts at its POC.
    pub(crate) fn set_next_output(&mut self, poc: i32) {
        self.poc_next_output = poc;
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::new();
    }
}
