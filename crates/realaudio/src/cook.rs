// Ported from FFmpeg n7.1.1 libavcodec/cook.c (Copyright (c) 2003 Sascha Sommer, 2005 Benjamin
// Larsson), with av_lfg_get from libavutil/lfg.h and ff_sine_window_init from
// libavcodec/sinewin_tablegen.h, LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes
// GPL-3.0-or-later.

//! The Cook decoder (RealAudio G2): 256, 512 or 1024 samples per channel and frame, 20-coefficient
//! subbands with envelope coding, scalar vector quantisation, joint-stereo coupling, gain control
//! and a modulated lapped transform.

use crate::bits::{Bits, Huff};
use crate::mdct::Imdct;
use crate::tables::*;
use crate::{Error, Result};

const MONO: u32 = 0x100_0001;
const STEREO: u32 = 0x100_0002;
const JOINT_STEREO: u32 = 0x100_0003;
const MC_COOK: u32 = 0x200_0000;

const SUBBAND_SIZE: usize = 20;
const MAX_SUBPACKETS: usize = 5;
/// `decode_buffer_0`: 53 subbands of 20.
const BUF: usize = 1060;

/// The stream parameters a RealMedia audio header gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Params {
    pub channels: u16,
    pub sample_rate: u32,
    /// Bytes per Cook frame (the RealMedia sub-packet size).
    pub block_align: usize,
    /// The codec data (one 8 to 20 byte record per sub-packet).
    pub extradata: Vec<u8>,
}

/// One decoded frame: planar samples, `channels` × `samples_per_channel`.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub channels: Vec<Vec<f32>>,
}

#[derive(Clone)]
struct Gains {
    a: [i32; 9],
    b: [i32; 9],
    /// `now` is `b` (and `previous` is `a`) when set.
    swapped: bool,
}

impl Gains {
    fn new() -> Self {
        Self { a: [0; 9], b: [0; 9], swapped: false }
    }
    fn now_mut(&mut self) -> &mut [i32; 9] {
        if self.swapped { &mut self.b } else { &mut self.a }
    }
    fn now(&self) -> &[i32; 9] {
        if self.swapped { &self.b } else { &self.a }
    }
    fn previous(&self) -> &[i32; 9] {
        if self.swapped { &self.a } else { &self.b }
    }
    fn swap(&mut self) {
        self.swapped = !self.swapped;
    }
}

#[derive(Clone)]
struct Subpacket {
    ch_idx: usize,
    size: usize,
    num_channels: usize,
    subbands: usize,
    js_subband_start: usize,
    js_vlc_bits: u32,
    samples_per_channel: usize,
    log2_numvector_size: u32,
    joint_stereo: bool,
    bits_per_subpacket: usize,
    bits_per_subpdiv: u32,
    total_subbands: usize,
    numvector_size: usize,
    channel_coupling: Option<Huff>,
    prev1: Vec<f32>,
    prev2: Vec<f32>,
    gains1: Gains,
    gains2: Gains,
}

#[derive(Clone)]
struct Lfg {
    state: [u32; 64],
    index: u32,
}

impl Lfg {
    fn get(&mut self) -> u32 {
        let i = self.index;
        let a = self.state[(i.wrapping_sub(24) & 63) as usize].wrapping_add(self.state[(i.wrapping_sub(55) & 63) as usize]);
        self.state[(i & 63) as usize] = a;
        self.index = i.wrapping_add(1);
        a
    }
}

/// `pow2tab` and `rootpow2tab` (2^i and 2^(i/2) for -63 <= i < 64), built as FFmpeg does in
/// `f32`.
fn pow_tables() -> &'static ([f32; 127], [f32; 127]) {
    static T: std::sync::OnceLock<([f32; 127], [f32; 127])> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut pow2 = [0f32; 127];
        let mut root = [0f32; 127];
        let exp2 = [1.0f32, std::f64::consts::SQRT_2 as f32];
        let mut exp2_val = 2f32.powi(-63);
        let mut root_val = 2f32.powi(-32);
        for i in -63i32..64 {
            if i & 1 == 0 {
                root_val *= 2.0;
            }
            let k = (63 + i) as usize;
            pow2[k] = exp2_val;
            root[k] = root_val * exp2[(i & 1) as usize];
            exp2_val *= 2.0;
        }
        (pow2, root)
    })
}

