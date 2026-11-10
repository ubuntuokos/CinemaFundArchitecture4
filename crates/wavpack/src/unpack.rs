// Ported from the WavPack library (unpack.c, unpack_floats.c, decorr_utils.c and the metadata
// handlers of open_utils.c), Copyright (c) 1998 - 2025 David Bryant, Conifer Software;
// BSD-3-Clause (see `LICENSE.WavPack`). CFA4 changes: rewritten in safe Rust, the correction-file
// (`.wvc`) and DSD paths left out, GPL-3.0-or-later.

//! One WavPack block → its samples: metadata, entropy decoding, the decorrelation passes, joint
//! stereo, and the final fix-ups (shifts, extended integers, floats, lossy clipping), checked
//! against the block CRCs. A block that fails any check decodes to silence, as in the reference.

use crate::words::{Bits, Mode, WORD_EOF, Words, exp2s, restore_weight};
use crate::{Header, flags as f};

const MAX_TERM: usize = 8;
const MAX_NTERMS: usize = 16;

const ID_LARGE: u8 = 0x80;
const ID_ODD_SIZE: u8 = 0x40;
const ID_OPTIONAL_DATA: u8 = 0x20;
pub(crate) const ID_DECORR_TERMS: u8 = 0x2;
const ID_DECORR_WEIGHTS: u8 = 0x3;
const ID_DECORR_SAMPLES: u8 = 0x4;
const ID_ENTROPY_VARS: u8 = 0x5;
const ID_HYBRID_PROFILE: u8 = 0x6;
const ID_SHAPING_WEIGHTS: u8 = 0x7;
const ID_FLOAT_INFO: u8 = 0x8;
const ID_INT32_INFO: u8 = 0x9;
const ID_WV_BITSTREAM: u8 = 0xa;
const ID_WVC_BITSTREAM: u8 = 0xb;
const ID_WVX_BITSTREAM: u8 = 0xc;
pub(crate) const ID_CHANNEL_INFO: u8 = 0xd;
const ID_DSD_BLOCK: u8 = 0xe;
pub(crate) const ID_SAMPLE_RATE: u8 = 0x27;
const ID_WVX_NEW_BITSTREAM: u8 = ID_OPTIONAL_DATA | ID_WVX_BITSTREAM;

const FLOAT_SHIFT_ONES: u8 = 1;
const FLOAT_SHIFT_SAME: u8 = 2;
const FLOAT_SHIFT_SENT: u8 = 4;
const FLOAT_ZEROS_SENT: u8 = 8;
const FLOAT_NEG_ZEROS: u8 = 0x10;

/// The metadata sub-blocks after a block's 32-byte header: `(id, data)`, ids without the size
/// bits. Stops at the first malformed sub-block (`Err` carries the ones before it).
pub(crate) fn metadata(block: &[u8]) -> Result<Vec<(u8, &[u8])>, Vec<(u8, &[u8])>> {
    let mut out = Vec::new();
    let mut p = crate::HEADER_LEN;
    while block.len().saturating_sub(p) >= 2 {
        let (Some(&id0), Some(&w)) = (block.get(p), block.get(p + 1)) else { break };
        p += 2;
        let mut id = id0;
        let mut len = usize::from(w) << 1;
        if id & ID_LARGE != 0 {
            id &= !ID_LARGE;
            let (Some(&a), Some(&b)) = (block.get(p), block.get(p + 1)) else { return Err(out) };
            len += (usize::from(a) << 9) + (usize::from(b) << 17);
            p += 2;
        }
        if id & ID_ODD_SIZE != 0 {
            if len == 0 {
                return Err(out);
            }
            id &= !ID_ODD_SIZE;
            len -= 1;
        }
        let padded = len + (len & 1);
        let Some(data) = block.get(p..p + len) else { return Err(out) };
        if block.len() - p < padded {
            return Err(out);
        }
        p += padded;
        out.push((id, data));
    }
    Ok(out)
}

