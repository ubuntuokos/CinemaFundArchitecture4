//! Decoded picture buffer: reference marking (8.2.5), reference list construction (8.2.4) and output
//! ordering / bumping (C.4).
//!
//! The buffer only does bookkeeping on picture headers; what a picture is (`F`: the software
//! decoder's [`crate::picture::FrameRef`], a hardware decoder's surface) is up to the decoder, so
//! hardware decoders driven by the same headers (VA-API) share this logic and output order.

use crate::error::{Result, ensure};
use crate::slice::{Mmco, SliceHeader, SliceType};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefMark {
    Unused,
    Short,
    Long,
}

/// Output-related metadata attached to a decoded frame.
#[derive(Clone, Debug)]
pub struct OutputMeta {
    pub pts: i64,
    pub key: bool,
    pub crop: (u32, u32, u32, u32),
    pub full_range: bool,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub sar: (u16, u16),
    /// Decoded without deblocking (draft mode, non-reference picture).
    pub draft: bool,
}

pub struct DpbEntry<F> {
    pub frame: F,
    /// PicOrderCnt of the frame: the smaller POC of its decoded fields.
    pub poc: i32,
    /// TopFieldOrderCnt and BottomFieldOrderCnt (a missing field repeats the other's).
    pub field_poc: [i32; 2],
    /// Decoded fields: bit 0 top, bit 1 bottom (3 for frames and complete field pairs).
    pub fields: u8,
    pub frame_num: u32,
    pub long_term_frame_idx: u32,
    /// Marking of the top and bottom field (both the same for a frame).
    pub field_mark: [RefMark; 2],
    pub needed_for_output: bool,
    /// The first field of a frame whose second field may still follow: kept, never output.
    pub incomplete: bool,
    pub non_existing: bool,
    pub meta: Arc<OutputMeta>,
}

impl<F> DpbEntry<F> {
    /// Marking of the entry as a frame (decoding frames uses only frames and complementary
    /// field pairs with both fields marked alike, 8.2.4.2.1).
    pub fn mark(&self) -> RefMark {
        if self.fields == 3 && self.field_mark[0] == self.field_mark[1] { self.field_mark[0] } else { RefMark::Unused }
    }

    /// At least one field is used for reference.
    pub fn is_ref(&self) -> bool {
        self.field_mark.iter().any(|m| *m != RefMark::Unused)
    }

    fn has_mark(&self, mark: RefMark) -> bool {
        self.field_mark.contains(&mark)
    }

    fn unmark(&mut self) {
        self.field_mark = [RefMark::Unused; 2];
    }
}

pub struct Dpb<F> {
    pub entries: Vec<DpbEntry<F>>,
    /// None = "no long-term frame indices".
    pub max_long_term_frame_idx: Option<u32>,
    pub capacity: usize,
    pub max_reorder: usize,
}

/// A picture leaving the DPB for output.
pub struct Output<F> {
    pub frame: F,
    pub poc: i32,
    pub meta: Arc<OutputMeta>,
}

/// A reference list entry under construction: a frame (None) or one field of it.
type Pic<'a, F> = (&'a DpbEntry<F>, Option<u8>);

fn same_pic<F>(a: Pic<'_, F>, b: Pic<'_, F>) -> bool {
    std::ptr::eq(a.0, b.0) && a.1 == b.1
}

/// Fields of the frames `frames` marked `mark`, alternating in parity starting with `parity`
/// (8.2.4.2.5).
fn alternate_fields<'a, F>(frames: &[&'a DpbEntry<F>], parity: u8, mark: RefMark) -> Vec<Pic<'a, F>> {
    let of = |p: u8| frames.iter().copied().filter(move |e| e.field_mark[p as usize] == mark).map(move |e| (e, Some(p)));
    let (mut same, mut opp) = (of(parity), of(1 - parity));
    let mut out = Vec::new();
    loop {
        let a = same.next();
        let b = opp.next();
        if a.is_none() && b.is_none() {
            break;
        }
        out.extend(a);
        out.extend(b);
    }
    out
}

impl<F: Clone> Dpb<F> {
    pub fn new() -> Self {
        Dpb { entries: Vec::new(), max_long_term_frame_idx: None, capacity: 16, max_reorder: 16 }
    }