fn pow2(i: i32) -> f32 {
    usize::try_from(i + 63).ok().and_then(|k| pow_tables().0.get(k)).copied().unwrap_or(0.0)
}

fn rootpow2(i: i32) -> f32 {
    usize::try_from(i + 63).ok().and_then(|k| pow_tables().1.get(k)).copied().unwrap_or(0.0)
}

/// The fixed code tables.
struct Codes {
    envelope: Vec<Huff>,
    sqvh: Vec<Huff>,
}

fn codes() -> &'static Codes {
    static C: std::sync::OnceLock<Codes> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let envelope = (0..13)
            .map(|i| {
                let counts = ENVELOPE_QUANT_INDEX_HUFFCOUNTS.get(i * 16..i * 16 + 16).unwrap_or(&[]);
                let syms = ENVELOPE_QUANT_INDEX_HUFFSYMS.get(i * 24..i * 24 + 24).unwrap_or(&[]);
                Huff::new(counts, syms.iter().map(|&s| i32::from(s) - 12))
            })
            .collect();
        let syms: [&[u8]; 7] = [&CVH_HUFFSYMS0, &CVH_HUFFSYMS1, &CVH_HUFFSYMS2, &[], &CVH_HUFFSYMS4, &CVH_HUFFSYMS5, &CVH_HUFFSYMS6];
        let sqvh = (0..7)
            .map(|i| {
                let counts = CVH_HUFFCOUNTS.get(i * 16..i * 16 + 16).unwrap_or(&[]);
                if i == 3 {
                    Huff::new(counts, CVH_HUFFSYMS3.iter().map(|&s| i32::from(s)))
                } else {
                    Huff::new(counts, syms.get(i).copied().unwrap_or(&[]).iter().map(|&s| i32::from(s)))
                }
            })
            .collect();
        Codes { envelope, sqvh }
    })
}

fn ccpl_code(js_vlc_bits: u32) -> Option<Huff> {
    let i = js_vlc_bits.checked_sub(2)? as usize;
    let syms: [&[u8]; 5] = [&CCPL_HUFFSYMS2, &CCPL_HUFFSYMS3, &CCPL_HUFFSYMS4, &CCPL_HUFFSYMS5, &CCPL_HUFFSYMS6];
    let counts = CCPL_HUFFCOUNTS.get(i * 16..i * 16 + 16)?;
    Some(Huff::new(counts, syms.get(i)?.iter().map(|&s| i32::from(s))))
}

fn cplscale(js_vlc_bits: u32, i: i32) -> f32 {
    let t: &[f32] = match js_vlc_bits {
        2 => &CPLSCALE2,
        3 => &CPLSCALE3,
        4 => &CPLSCALE4,
        5 => &CPLSCALE5,
        6 => &CPLSCALE6,
        _ => &[],
    };
    usize::try_from(i).ok().and_then(|i| t.get(i)).copied().unwrap_or(0.0)
}

fn tab(t: &[i32], i: usize) -> i32 {
    t.get(i).copied().unwrap_or(0)
}

/// A Cook decoder. Cloning it is a checkpoint: the clone decodes on exactly as the original.
#[derive(Clone)]
pub struct Decoder {
    channels: usize,
    block_align: usize,
    samples_per_channel: usize,
    gain_size_factor: usize,
    gain_table: [f32; 31],
    window: Vec<f32>,
    imdct: Imdct,
    random: Lfg,
    discarded: u32,
    subpackets: Vec<Subpacket>,
    num_vectors: usize,
    decode_buffer_0: Vec<f32>,
    decode_buffer_1: Vec<f32>,
    decode_buffer_2: Vec<f32>,
    mdct_out: Vec<f32>,
}