#[derive(Clone, Copy, Default)]
struct Pass {
    term: i32,
    delta: i32,
    weight_a: i32,
    weight_b: i32,
    samples_a: [i32; MAX_TERM],
    samples_b: [i32; MAX_TERM],
}

#[derive(Default)]
struct Stream<'a> {
    flags: u32,
    passes: Vec<Pass>,
    words: Words,
    wv: Bits<'a>,
    wvx: Bits<'a>,
    crc_wvx: u32,
    int32_sent_bits: u8,
    int32_zeros: u8,
    int32_ones: u8,
    int32_dups: u8,
    int32_max_width: u32,
    float_flags: u8,
    float_shift: u8,
    float_max_exp: u8,
    float_max_shifted_ones: u32,
    float_min_shifted_zeros: u32,
}

#[inline]
fn le16(d: &[u8], i: usize) -> i32 {
    match (d.get(i), d.get(i + 1)) {
        (Some(a), Some(b)) => i32::from(*a) | i32::from(*b) << 8,
        _ => 0,
    }
}

#[inline]
fn le16s(d: &[u8], i: usize) -> i32 {
    le16(d, i) as u16 as i16 as i32
}

impl<'a> Stream<'a> {
    fn mono(&self) -> bool {
        self.flags & f::MONO_DATA != 0
    }

    /// The reference `process_metadata` for one sub-block; `false` mutes the block.
    fn metadata(&mut self, version: u16, id: u8, d: &'a [u8]) -> bool {
        let mono = self.mono();
        match id {
            ID_DECORR_TERMS => {
                if d.len() > MAX_NTERMS {
                    return false;
                }
                self.passes = vec![Pass::default(); d.len()];
                for (byte, pass) in d.iter().zip(self.passes.iter_mut().rev()) {
                    pass.term = i32::from(byte & 0x1f) - 5;
                    pass.delta = i32::from((byte >> 5) & 7);
                    let t = pass.term;
                    if t == 0 || t < -3 || (t > MAX_TERM as i32 && t < 17) || t > 18 || (mono && t < 0) {
                        return false;
                    }
                }
                true
            }
            ID_DECORR_WEIGHTS => {
                let terms = if mono { d.len() } else { d.len() / 2 };
                if terms > self.passes.len() {
                    return false;
                }
                for p in self.passes.iter_mut() {
                    p.weight_a = 0;
                    p.weight_b = 0;
                }
                let mut bytes = d.iter().map(|b| restore_weight(*b as i8));
                for p in self.passes.iter_mut().rev().take(terms) {
                    p.weight_a = bytes.next().unwrap_or(0);
                    if !mono {
                        p.weight_b = bytes.next().unwrap_or(0);
                    }
                }
                true
            }
            ID_DECORR_SAMPLES => self.decorr_samples(version, d),
            ID_ENTROPY_VARS => {
                if d.len() != if mono { 6 } else { 12 } {
                    return false;
                }
                for ch in 0..if mono { 1 } else { 2 } {
                    for k in 0..3 {
                        self.words.c[ch].median[k] = exp2s(le16(d, ch * 6 + k * 2)) as u32;
                    }
                }
                true
            }
            ID_HYBRID_PROFILE => self.hybrid_profile(d),
            ID_SHAPING_WEIGHTS => d.len() == 2 || d.len() >= if mono { 4 } else { 8 },
            ID_FLOAT_INFO => {
                let [a, b, c, _] = d else { return false };
                (self.float_flags, self.float_shift, self.float_max_exp) = (*a, *b, *c);
                true
            }
            ID_INT32_INFO => {
                let [a, b, c, e] = d else { return false };
                (self.int32_sent_bits, self.int32_zeros, self.int32_ones, self.int32_dups) = (*a, *b, *c, *e);
                true
            }
            ID_WV_BITSTREAM => {
                if d.is_empty() || d.len() & 1 != 0 {
                    return false;
                }
                self.wv = Bits::new(d);
                true
            }
            // a correction stream belongs to a `.wvc` file; inside the main file it is malformed
            ID_WVC_BITSTREAM => !d.is_empty() && d.len() & 1 == 0,
            ID_WVX_BITSTREAM | ID_WVX_NEW_BITSTREAM => {
                if d.len() <= 4 || d.len() & 1 != 0 {
                    return false;
                }
                self.crc_wvx = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                self.wvx = Bits::new(&d[4..]);
                if id == ID_WVX_NEW_BITSTREAM {
                    if self.flags & f::FLOAT_DATA != 0 {
                        self.float_min_shifted_zeros = self.wvx.bits(5);
                        self.float_max_shifted_ones = self.wvx.bits(5);
                    } else {
                        self.int32_max_width = self.wvx.bits(5);
                    }
                }
                true
            }
            ID_DSD_BLOCK => false,
            // file-level information (channels, rate, wrappers, checksums) is read elsewhere
            0 | ID_CHANNEL_INFO | 0x2b | 0x25 | 0x2a | ID_SAMPLE_RATE | 0x21..=0x24 | 0x26 | 0x28 | 0x29 | 0x2f => true,
            _ => id & ID_OPTIONAL_DATA != 0,
        }
    }