    fn frame_num_wrap(e: &DpbEntry<F>, cur_frame_num: u32, max_frame_num: u32) -> i32 {
        if e.frame_num > cur_frame_num { e.frame_num as i32 - max_frame_num as i32 } else { e.frame_num as i32 }
    }

    /// Frames with at least one field used for reference.
    fn num_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_ref()).count()
    }

    /// Remove entries not needed for output and not used for reference.
    fn prune(&mut self) {
        self.entries.retain(|e| e.needed_for_output || e.is_ref() || e.incomplete);
    }

    /// Output the picture with the smallest POC (C.4.5.3 "bumping"). Returns false if none waits.
    fn bump(&mut self, out: &mut Vec<Output<F>>) -> bool {
        let Some(i) = self.entries.iter().enumerate().filter(|(_, e)| e.needed_for_output).min_by_key(|(_, e)| e.poc).map(|(i, _)| i) else {
            return false;
        };
        let e = &mut self.entries[i];
        e.needed_for_output = false;
        out.push(Output { frame: e.frame.clone(), poc: e.poc, meta: e.meta.clone() });
        self.prune();
        true
    }

    /// Output everything in POC order and clear non-reference entries.
    pub fn flush(&mut self, out: &mut Vec<Output<F>>) {
        while self.bump(out) {}
        self.prune();
    }

    /// IDR: mark all references unused; output (or drop) prior pictures.
    pub fn idr(&mut self, no_output_of_prior_pics: bool, out: &mut Vec<Output<F>>) {
        for e in &mut self.entries {
            e.unmark();
        }
        if no_output_of_prior_pics {
            self.entries.clear();
        } else {
            self.flush(out);
        }
        self.max_long_term_frame_idx = None;
    }

    /// Sliding window marking (8.2.5.3); the entry `keep` (the current frame's first field) is
    /// never the one removed.
    fn sliding_window(&mut self, cur_frame_num: u32, max_frame_num: u32, max_num_ref_frames: usize, keep: Option<usize>) {
        let limit = max_num_ref_frames.max(1);
        while self.num_refs() >= limit {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .filter(|(i, e)| e.has_mark(RefMark::Short) && Some(*i) != keep)
                .min_by_key(|(_, e)| Self::frame_num_wrap(e, cur_frame_num, max_frame_num))
                .map(|(i, _)| i);
            match victim {
                Some(i) => self.entries[i].unmark(),
                None => break,
            }
        }
    }

    /// Insert "non-existing" frames for a frame_num gap (8.2.5.2). `make_frame` gives each one's
    /// picture and POC.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_frame_num_gap(
        &mut self,
        prev_ref_frame_num: u32,
        frame_num: u32,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        make_frame: &mut dyn FnMut(u32) -> (F, i32, Arc<OutputMeta>),
        poc_state: &mut dyn FnMut(u32),
    ) {
        let mut unused = (prev_ref_frame_num + 1) % max_frame_num;
        let mut guard = 0;
        while unused != frame_num && guard < max_frame_num {
            self.sliding_window(unused, max_frame_num, max_num_ref_frames, None);
            self.prune();
            let (frame, poc, meta) = make_frame(unused);
            poc_state(unused);
            self.entries.push(DpbEntry {
                poc,
                field_poc: [poc; 2],
                fields: 3,
                frame,
                frame_num: unused,
                long_term_frame_idx: 0,
                field_mark: [RefMark::Short; 2],
                needed_for_output: false,
                incomplete: false,
                non_existing: true,
                meta,
            });
            unused = (unused + 1) % max_frame_num;
            guard += 1;
        }
    }

    /// The first field of a frame still waiting for its second field: (decoded parity,
    /// frame_num, used for reference).
    pub fn incomplete_field(&self) -> Option<(u8, u32, bool)> {
        self.entries.iter().find(|e| e.incomplete).map(|e| (if e.fields == 2 { 1 } else { 0 }, e.frame_num, e.is_ref()))
    }

    /// Initial reference picture lists (8.2.4.2) + modification (8.2.4.3) for a frame or field
    /// slice, each entry made by `to_ref` from the DPB entry it refers to and the field of it
    /// (None for a frame). `cur_poc` is the POC of the current frame or field.
    pub fn build_ref_lists<R>(
        &self,
        sh: &SliceHeader,
        cur_poc: i32,
        max_frame_num: u32,
        to_ref: impl Fn(&DpbEntry<F>, Option<u8>) -> R,
    ) -> Result<[Vec<R>; 2]> {
        let mut lists: [Vec<R>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type.is_intra() {
            return Ok(lists);
        }
        let field = sh.field_pic.then_some(sh.bottom_field as u8);
        let cur_fn = sh.frame_num;
        let wrap = |e: &DpbEntry<F>| Self::frame_num_wrap(e, cur_fn, max_frame_num);
        let is_b = sh.slice_type == SliceType::B;
        let mut init: [Vec<Pic<'_, F>>; 2] = [Vec::new(), Vec::new()];
        match field {
            None => {
                let short: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.mark() == RefMark::Short).collect();
                let mut long: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.mark() == RefMark::Long).collect();
                long.sort_by_key(|e| e.long_term_frame_idx);
                if !is_b {
                    let mut s = short.clone();
                    s.sort_by_key(|e| std::cmp::Reverse(wrap(e)));
                    init[0] = s.into_iter().chain(long.iter().copied()).map(|e| (e, None)).collect();
                } else {
                    let mut before: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc < cur_poc).collect();
                    let mut after: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc > cur_poc).collect();
                    before.sort_by_key(|e| std::cmp::Reverse(e.poc));
                    after.sort_by_key(|e| e.poc);
                    init[0] = before.iter().chain(after.iter()).chain(long.iter()).map(|e| (*e, None)).collect();
                    init[1] = after.iter().chain(before.iter()).chain(long.iter()).map(|e| (*e, None)).collect();
                }
            }
            Some(p) => {
                // 8.2.4.2.2 / 8.2.4.2.4: lists of frames, then fields of alternating parity
                let short: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.has_mark(RefMark::Short)).collect();
                let mut long: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.has_mark(RefMark::Long)).collect();
                long.sort_by_key(|e| e.long_term_frame_idx);
                let long_fields = alternate_fields(&long, p, RefMark::Long);
                if !is_b {
                    let mut s = short.clone();
                    s.sort_by_key(|e| std::cmp::Reverse(wrap(e)));
                    init[0] = alternate_fields(&s, p, RefMark::Short);
                    init[0].extend(long_fields.iter().copied());
                } else {
                    let mut before: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc <= cur_poc).collect();
                    let mut after: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc > cur_poc).collect();
                    before.sort_by_key(|e| std::cmp::Reverse(e.poc));
                    after.sort_by_key(|e| e.poc);
                    let l0: Vec<&DpbEntry<F>> = before.iter().chain(after.iter()).copied().collect();
                    let l1: Vec<&DpbEntry<F>> = after.iter().chain(before.iter()).copied().collect();
                    init[0] = alternate_fields(&l0, p, RefMark::Short);
                    init[0].extend(long_fields.iter().copied());
                    init[1] = alternate_fields(&l1, p, RefMark::Short);
                    init[1].extend(long_fields.iter().copied());
                }
            }
        }
        if is_b && init[1].len() > 1 && init[0].len() == init[1].len() && init[0].iter().zip(init[1].iter()).all(|(a, b)| same_pic(*a, *b)) {
            init[1].swap(0, 1);
        }
        // picture numbers (8.2.4.1)
        let (max_pic_num, curr_pic_num) = match field {
            None => (max_frame_num as i32, cur_fn as i32),
            Some(_) => (2 * max_frame_num as i32, 2 * cur_fn as i32 + 1),
        };
        let same_parity = |f: Option<u8>| (f.is_some() && f == field) as i32;
        let pic_num = |(e, f): Pic<'_, F>| if f.is_some() { 2 * wrap(e) + same_parity(f) } else { wrap(e) };
        let long_num = |(e, f): Pic<'_, F>| if f.is_some() { 2 * e.long_term_frame_idx as i32 + same_parity(f) } else { e.long_term_frame_idx as i32 };
        // every picture of the given marking that can be referenced (frames or fields)
        let candidates = |mark: RefMark| -> Vec<Pic<'_, F>> {
            match field {
                None => self.entries.iter().filter(|e| e.mark() == mark).map(|e| (e, None)).collect(),
                Some(_) => self.entries.iter().flat_map(|e| (0..2u8).filter(move |&q| e.field_mark[q as usize] == mark).map(move |q| (e, Some(q)))).collect(),
            }
        };
        let nlists = if is_b { 2 } else { 1 };
        for l in 0..nlists {
            let n = sh.num_ref_idx_active[l] as usize;
            let mut list: Vec<Option<Pic<'_, F>>> = init[l].iter().copied().map(Some).collect();
            list.truncate(n);
            list.resize(n, None);
            if !sh.ref_pic_list_mod[l].is_empty() {
                let mut pred = curr_pic_num;
                let mut ref_idx = 0usize;
                for m in &sh.ref_pic_list_mod[l] {
                    ensure!(ref_idx < n, "too many ref_pic_list_modification entries");
                    list.push(None); // temporarily one longer
                    let target: Option<Pic<'_, F>> = match m.idc {
                        0 | 1 => {
                            let d = m.value as i32 + 1;
                            let no_wrap = if m.idc == 0 {
                                if pred - d < 0 { pred - d + max_pic_num } else { pred - d }
                            } else if pred + d >= max_pic_num {
                                pred + d - max_pic_num
                            } else {
                                pred + d
                            };
                            pred = no_wrap;
                            let pn = if no_wrap > curr_pic_num { no_wrap - max_pic_num } else { no_wrap };
                            candidates(RefMark::Short).into_iter().find(|&c| pic_num(c) == pn)
                        }
                        2 => candidates(RefMark::Long).into_iter().find(|&c| long_num(c) == m.value as i32),
                        _ => continue,
                    };
                    let Some(target) = target else {
                        // Missing picture: keep the list unchanged (error concealment).
                        list.pop();
                        ref_idx += 1;
                        continue;
                    };
                    for c in (ref_idx + 1..=n).rev() {
                        list[c] = list[c - 1];
                    }
                    list[ref_idx] = Some(target);
                    ref_idx += 1;
                    let mut nidx = ref_idx;
                    for c in ref_idx..=n {
                        if !list[c].is_some_and(|e| same_pic(e, target)) {
                            list[nidx] = list[c];
                            nidx += 1;
                        }
                    }
                    list.truncate(n);
                }
            }
            // Fill missing entries (should not happen in conforming streams) with the first valid entry.
            let fallback = list.iter().flatten().next().copied().or_else(|| self.entries.last().map(|e| (e, field)));
            lists[l] = list.iter().filter_map(|e| e.or(fallback)).map(|(e, f)| to_ref(e, f)).collect();
        }
        Ok(lists)
    }

    /// Adaptive memory control (8.2.5.4) for a frame (`field` None) or a field. The entry `cur`
    /// is the current frame's first field (when decoding the second one). Returns true if MMCO 5
    /// was executed.
    fn apply_mmcos(
        &mut self,
        mmcos: &[Mmco],
        cur_frame_num: u32,
        max_frame_num: u32,
        field: Option<u8>,
        cur: Option<usize>,
        cur_long: &mut Option<u32>,
    ) -> Result<bool> {
        let mut had5 = false;
        let curr_pic_num = match field {
            None => cur_frame_num as i32,
            Some(_) => 2 * cur_frame_num as i32 + 1,
        };
        let same = |q: usize| field.is_some_and(|p| p as usize == q) as i32;
        // the (entry, fields) a picture number selects
        let short_target = |entries: &[DpbEntry<F>], pn: i32| -> Option<(usize, [bool; 2])> {
            entries.iter().enumerate().find_map(|(i, e)| {
                let w = Self::frame_num_wrap(e, cur_frame_num, max_frame_num);
                match field {
                    None => (e.mark() == RefMark::Short && w == pn).then_some((i, [true; 2])),
                    Some(_) => (0..2).find(|&q| e.field_mark[q] == RefMark::Short && 2 * w + same(q) == pn).map(|q| (i, [q == 0, q == 1])),
                }
            })
        };
        for m in mmcos {
            match m.op {
                1 => {
                    let pn = curr_pic_num - (m.difference_of_pic_nums_minus1 as i32 + 1);
                    if let Some((i, which)) = short_target(&self.entries, pn) {
                        for q in 0..2 {
                            if which[q] {
                                self.entries[i].field_mark[q] = RefMark::Unused;
                            }
                        }
                    }
                }
                2 => {
                    let ln = m.long_term_pic_num as i32;
                    for e in &mut self.entries {
                        let idx = e.long_term_frame_idx as i32;
                        match field {
                            None if e.mark() == RefMark::Long && idx == ln => e.unmark(),
                            None => {}
                            Some(_) => {
                                for q in 0..2 {
                                    if e.field_mark[q] == RefMark::Long && 2 * idx + same(q) == ln {
                                        e.field_mark[q] = RefMark::Unused;
                                    }
                                }
                            }
                        }
                    }
                }
                3 => {
                    ensure!(self.max_long_term_frame_idx.is_some_and(|max| m.long_term_frame_idx <= max), "MMCO long-term index exceeds the configured limit");
                    let pn = curr_pic_num - (m.difference_of_pic_nums_minus1 as i32 + 1);
                    let target = short_target(&self.entries, pn);
                    let keep = target.map(|t| t.0);
                    self.release_long_term_idx(m.long_term_frame_idx, keep);
                    if let Some((i, which)) = target {
                        let e = &mut self.entries[i];
                        for q in 0..2 {
                            if which[q] {
                                e.field_mark[q] = RefMark::Long;
                            }
                        }
                        e.long_term_frame_idx = m.long_term_frame_idx;
                    }
                }
                4 => {
                    let max = m.max_long_term_frame_idx_plus1;
                    for e in &mut self.entries {
                        if max == 0 || e.long_term_frame_idx > max - 1 {
                            for q in 0..2 {
                                if e.field_mark[q] == RefMark::Long {
                                    e.field_mark[q] = RefMark::Unused;
                                }
                            }
                        }
                    }
                    self.max_long_term_frame_idx = if max == 0 { None } else { Some(max - 1) };
                }
                5 => {
                    for e in &mut self.entries {
                        e.unmark();
                    }
                    self.max_long_term_frame_idx = None;
                    had5 = true;
                }
                6 => {
                    ensure!(self.max_long_term_frame_idx.is_some_and(|max| m.long_term_frame_idx <= max), "MMCO long-term index exceeds the configured limit");
                    self.release_long_term_idx(m.long_term_frame_idx, cur);
                    *cur_long = Some(m.long_term_frame_idx);
                }
                _ => {}
            }
        }
        Ok(had5)
    }

    /// LongTermFrameIdx `idx` is being assigned: long-term fields holding it in entries other
    /// than `keep` (the frame of the picture receiving it) become unused.
    fn release_long_term_idx(&mut self, idx: u32, keep: Option<usize>) {
        for (i, e) in self.entries.iter_mut().enumerate() {
            if Some(i) != keep && e.long_term_frame_idx == idx {
                for q in 0..2 {
                    if e.field_mark[q] == RefMark::Long {
                        e.field_mark[q] = RefMark::Unused;
                    }
                }
            }
        }
    }

    /// Reference marking of the current picture (8.2.5.1): its marking and LongTermFrameIdx, and
    /// whether MMCO 5 was executed. `cur` is the entry of the current frame's first field when
    /// decoding a second field.
    #[allow(clippy::too_many_arguments)]
    fn mark_current(
        &mut self,
        sh: &SliceHeader,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        field: Option<u8>,
        cur: Option<usize>,
    ) -> Result<(RefMark, u32, bool)> {
        if sh.nal_ref_idc == 0 {
            return Ok((RefMark::Unused, 0, false));
        }
        if sh.idr {
            return if sh.long_term_reference {
                self.max_long_term_frame_idx = Some(0);
                Ok((RefMark::Long, 0, false))
            } else {
                self.max_long_term_frame_idx = None;
                Ok((RefMark::Short, 0, false))
            };
        }
        let mut cur_long = None;
        let mut had5 = false;
        // the second field of a complementary reference field pair whose first field is a
        // short-term reference joins it without sliding window (8.2.5.3)
        let joins_short = cur.zip(field).is_some_and(|(i, p)| self.entries.get(i).is_some_and(|e| e.field_mark[1 - p as usize] == RefMark::Short));
        if sh.adaptive_ref_pic_marking {
            had5 = self.apply_mmcos(&sh.mmcos, sh.frame_num, max_frame_num, field, cur, &mut cur_long)?;
        } else if !joins_short {
            self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames, cur);
        }
        if let Some(i) = cur_long {
            return Ok((RefMark::Long, i, had5));
        }
        // make room if MMCOs left the DPB over-full (non-conforming streams)
        if cur.is_none() && self.num_refs() >= max_num_ref_frames.max(1) && sh.adaptive_ref_pic_marking {
            self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames, cur);
        }
        Ok((RefMark::Short, 0, had5))
    }

    /// Reference marking for the just-decoded frame and insertion into the DPB with output
    /// bumping. `field_poc` holds its TopFieldOrderCnt and BottomFieldOrderCnt. Returns pictures
    /// ready for output.
    #[allow(clippy::too_many_arguments)]
    pub fn store_picture(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        poc: i32,
        field_poc: [i32; 2],
        max_frame_num: u32,
        max_num_ref_frames: usize,
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) -> Result<()> {
        let (mark, long_idx, had5) = self.mark_current(sh, max_frame_num, max_num_ref_frames, None, None)?;
        if mark == RefMark::Long {
            ensure!(self.num_refs() < max_num_ref_frames.max(1), "long-term reference exceeds max_num_ref_frames");
        }
        self.prune();
        if had5 {
            // All earlier pictures are output before the current one (C.4.4).
            self.flush(out);
        }
        let frame_num = if had5 { 0 } else { sh.frame_num };
        // Bumping until there is space (C.4.5.1 / C.4.5.2).
        if mark == RefMark::Unused {
            loop {
                let waiting = self.entries.len();
                if waiting < self.capacity {
                    break;
                }
                // non-reference picture with the smallest POC is output directly
                let min_poc = self.entries.iter().filter(|e| e.needed_for_output).map(|e| e.poc).min();
                if min_poc.is_none_or(|m| poc < m) {
                    out.push(Output { frame, poc, meta });
                    return Ok(());
                }
                if !self.bump(out) {
                    break;
                }
            }
        } else {
            while self.entries.len() >= self.capacity {
                if !self.bump(out) {
                    break;
                }
            }
        }
        self.entries.push(DpbEntry {
            frame,
            poc,
            field_poc,
            fields: 3,
            frame_num,
            long_term_frame_idx: long_idx,
            field_mark: [mark; 2],
            needed_for_output: true,
            incomplete: false,
            non_existing: false,
            meta,
        });
        self.reorder_output(out);
        Ok(())
    }

    /// Reference marking for a just-decoded field `parity` (POC `poc`) and its storage: the first
    /// field of a new frame (kept until its second field or [`Dpb::complete_field`]), or with
    /// `second` the second field of the frame of the pending first field.
    #[allow(clippy::too_many_arguments)]
    pub fn store_field(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        parity: u8,
        poc: i32,
        second: bool,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) -> Result<()> {
        let p = parity as usize & 1;
        let cur = if second { self.entries.iter().position(|e| e.incomplete) } else { None };
        if !second {
            self.complete_field(out);
        }
        let (mark, long_idx, had5) = self.mark_current(sh, max_frame_num, max_num_ref_frames, Some(parity), cur)?;
        self.prune();
        if had5 {
            self.flush(out);
        }
        if let Some(i) = self.entries.iter().position(|e| e.incomplete).filter(|_| second) {
            let e = &mut self.entries[i];
            e.field_mark[p] = mark;
            if mark == RefMark::Long {
                e.long_term_frame_idx = long_idx;
            }
            e.field_poc[p] = poc;
            e.fields |= 1 << p;
            e.poc = e.field_poc[0].min(e.field_poc[1]);
            self.finish_entry(i, out);
            return Ok(());
        }
        // first field: make room like for a frame, but a non-reference field whose POC precedes
        // every waiting picture must not push those out before it
        while self.entries.len() >= self.capacity {
            let min_poc = self.entries.iter().filter(|e| e.needed_for_output).map(|e| e.poc).min();
            if mark == RefMark::Unused && min_poc.is_none_or(|m| poc < m) {
                break;
            }
            if !self.bump(out) {
                break;
            }
        }
        let mut field_mark = [RefMark::Unused; 2];
        field_mark[p] = mark;
        self.entries.push(DpbEntry {
            frame,
            poc,
            field_poc: [poc; 2],
            fields: 1 << p,
            frame_num: if had5 { 0 } else { sh.frame_num },
            long_term_frame_idx: long_idx,
            field_mark,
            needed_for_output: false,
            incomplete: true,
            non_existing: false,
            meta,
        });
        Ok(())
    }

    /// The pending first field gets no second field: it is output on its own.
    pub fn complete_field(&mut self, out: &mut Vec<Output<F>>) {
        if let Some(i) = self.entries.iter().position(|e| e.incomplete) {
            self.finish_entry(i, out);
        }
    }

    /// A frame whose fields are complete waits for output.
    fn finish_entry(&mut self, i: usize, out: &mut Vec<Output<F>>) {
        if let Some(e) = self.entries.get_mut(i) {
            e.incomplete = false;
            e.needed_for_output = true;
        }
        while self.entries.len() > self.capacity {
            if !self.bump(out) {
                break;
            }
        }
        self.reorder_output(out);
    }

    /// Low-delay output: honour max_num_reorder_frames.
    fn reorder_output(&mut self, out: &mut Vec<Output<F>>) {
        while self.entries.iter().filter(|e| e.needed_for_output).count() > self.max_reorder {
            if !self.bump(out) {
                break;
            }
        }
    }
}

