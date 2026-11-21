//! ATRAC3 (Sony's Adaptive TRansform Acoustic Coding 3; RealMedia `atrc`, WAV format 0x0270):
//! a four-band QMF split, a 256-line MDCT per band with gain control, tonal components and joint
//! stereo. A port of FFmpeg n7.1.1 `libavcodec/atrac3.c` and `atrac.c` (Copyright (c) 2006-2008
//! Maxim Poliakovski, Benjamin Larsson; LGPL-2.1-or-later). Output: planar `f32` in `[-1, 1]`,
//! 1024 samples per channel and frame.

use std::f64::consts::PI;
use std::sync::Arc;

use crate::bits::{Bits, Huff};
use crate::mdct::Imdct;
use crate::{Error, Result};

const SAMPLES_PER_FRAME: usize = 1024;
const MAX_CHANNELS: usize = 8;
const JOINT_STEREO: u16 = 0x12;
const SINGLE: u16 = 0x2;

/// `atrac3_hufftabs` (symbol + 31, length), the seven spectral-coefficient tables in turn.
const HUFFTABS: [(u8, u8); 139] = [
    (31, 1),
    (32, 3),
    (33, 3),
    (34, 4),
    (35, 4),
    (36, 5),
    (37, 5),
    (38, 5),
    (39, 5),
    (31, 1),
    (32, 3),
    (30, 3),
    (33, 3),
    (29, 3),
    (31, 1),
    (32, 3),
    (30, 3),
    (33, 4),
    (29, 4),
    (34, 4),
    (28, 4),
    (31, 1),
    (32, 3),
    (30, 3),
    (33, 4),
    (29, 4),
    (34, 5),
    (28, 5),
    (35, 5),
    (27, 5),
    (31, 2),
    (32, 3),
    (30, 3),
    (33, 4),
    (29, 4),
    (34, 4),
    (28, 4),
    (38, 4),
    (24, 4),
    (35, 5),
    (27, 5),
    (36, 6),
    (26, 6),
    (37, 6),
    (25, 6),
    (31, 3),
    (32, 4),
    (30, 4),
    (33, 4),
    (29, 4),
    (34, 4),
    (28, 4),
    (46, 4),
    (16, 4),
    (35, 5),
    (27, 5),
    (36, 5),
    (26, 5),
    (37, 5),
    (25, 5),
    (38, 6),
    (24, 6),
    (39, 6),
    (23, 6),
    (40, 6),
    (22, 6),
    (41, 6),
    (21, 6),
    (42, 7),
    (20, 7),
    (43, 7),
    (19, 7),
    (44, 7),
    (18, 7),
    (45, 7),
    (17, 7),
    (31, 3),
    (62, 4),
    (0, 4),
    (32, 5),
    (30, 5),
    (33, 5),
    (29, 5),
    (34, 5),
    (28, 5),
    (35, 5),
    (27, 5),
    (36, 5),
    (26, 5),
    (37, 6),
    (25, 6),
    (38, 6),
    (24, 6),
    (39, 6),
    (23, 6),
    (40, 6),
    (22, 6),
    (41, 6),
    (21, 6),
    (42, 6),
    (20, 6),
    (43, 6),
    (19, 6),
    (44, 6),
    (18, 6),
    (45, 7),
    (17, 7),
    (46, 7),
    (16, 7),
    (47, 7),
    (15, 7),
    (48, 7),
    (14, 7),
    (49, 7),
    (13, 7),
    (50, 7),
    (12, 7),
    (51, 7),
    (11, 7),
    (52, 8),
    (10, 8),
    (53, 8),
    (9, 8),
    (54, 8),
    (8, 8),
    (55, 8),
    (7, 8),
    (56, 8),
    (6, 8),
    (57, 8),
    (5, 8),
    (58, 8),
    (4, 8),
    (59, 8),
    (3, 8),
    (60, 8),
    (2, 8),
    (61, 8),
    (1, 8),
];
const HUFF_TAB_SIZES: [usize; 7] = [9, 5, 7, 9, 15, 31, 63];
const CLC_LENGTH_TAB: [u32; 8] = [0, 4, 3, 3, 4, 4, 5, 6];
const MANTISSA_CLC_TAB: [i32; 4] = [0, 1, -2, -1];
const MANTISSA_VLC_TAB: [i32; 18] = [0, 0, 0, 1, 0, -1, 1, 0, -1, 0, 1, 1, 1, -1, -1, 1, -1, -1];
const SUBBAND_TAB: [usize; 33] =
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 288, 320, 352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024];
const MATRIX_COEFFS: [f32; 8] = [0.0, 2.0, 2.0, 2.0, 0.0, 0.0, 1.0, 1.0];
/// `qmf_48tap_half` (atrac.c).
#[allow(clippy::excessive_precision, clippy::unreadable_literal)]
const QMF_48TAP_HALF: [f64; 24] = [
    -0.00001461907,
    -0.00009205479,
    -0.000056157569,
    0.00030117269,
    0.0002422519,
    -0.00085293897,
    -0.0005205574,
    0.0020340169,
    0.00078333891,
    -0.0042153862,
    -0.00075614988,
    0.0078402944,
    -0.000061169922,
    -0.01344162,
    0.0024626821,
    0.021736089,
    -0.007801671,
    -0.034090221,
    0.01880949,
    0.054326009,
    -0.043596379,
    -0.099384367,
    0.13207909,
    0.46424159,
];