    fn decorr_samples(&mut self, version: u16, d: &[u8]) -> bool {
        let mono = self.mono();
        for p in self.passes.iter_mut() {
            p.samples_a = [0; MAX_TERM];
            p.samples_b = [0; MAX_TERM];
        }
        let mut i = 0usize;
        let per = if mono { 2 } else { 4 };
        if version == 0x402 && self.flags & f::HYBRID_FLAG != 0 {
            // dc.error (used only with a correction file)
            if i + per > d.len() {
                return false;
            }
            i += per;
        }
        for p in self.passes.iter_mut().rev() {
            if i >= d.len() {
                break;
            }
            if p.term > MAX_TERM as i32 {
                if i + 2 * per > d.len() {
                    return false;
                }
                p.samples_a[0] = exp2s(le16s(d, i));
                p.samples_a[1] = exp2s(le16s(d, i + 2));
                i += 4;
                if !mono {
                    p.samples_b[0] = exp2s(le16s(d, i));
                    p.samples_b[1] = exp2s(le16s(d, i + 2));
                    i += 4;
                }
            } else if p.term < 0 {
                if i + 4 > d.len() {
                    return false;
                }
                p.samples_a[0] = exp2s(le16s(d, i));
                p.samples_b[0] = exp2s(le16s(d, i + 2));
                i += 4;
            } else {
                for m in 0..p.term.clamp(0, MAX_TERM as i32) as usize {
                    if i + per > d.len() {
                        return false;
                    }
                    p.samples_a[m] = exp2s(le16s(d, i));
                    i += 2;
                    if !mono {
                        p.samples_b[m] = exp2s(le16s(d, i));
                        i += 2;
                    }
                }
            }
        }
        i == d.len()
    }

    fn hybrid_profile(&mut self, d: &[u8]) -> bool {
        let mono = self.mono();
        let per = if mono { 2 } else { 4 };
        let mut i = 0;
        let w = &mut self.words;
        if self.flags & f::HYBRID_BITRATE != 0 {
            if i + per > d.len() {
                return false;
            }
            w.c[0].slow_level = exp2s(le16(d, i)) as u32;
            if !mono {
                w.c[1].slow_level = exp2s(le16(d, i + 2)) as u32;
            }
            i += per;
        }
        if i + per > d.len() {
            return false;
        }
        w.bitrate_acc[0] = (le16(d, i) as u32) << 16;
        if !mono {
            w.bitrate_acc[1] = (le16(d, i + 2) as u32) << 16;
        }
        i += per;
        if i < d.len() {
            if i + per > d.len() {
                return false;
            }
            w.bitrate_delta[0] = exp2s(le16s(d, i)) as u32;
            if !mono {
                w.bitrate_delta[1] = exp2s(le16s(d, i + 2)) as u32;
            }
            i += per;
            if i < d.len() {
                return false;
            }
        } else {
            w.bitrate_delta = [0; 2];
        }
        true
    }
}