impl<F: Clone> Default for Dpb<F> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, marks: [RefMark; 2]) -> DpbEntry<u32> {
        let meta = Arc::new(OutputMeta {
            pts: 0,
            key: false,
            crop: (0, 0, 16, 16),
            full_range: false,
            colour_primaries: 2,
            transfer_characteristics: 2,
            matrix_coefficients: 2,
            sar: (1, 1),
            draft: false,
        });
        let fields = (marks[0] != RefMark::Unused) as u8 | ((marks[1] != RefMark::Unused) as u8) << 1;
        DpbEntry {
            frame: id,
            poc: 0,
            field_poc: [0; 2],
            fields,
            frame_num: id,
            long_term_frame_idx: 0,
            field_mark: marks,
            needed_for_output: false,
            incomplete: false,
            non_existing: false,
            meta,
        }
    }

    #[test]
    fn field_lists_alternate_parity_starting_with_the_current_one() {
        use RefMark::{Short as S, Unused as U};
        // frames in list order: f0 both fields, f1 top only, f2 both, f3 bottom only
        let es = [entry(0, [S, S]), entry(1, [S, U]), entry(2, [S, S]), entry(3, [U, S])];
        let frames: Vec<&DpbEntry<u32>> = es.iter().collect();
        let ids = |v: Vec<Pic<'_, u32>>| v.into_iter().map(|(e, f)| (e.frame, f.unwrap_or(9))).collect::<Vec<_>>();
        // bottom field: bottoms of f0, f2, f3 alternate with tops of f0, f1, f2
        assert_eq!(ids(alternate_fields(&frames, 1, S)), [(0, 1), (0, 0), (2, 1), (1, 0), (3, 1), (2, 0)]);
        // top field: once the bottoms run out, the remaining tops follow in order
        let two = [entry(0, [S, S]), entry(1, [S, U]), entry(2, [S, U])];
        let frames: Vec<&DpbEntry<u32>> = two.iter().collect();
        assert_eq!(ids(alternate_fields(&frames, 0, S)), [(0, 0), (0, 1), (1, 0), (2, 0)]);
        assert!(alternate_fields(&frames, 0, RefMark::Long).is_empty());
    }
}
