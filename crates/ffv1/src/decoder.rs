//! Frames, slices and sample decoding (RFC 9043 sections 3 and 4.4-4.9).

use crate::bits::BitReader;
use crate::error::{Error, Result, ensure};
use crate::header::{Parameters, crc32, read_configuration_record, read_parameters};
use crate::range::{RangeDecoder, StateTables};
use crate::tables::{CONTEXT_SIZE, LOG2_RUN, MAX_CONTEXT_INPUTS};
use crate::{Format, Picture};

/// Largest picture accepted (pixels), so a hostile size cannot allocate unbounded memory.
const MAX_PIXELS: u64 = 1 << 28;

/// Section 3.8.2.5: one Golomb Rice context state.
#[derive(Clone, Copy, Debug)]
struct VlcState {
    drift: i32,
    error_sum: i32,
    bias: i32,
    count: i32,
}

impl VlcState {
    const INITIAL: VlcState = VlcState { drift: 0, error_sum: 4, bias: 0, count: 1 };
}

/// The adaptive states of one context group (Y, chroma or transparency) of one slice.
#[derive(Clone, Debug)]
struct GroupState {
    quant_table_set: usize,
    range: Vec<[u8; CONTEXT_SIZE]>,
    vlc: Vec<VlcState>,
}

impl GroupState {
    fn initial(p: &Parameters, set: usize) -> Result<GroupState> {
        let qs = p.quant_table_sets.get(set).ok_or_else(|| Error::Invalid(format!("quant_table_set_index {set}")))?;
        let range = match &qs.initial_states {
            Some(rows) => rows.clone(),
            None => vec![[128u8; CONTEXT_SIZE]; qs.context_count],
        };
        Ok(GroupState { quant_table_set: set, range, vlc: vec![VlcState::INITIAL; qs.context_count] })
    }
}

/// The context states a slice position carries from one frame to the next (non-key frames).
#[derive(Clone, Debug)]
struct SliceState {
    groups: Vec<GroupState>,
}

/// Three rows of one plane with the border of section 3.1: index `x + 2` is column `x`; columns
/// -2 and -1 and column `w` are the assumed border.
struct Lines {
    w: usize,
    cur: Vec<i32>,
    prev: Vec<i32>,
    prev2: Vec<i32>,
}

impl Lines {
    fn new(w: usize) -> Lines {
        Lines { w, cur: vec![0; w + 3], prev: vec![0; w + 3], prev2: vec![0; w + 3] }
    }

    /// Moves to the next row: column -1 of the new row is column 0 of the row above.
    fn next_row(&mut self) {
        std::mem::swap(&mut self.prev2, &mut self.prev);
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.cur[0] = 0;
        self.cur[1] = self.prev[2];
    }

    /// Column `w` repeats column `w - 1` (the right border).
    fn finish_row(&mut self) {
        let w = self.w;
        self.cur[w + 2] = self.cur[w + 1];
    }

    fn row(&self) -> &[i32] {
        self.cur.get(2..self.w + 2).unwrap_or_default()
    }
}

/// The entropy decoder of a slice's content.
enum Coder<'a> {
    Range(RangeDecoder<'a>),
    Golomb(BitReader<'a>),
}

/// Per-plane Golomb run state (`run_index`, section 3.8.2.2.1).
#[derive(Default, Clone, Copy)]
struct RunState {
    run_index: usize,
}

/// What a line decode needs besides the rows.
struct LineCtx<'a> {
    tables: &'a [[i32; 256]; MAX_CONTEXT_INPUTS],
    states: &'a StateTables,
    bits: u32,
    /// Section 3.3.1: 16-bit samples predicted as signed values.
    signed16: bool,
}