#[inline]
fn apply_weight(weight: i32, sample: i32) -> i32 {
    if sample == i32::from(sample as i16) {
        weight.wrapping_mul(sample).wrapping_add(512) >> 10
    } else {
        ((((sample & 0xffff).wrapping_mul(weight)) >> 9).wrapping_add(((sample & !0xffff) >> 9).wrapping_mul(weight)).wrapping_add(1)) >> 1
    }
}

#[inline]
fn update_weight(weight: &mut i32, delta: i32, source: i32, result: i32) {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        *weight = (delta ^ s).wrapping_add(weight.wrapping_sub(s));
    }
}

#[inline]
fn update_weight_clip(weight: &mut i32, delta: i32, source: i32, result: i32) {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        let mut w = (*weight ^ s).wrapping_add(delta.wrapping_sub(s));
        if w > 1024 {
            w = 1024;
        }
        *weight = (w ^ s).wrapping_sub(s);
    }
}

fn decorr_mono_pass(p: &mut Pass, buf: &mut [i32]) {
    let delta = p.delta;
    match p.term {
        17 | 18 => {
            for b in buf.iter_mut() {
                let (s0, s1) = (p.samples_a[0], p.samples_a[1]);
                let sam = if p.term == 17 { s0.wrapping_mul(2).wrapping_sub(s1) } else { s0.wrapping_mul(3).wrapping_sub(s1) >> 1 };
                p.samples_a[1] = s0;
                p.samples_a[0] = apply_weight(p.weight_a, sam).wrapping_add(*b);
                update_weight(&mut p.weight_a, delta, sam, *b);
                *b = p.samples_a[0];
            }
        }
        _ => {
            let mut m = 0usize;
            let mut k = (p.term as usize) & (MAX_TERM - 1);
            for b in buf.iter_mut() {
                let sam = p.samples_a[m];
                p.samples_a[k] = apply_weight(p.weight_a, sam).wrapping_add(*b);
                update_weight(&mut p.weight_a, delta, sam, *b);
                *b = p.samples_a[k];
                m = (m + 1) & (MAX_TERM - 1);
                k = (k + 1) & (MAX_TERM - 1);
            }
        }
    }
}