fn inv_max_quant(i: usize) -> f32 {
    const D: [f64; 8] = [0.0, 1.5, 2.5, 3.5, 4.5, 7.5, 15.5, 31.5];
    match D.get(i) {
        Some(&d) if d != 0.0 => (1.0 / d) as f32,
        _ => 0.0,
    }
}

/// Decoder parameters from the container.
#[derive(Clone, Debug)]
pub struct Params {
    pub channels: u16,
    /// Bytes per frame (all channels).
    pub block_align: usize,
    /// 14 bytes from WAV (`fmt ` extra data), or 10 / 12 from RealMedia.
    pub extradata: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
struct GainInfo {
    num_points: usize,
    lev: [usize; 7],
    loc: [usize; 7],
}

#[derive(Clone, Copy)]
struct Tonal {
    pos: usize,
    num_coefs: usize,
    coef: [f32; 8],
}

#[derive(Clone)]
struct Unit {
    prev_frame: Vec<f32>,
    gc_blk_switch: usize,
    gain_block: [[GainInfo; 4]; 2],
    spectrum: Vec<f32>,
    imdct_buf: Vec<f32>,
    delay1: [f32; 46],
    delay2: [f32; 46],
    delay3: [f32; 46],
}

impl Unit {
    fn new() -> Self {
        Self {
            prev_frame: vec![0.0; SAMPLES_PER_FRAME],
            gc_blk_switch: 0,
            gain_block: [[GainInfo::default(); 4]; 2],
            spectrum: vec![0.0; SAMPLES_PER_FRAME],
            imdct_buf: vec![0.0; SAMPLES_PER_FRAME],
            delay1: [0.0; 46],
            delay2: [0.0; 46],
            delay3: [0.0; 46],
        }
    }
}

/// Tables shared by every stream.
#[derive(Clone)]
struct Tables {
    huff: Vec<Huff>,
    window: [f32; 512],
    sf: [f32; 64],
    qmf: [f32; 48],
    gain1: [f32; 16],
    gain2: [f32; 31],
}

impl Tables {
    fn new() -> Self {
        let mut huff = Vec::with_capacity(7);
        let mut at = 0;
        for &n in &HUFF_TAB_SIZES {
            let t = &HUFFTABS[at..at + n];
            let mut counts = [0u8; 16];
            for &(_, l) in t {
                counts[usize::from(l) - 1] += 1;
            }
            huff.push(Huff::new(&counts, t.iter().map(|&(s, _)| i32::from(s) - 31)));
            at += n;
        }
        let mut window = [0f32; 512];
        let mut j = 255;
        for i in 0..128 {
            let wi = (((i as f64 + 0.5) / 256.0 - 0.5) * PI).sin() + 1.0;
            let wj = (((j as f64 + 0.5) / 256.0 - 0.5) * PI).sin() + 1.0;
            let (wi, wj) = (wi as f32, wj as f32);
            let w = (0.5 * f64::from(wi * wi + wj * wj)) as f32;
            window[i] = wi / w;
            window[511 - i] = wi / w;
            window[j] = wj / w;
            window[511 - j] = wj / w;
            j -= 1;
        }
        let mut sf = [0f32; 64];
        for (i, v) in sf.iter_mut().enumerate() {
            *v = 2f64.powf((i as f64 - 15.0) / 3.0) as f32;
        }
        let mut qmf = [0f32; 48];
        for (i, &h) in QMF_48TAP_HALF.iter().enumerate() {
            let s = (f64::from(h as f32) * 2.0) as f32;
            qmf[i] = s;
            qmf[47 - i] = s;
        }
        // ff_atrac_init_gain_compensation(4, 3)
        let mut gain1 = [0f32; 16];
        for (i, v) in gain1.iter_mut().enumerate() {
            *v = 2f32.powf((4 - i as i32) as f32);
        }
        let mut gain2 = [0f32; 31];
        for (k, v) in gain2.iter_mut().enumerate() {
            let i = k as i32 - 15;
            *v = 2f32.powf(-1.0f32 / 8.0 * i as f32);
        }
        Self { huff, window, sf, qmf, gain1, gain2 }
    }
}

/// An ATRAC3 decoder.
#[derive(Clone)]
pub struct Decoder {
    channels: usize,
    block_align: usize,
    coding_mode: u16,
    scrambled: bool,
    units: Vec<Unit>,
    matrix_prev: [[usize; 4]; 4],
    matrix_now: [[usize; 4]; 4],
    matrix_next: [[usize; 4]; 4],
    weighting_delay: [[u32; 6]; 4],
    imdct: Imdct,
    t: Arc<Tables>,
    /// `mantissa` of `decode_tonal_components`, kept between components as in FFmpeg.
    tonal_mantissa: [i32; 8],
}

fn be16(d: &[u8], at: usize) -> u16 {
    d.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn le16(d: &[u8], at: usize) -> u16 {
    d.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

impl Decoder {
    pub fn new(p: &Params) -> Result<Self> {
        let channels = usize::from(p.channels);
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(Error::Unsupported("ATRAC3 channel count"));
        }
        let e = &p.extradata;
        let (version, spf, delay, coding_mode, scrambled) = match e.len() {
            14 => {
                let mode = le16(e, 6);
                let frame_factor = usize::from(le16(e, 10));
                if ![96, 152, 192].iter().any(|&b| p.block_align == b * channels * frame_factor) {
                    return Err(Error::Unsupported("ATRAC3 frame size"));
                }
                (4, SAMPLES_PER_FRAME * channels, 0x88E, if mode != 0 { JOINT_STEREO } else { SINGLE }, false)
            }
            10 | 12 => {
                let version = u32::from(be16(e, 0)) << 16 | u32::from(be16(e, 2));
                (version, usize::from(be16(e, 4)), be16(e, 6), be16(e, 8), true)
            }
            _ => return Err(Error::Unsupported("ATRAC3 extradata size")),
        };
        if version != 4 || spf != SAMPLES_PER_FRAME * channels || delay != 0x88E {
            return Err(Error::Unsupported("ATRAC3 extradata"));
        }
        if coding_mode == JOINT_STEREO {
            if channels % 2 == 1 {
                return Err(Error::Invalid("ATRAC3 joint stereo with an odd channel count"));
            }
        } else if coding_mode != SINGLE {
            return Err(Error::Unsupported("ATRAC3 coding mode"));
        }
        if p.block_align == 0 || p.block_align > 4096 {
            return Err(Error::Invalid("ATRAC3 frame size"));
        }
        Ok(Self {
            channels,
            block_align: p.block_align,
            coding_mode,
            scrambled,
            units: vec![Unit::new(); channels],
            matrix_prev: [[3; 4]; 4],
            matrix_now: [[3; 4]; 4],
            matrix_next: [[3; 4]; 4],
            weighting_delay: [[0, 7, 0, 7, 0, 7]; 4],
            imdct: Imdct::new(256, -1.0 / 32768.0),
            t: Arc::new(Tables::new()),
            tonal_mantissa: [0; 8],
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn block_align(&self) -> usize {
        self.block_align
    }

    pub fn samples_per_channel(&self) -> usize {
        SAMPLES_PER_FRAME
    }

    /// Decodes one frame of [`Self::block_align`] bytes into planar samples.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Vec<Vec<f32>>> {
        let ba = self.block_align;
        let data = frame.get(..ba).ok_or(Error::Invalid("ATRAC3 frame too short"))?;
        let buf: Vec<u8> = if self.scrambled {
            const KEY: [u8; 4] = [0x53, 0x7F, 0x61, 0x03];
            data.iter().enumerate().map(|(i, &b)| b ^ KEY[i % 4]).collect()
        } else {
            data.to_vec()
        };
        let mut out = vec![vec![0f32; SAMPLES_PER_FRAME]; self.channels];
        if self.coding_mode == JOINT_STEREO {
            let js_ba = (ba / self.channels) * 2;
            for ch in (0..self.channels).step_by(2) {
                let pair = ch / 2;
                let js = buf.get(pair * js_ba..(pair + 1) * js_ba).ok_or(Error::Invalid("ATRAC3 frame too short"))?;
                let (lo, hi) = out.split_at_mut(ch + 1);
                let (o1, o2) = (&mut lo[ch], &mut hi[0]);
                let mut b = Bits::new(js, js_ba * 8);
                self.sound_unit(&mut b, ch, o1, true)?;
                // the second sound unit is stored backwards
                let rev: Vec<u8> = js.iter().rev().copied().collect();
                let mut k = 0;
                while rev.get(k) == Some(&0xF8) {
                    if 4 + k >= js_ba {
                        return Err(Error::Invalid("ATRAC3 sync run"));
                    }
                    k += 1;
                }
                let rest = &rev[k..];
                let mut b = Bits::new(rest, rest.len() * 8);
                let wd = &mut self.weighting_delay[pair];
                wd.copy_within(2..6, 0);
                wd[4] = b.get(1);
                wd[5] = b.get(3);
                for i in 0..4 {
                    self.matrix_prev[pair][i] = self.matrix_now[pair][i];
                    self.matrix_now[pair][i] = self.matrix_next[pair][i];
                    self.matrix_next[pair][i] = b.get(2) as usize;
                }
                self.sound_unit(&mut b, ch + 1, o2, true)?;
                reverse_matrixing(o1, o2, &self.matrix_prev[pair], &self.matrix_now[pair]);
                channel_weighting(o1, o2, &self.weighting_delay[pair]);
            }
        } else {
            for (i, o) in out.iter_mut().enumerate() {
                let d = buf.get(i * ba / self.channels..).unwrap_or(&[]);
                let mut b = Bits::new(d, ba * 8 / self.channels);
                self.sound_unit(&mut b, i, o, false)?;
            }
        }
        for (i, o) in out.iter_mut().enumerate() {
            let u = &mut self.units[i];
            let q = &self.t.qmf;
            let lo: Vec<f32> = o[0..256].to_vec();
            let hi: Vec<f32> = o[256..512].to_vec();
            iqmf(&lo, &hi, &mut o[0..512], &mut u.delay1, q);
            let lo: Vec<f32> = o[768..1024].to_vec();
            let hi: Vec<f32> = o[512..768].to_vec();
            iqmf(&lo, &hi, &mut o[512..1024], &mut u.delay2, q);
            let lo: Vec<f32> = o[0..512].to_vec();
            let hi: Vec<f32> = o[512..1024].to_vec();
            iqmf(&lo, &hi, &mut o[0..1024], &mut u.delay3, q);
        }
        Ok(out)
    }

    /// `decode_channel_sound_unit`.
    fn sound_unit(&mut self, b: &mut Bits<'_>, ch: usize, output: &mut [f32], joint: bool) -> Result<()> {
        if joint && ch % 2 == 1 {
            if b.get(2) != 3 {
                return Err(Error::Invalid("ATRAC3 sound unit id"));
            }
        } else if b.get(6) != 0x28 {
            return Err(Error::Invalid("ATRAC3 sound unit id"));
        }
        let bands_coded = b.get(2) as usize;
        let sw = self.units[ch].gc_blk_switch;
        // gain2 = gain_block[1 - switch]
        let mut g2 = [GainInfo::default(); 4];
        for g in g2.iter_mut().take(bands_coded + 1) {
            g.num_points = b.get(3) as usize;
            for j in 0..g.num_points {
                g.lev[j] = b.get(4) as usize;
                g.loc[j] = b.get(5) as usize;
                if j > 0 && g.loc[j] <= g.loc[j - 1] {
                    return Err(Error::Invalid("ATRAC3 gain control"));
                }
            }
        }
        self.units[ch].gain_block[1 - sw] = g2;
        let tonal = self.tonal_components(b, bands_coded)?;
        let num_subbands = self.spectrum(b, ch)?;
        let u = &mut self.units[ch];
        let mut last_tonal: i64 = -1;
        for c in &tonal {
            last_tonal = last_tonal.max((c.pos + c.num_coefs) as i64);
            for (j, v) in c.coef.iter().take(c.num_coefs).enumerate() {
                if let Some(s) = u.spectrum.get_mut(c.pos + j) {
                    *s += v;
                }
            }
        }
        let mut num_bands = (SUBBAND_TAB[num_subbands] as i64 - 1) >> 8;
        if last_tonal >= 0 {
            num_bands = num_bands.max((last_tonal + 256) >> 8);
        }
        let g1 = u.gain_block[sw];
        for band in 0..4 {
            if band as i64 <= num_bands {
                let s = &mut u.spectrum[band * 256..band * 256 + 256];
                if band & 1 == 1 {
                    s.reverse();
                }
                self.imdct.inverse(s, &mut u.imdct_buf[..512]);
                for (v, w) in u.imdct_buf[..512].iter_mut().zip(&self.t.window) {
                    *v *= w;
                }
            } else {
                u.imdct_buf[..512].fill(0.0);
            }
            gain_compensation(
                &self.t,
                &u.imdct_buf[..512],
                &mut u.prev_frame[band * 256..band * 256 + 256],
                &g1[band],
                &g2[band],
                &mut output[band * 256..band * 256 + 256],
            );
        }
        u.gc_blk_switch ^= 1;
        Ok(())
    }

    /// `read_quant_spectral_coeffs` into `m`.
    fn mantissas(&self, b: &mut Bits<'_>, selector: usize, clc: bool, m: &mut [i32], mut n: usize) -> Result<()> {
        if selector == 1 {
            n /= 2;
        }
        if clc {
            let bits = CLC_LENGTH_TAB[selector];
            if selector > 1 {
                for v in m.iter_mut().take(n) {
                    *v = if bits > 0 {
                        let x = b.get(bits) as i32;
                        (x << (32 - bits)) >> (32 - bits)
                    } else {
                        0
                    };
                }
            } else {
                for i in 0..n {
                    let code = if bits > 0 { b.get(bits) as usize } else { 0 };
                    m[i * 2] = MANTISSA_CLC_TAB[code >> 2 & 3];
                    m[i * 2 + 1] = MANTISSA_CLC_TAB[code & 3];
                }
            }
        } else {
            let h = self.t.huff.get(selector - 1).ok_or(Error::Invalid("ATRAC3 table"))?;
            if selector != 1 {
                for v in m.iter_mut().take(n) {
                    *v = h.read(b).ok_or(Error::Invalid("ATRAC3 code"))?;
                }
            } else {
                for i in 0..n {
                    let s = h.read(b).ok_or(Error::Invalid("ATRAC3 code"))? as usize;
                    m[i * 2] = MANTISSA_VLC_TAB[(s * 2) % 18];
                    m[i * 2 + 1] = MANTISSA_VLC_TAB[(s * 2 + 1) % 18];
                }
            }
        }
        Ok(())
    }

    /// `decode_spectrum`: the subband count.
    fn spectrum(&mut self, b: &mut Bits<'_>, ch: usize) -> Result<usize> {
        let num_subbands = b.get(5) as usize;
        let clc = b.get1();
        let mut vlc_index = [0usize; 32];
        let mut sf_index = [0usize; 32];
        for v in vlc_index.iter_mut().take(num_subbands + 1) {
            *v = b.get(3) as usize;
        }
        for i in 0..=num_subbands {
            if vlc_index[i] != 0 {
                sf_index[i] = b.get(6) as usize;
            }
        }
        let mut m = [0i32; 128];
        for i in 0..=num_subbands {
            let (first, last) = (SUBBAND_TAB[i], SUBBAND_TAB[i + 1]);
            if vlc_index[i] != 0 {
                self.mantissas(b, vlc_index[i], clc, &mut m, last - first)?;
                let sf = self.t.sf[sf_index[i]] * inv_max_quant(vlc_index[i]);
                for (o, &q) in self.units[ch].spectrum[first..last].iter_mut().zip(&m) {
                    *o = q as f32 * sf;
                }
            } else {
                self.units[ch].spectrum[first..last].fill(0.0);
            }
        }
        let first = SUBBAND_TAB[num_subbands + 1];
        self.units[ch].spectrum[first..].fill(0.0);
        Ok(num_subbands)
    }

    /// `decode_tonal_components`.
    fn tonal_components(&mut self, b: &mut Bits<'_>, num_bands: usize) -> Result<Vec<Tonal>> {
        let mut out = Vec::new();
        let n = b.get(5);
        if n == 0 {
            return Ok(out);
        }
        let selector = b.get(2);
        if selector == 2 {
            return Err(Error::Invalid("ATRAC3 tonal coding mode"));
        }
        let mut clc = selector & 1 == 1;
        for _ in 0..n {
            let mut flags = [false; 4];
            for f in flags.iter_mut().take(num_bands + 1) {
                *f = b.get1();
            }
            let per = b.get(3) as usize;
            let step = b.get(3) as usize;
            if step <= 1 {
                return Err(Error::Invalid("ATRAC3 tonal quantiser"));
            }
            if selector == 3 {
                clc = b.get1();
            }
            for band in 0..(num_bands + 1) * 4 {
                if !flags[band >> 2] {
                    continue;
                }
                let coded = b.get(3);
                for _ in 0..coded {
                    let sf_index = b.get(6) as usize;
                    if out.len() >= 64 {
                        return Err(Error::Invalid("ATRAC3 tonal components"));
                    }
                    let pos = band * 64 + b.get(6) as usize;
                    let values = (per + 1).min(SAMPLES_PER_FRAME - pos);
                    let sf = self.t.sf[sf_index] * inv_max_quant(step);
                    let mut m = self.tonal_mantissa;
                    self.mantissas(b, step, clc, &mut m, values)?;
                    self.tonal_mantissa = m;
                    let mut c = Tonal { pos, num_coefs: values, coef: [0.0; 8] };
                    for (o, &q) in c.coef.iter_mut().zip(&m).take(values) {
                        *o = q as f32 * sf;
                    }
                    out.push(c);
                }
            }
        }
        Ok(out)
    }
}

/// `ff_atrac_gain_compensation` (256 samples, location scale 3).
fn gain_compensation(t: &Tables, input: &[f32], prev: &mut [f32], now: &GainInfo, next: &GainInfo, out: &mut [f32]) {
    let n = 256;
    let gc_scale = if next.num_points > 0 { t.gain1[next.lev[0] & 15] } else { 1.0 };
    if now.num_points == 0 {
        for p in 0..n {
            out[p] = input[p] * gc_scale + prev[p];
        }
    } else {
        let mut pos = 0;
        for i in 0..now.num_points {
            let lastpos = (now.loc[i] << 3).min(n);
            let mut lev = t.gain1[now.lev[i] & 15];
            let next_lev = if i + 1 < now.num_points { now.lev[i + 1] } else { 4 };
            let gain_inc = t.gain2[(next_lev + 15 - now.lev[i]).min(30)];
            while pos < lastpos {
                out[pos] = (input[pos] * gc_scale + prev[pos]) * lev;
                pos += 1;
            }
            while pos < (lastpos + 8).min(n) {
                out[pos] = (input[pos] * gc_scale + prev[pos]) * lev;
                lev *= gain_inc;
                pos += 1;
            }
        }
        while pos < n {
            out[pos] = input[pos] * gc_scale + prev[pos];
            pos += 1;
        }
    }
    prev.copy_from_slice(&input[n..2 * n]);
}

/// `ff_atrac_iqmf`: `lo` and `hi` (`n` each) to `2n` samples in `out`.
fn iqmf(lo: &[f32], hi: &[f32], out: &mut [f32], delay: &mut [f32; 46], w: &[f32; 48]) {
    let n = lo.len();
    let mut temp = vec![0f32; 46 + 2 * n];
    temp[..46].copy_from_slice(delay);
    for i in (0..n).step_by(2) {
        temp[46 + 2 * i] = lo[i] + hi[i];
        temp[46 + 2 * i + 1] = lo[i] - hi[i];
        temp[46 + 2 * i + 2] = lo[i + 1] + hi[i + 1];
        temp[46 + 2 * i + 3] = lo[i + 1] - hi[i + 1];
    }
    for j in 0..n {
        let p = &temp[2 * j..2 * j + 48];
        let (mut s1, mut s2) = (0f32, 0f32);
        for i in (0..48).step_by(2) {
            s1 += p[i] * w[i];
            s2 += p[i + 1] * w[i + 1];
        }
        out[2 * j] = s2;
        out[2 * j + 1] = s1;
    }
    delay.copy_from_slice(&temp[2 * n..2 * n + 46]);
}

/// `INTERPOLATE(old, new, nsample)`, in `f64` as in C.
fn interpolate(old: f32, new: f32, n: usize) -> f64 {
    f64::from(old) + n as f64 * 0.125 * f64::from(new - old)
}

/// `reverse_matrixing`.
fn reverse_matrixing(su1: &mut [f32], su2: &mut [f32], prev: &[usize; 4], curr: &[usize; 4]) {
    for (i, band) in (0..4 * 256).step_by(256).enumerate() {
        let s1 = prev[i] & 3;
        let s2 = curr[i] & 3;
        let mut n = band;
        if s1 != s2 {
            let (mc1l, mc1r) = (MATRIX_COEFFS[s1 * 2], MATRIX_COEFFS[s1 * 2 + 1]);
            let (mc2l, mc2r) = (MATRIX_COEFFS[s2 * 2], MATRIX_COEFFS[s2 * 2 + 1]);
            while n < band + 8 {
                let c1 = su1[n];
                let c2 = su2[n];
                let c2 = (f64::from(c1) * interpolate(mc1l, mc2l, n - band) + f64::from(c2) * interpolate(mc1r, mc2r, n - band)) as f32;
                su1[n] = c2;
                su2[n] = (f64::from(c1) * 2.0 - f64::from(c2)) as f32;
                n += 1;
            }
        }
        while n < band + 256 {
            let (c1, c2) = (su1[n], su2[n]);
            match s2 {
                0 => {
                    su1[n] = c2 * 2.0;
                    su2[n] = (c1 - c2) * 2.0;
                }
                1 => {
                    su1[n] = (c1 + c2) * 2.0;
                    su2[n] = c2 * -2.0;
                }
                _ => {
                    su1[n] = c1 + c2;
                    su2[n] = c1 - c2;
                }
            }
            n += 1;
        }
    }
}

/// `get_channel_weights`.
fn channel_weights(index: u32, flag: u32) -> [f32; 2] {
    if index == 7 {
        return [1.0, 1.0];
    }
    let a = (f64::from(index & 7) / 7.0) as f32;
    let b = f64::from(2.0 - a * a).sqrt() as f32;
    if flag != 0 { [b, a] } else { [a, b] }
}

/// `channel_weighting`.
fn channel_weighting(su1: &mut [f32], su2: &mut [f32], p3: &[u32; 6]) {
    if p3[1] == 7 && p3[3] == 7 {
        return;
    }
    let w = [channel_weights(p3[1], p3[0]), channel_weights(p3[3], p3[2])];
    for band in (256..4 * 256).step_by(256) {
        let mut n = band;
        while n < band + 8 {
            su1[n] = (f64::from(su1[n]) * interpolate(w[0][0], w[0][1], n - band)) as f32;
            su2[n] = (f64::from(su2[n]) * interpolate(w[1][0], w[1][1], n - band)) as f32;
            n += 1;
        }
        while n < band + 256 {
            su1[n] *= w[1][0];
            su2[n] *= w[1][1];
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rm_extradata(channels: u16, mode: u16) -> Vec<u8> {
        let mut e = 4u32.to_be_bytes().to_vec();
        e.extend_from_slice(&(1024 * channels).to_be_bytes());
        e.extend_from_slice(&0x88Eu16.to_be_bytes());
        e.extend_from_slice(&mode.to_be_bytes());
        e
    }

    #[test]
    fn bad_parameters_are_refused() {
        let p = |channels, block_align, extradata| Decoder::new(&Params { channels, block_align, extradata }).is_err();
        assert!(p(0, 384, rm_extradata(1, 2)));
        assert!(p(9, 384, rm_extradata(9, 2)));
        assert!(p(2, 384, rm_extradata(1, 2)));
        assert!(p(1, 384, rm_extradata(1, 0x12)));
        assert!(p(2, 0, rm_extradata(2, 2)));
        assert!(p(2, 5000, rm_extradata(2, 2)));
        assert!(p(2, 384, vec![0; 3]));
        // WAV: the frame size must fit the channel count
        let wav = vec![1, 0, 0, 0x10, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0];
        assert!(p(2, 383, wav.clone()));
        assert!(!p(2, 384, wav));
    }

    #[test]
    fn random_frames_do_not_panic() {
        let mut x = 0x1357_9BDFu32;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for (ch, ba, mode) in [(1u16, 192usize, 2u16), (2, 384, 2), (2, 304, 0x12), (2, 384, 0x12), (6, 1152, 0x12), (8, 768, 2)] {
            let mut d = Decoder::new(&Params { channels: ch, block_align: ba, extradata: rm_extradata(ch, mode) }).unwrap();
            assert!(d.decode(&vec![0; ba - 1]).is_err());
            for i in 0..1500 {
                let mut f: Vec<u8> = (0..ba).map(|_| rnd() as u8).collect();
                // valid sound unit ids (after descrambling) let decoding go deeper
                if i % 2 == 0 {
                    f[0] = 0x28 << 2 ^ 0x53;
                }
                if let Ok(p) = d.decode(&f) {
                    assert_eq!(p.len(), usize::from(ch));
                    assert!(p.iter().all(|c| c.len() == 1024));
                }
            }
        }
    }
}