#[inline]
fn median(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

#[inline]
fn s16(v: i32) -> i32 {
    if v >= 32768 { v - 65536 } else { v }
}

/// Section 3.8.2.3.
#[inline]
fn sign_extend(v: i32, bits: u32) -> i32 {
    let bits = bits.clamp(1, 31);
    let shift = 32 - bits;
    (v << shift) >> shift
}

/// Figure 26 / 27: a signed Golomb Rice code.
#[inline]
fn get_sr_golomb(br: &mut BitReader<'_>, k: u32, bits: u32) -> i32 {
    let mut v: u32 = 0;
    let mut found = false;
    for prefix in 0..12u32 {
        if br.bit() == 1 {
            v = br.bits(k).wrapping_add(prefix << k.min(31));
            found = true;
            break;
        }
    }
    if !found {
        v = br.bits(bits).wrapping_add(11);
    }
    if v & 1 == 1 { -((v >> 1) as i32) - 1 } else { (v >> 1) as i32 }
}

/// Section 3.8.2.4: `get_vlc_symbol`.
fn get_vlc_symbol(br: &mut BitReader<'_>, s: &mut VlcState, bits: u32) -> i32 {
    let mut i = i64::from(s.count);
    let mut k = 0u32;
    while i < i64::from(s.error_sum) && k < 31 {
        k += 1;
        i += i;
    }
    let mut v = get_sr_golomb(br, k, bits);
    if 2 * i64::from(s.drift) < -i64::from(s.count) {
        v = -1 - v;
    }
    let ret = sign_extend(v.wrapping_add(s.bias), bits);
    s.error_sum = s.error_sum.saturating_add(v.saturating_abs());
    s.drift = s.drift.saturating_add(v);
    if s.count == 128 {
        s.count >>= 1;
        s.drift >>= 1;
        s.error_sum >>= 1;
    }
    s.count += 1;
    if s.drift <= -s.count {
        s.bias = (s.bias - 1).max(-128);
        s.drift = (s.drift + s.count).max(-s.count + 1);
    } else if s.drift > 0 {
        s.bias = (s.bias + 1).min(127);
        s.drift = (s.drift - s.count).min(0);
    }
    ret
}

/// One `Line( p, y )`: decodes `lines.cur` from the rows above.
fn decode_line(coder: &mut Coder<'_>, lines: &mut Lines, g: &mut GroupState, run: &mut RunState, c: &LineCtx<'_>) -> Result<()> {
    let w = lines.w;
    let mask = ((1i64 << c.bits) - 1) as i32;
    let [q0, q1, q2, q3, q4] = c.tables;
    let mut run_index = run.run_index;
    let mut run_mode = 0u8;
    let mut run_count: i64 = 0;
    for x in 0..w {
        let i = x + 2;
        let t = lines.prev[i];
        let tl = lines.prev[i - 1];
        let tr = lines.prev[i + 1];
        let l = lines.cur[i - 1];
        let ll = lines.cur[i - 2];
        let tt = lines.prev2[i];
        let ctx = q0[((l - tl) & 255) as usize]
            + q1[((tl - t) & 255) as usize]
            + q2[((t - tr) & 255) as usize]
            + q3[((ll - l) & 255) as usize]
            + q4[((tt - t) & 255) as usize];
        let pred = if c.signed16 {
            let (l, t, tl) = (s16(l), s16(t), s16(tl));
            median(l, t, l + t - tl)
        } else {
            median(l, t, l + t - tl)
        };
        let ci = ctx.unsigned_abs() as usize;
        let mut diff = match coder {
            Coder::Range(rc) => {
                let st = g.range.get_mut(ci).ok_or_else(|| Error::Invalid(format!("context {ctx} out of range")))?;
                rc.get_symbol(c.states, st, true)
            }
            Coder::Golomb(br) => {
                let st = g.vlc.get_mut(ci).ok_or_else(|| Error::Invalid(format!("context {ctx} out of range")))?;
                if ctx == 0 && run_mode == 0 {
                    run_mode = 1;
                }
                if run_mode != 0 {
                    if run_count == 0 && run_mode == 1 {
                        let lr = LOG2_RUN[run_index.min(LOG2_RUN.len() - 1)];
                        if br.bit() == 1 {
                            run_count = 1i64 << lr;
                            if x as i64 + run_count <= w as i64 {
                                run_index = (run_index + 1).min(LOG2_RUN.len() - 1);
                            }
                        } else {
                            run_count = if lr > 0 { i64::from(br.bits(u32::from(lr))) } else { 0 };
                            run_index = run_index.saturating_sub(1);
                            run_mode = 2;
                        }
                    }
                    run_count -= 1;
                    if run_count < 0 {
                        run_mode = 0;
                        run_count = 0;
                        let d = get_vlc_symbol(br, st, c.bits);
                        if d >= 0 { d + 1 } else { d }
                    } else {
                        0
                    }
                } else {
                    get_vlc_symbol(br, st, c.bits)
                }
            }
        };
        if ctx < 0 {
            diff = diff.wrapping_neg();
        }
        lines.cur[i] = pred.wrapping_add(diff) & mask;
    }
    run.run_index = run_index;
    if let Coder::Golomb(br) = coder {
        ensure!(!br.overrun(), "Golomb slice read past its end");
    }
    lines.finish_row();
    Ok(())
}

/// A slice's place in the frame, in pixels.
#[derive(Clone, Copy, Debug)]
struct SliceRect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// A FFV1 decoder for one stream.
pub struct Decoder {
    width: u32,
    height: u32,
    /// From the configuration record (version 3), or from the last key frame (versions 0 and 1).
    params: Option<Parameters>,
    record: bool,
    /// Context states by slice raster position (`slice_y * num_h_slices + slice_x`).
    slices: Vec<Option<SliceState>>,
    have_key: bool,
}

impl Decoder {
    /// A decoder for `width` x `height` frames. `config` is the container's codec private data:
    /// the configuration record for version 3 (empty for versions 0 and 1).
    pub fn new(width: u32, height: u32, config: &[u8]) -> Result<Decoder> {
        ensure!(width > 0 && height > 0, "frame size {width}x{height}");
        ensure!(u64::from(width) * u64::from(height) <= MAX_PIXELS, "frame size {width}x{height} too large");
        let (params, record) = if config.len() > 4 { (Some(read_configuration_record(config)?), true) } else { (None, false) };
        Ok(Decoder { width, height, params, record, slices: Vec::new(), have_key: false })
    }

    /// Forgets the context states (after a seek: decoding resumes at a key frame).
    pub fn reset(&mut self) {
        self.slices.clear();
        self.have_key = false;
    }

    /// Decodes one frame (one container sample).
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture> {
        ensure!(!data.is_empty(), "empty frame");
        let default_tables = StateTables::default_tables();
        let mut rc = RangeDecoder::new(data);
        let mut kstate = 128u8;
        let key = rc.get_rac(&default_tables, &mut kstate);
        if key && !self.record {
            let p = read_parameters(&mut rc)?;
            ensure!(p.version <= 1, "FFV1 version {} without a configuration record", p.version);
            self.params = Some(p);
        }
        let p = self.params.clone().ok_or(Error::MissingKeyframe)?;
        if key {
            self.slices = vec![None; (p.num_h_slices * p.num_v_slices) as usize];
            self.have_key = true;
        } else if !self.have_key {
            return Err(Error::MissingKeyframe);
        } else if p.intra == 1 {
            return Err(Error::Invalid("non-key frame in an intra-only stream".into()));
        }
        let mut pic = self.new_picture(&p, key)?;
        if p.version <= 1 {
            let rect = SliceRect { x: 0, y: 0, w: self.width as usize, h: self.height as usize };
            self.decode_slice_content(&p, rc, data, 0, rect, &[0, 0, 0], key, &mut pic)?;
        } else {
            for (n, (start, content_end, end)) in locate_slices(data, &p)?.into_iter().enumerate() {
                let bytes = data.get(start..content_end).unwrap_or_default();
                if p.ec != 0 && crc32(data.get(start..end).unwrap_or_default()) != 0 {
                    return Err(Error::Crc(format!("slice {n}")));
                }
                let mut rc = RangeDecoder::new(bytes);
                if n == 0 {
                    let mut ks = 128u8;
                    rc.get_rac(&default_tables, &mut ks);
                }
                self.decode_slice(&p, rc, bytes, key, &mut pic)?;
            }
        }
        Ok(pic)
    }

    fn new_picture(&self, p: &Parameters, key: bool) -> Result<Picture> {
        let (w, h) = (self.width as usize, self.height as usize);
        let format = if p.colorspace_type == 1 {
            Format::Rgb
        } else if p.chroma_planes {
            Format::Yuv { log2_h: p.log2_h_chroma_subsample, log2_v: p.log2_v_chroma_subsample }
        } else {
            Format::Gray
        };
        let (cw, ch) = self.chroma_size(p);
        let mut planes = vec![vec![0u16; w * h]];
        match format {
            Format::Rgb => {
                planes.push(vec![0u16; w * h]);
                planes.push(vec![0u16; w * h]);
            }
            Format::Yuv { .. } => {
                planes.push(vec![0u16; cw * ch]);
                planes.push(vec![0u16; cw * ch]);
            }
            Format::Gray => {}
        }
        let alpha = p.extra_plane.then(|| vec![0u16; w * h]);
        Ok(Picture { width: self.width, height: self.height, bits: p.bits_per_raw_sample, format, planes, alpha, key, sar: None, picture_structure: 0 })
    }

    fn chroma_size(&self, p: &Parameters) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        if p.colorspace_type == 0 && p.chroma_planes {
            (w.div_ceil(1 << p.log2_h_chroma_subsample), h.div_ceil(1 << p.log2_v_chroma_subsample))
        } else {
            (w, h)
        }
    }

    /// Section 4.6 and the slice's content (version 3).
    fn decode_slice(&mut self, p: &Parameters, mut rc: RangeDecoder<'_>, bytes: &[u8], key: bool, pic: &mut Picture) -> Result<()> {
        let tables = &p.states;
        let mut st = [128u8; CONTEXT_SIZE];
        let mut ur = |rc: &mut RangeDecoder<'_>| -> Result<usize> {
            usize::try_from(rc.get_symbol(tables, &mut st, false)).map_err(|_| Error::Invalid("negative slice header field".into()))
        };
        let sx = ur(&mut rc)?;
        let sy = ur(&mut rc)?;
        let sw = ur(&mut rc)? + 1;
        let sh = ur(&mut rc)? + 1;
        let (nh, nv) = (p.num_h_slices as usize, p.num_v_slices as usize);
        ensure!(sx + sw <= nh && sy + sh <= nv, "slice {sx},{sy} {sw}x{sh} outside the {nh}x{nv} raster");
        let mut qidx = [0usize; 3];
        for q in qidx.iter_mut().take(p.quant_table_set_index_count()) {
            *q = ur(&mut rc)?;
            ensure!(*q < p.quant_table_sets.len(), "quant_table_set_index {q}");
        }
        let picture_structure = ur(&mut rc)?;
        let sar_num = ur(&mut rc)?;
        let sar_den = ur(&mut rc)?;
        if sar_num > 0 && sar_den > 0 && pic.sar.is_none() {
            pic.sar = Some((sar_num as u32, sar_den as u32));
        }
        pic.picture_structure = picture_structure.min(255) as u8;
        let (fw, fh) = (self.width as usize, self.height as usize);
        let x0 = sx * fw / nh;
        let y0 = sy * fh / nv;
        let rect = SliceRect { x: x0, y: y0, w: (sx + sw) * fw / nh - x0, h: (sy + sh) * fh / nv - y0 };
        let index = sy * nh + sx;
        self.decode_slice_content(p, rc, bytes, index, rect, &qidx, key, pic)
    }

    /// `SliceContent( )` of one slice; `rc` stands after the slice header (or the frame header).
    fn decode_slice_content(
        &mut self,
        p: &Parameters,
        mut rc: RangeDecoder<'_>,
        bytes: &[u8],
        index: usize,
        rect: SliceRect,
        qidx: &[usize; 3],
        key: bool,
        pic: &mut Picture,
    ) -> Result<()> {
        if rect.w == 0 || rect.h == 0 {
            return Ok(());
        }
        let groups = p.quant_table_set_index_count();
        let mut state = match self.slices.get_mut(index).and_then(Option::take) {
            Some(s) if !key => s,
            _ => {
                ensure!(key, "non-key frame slice without a previous slice state");
                let mut g = Vec::with_capacity(groups);
                for q in qidx.iter().take(groups) {
                    g.push(GroupState::initial(p, *q)?);
                }
                SliceState { groups: g }
            }
        };
        let mut coder = if p.coder_type == 0 {
            // Sentinel mode: the range coded header ends with a state-129 symbol, after which the
            // range decoder has read one byte past it
            let mut s = 129u8;
            rc.get_rac(&p.states, &mut s);
            let start = rc.bytes_read().saturating_sub(1).min(bytes.len());
            Coder::Golomb(BitReader::new(bytes.get(start..).unwrap_or_default()))
        } else {
            Coder::Range(rc)
        };
        let res = if p.colorspace_type == 0 {
            self.decode_ycbcr(p, &mut coder, &mut state, rect, pic)
        } else {
            self.decode_rct(p, &mut coder, &mut state, rect, pic)
        };
        if let Some(slot) = self.slices.get_mut(index) {
            *slot = Some(state);
        }
        res
    }

    fn line_ctx<'a>(&self, p: &'a Parameters, g: &GroupState, bits: u32) -> Result<LineCtx<'a>> {
        let qs = p.quant_table_sets.get(g.quant_table_set).ok_or_else(|| Error::Invalid("quant table set".into()))?;
        let signed16 = p.colorspace_type == 0 && p.bits_per_raw_sample == 16 && p.coder_type != 0;
        Ok(LineCtx { tables: &qs.tables, states: &p.states, bits, signed16 })
    }

    /// YCbCr: plane by plane, line by line (section 4.7).
    fn decode_ycbcr(&self, p: &Parameters, coder: &mut Coder<'_>, state: &mut SliceState, r: SliceRect, pic: &mut Picture) -> Result<()> {
        let (cw, ch) = self.chroma_size(p);
        let fw = self.width as usize;
        let (hs, vs) = (p.log2_h_chroma_subsample, p.log2_v_chroma_subsample);
        for plane in 0..p.primary_color_count() {
            let chroma = p.chroma_planes && (plane == 1 || plane == 2);
            let (px, py, pw, ph, stride) = if chroma {
                // the slice's chroma origin rounds down (a slice of odd luma width overlaps the next
                // one by a chroma column, which the later slice writes)
                let x = r.x >> hs;
                let y = r.y >> vs;
                (x, y, r.w.div_ceil(1 << hs).min(cw.saturating_sub(x)), r.h.div_ceil(1 << vs).min(ch.saturating_sub(y)), cw)
            } else {
                (r.x, r.y, r.w, r.h, fw)
            };
            let gi = p.plane_group(plane);
            let g = state.groups.get_mut(gi).ok_or_else(|| Error::Invalid("context group".into()))?;
            let lc = self.line_ctx(p, g, p.bits_per_raw_sample)?;
            let mut lines = Lines::new(pw);
            let mut run = RunState::default();
            let out: &mut Vec<u16> = if p.extra_plane && plane == p.primary_color_count() - 1 {
                pic.alpha.as_mut().ok_or_else(|| Error::Invalid("transparency plane".into()))?
            } else {
                pic.planes.get_mut(plane).ok_or_else(|| Error::Invalid("plane".into()))?
            };
            for y in 0..ph {
                lines.next_row();
                decode_line(coder, &mut lines, g, &mut run, &lc)?;
                let base = (py + y) * stride + px;
                if let Some(dst) = out.get_mut(base..base + pw) {
                    for (d, &s) in dst.iter_mut().zip(lines.row()) {
                        *d = s as u16;
                    }
                }
            }
        }
        Ok(())
    }

    /// RGB through the JPEG 2000 RCT: line by line, the planes of each line in turn (sections
    /// 3.7.2 and 4.7).
    fn decode_rct(&self, p: &Parameters, coder: &mut Coder<'_>, state: &mut SliceState, r: SliceRect, pic: &mut Picture) -> Result<()> {
        let bits = p.bits_per_raw_sample;
        let count = p.primary_color_count();
        let mut lines: Vec<Lines> = (0..count).map(|_| Lines::new(r.w)).collect();
        // with the planes of each line interleaved, one run_index serves the whole slice
        let mut run = RunState::default();
        let fw = self.width as usize;
        let off = 1i32 << bits;
        let maxv = (1i32 << bits) - 1;
        // section 3.7.2.1: 9-15 bits without transparency code blue as the reference
        let bgr = (9..=15).contains(&bits) && !p.extra_plane;
        for y in 0..r.h {
            for (plane, l) in lines.iter_mut().enumerate() {
                let gi = p.plane_group(plane);
                let g = state.groups.get_mut(gi).ok_or_else(|| Error::Invalid("context group".into()))?;
                let lc = self.line_ctx(p, g, bits + 1)?;
                l.next_row();
                decode_line(coder, l, g, &mut run, &lc)?;
            }
            let base = (r.y + y) * fw + r.x;
            for x in 0..r.w {
                let i = x + 2;
                let yv = lines[0].cur[i];
                let cb = lines[1].cur[i] - off;
                let cr = lines[2].cur[i] - off;
                let (rv, gv, bv) = if bgr {
                    let b = yv - ((cb + cr) >> 2);
                    (cr + b, cb + b, b)
                } else {
                    let g = yv - ((cb + cr) >> 2);
                    (cr + g, g, cb + g)
                };
                let k = base + x;
                for (plane, v) in [rv, gv, bv].into_iter().enumerate() {
                    if let Some(d) = pic.planes.get_mut(plane).and_then(|pl| pl.get_mut(k)) {
                        *d = v.clamp(0, maxv) as u16;
                    }
                }
                if let (Some(d), Some(&s)) = (pic.alpha.as_mut().and_then(|a| a.get_mut(k)), lines.get(3).and_then(|l| l.cur.get(i))) {
                    *d = (s & maxv) as u16;
                }
            }
        }
        Ok(())
    }
}

/// The slices of a version 3 frame from their footers, last to first (Appendix A): (start,
/// end of content, end of footer).
fn locate_slices(data: &[u8], p: &Parameters) -> Result<Vec<(usize, usize, usize)>> {
    let trailer = 3 + if p.ec != 0 { 5 } else { 0 };
    let max = (p.num_h_slices * p.num_v_slices) as usize;
    let mut out = Vec::new();
    let mut end = data.len();
    while end > 0 {
        ensure!(end >= trailer && out.len() < max, "slice footers do not add up");
        let b = |i: usize| usize::from(data.get(end - trailer + i).copied().unwrap_or(0));
        let size = (b(0) << 16) | (b(1) << 8) | b(2);
        let content_end = end - trailer;
        ensure!(size <= content_end, "slice_size {size} past the frame start");
        let start = content_end - size;
        out.push((start, content_end, end));
        end = start;
    }
    out.reverse();
    Ok(out)
}

/// Whether `data` is a key frame (its first range coded symbol, section 4.4).
pub fn is_keyframe(data: &[u8]) -> bool {
    let t = StateTables::default_tables();
    let mut s = 128u8;
    !data.is_empty() && RangeDecoder::new(data).get_rac(&t, &mut s)
}