fn decorr_stereo_pass(p: &mut Pass, buf: &mut [i32]) {
    let delta = p.delta;
    match p.term {
        17 | 18 => {
            for b in buf.chunks_exact_mut(2) {
                for (ch, x) in b.iter_mut().enumerate() {
                    let (samples, weight) = if ch == 0 { (&mut p.samples_a, &mut p.weight_a) } else { (&mut p.samples_b, &mut p.weight_b) };
                    let (s0, s1) = (samples[0], samples[1]);
                    let sam = if p.term == 17 { s0.wrapping_mul(2).wrapping_sub(s1) } else { s0.wrapping_add(s0.wrapping_sub(s1) >> 1) };
                    samples[1] = s0;
                    let tmp = *x;
                    samples[0] = apply_weight(*weight, sam).wrapping_add(tmp);
                    *x = samples[0];
                    update_weight(weight, delta, sam, tmp);
                }
            }
        }
        -1 => {
            for b in buf.chunks_exact_mut(2) {
                let sam = b[0].wrapping_add(apply_weight(p.weight_a, p.samples_a[0]));
                update_weight_clip(&mut p.weight_a, delta, p.samples_a[0], b[0]);
                b[0] = sam;
                p.samples_a[0] = b[1].wrapping_add(apply_weight(p.weight_b, sam));
                update_weight_clip(&mut p.weight_b, delta, sam, b[1]);
                b[1] = p.samples_a[0];
            }
        }
        -2 => {
            for b in buf.chunks_exact_mut(2) {
                let sam = b[1].wrapping_add(apply_weight(p.weight_b, p.samples_b[0]));
                update_weight_clip(&mut p.weight_b, delta, p.samples_b[0], b[1]);
                b[1] = sam;
                p.samples_b[0] = b[0].wrapping_add(apply_weight(p.weight_a, sam));
                update_weight_clip(&mut p.weight_a, delta, sam, b[0]);
                b[0] = p.samples_b[0];
            }
        }
        -3 => {
            for b in buf.chunks_exact_mut(2) {
                let sam_a = b[0].wrapping_add(apply_weight(p.weight_a, p.samples_a[0]));
                update_weight_clip(&mut p.weight_a, delta, p.samples_a[0], b[0]);
                let sam_b = b[1].wrapping_add(apply_weight(p.weight_b, p.samples_b[0]));
                update_weight_clip(&mut p.weight_b, delta, p.samples_b[0], b[1]);
                p.samples_b[0] = sam_a;
                b[0] = sam_a;
                p.samples_a[0] = sam_b;
                b[1] = sam_b;
            }
        }
        _ => {
            let mut m = 0usize;
            let mut k = (p.term as usize) & (MAX_TERM - 1);
            for b in buf.chunks_exact_mut(2) {
                let sam = p.samples_a[m];
                p.samples_a[k] = apply_weight(p.weight_a, sam).wrapping_add(b[0]);
                update_weight(&mut p.weight_a, delta, sam, b[0]);
                b[0] = p.samples_a[k];
                let sam = p.samples_b[m];
                p.samples_b[k] = apply_weight(p.weight_b, sam).wrapping_add(b[1]);
                update_weight(&mut p.weight_b, delta, sam, b[1]);
                b[1] = p.samples_b[k];
                m = (m + 1) & (MAX_TERM - 1);
                k = (k + 1) & (MAX_TERM - 1);
            }
        }
    }
}

#[inline]
fn set_mantissa(f: &mut u32, v: u32) {
    *f ^= (*f ^ v) & 0x7f_ffff;
}
#[inline]
fn set_exponent(f: &mut u32, v: u32) {
    *f ^= (*f ^ (v << 23)) & 0x7f80_0000;
}
#[inline]
fn set_sign(f: &mut u32, v: u32) {
    *f ^= (*f ^ (v << 31)) & 0x8000_0000;
}