impl Decoder {
    /// `cook_decode_init`.
    pub fn new(p: &Params) -> Result<Self> {
        let channels = usize::from(p.channels);
        let ex = &p.extradata;
        if ex.len() < 8 {
            return Err(Error::Invalid("Cook extradata missing"));
        }
        if channels == 0 {
            return Err(Error::Invalid("no channels"));
        }
        if p.block_align >= (i32::MAX / 8) as usize || p.block_align == 0 {
            return Err(Error::Invalid("frame size"));
        }
        // bytestream2: reads past the end give zeros and stop at the end
        let mut rd = ByteReader { data: ex, pos: 0 };
        let mut be = |n: usize| rd.be(n);
        let mut subpackets: Vec<Subpacket> = Vec::new();
        let mut spc_all = 0usize;
        let mut channel_mask = 0u32;
        loop {
            if subpackets.len() >= MAX_SUBPACKETS.min(p.block_align) {
                return Err(Error::Unsupported("too many Cook sub-packets"));
            }
            let cookversion = be(4);
            let samples_per_frame = be(2) as usize;
            let subbands = be(2) as usize;
            be(4);
            let js_subband_start = be(2) as usize;
            if js_subband_start >= 51 {
                return Err(Error::Invalid("js_subband_start"));
            }
            let js_vlc_bits = be(2);
            let mut s = Subpacket {
                ch_idx: 0,
                size: 0,
                num_channels: 1,
                subbands,
                js_subband_start,
                js_vlc_bits,
                samples_per_channel: samples_per_frame / channels,
                log2_numvector_size: 5,
                joint_stereo: false,
                bits_per_subpacket: p.block_align * 8,
                bits_per_subpdiv: 0,
                total_subbands: subbands,
                numvector_size: 0,
                channel_coupling: None,
                prev1: vec![0.0; 1024],
                prev2: vec![0.0; 1024],
                gains1: Gains::new(),
                gains2: Gains::new(),
            };
            match cookversion {
                MONO => {
                    if channels != 1 {
                        return Err(Error::Unsupported("Cook mono with more than one channel"));
                    }
                }
                STEREO => {
                    if channels != 1 {
                        s.bits_per_subpdiv = 1;
                        s.num_channels = 2;
                    }
                }
                JOINT_STEREO => {
                    if channels != 2 {
                        return Err(Error::Unsupported("Cook joint stereo without two channels"));
                    }
                    if ex.len() >= 16 {
                        s.total_subbands = s.subbands + s.js_subband_start;
                        s.joint_stereo = true;
                        s.num_channels = 2;
                    }
                    if s.samples_per_channel > 256 {
                        s.log2_numvector_size = 6;
                    }
                    if s.samples_per_channel > 512 {
                        s.log2_numvector_size = 7;
                    }
                }
                MC_COOK => {
                    let mask = be(4);
                    channel_mask |= mask;
                    if mask.count_ones() > 1 {
                        s.total_subbands = s.subbands + s.js_subband_start;
                        s.joint_stereo = true;
                        s.num_channels = 2;
                        s.samples_per_channel = samples_per_frame >> 1;
                        if s.samples_per_channel > 256 {
                            s.log2_numvector_size = 6;
                        }
                        if s.samples_per_channel > 512 {
                            s.log2_numvector_size = 7;
                        }
                    } else {
                        s.samples_per_channel = samples_per_frame;
                    }
                }
                _ => return Err(Error::Unsupported("Cook version")),
            }
            if subpackets.len() > 1 && s.samples_per_channel != spc_all {
                return Err(Error::Invalid("different number of samples per channel"));
            }
            if let Some(first) = subpackets.first() {
                spc_all = first.samples_per_channel;
            } else {
                spc_all = s.samples_per_channel;
            }
            s.numvector_size = 1 << s.log2_numvector_size;
            if s.total_subbands > 53 {
                return Err(Error::Unsupported("more than 53 subbands"));
            }
            if s.js_vlc_bits > 6 || s.js_vlc_bits < 2 * u32::from(s.joint_stereo) {
                return Err(Error::Invalid("js_vlc_bits"));
            }
            if s.subbands > 50 {
                return Err(Error::Unsupported("more than 50 subbands"));
            }
            if s.subbands == 0 {
                return Err(Error::Unsupported("no subbands"));
            }
            if subpackets.len() + s.num_channels > channels {
                return Err(Error::Invalid("too many sub-packets for the channels"));
            }
            if s.joint_stereo {
                s.channel_coupling = ccpl_code(s.js_vlc_bits);
            }
            subpackets.push(s);
            if be(0) == u32::MAX {
                break;
            }
        }
        let spc = spc_all;
        if spc != 256 && spc != 512 && spc != 1024 {
            return Err(Error::Unsupported("samples per channel"));
        }
        let _ = channel_mask;
        let gain_size_factor = spc / 8;
        let mut gain_table = [0f32; 31];
        for (i, g) in gain_table.iter_mut().enumerate() {
            *g = f64::from(pow2(i as i32 + 48 - 63)).powf(1.0 / gain_size_factor as f64) as f32;
        }
        let scale = (2.0 / spc as f64).sqrt();
        let window = (0..spc)
            .map(|i| {
                let w = (((i as f64 + 0.5) * (std::f64::consts::PI / (2.0 * spc as f64))) as f32).sin();
                (f64::from(w) * scale) as f32
            })
            .collect();
        Ok(Self {
            channels,
            block_align: p.block_align,
            samples_per_channel: spc,
            gain_size_factor,
            gain_table,
            window,
            imdct: Imdct::new(spc, -1.0 / 32768.0),
            random: Lfg { state: LFG_SEED0, index: 0 },
            discarded: 0,
            subpackets,
            num_vectors: 0,
            decode_buffer_0: vec![0.0; BUF],
            decode_buffer_1: vec![0.0; BUF],
            decode_buffer_2: vec![0.0; BUF],
            mdct_out: vec![0.0; 2048],
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn samples_per_channel(&self) -> usize {
        self.samples_per_channel
    }

    /// `cook_decode_frame`: one `block_align`-byte frame. `Ok(None)` for the first two frames
    /// (no audio yet) and for a short packet.
    pub fn decode(&mut self, buf: &[u8]) -> Result<Option<Frame>> {
        let ba = self.block_align;
        if buf.len() < ba {
            return Ok(None);
        }
        let n = self.subpackets.len();
        let mut size0 = ba as isize;
        for i in 1..n {
            let sz = 2 * usize::from(buf.get(ba - n + i).copied().unwrap_or(0));
            if let Some(s) = self.subpackets.get_mut(i) {
                s.size = sz;
            }
            size0 -= sz as isize + 1;
            if size0 < 0 {
                return Err(Error::Invalid("Cook sub-packet sizes"));
            }
        }
        if let Some(s) = self.subpackets.first_mut() {
            s.size = size0 as usize;
        }
        let output = self.discarded >= 2;
        let mut out: Vec<Vec<f32>> = if output { vec![vec![0.0; self.samples_per_channel]; self.channels] } else { Vec::new() };
        let mut offset = 0usize;
        let mut chidx = 0usize;
        for i in 0..n {
            let Some(mut p) = self.subpackets.get_mut(i).map(|s| std::mem::replace(s, placeholder())) else { break };
            p.bits_per_subpacket = (p.size * 8) >> p.bits_per_subpdiv;
            p.ch_idx = chidx;
            let r = self.decode_subpacket(&mut p, buf, offset, if output { Some(&mut out) } else { None });
            offset += p.size;
            chidx += p.num_channels;
            if let Some(s) = self.subpackets.get_mut(i) {
                *s = p;
            }
            r?;
        }
        if self.discarded < 2 {
            self.discarded += 1;
            return Ok(None);
        }
        Ok(Some(Frame { channels: out }))
    }

    /// `decode_bytes_and_gain`: the XOR descrambling (aligned like FFmpeg's packet buffers) and
    /// the gain info. Returns the descrambled bytes.
    fn descramble(buf: &[u8], start: usize, bits: usize) -> Vec<u8> {
        const KEY: [u8; 4] = [0x37, 0xC5, 0x11, 0xF2];
        let bytes = bits / 8;
        let off = start & 3;
        // FFmpeg XORs whole aligned words: (bytes + 3 + off) / 4 of them from start - off.
        let words = (bytes + 3 + off) / 4;
        let have = (words * 4).saturating_sub(off);
        let mut v = vec![0u8; have + 8];
        for (j, d) in v.iter_mut().take(have).enumerate() {
            *d = buf.get(start + j).copied().unwrap_or(0) ^ KEY[j & 3];
        }
        v
    }

    fn decode_subpacket(&mut self, p: &mut Subpacket, buf: &[u8], offset: usize, mut out: Option<&mut Vec<Vec<f32>>>) -> Result<()> {
        self.decode_buffer_1.fill(0.0);
        let data1 = Self::descramble(buf, offset, p.bits_per_subpacket);
        let mut gb = Bits::new(&data1, p.bits_per_subpacket);
        decode_gain_info(&mut gb, p.gains1.now_mut());
        p.gains1.swap();
        if p.joint_stereo {
            self.joint_decode(p, &mut gb)?;
        } else {
            let mut b1 = std::mem::take(&mut self.decode_buffer_1);
            let r = self.mono_decode(p, &mut gb, &mut b1);
            self.decode_buffer_1 = b1;
            r?;
            if p.num_channels == 2 {
                let data2 = Self::descramble(buf, offset + p.size / 2, p.bits_per_subpacket);
                let mut gb2 = Bits::new(&data2, p.bits_per_subpacket);
                decode_gain_info(&mut gb2, p.gains2.now_mut());
                p.gains2.swap();
                let mut b2 = std::mem::take(&mut self.decode_buffer_2);
                let r = self.mono_decode(p, &mut gb2, &mut b2);
                self.decode_buffer_2 = b2;
                r?;
            }
        }
        let b1 = std::mem::take(&mut self.decode_buffer_1);
        let mut prev = std::mem::take(&mut p.prev1);
        let o = out.as_deref_mut().and_then(|o| o.get_mut(p.ch_idx));
        self.mlt_compensate_output(&b1, &p.gains1, &mut prev, o);
        p.prev1 = prev;
        self.decode_buffer_1 = b1;
        if p.num_channels == 2 {
            let b2 = std::mem::take(&mut self.decode_buffer_2);
            let mut prev = std::mem::take(&mut p.prev2);
            let o = out.and_then(|o| o.get_mut(p.ch_idx + 1));
            let gains = if p.joint_stereo { &p.gains1 } else { &p.gains2 };
            self.mlt_compensate_output(&b2, gains, &mut prev, o);
            p.prev2 = prev;
            self.decode_buffer_2 = b2;
        }
        Ok(())
    }

    /// `decode_envelope`.
    fn decode_envelope(&self, p: &Subpacket, gb: &mut Bits<'_>, q: &mut [i32; 102]) -> Result<()> {
        q[0] = gb.get(6) as i32 - 6;
        let codes = codes();
        for i in 1..p.total_subbands {
            let mut vlc_index = i;
            if i >= p.js_subband_start * 2 {
                vlc_index -= p.js_subband_start;
            } else {
                vlc_index /= 2;
                if vlc_index < 1 {
                    vlc_index = 1;
                }
            }
            vlc_index = vlc_index.min(13);
            let j = codes.envelope.get(vlc_index - 1).and_then(|h| h.read(gb)).unwrap_or(-1);
            let v = q.get(i - 1).copied().unwrap_or(0) + j;
            if let Some(d) = q.get_mut(i) {
                *d = v;
            }
            if !(-63..=63).contains(&v) {
                return Err(Error::Invalid("Cook quantiser out of range"));
            }
        }
        Ok(())
    }

    /// `categorize`.
    fn categorize(&self, p: &Subpacket, gb: &Bits<'_>, q: &[i32; 102], category: &mut [i32; 128], category_index: &mut [i32; 128]) {
        let mut exp_index2 = [0i32; 102];
        let mut exp_index1 = [0i32; 102];
        let mut tmp = [0i32; 256];
        let mut idx1 = p.numvector_size;
        let mut idx2 = p.numvector_size;
        let spc = self.samples_per_channel as i32;
        let mut bits_left = p.bits_per_subpacket as i32 - gb.count() as i32;
        if bits_left > spc {
            bits_left = spc + ((bits_left - spc) * 5) / 8;
        }
        let eb = |i: i32| tab(&EXPBITS_TAB, usize::try_from(i).unwrap_or(usize::MAX));
        let mut bias = -32i32;
        let mut i = 32i32;
        while i > 0 {
            let mut num_bits = 0;
            for j in 0..p.total_subbands {
                let e = ((i - q.get(j).copied().unwrap_or(0) + bias) / 2).clamp(0, 7);
                num_bits += eb(e);
            }
            if num_bits >= bits_left - 32 {
                bias += i;
            }
            i /= 2;
        }
        let mut num_bits = 0;
        for i in 0..p.total_subbands {
            let e = ((bias - q.get(i).copied().unwrap_or(0)) / 2).clamp(0, 7);
            num_bits += eb(e);
            exp_index1[i] = e;
            exp_index2[i] = e;
        }
        let mut tmpbias1 = num_bits;
        let mut tmpbias2 = num_bits;
        for _ in 1..p.numvector_size {
            if tmpbias1 + tmpbias2 > 2 * bits_left {
                let mut max = -999_999;
                let mut index = None;
                for i in 0..p.total_subbands {
                    if exp_index1[i] < 7 {
                        let v = -2 * exp_index1[i] - q.get(i).copied().unwrap_or(0) + bias;
                        if v >= max {
                            max = v;
                            index = Some(i);
                        }
                    }
                }
                let Some(index) = index else { break };
                if let Some(t) = tmp.get_mut(idx1) {
                    *t = index as i32;
                }
                idx1 += 1;
                tmpbias1 -= eb(exp_index1[index]) - eb(exp_index1[index] + 1);
                exp_index1[index] += 1;
            } else {
                let mut min = 999_999;
                let mut index = None;
                for i in 0..p.total_subbands {
                    if exp_index2[i] > 0 {
                        let v = -2 * exp_index2[i] - q.get(i).copied().unwrap_or(0) + bias;
                        if v < min {
                            min = v;
                            index = Some(i);
                        }
                    }
                }
                let Some(index) = index else { break };
                idx2 = idx2.saturating_sub(1);
                if let Some(t) = tmp.get_mut(idx2) {
                    *t = index as i32;
                }
                tmpbias2 -= eb(exp_index2[index]) - eb(exp_index2[index] - 1);
                exp_index2[index] -= 1;
            }
        }
        let t = p.total_subbands.min(102);
        category[..t].copy_from_slice(&exp_index2[..t]);
        for i in 0..p.numvector_size.saturating_sub(1) {
            if let Some(c) = category_index.get_mut(i) {
                *c = tmp.get(idx2).copied().unwrap_or(0);
            }
            idx2 += 1;
        }
    }

    /// `unpack_SQVH`: true when the sub-packet ran out of bits.
    fn unpack_sqvh(p: &Subpacket, gb: &mut Bits<'_>, category: usize, index: &mut [i32; SUBBAND_SIZE], sign: &mut [bool; SUBBAND_SIZE]) -> bool {
        let vd = tab(&VD_TAB, category) as usize;
        let mut result = false;
        let Some(code) = codes().sqvh.get(category) else { return true };
        for i in 0..tab(&VPR_TAB, category) as usize {
            let mut vlc = code.read(gb).unwrap_or(-1);
            if p.bits_per_subpacket < gb.count() {
                vlc = 0;
                result = true;
            }
            for j in (0..vd).rev() {
                let t = (vlc.wrapping_mul(tab(&INVRADIX_TAB, category))) / 0x10_0000;
                if let Some(d) = index.get_mut(vd * i + j) {
                    *d = vlc - t * (tab(&KMAX_TAB, category) + 1);
                }
                vlc = t;
            }
            for j in 0..vd {
                let k = i * vd + j;
                let nz = index.get(k).copied().unwrap_or(0) != 0;
                let s = if nz {
                    if gb.count() < p.bits_per_subpacket {
                        gb.get1()
                    } else {
                        result = true;
                        false
                    }
                } else {
                    false
                };
                if let Some(d) = sign.get_mut(k) {
                    *d = s;
                }
            }
        }
        result
    }

    /// `decode_vectors` with `scalar_dequant_float`.
    fn decode_vectors(&mut self, p: &Subpacket, gb: &mut Bits<'_>, category: &mut [i32; 128], q: &[i32; 102], mlt: &mut [f32]) {
        let mut index_t = [0i32; SUBBAND_SIZE];
        let mut sign = [false; SUBBAND_SIZE];
        for band in 0..p.total_subbands {
            let mut index = category[band];
            if category[band] < 7 && Self::unpack_sqvh(p, gb, category[band].max(0) as usize, &mut index_t, &mut sign) {
                index = 7;
                for j in 0..p.total_subbands {
                    if let Some(c) = category.get_mut(band + j) {
                        *c = 7;
                    }
                }
            }
            if index >= 7 {
                index_t = [0; SUBBAND_SIZE];
                sign = [false; SUBBAND_SIZE];
            }
            let idx = usize::try_from(index).unwrap_or(0);
            let scale = rootpow2(q.get(band).copied().unwrap_or(0));
            for i in 0..SUBBAND_SIZE {
                let mut f1;
                if index_t[i] != 0 {
                    f1 = QUANT_CENTROID_TAB.get(idx * 14 + index_t[i].max(0) as usize).copied().unwrap_or(0.0);
                    if sign[i] {
                        f1 = -f1;
                    }
                } else {
                    f1 = DITHER_TAB.get(idx).copied().unwrap_or(0.0);
                    if self.random.get() < 0x8000_0000 {
                        f1 = -f1;
                    }
                }
                if let Some(m) = mlt.get_mut(band * SUBBAND_SIZE + i) {
                    *m = f1 * scale;
                }
            }
        }
    }

    /// `mono_decode`.
    fn mono_decode(&mut self, p: &Subpacket, gb: &mut Bits<'_>, mlt: &mut [f32]) -> Result<()> {
        let mut category_index = [0i32; 128];
        let mut category = [0i32; 128];
        let mut q = [0i32; 102];
        self.decode_envelope(p, gb, &mut q)?;
        self.num_vectors = gb.get(p.log2_numvector_size) as usize;
        self.categorize(p, gb, &q, &mut category, &mut category_index);
        // expand_category
        for i in 0..self.num_vectors {
            let idx = category_index.get(i).copied().unwrap_or(0).max(0) as usize;
            if let Some(c) = category.get_mut(idx) {
                *c += 1;
                if *c >= DITHER_TAB.len() as i32 {
                    *c -= 1;
                }
            }
        }
        if category.iter().take(p.total_subbands).any(|&c| c > 7) {
            return Err(Error::Invalid("Cook category"));
        }
        self.decode_vectors(p, gb, &mut category, &q, mlt);
        Ok(())
    }

    /// `joint_decode`.
    fn joint_decode(&mut self, p: &Subpacket, gb: &mut Bits<'_>) -> Result<()> {
        let mut decouple = [0i32; SUBBAND_SIZE];
        self.decode_buffer_0.fill(0.0);
        self.decode_buffer_1.fill(0.0);
        self.decode_buffer_2.fill(0.0);
        // decouple_info
        let vlc = gb.get1();
        let start = tab(&CPLBAND, p.js_subband_start);
        let end = tab(&CPLBAND, p.subbands.wrapping_sub(1));
        if start <= end {
            for i in 0..(end - start + 1) as usize {
                let v = if vlc {
                    p.channel_coupling.as_ref().and_then(|h| h.read(gb)).unwrap_or(-1)
                } else {
                    let v = gb.get(p.js_vlc_bits) as i32;
                    if v == (1 << p.js_vlc_bits) - 1 {
                        return Err(Error::Invalid("Cook decouple value"));
                    }
                    v
                };
                if let Some(d) = decouple.get_mut(start as usize + i) {
                    *d = v;
                }
            }
        }
        let mut b0 = std::mem::take(&mut self.decode_buffer_0);
        let r = self.mono_decode(p, gb, &mut b0);
        self.decode_buffer_0 = b0;
        r?;
        let d = &self.decode_buffer_0;
        for i in 0..p.js_subband_start {
            for j in 0..SUBBAND_SIZE {
                if let Some(v) = self.decode_buffer_1.get_mut(i * 20 + j) {
                    *v = d.get(i * 40 + j).copied().unwrap_or(0.0);
                }
                if let Some(v) = self.decode_buffer_2.get_mut(i * 20 + j) {
                    *v = d.get(i * 40 + 20 + j).copied().unwrap_or(0.0);
                }
            }
        }
        let full = (1i32 << p.js_vlc_bits) - 1;
        for i in p.js_subband_start..p.subbands {
            let c = decouple.get(tab(&CPLBAND, i).max(0) as usize).copied().unwrap_or(0);
            let idx = full - c;
            let f1 = cplscale(p.js_vlc_bits, c + 1);
            let f2 = cplscale(p.js_vlc_bits, idx);
            for j in 0..SUBBAND_SIZE {
                let t = (p.js_subband_start + i) * SUBBAND_SIZE + j;
                let s = d.get(t).copied().unwrap_or(0.0);
                if let Some(v) = self.decode_buffer_1.get_mut(SUBBAND_SIZE * i + j) {
                    *v = f1 * s;
                }
                if let Some(v) = self.decode_buffer_2.get_mut(SUBBAND_SIZE * i + j) {
                    *v = f2 * s;
                }
            }
        }
        Ok(())
    }

    /// `imlt_gain` and `saturate_output_float`.
    fn mlt_compensate_output(&mut self, coefs: &[f32], gains: &Gains, previous: &mut [f32], out: Option<&mut Vec<f32>>) {
        let n = self.samples_per_channel;
        let mut mo = std::mem::take(&mut self.mdct_out);
        self.imdct.inverse(coefs, &mut mo);
        // imlt_window_float on the second half
        let fc = pow2(gains.previous().first().copied().unwrap_or(0));
        for i in 0..n {
            let w = self.window.get(i).copied().unwrap_or(0.0);
            let w2 = self.window.get(n - 1 - i).copied().unwrap_or(0.0);
            let p = previous.get(i).copied().unwrap_or(0.0);
            if let Some(v) = mo.get_mut(n + i) {
                *v = *v * fc * w - p * w2;
            }
        }
        // gain profile
        let now = gains.now();
        let g = self.gain_size_factor;
        for i in 0..8 {
            let (a, b) = (now[i], now[i + 1]);
            if a != 0 || b != 0 {
                let mut fc1 = pow2(a);
                let seg = mo.get_mut(n + g * i..n + g * (i + 1)).unwrap_or(&mut []);
                if a == b {
                    for v in seg {
                        *v *= fc1;
                    }
                } else {
                    let fc2 = usize::try_from(15 + (b - a)).ok().and_then(|k| self.gain_table.get(k)).copied().unwrap_or(0.0);
                    for v in seg {
                        *v *= fc1;
                        fc1 *= fc2;
                    }
                }
            }
        }
        for (d, s) in previous.iter_mut().zip(mo.iter()).take(n) {
            *d = *s;
        }
        if let Some(o) = out {
            for (d, s) in o.iter_mut().zip(mo.iter().skip(n)) {
                *d = s.clamp(-1.0, 1.0);
            }
        }
        self.mdct_out = mo;
    }
}

struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl ByteReader<'_> {
    /// Big-endian `n` bytes; `n == 0` answers whether the data is used up (`u32::MAX`).
    fn be(&mut self, n: usize) -> u32 {
        if n == 0 {
            return if self.pos >= self.data.len() { u32::MAX } else { 0 };
        }
        if self.pos + n > self.data.len() {
            self.pos = self.data.len();
            return 0;
        }
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 8) | u32::from(self.data.get(self.pos).copied().unwrap_or(0));
            self.pos += 1;
        }
        v
    }
}