impl Stream<'_> {
    /// The reference `float_values` (with the extended stream) / `float_values_nowvx`.
    fn float_values(&mut self, values: &mut [i32], crc_x: &mut u32) {
        let wvx_open = self.wvx.open;
        let ff = self.float_flags;
        for v in values.iter_mut() {
            let mut shift_count = 0u32;
            let mut exp = i32::from(self.float_max_exp);
            let mut out = 0u32;
            if *v == 0 {
                if wvx_open && ff & FLOAT_ZEROS_SENT != 0 {
                    if self.wvx.bit() != 0 {
                        set_mantissa(&mut out, self.wvx.bits(23));
                        if exp >= 25 {
                            set_exponent(&mut out, self.wvx.bits(8));
                        }
                        set_sign(&mut out, self.wvx.bit());
                    } else if ff & FLOAT_NEG_ZEROS != 0 {
                        set_sign(&mut out, self.wvx.bit());
                    }
                }
            } else {
                *v = (*v as u32).wrapping_shl(u32::from(self.float_shift & 0x1f)) as i32;
                if *v < 0 {
                    *v = v.wrapping_neg();
                    set_sign(&mut out, 1);
                }
                if wvx_open && *v == 0x100_0000 {
                    if self.wvx.bit() != 0 {
                        set_mantissa(&mut out, self.wvx.bits(23));
                    }
                    set_exponent(&mut out, 255);
                } else if !wvx_open && *v >= 0x100_0000 {
                    while *v & 0xf00_0000 != 0 {
                        *v >>= 1;
                        exp += 1;
                    }
                    set_mantissa(&mut out, *v as u32);
                    set_exponent(&mut out, exp as u32);
                } else {
                    if exp != 0 {
                        while *v & 0x80_0000 == 0 {
                            exp -= 1;
                            if exp == 0 {
                                break;
                            }
                            shift_count += 1;
                            *v = (*v as u32).wrapping_shl(1) as i32;
                        }
                    }
                    shift_count &= 0x1f;
                    if shift_count != 0 {
                        if ff & FLOAT_SHIFT_ONES != 0 || (wvx_open && ff & FLOAT_SHIFT_SAME != 0 && self.wvx.bit() != 0) {
                            *v |= ((1u32 << shift_count) - 1) as i32;
                        } else if wvx_open && ff & FLOAT_SHIFT_SENT != 0 {
                            let mask = (1u32 << shift_count) - 1;
                            let mut num_zeros = 0;
                            if self.float_max_shifted_ones != 0 && shift_count > self.float_max_shifted_ones {
                                num_zeros = shift_count - self.float_max_shifted_ones;
                            }
                            if self.float_min_shifted_zeros > num_zeros {
                                num_zeros = self.float_min_shifted_zeros.min(shift_count);
                            }
                            let n = shift_count - num_zeros;
                            if n > 0 {
                                let temp = self.wvx.bits(n);
                                *v |= (temp.wrapping_shl(num_zeros) & mask) as i32;
                            }
                        }
                    }
                    set_mantissa(&mut out, *v as u32);
                    set_exponent(&mut out, exp as u32);
                }
            }
            if wvx_open {
                *crc_x = crc_x.wrapping_mul(27).wrapping_add((out & 0x7f_ffff).wrapping_mul(9)).wrapping_add(((out >> 23) & 0xff) * 3).wrapping_add(out >> 31);
            }
            *v = out as i32;
        }
    }

    /// The reference `fixup_samples`.
    fn fixup(&mut self, buf: &mut [i32], crc_x: &mut u32) {
        let flags = self.flags;
        let lossy = flags & f::HYBRID_FLAG != 0;
        let mut shift = (flags & f::SHIFT_MASK) >> f::SHIFT_LSB;
        if flags & f::FLOAT_DATA != 0 {
            self.float_values(buf, crc_x);
            return;
        }
        if flags & f::INT32_DATA != 0 {
            let sent_bits = u32::from(self.int32_sent_bits & 0x1f);
            let (mut zeros, mut ones, mut dups) = (u32::from(self.int32_zeros & 0x1f), u32::from(self.int32_ones & 0x1f), u32::from(self.int32_dups & 0x1f));
            let expand = |v: i32, zeros: u32, ones: u32, dups: u32| -> i32 {
                if zeros != 0 {
                    (v as u32).wrapping_shl(zeros) as i32
                } else if ones != 0 {
                    (v.wrapping_add(1) as u32).wrapping_shl(ones).wrapping_sub(1) as i32
                } else if dups != 0 {
                    let odd = v & 1;
                    (v.wrapping_add(odd) as u32).wrapping_shl(dups).wrapping_sub(odd as u32) as i32
                } else {
                    v
                }
            };
            if self.wvx.open {
                let max_width = self.int32_max_width;
                for v in buf.iter_mut() {
                    if sent_bits != 0 {
                        if max_width != 0 {
                            let pvalue = if *v < 0 { !*v } else { *v } as u32;
                            let width = (32 - pvalue.leading_zeros()) + sent_bits;
                            let mut bits_to_read = sent_bits as i32;
                            if width <= max_width || {
                                bits_to_read -= (width - max_width) as i32;
                                bits_to_read > 0
                            } {
                                let n = bits_to_read as u32;
                                let data = self.wvx.bits(n);
                                *v = ((*v as u32).wrapping_shl(n) | data).wrapping_shl(sent_bits - n) as i32;
                            } else {
                                *v = (*v as u32).wrapping_shl(sent_bits) as i32;
                            }
                        } else {
                            let data = self.wvx.bits(sent_bits);
                            *v = ((*v as u32).wrapping_shl(sent_bits) | data) as i32;
                        }
                    }
                    *v = expand(*v, zeros, ones, dups);
                    *crc_x = crc_x.wrapping_mul(9).wrapping_add(((*v & 0xffff) as u32).wrapping_mul(3)).wrapping_add(((*v >> 16) & 0xffff) as u32);
                }
            } else if sent_bits == 0 && zeros + ones + dups != 0 {
                while lossy && flags & f::BYTES_STORED == 3 && shift < 8 {
                    if zeros != 0 {
                        zeros -= 1;
                    } else if ones != 0 {
                        ones -= 1;
                    } else if dups != 0 {
                        dups -= 1;
                    } else {
                        break;
                    }
                    shift += 1;
                }
                for v in buf.iter_mut() {
                    *v = expand(*v, zeros, ones, dups);
                }
            } else {
                shift += zeros + sent_bits + ones + dups;
            }
        }
        let shift = shift & 0x1f;
        if lossy {
            let (min_value, max_value) = match flags & f::BYTES_STORED {
                0 => (-128i32 >> shift, 127i32 >> shift),
                1 => (-32768 >> shift, 32767 >> shift),
                2 => (-8_388_608 >> shift, 8_388_607 >> shift),
                _ => (i32::MIN >> shift, i32::MAX >> shift),
            };
            let min_shifted = (min_value as u32).wrapping_shl(shift) as i32;
            let max_shifted = (max_value as u32).wrapping_shl(shift) as i32;
            for v in buf.iter_mut() {
                *v = if *v < min_value {
                    min_shifted
                } else if *v > max_value {
                    max_shifted
                } else {
                    (*v as u32).wrapping_shl(shift) as i32
                };
            }
        } else if shift != 0 {
            for v in buf.iter_mut() {
                *v = (*v as u32).wrapping_shl(shift) as i32;
            }
        }
    }
}

/// Decodes the audio of one block (its header already parsed into `hdr`): `block_samples`
/// samples of one channel, or interleaved pairs for a stereo block (also for false stereo).
/// `Ok(None)` when the block fails a check; the caller outputs silence for it, as the reference does.
pub(crate) fn unpack(hdr: &Header, block: &[u8]) -> Option<Vec<i32>> {
    let flags = hdr.flags;
    if flags & f::MONO_DATA == f::MONO_DATA || flags & f::DSD_FLAG != 0 {
        return None;
    }
    let mut s = Stream { flags, ..Stream::default() };
    let subs = metadata(block).ok()?;
    for (id, d) in subs {
        if !s.metadata(hdr.version, id, d) {
            return None;
        }
    }
    if !s.wv.open {
        return None;
    }
    let n = hdr.block_samples as usize;
    let mono = s.mono();
    let mut buf = vec![0i32; if mono { n } else { n * 2 }];
    let hybrid = flags & f::HYBRID_FLAG != 0;
    let mag = (flags & f::MAG_MASK) >> f::MAG_LSB;
    let mut mute_limit = ((1i64 << mag) + 2) as i32;
    if hybrid {
        mute_limit = mute_limit.wrapping_mul(3).wrapping_add(128);
    }
    let mute_limit = i64::from(mute_limit);
    let mut wv = std::mem::take(&mut s.wv);
    let read = if hybrid {
        let mode = Mode { mono, hybrid, hybrid_bitrate: flags & f::HYBRID_BITRATE != 0, hybrid_balance: flags & f::HYBRID_BALANCE != 0 };
        let mut i = 0;
        for (j, v) in buf.iter_mut().enumerate() {
            let w = s.words.word(&mut wv, if mono { 0 } else { j & 1 }, &mode);
            if w == WORD_EOF {
                break;
            }
            *v = w;
            i += 1;
        }
        i
    } else {
        s.words.lossless(&mut wv, mono, &mut buf)
    };
    if read != buf.len() {
        return None;
    }
    let mut crc: u32 = 0xffff_ffff;
    let mut passes = std::mem::take(&mut s.passes);
    if mono {
        for p in passes.iter_mut() {
            decorr_mono_pass(p, &mut buf);
        }
        for v in &buf {
            if i64::from(*v).abs() > mute_limit {
                return None;
            }
            crc = crc.wrapping_mul(3).wrapping_add(*v as u32);
        }
    } else {
        for p in passes.iter_mut() {
            decorr_stereo_pass(p, &mut buf);
        }
        let joint = flags & f::JOINT_STEREO != 0;
        for b in buf.chunks_exact_mut(2) {
            if joint {
                b[1] = b[1].wrapping_sub(b[0] >> 1);
                b[0] = b[0].wrapping_add(b[1]);
            }
            crc = crc.wrapping_add(crc << 3).wrapping_add((b[0] as u32) << 1).wrapping_add(b[0] as u32).wrapping_add(b[1] as u32);
        }
        for b in buf.chunks_exact(2).step_by(8) {
            if i64::from(b[0]).abs() > mute_limit || i64::from(b[1]).abs() > mute_limit {
                return None;
            }
        }
    }
    let mut crc_x: u32 = 0xffff_ffff;
    s.fixup(&mut buf, &mut crc_x);
    if crc != hdr.crc || (s.wvx.open && crc_x != s.crc_wvx) {
        return None;
    }
    if flags & f::FALSE_STEREO != 0 {
        buf = buf.iter().flat_map(|v| [*v, *v]).collect();
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_match_the_reference_forms() {
        // the 16-bit form and the overflow-safe form agree where both are defined
        for (w, s) in [(1024, 1000), (-1024, 30000), (700, -32768), (1, 1), (-300, 12345)] {
            assert_eq!(apply_weight(w, s), ((i64::from(w) * i64::from(s) + 512) >> 10) as i32, "{w} {s}");
        }
        let big = 1 << 24;
        assert_eq!(apply_weight(1024, big), big);
        let mut w = 1020;
        update_weight_clip(&mut w, 8, 5, 7);
        assert_eq!(w, 1024);
        update_weight_clip(&mut w, 8, -5, 7);
        assert_eq!(w, 1016);
        let mut w = 0;
        update_weight(&mut w, 2, 3, -3);
        assert_eq!(w, -2);
    }

    #[test]
    fn hostile_metadata_is_rejected_not_panicking() {
        let mut block = vec![0u8; 32];
        // large sub-block claiming more bytes than exist; odd size with zero length
        block.extend([ID_WV_BITSTREAM | ID_LARGE, 0xff, 0xff, 0xff]);
        assert!(metadata(&block).is_err());
        let mut b2 = vec![0u8; 32];
        b2.extend([ID_WV_BITSTREAM | ID_ODD_SIZE, 0]);
        assert!(metadata(&b2).is_err());
        let mut s = Stream { flags: 0, ..Stream::default() };
        assert!(!s.metadata(0x407, ID_DECORR_TERMS, &[0u8; 17]));
        assert!(!s.metadata(0x407, ID_DECORR_TERMS, &[5]));
        assert!(s.metadata(0x407, ID_DECORR_TERMS, &[2 + 5, 18 + 5]));
        assert!(!s.metadata(0x407, ID_DECORR_WEIGHTS, &[1; 6]));
        assert!(!s.metadata(0x407, ID_DECORR_SAMPLES, &[1; 3]));
        assert!(!s.metadata(0x407, ID_ENTROPY_VARS, &[1; 6]));
        assert!(!s.metadata(0x407, ID_WVX_BITSTREAM, &[1; 4]));
        assert!(!s.metadata(0x407, 0x1f, &[]));
        assert!(s.metadata(0x407, 0x3f, &[]));
    }
}