fn placeholder() -> Subpacket {
    Subpacket {
        ch_idx: 0,
        size: 0,
        num_channels: 0,
        subbands: 0,
        js_subband_start: 0,
        js_vlc_bits: 0,
        samples_per_channel: 0,
        log2_numvector_size: 0,
        joint_stereo: false,
        bits_per_subpacket: 0,
        bits_per_subpdiv: 0,
        total_subbands: 0,
        numvector_size: 0,
        channel_coupling: None,
        prev1: Vec::new(),
        prev2: Vec::new(),
        gains1: Gains::new(),
        gains2: Gains::new(),
    }
}

/// `decode_gain_info`.
fn decode_gain_info(gb: &mut Bits<'_>, gaininfo: &mut [i32; 9]) {
    // get_unary(gb, 0, get_bits_left(gb))
    let left = gb.left().max(0) as usize;
    let mut n = 0usize;
    while n < left && gb.get1() {
        n += 1;
    }
    let mut i = 0usize;
    for _ in 0..n {
        let index = gb.get(3) as usize;
        let gain = if gb.get1() { gb.get(4) as i32 - 7 } else { -1 };
        while i <= index {
            if let Some(g) = gaininfo.get_mut(i) {
                *g = gain;
            }
            i += 1;
        }
    }
    while i <= 8 {
        gaininfo[i] = 0;
        i += 1;
    }
}
