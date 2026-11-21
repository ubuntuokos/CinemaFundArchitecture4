// A port of atracdenc 0.2.3 (Copyright (c) Daniil Cherednik; LGPL-2.1-or-later):
// src/atrac3denc.{h,cpp}, src/atrac/atrac3.{h,cpp}, src/atrac/atrac3_bitstream.{h,cpp},
// src/transient_detector.cpp (AnalyzeGain), src/gain_processor.h (Modulate), src/util.h. See
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! The ATRAC3 encoder (Sony's Adaptive TRansform Acoustic Coding 3, as in PSP / PS3 `.at3`
//! files and MiniDisc LP2 / LP4): 44.1 kHz, 1024 samples per channel and frame, a four-band QMF
//! split, a 256-line MDCT per band with gain control for attacks, and joint stereo (LP4 and the
//! 94 kbit/s mode). Tonal components are not used, as in atracdenc 0.2.3.

use crate::bits::BitWriter;
use crate::mdct::Mdct;
use crate::psy::{self, ScaledBlock};
use crate::qmf::Atrac3Analysis;
use crate::{Error, Result};

/// Samples per channel and frame.
pub const SAMPLES_PER_FRAME: usize = 1024;
/// The only sample rate of ATRAC3.
pub const SAMPLE_RATE: u32 = 44_100;

/// An ATRAC3 bit rate: the frame size (both channels) and whether it is joint stereo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// Bits per second as atracdenc counts them (frame bits × 44100 / 1024).
    pub bitrate: u32,
    /// Bytes per frame (`block_align`).
    pub frame_size: u16,
    /// Joint stereo (matrixed M/S with a movable split of the frame).
    pub joint_stereo: bool,
}

impl Mode {
    /// The nominal rate in kbit/s (66, 94, 105, 132, 146, 176, 264 or 352).
    pub fn kbps(&self) -> u32 {
        (self.bitrate + 500) / 1000
    }
}

/// The ATRAC3 modes of AT3 (WAV) files, as Sony's and FFmpeg's decoders accept them: LP4
/// (66 kbit/s, joint stereo), 105 kbit/s and LP2 (132 kbit/s, the PSP's usual rate). A mono stream
/// uses half the frame (33, 52 or 66 kbit/s), without joint stereo.
pub const MODES: [Mode; 3] = [
    Mode { bitrate: 66_150, frame_size: 192, joint_stereo: true },
    Mode { bitrate: 104_738, frame_size: 304, joint_stereo: false },
    Mode { bitrate: 132_300, frame_size: 384, joint_stereo: false },
];

/// The mode for a nominal stereo rate: the lowest mode of at least `kbps` (within 2 kbit/s, so
/// 64 gives LP4 and 128 LP2), LP2 above 132.
pub fn mode_for_kbps(kbps: u32) -> Mode {
    MODES.iter().copied().find(|m| m.kbps() + 2 >= kbps).unwrap_or(MODES[2])
}

const BLOCK_SIZE_TAB: [usize; 33] =
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 288, 320, 352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024];
const MAX_BFUS: usize = 32;
const CLC_LENGTH_TAB: [u32; 8] = [0, 4, 3, 3, 4, 4, 5, 6];
const MAX_QUANT: [f32; 8] = [0.0, 1.5, 2.5, 3.5, 4.5, 7.5, 15.5, 31.5];
const FIXED_BIT_ALLOC: [u32; MAX_BFUS] = [4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 0, 0];
/// Blocks above this use energy-aware quantisation.
const LOSSY_NAQ_START: usize = 18;
/// Blocks below this get a bit more when their energy error is large.
const BOOST_NAQ_END: usize = 10;
const EXPONENT_OFFSET: i32 = 4;
const LOC_SCALE: u32 = 3;
const LOC_SZ: usize = 1 << LOC_SCALE;
const GAIN_INTERPOLATION_POS_SHIFT: i32 = 15;
const MAX_GAIN_POINTS: usize = 8;

/// (code, length) of the spectral Huffman codes, by table.
type Huff = &'static [(u8, u8)];
const HUFF1: Huff = &[(0x0, 1), (0x4, 3), (0x5, 3), (0xC, 4), (0xD, 4), (0x1C, 5), (0x1D, 5), (0x1E, 5), (0x1F, 5)];
const HUFF2: Huff = &[(0x0, 1), (0x4, 3), (0x5, 3), (0x6, 3), (0x7, 3)];
const HUFF3: Huff = &[(0x0, 1), (0x4, 3), (0x5, 3), (0xC, 4), (0xD, 4), (0xE, 4), (0xF, 4)];
const HUFF5: Huff = &[
    (0x0, 2),
    (0x2, 3),
    (0x3, 3),
    (0x8, 4),
    (0x9, 4),
    (0xA, 4),
    (0xB, 4),
    (0x1C, 5),
    (0x1D, 5),
    (0x3C, 6),
    (0x3D, 6),
    (0x3E, 6),
    (0x3F, 6),
    (0xC, 4),
    (0xD, 4),
];
const HUFF6: Huff = &[
    (0x0, 3),
    (0x2, 4),
    (0x3, 4),
    (0x4, 4),
    (0x5, 4),
    (0x6, 4),
    (0x7, 4),
    (0x14, 5),
    (0x15, 5),
    (0x16, 5),
    (0x17, 5),
    (0x18, 5),
    (0x19, 5),
    (0x34, 6),
    (0x35, 6),
    (0x36, 6),
    (0x37, 6),
    (0x38, 6),
    (0x39, 6),
    (0x3A, 6),
    (0x3B, 6),
    (0x78, 7),
    (0x79, 7),
    (0x7A, 7),
    (0x7B, 7),
    (0x7C, 7),
    (0x7D, 7),
    (0x7E, 7),
    (0x7F, 7),
    (0x8, 4),
    (0x9, 4),
];
const HUFF7: Huff = &[
    (0x0, 3),
    (0x8, 5),
    (0x9, 5),
    (0xA, 5),
    (0xB, 5),
    (0xC, 5),
    (0xD, 5),
    (0xE, 5),
    (0xF, 5),
    (0x10, 5),
    (0x11, 5),
    (0x24, 6),
    (0x25, 6),
    (0x26, 6),
    (0x27, 6),
    (0x28, 6),
    (0x29, 6),
    (0x2A, 6),
    (0x2B, 6),
    (0x2C, 6),
    (0x2D, 6),
    (0x2E, 6),
    (0x2F, 6),
    (0x30, 6),
    (0x31, 6),
    (0x32, 6),
    (0x33, 6),
    (0x68, 7),
    (0x69, 7),
    (0x6A, 7),
    (0x6B, 7),
    (0x6C, 7),
    (0x6D, 7),
    (0x6E, 7),
    (0x6F, 7),
    (0x70, 7),
    (0x71, 7),
    (0x72, 7),
    (0x73, 7),
    (0x74, 7),
    (0x75, 7),
    (0xEC, 8),
    (0xED, 8),
    (0xEE, 8),
    (0xEF, 8),
    (0xF0, 8),
    (0xF1, 8),
    (0xF2, 8),
    (0xF3, 8),
    (0xF4, 8),
    (0xF5, 8),
    (0xF6, 8),
    (0xF7, 8),
    (0xF8, 8),
    (0xF9, 8),
    (0xFA, 8),
    (0xFB, 8),
    (0xFC, 8),
    (0xFD, 8),
    (0xFE, 8),
    (0xFF, 8),
    (0x2, 4),
    (0x3, 4),
];
/// The table of each VLC selector (1..=7).
const HUFF_TABLES: [Huff; 7] = [HUFF1, HUFF2, HUFF3, HUFF1, HUFF5, HUFF6, HUFF7];

fn specs_per_block(block: usize) -> usize {
    BLOCK_SIZE_TAB[block + 1] - BLOCK_SIZE_TAB[block]
}

/// Static tables computed once.
struct Tables {
    scale: [f32; 64],
    encode_window: [f32; 256],
    gain_level: [f32; 16],
    gain_interpolation: [f32; 31],
    /// The threshold of hearing of each block (energy).
    ath: [f32; MAX_BFUS],
    loudness_curve: Vec<f32>,
}

impl Tables {
    fn new() -> Self {
        let mut scale = [0f32; 64];
        for (i, s) in scale.iter_mut().enumerate() {
            *s = 2f64.powf(i as f64 / 3.0 - 21.0) as f32;
        }
        let mut encode_window = [0f32; 256];
        for (i, w) in encode_window.iter_mut().enumerate() {
            *w = (((i as f64 + 0.5) / 256.0 - 0.5) * std::f64::consts::PI).sin() as f32 + 1.0;
        }
        let mut gain_level = [0f32; 16];
        for (i, g) in gain_level.iter_mut().enumerate() {
            *g = 2f64.powi(EXPONENT_OFFSET - i as i32) as f32;
        }
        let mut gain_interpolation = [0f32; 31];
        for (i, g) in gain_interpolation.iter_mut().enumerate() {
            *g = 2f64.powf(-1.0 / LOC_SZ as f64 * (i as f64 - 15.0)) as f32;
        }
        let spec = psy::calc_ath(1024, SAMPLE_RATE);
        let mut ath = [0f32; MAX_BFUS];
        for (block, a) in ath.iter_mut().enumerate() {
            let start = BLOCK_SIZE_TAB[block];
            let x = spec[start..start + specs_per_block(block)].iter().fold(999f32, |m, &v| m.min(v));
            *a = 10f64.powf(0.1 * f64::from(x)) as f32 / 100.0;
        }
        Self { scale, encode_window, gain_level, gain_interpolation, ath, loudness_curve: psy::loudness_curve(SAMPLES_PER_FRAME) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GainPoint {
    level: u32,
    location: u32,
}

/// A channel's coded data for one frame.
#[derive(Clone, Default)]
struct Sce {
    /// Gain points of each QMF band (4, or 1 for an empty joint-stereo side channel).
    gain: Vec<Vec<GainPoint>>,
    scaled: Vec<ScaledBlock>,
    loudness: f32,
}

#[derive(Clone, Copy)]
struct Transient {
    attack0_loc: i32,
    attack0_rel: f32,
    attack1_loc: i32,
    release_loc: i32,
}

const NO_TRANSIENT: Transient = Transient { attack0_loc: -1, attack0_rel: 1.0, attack1_loc: -1, release_loc: -1 };

/// atracdenc's `GetFirstSetBit`: the index of the highest set bit (0 for 0).
fn first_set_bit(x: u32) -> u32 {
    if x == 0 { 0 } else { 31 - x.leading_zeros() }
}

/// atracdenc's `RelationToIdx`: the gain level index of a level ratio.
fn relation_to_idx(x: f32) -> u32 {
    if x <= 0.5 {
        let x = 1.0 / x.max(0.000_488_281_25);
        4 + first_set_bit(x.trunc() as u32)
    } else {
        let x = x.min(16.0);
        4u32.saturating_sub(first_set_bit(x.trunc() as u32))
    }
}

/// Peak of each `len / points`-sample step.
fn analyze_gain(input: &[f32], points: usize) -> Vec<f32> {
    let step = input.len() / points;
    input.chunks(step).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs()))).collect()
}

fn calc_transient(gain: &[f32], last_max: f32) -> Transient {
    const ATTACK: f32 = 2.0;
    const RELEASE: f32 = 2.0;
    let mut t = NO_TRANSIENT;
    for (i, &g) in gain.iter().enumerate() {
        let r = g / last_max;
        if r > ATTACK {
            t.attack0_rel = r;
            t.attack0_loc = i as i32;
            break;
        }
    }
    let mut q = gain[0];
    for (i, &g) in gain.iter().enumerate().skip(1) {
        if g / q > ATTACK {
            t.attack1_loc = i as i32;
        }
        q = q.max(g);
    }
    let mut q = gain[gain.len() - 1];
    for i in (1..gain.len() - 1).rev() {
        if gain[i] / q > RELEASE {
            t.release_loc = i as i32;
            break;
        }
        q = q.max(gain[i]);
    }
    t
}

/// The ATRAC3 encoder of one stream (mono or stereo).
#[derive(Clone)]
pub struct Encoder {
    channels: usize,
    mode: Mode,
    gain_control: bool,
    analysis: Vec<Atrac3Analysis>,
    /// [channel][band]: the previous windowed half, then the current band samples.
    buf: Vec<[[f32; 512]; 4]>,
    prev_peak: Vec<[f32; 4]>,
    history: Vec<[Transient; 4]>,
    loudness: f32,
    mdct: Mdct,
    sce: Vec<Sce>,
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(Tables::new)
}

impl Encoder {
    /// An encoder of `channels` (1 or 2) at `mode`.
    pub fn new(channels: u16, mode: Mode) -> Result<Self> {
        if !(1..=2).contains(&channels) {
            return Err(Error::Unsupported("ATRAC3 encodes one or two channels"));
        }
        let c = usize::from(channels);
        // mono: one half-frame per frame, never joint stereo
        let mode = if c == 1 { Mode { joint_stereo: false, ..mode } } else { mode };
        let zero = Transient { attack0_loc: 0, attack0_rel: 0.0, attack1_loc: 0, release_loc: 0 };
        Ok(Self {
            channels: c,
            mode,
            gain_control: true,
            analysis: vec![Atrac3Analysis::new(); c],
            buf: vec![[[0.0; 512]; 4]; c],
            prev_peak: vec![[0.0; 4]; c],
            history: vec![[zero; 4]; c],
            loudness: 0.006,
            mdct: Mdct::new(512, 1.0),
            sce: vec![Sce { gain: vec![Vec::new(); 4], ..Sce::default() }; c],
        })
    }

    /// Switches gain control (attack handling) off or on (default on).
    pub fn set_gain_control(&mut self, on: bool) {
        self.gain_control = on;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Bytes per encoded frame (`block_align`): the mode's frame, half of it for mono.
    pub fn block_align(&self) -> usize {
        usize::from(self.mode.frame_size) / (3 - self.channels)
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Encodes one frame: 1024 samples per channel, interleaved, in [-1, 1] (missing samples
    /// are silence). Returns [`Encoder::block_align`] bytes.
    pub fn encode(&mut self, pcm: &[f32]) -> Vec<u8> {
        let c = self.channels;
        let mut src = [0f32; SAMPLES_PER_FRAME];
        for ch in 0..c {
            for (i, s) in src.iter_mut().enumerate() {
                let v = pcm.get(i * c + ch).copied().unwrap_or(0.0);
                // negated: with atracdenc's MDCT sign, Sony's and FFmpeg's decoders would
                // otherwise play the stream back with inverted polarity
                *s = -(if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }) / 4.0;
            }
            let mut bands = [[0f32; 256]; 4];
            self.analysis[ch].analysis(&src, &mut bands);
            for (b, band) in bands.iter().enumerate() {
                self.buf[ch][b][256..].copy_from_slice(band);
            }
        }
        if self.mode.joint_stereo && c == 2 {
            for b in 0..4 {
                for i in 256..512 {
                    let (l, r) = (self.buf[0][b][i], self.buf[1][b][i]);
                    self.buf[0][b][i] = (l + r) / 2.0;
                    self.buf[1][b][i] = (l - r) / 2.0;
                }
            }
        }
        let t = tables();
        for ch in 0..c {
            let mut gain = vec![Vec::new(); 4];
            if self.gain_control {
                self.subband_info(ch, &mut gain);
            }
            let mut specs = vec![0f32; SAMPLES_PER_FRAME];
            for b in 0..4 {
                self.mdct_band(t, ch, b, &gain[b], &mut specs[b * 256..(b + 1) * 256]);
            }
            let loudness = specs.iter().zip(&t.loudness_curve).map(|(s, w)| s * s * w).sum();
            let mut scaled = Vec::with_capacity(MAX_BFUS);
            for block in 0..MAX_BFUS {
                let start = BLOCK_SIZE_TAB[block];
                scaled.push(psy::scale(&t.scale, &specs[start..start + specs_per_block(block)]));
            }
            self.sce[ch] = Sce { gain, scaled, loudness };
        }
        self.loudness = if c == 2 && !self.mode.joint_stereo {
            psy::track_loudness2(self.loudness, self.sce[0].loudness, self.sce[1].loudness)
        } else {
            psy::track_loudness(self.loudness, self.sce[0].loudness)
        };
        let mut frame = write_sound_unit(t, self.mode, &self.sce, self.loudness);
        frame.truncate(self.block_align());
        frame
    }

    /// atracdenc's `CreateSubbandInfo`: a gain point where a band's level jumps.
    fn subband_info(&mut self, ch: usize, gain: &mut [Vec<GainPoint>]) {
        for b in 0..4 {
            let g = analyze_gain(&self.buf[ch][b][256..], 32);
            let tp = calc_transient(&g, self.prev_peak[ch][b]);
            if tp.attack0_loc == -1 && tp.attack1_loc == -1 && tp.release_loc == -1 {
                self.history[ch][b] = NO_TRANSIENT;
                continue;
            }
            if tp.attack0_loc == -1 && tp.attack1_loc == -1 {
                self.history[ch][b] = tp;
                continue;
            }
            let h = self.history[ch][b];
            if h.attack0_loc == -1 && h.attack1_loc == -1 && h.release_loc == -1 && tp.attack0_loc != -1 && tp.release_loc == -1 {
                let idx = relation_to_idx(1.0 / tp.attack0_rel).min(15);
                gain[b] = vec![GainPoint { level: idx, location: tp.attack0_loc as u32 }];
            }
            self.history[ch][b] = tp;
        }
    }

    /// Gain modulation, windowing and the MDCT of one band (atracdenc's `TAtrac3MDCT::Mdct`).
    fn mdct_band(&mut self, t: &Tables, ch: usize, b: usize, gain: &[GainPoint], out: &mut [f32]) {
        let buf = &mut self.buf[ch][b];
        let mut tmp = [0f32; 512];
        tmp[..256].copy_from_slice(&buf[..256]);
        if !gain.is_empty() {
            let (cur, next) = (&mut tmp[..256], &mut buf[256..]);
            modulate(t, gain, cur, next);
        }
        let mut max = 0f32;
        for i in 0..256 {
            let v = buf[256 + i];
            max = max.max(v.abs());
            buf[i] = t.encode_window[i] * v;
            tmp[256 + i] = t.encode_window[255 - i] * v;
        }
        self.mdct.forward(&tmp, out);
        if b & 1 == 1 {
            out.reverse();
        }
        self.prev_peak[ch][b] = max;
    }
}

/// atracdenc's `TGainProcessor::Modulate`.
fn modulate(t: &Tables, gi: &[GainPoint], cur: &mut [f32], next: &mut [f32]) {
    let mut pos = 0usize;
    let scale = t.gain_level[gi[0].level as usize];
    for (i, p) in gi.iter().enumerate() {
        let last = (p.location as usize) << LOC_SCALE;
        let mut level = t.gain_level[p.level as usize];
        let next_level = gi.get(i + 1).map_or(EXPONENT_OFFSET, |n| n.level as i32);
        let inc = t.gain_interpolation[(next_level - p.level as i32 + GAIN_INTERPOLATION_POS_SHIFT).clamp(0, 30) as usize];
        while pos < last.min(256) {
            cur[pos] /= scale;
            next[pos] /= level;
            pos += 1;
        }
        while pos < (last + LOC_SZ).min(256) {
            cur[pos] /= scale;
            next[pos] /= level;
            level *= inc;
            pos += 1;
        }
    }
    while pos < 256 {
        cur[pos] /= scale;
        pos += 1;
    }
}

fn mantissa_to_clc_idx(m: i32) -> u32 {
    [2, 3, 0, 1][(m + 2).clamp(0, 3) as usize]
}

fn mantissas_to_vlc_idx(a: i32, b: i32) -> usize {
    const TAB: [usize; 9] = [8, 4, 7, 2, 0, 1, 6, 3, 5];
    TAB[(3 * (a.clamp(-1, 1) + 1) + (b.clamp(-1, 1) + 1)) as usize]
}

/// Constant-length coding of a block's mantissas: bits used (and written, given a writer).
fn clc_enc(sel: u32, m: &[i32], w: Option<&mut BitWriter>) -> u32 {
    let nb = CLC_LENGTH_TAB[sel as usize];
    let len = m.len() as u32;
    let used = if sel > 1 { nb * len } else { nb * len / 2 };
    if let Some(w) = w {
        if sel > 1 {
            let mask = (1u32 << nb) - 1;
            for &v in m {
                w.write(v as u32 & mask, nb);
            }
        } else {
            for p in m.chunks_exact(2) {
                w.write(mantissa_to_clc_idx(p[0]) << 2 | mantissa_to_clc_idx(p[1]), nb);
            }
        }
    }
    used
}

/// Variable-length coding of a block's mantissas.
fn vlc_enc(sel: u32, m: &[i32], mut w: Option<&mut BitWriter>) -> u32 {
    let table = HUFF_TABLES[(sel as usize - 1).min(6)];
    let mut used = 0;
    let mut put = |idx: usize, w: &mut Option<&mut BitWriter>| {
        let (code, bits) = table[idx.min(table.len() - 1)];
        used += u32::from(bits);
        if let Some(w) = w.as_deref_mut() {
            w.write(u32::from(code), u32::from(bits));
        }
    };
    if sel > 1 {
        for &v in m {
            let mut s = if v < 0 { (v.unsigned_abs() << 1) | 1 } else { (v as u32) << 1 };
            s = s.saturating_sub(1);
            put(s as usize, &mut w);
        }
    } else {
        for p in m.chunks_exact(2) {
            put(mantissas_to_vlc_idx(p[0], p[1]), &mut w);
        }
    }
    used
}

/// (CLC mode, bits) of a channel's spectrum at `alloc`; quantises into `mt` and the energy
/// error of each block into `err`.
fn specs_bits(sce: &Sce, alloc: &[u32], mt: &mut [i32; SAMPLES_PER_FRAME], err: &mut [f32]) -> (bool, u32) {
    let n = alloc.len();
    let mut clc = 0;
    let mut vlc = 0;
    for i in 0..n {
        if alloc[i] == 0 {
            continue;
        }
        let (first, last) = (BLOCK_SIZE_TAB[i], BLOCK_SIZE_TAB[i + 1]);
        let mul = MAX_QUANT[alloc[i].min(7) as usize];
        err[i] = psy::quant_mantissas(&sce.scaled[i].values, mul, i > LOSSY_NAQ_START, &mut mt[first..last]);
        clc += 6 + clc_enc(alloc[i], &mt[first..last], None);
        vlc += 6 + vlc_enc(alloc[i], &mt[first..last], None);
    }
    let mode = clc <= vlc;
    (mode, n as u32 * 3 + if mode { clc } else { vlc })
}

fn consider_energy_err(err: &[f32], bits: &mut [u32]) -> bool {
    let mut adjusted = false;
    for i in 0..BOOST_NAQ_END.min(bits.len()) {
        let e = err[i];
        if ((e > 0.0 && e < 0.7) || e > 1.2) && bits[i] < 7 {
            bits[i] += 1;
            adjusted = true;
        }
    }
    adjusted
}

fn calc_bits_allocation(t: &Tables, blocks: &[ScaledBlock], n: usize, spread: f32, shift: f32, loudness: f32) -> Vec<u32> {
    (0..n)
        .map(|i| {
            if blocks[i].max_energy < t.ath[i] * loudness {
                return 0;
            }
            let x: f32 = match i {
                0..=2 => 2.8,
                3..=9 => 2.6,
                10..=14 => 3.3,
                15..=20 => 3.6,
                21..=28 => 4.2,
                _ => 6.0,
            };
            let a = f64::from(spread * (f32::from(blocks[i].sfi) / x));
            let v = (a + (1.0 - f64::from(spread)) * f64::from(FIXED_BIT_ALLOC[i]) - f64::from(shift)) as i32;
            match v {
                v if v > 7 => 7,
                v if v < 0 => 0,
                0 => 1,
                v => v as u32,
            }
        })
        .collect()
}

/// (CLC mode, precision of each block) fitting `target` bits; the mantissas in `mt`.
fn create_allocation(t: &Tables, sce: &Sce, target: i32, mt: &mut [i32; SAMPLES_PER_FRAME], loudness: f32) -> (bool, Vec<u32>) {
    if sce.scaled.is_empty() {
        return (true, vec![0]);
    }
    let spread = psy::scale_factor_spread(&sce.scaled);
    let mut n = MAX_BFUS;
    // 3 bits per block without data and 5 for the tonal header
    if target < 101 {
        n = n.min(((target - 5).max(3) / 3) as usize).max(1);
    }
    loop {
        let mut max_shift = 20f64;
        let mut min_shift = -8f64;
        let mut found: Option<(bool, Vec<u32>)> = None;
        for _ in 0..400 {
            let shift = (max_shift + min_shift) / 2.0;
            let mut alloc = calc_bits_allocation(t, &sce.scaled, n, spread, shift as f32, loudness);
            let mut err = vec![0f32; n];
            let mut used;
            loop {
                used = specs_bits(sce, &alloc, mt, &mut err);
                if !consider_energy_err(&err, &mut alloc) {
                    break;
                }
            }
            let bits = used.1 as i32 + 5; // + the tonal component header (none)
            if bits < target {
                if max_shift - min_shift < 0.1 {
                    found = Some((used.0, alloc));
                    break;
                }
                max_shift = shift - 0.01;
            } else if bits > target {
                min_shift = shift + 0.01;
            } else {
                found = Some((used.0, alloc));
                break;
            }
        }
        // never more bits than the frame holds: nothing coded rather than a broken frame
        let (mode, alloc) = found.unwrap_or_else(|| (true, vec![0; n]));
        if n > 1 && alloc[n - 1] == 0 {
            n -= 1;
            continue;
        }
        // the mantissas of the chosen allocation
        let mut err = vec![0f32; alloc.len()];
        specs_bits(sce, &alloc, mt, &mut err);
        return (mode, alloc);
    }
}

fn encode_specs(sce: &Sce, w: &mut BitWriter, mode: bool, alloc: &[u32], mt: &[i32; SAMPLES_PER_FRAME]) {
    w.write(0, 5); // no tonal components
    w.write(alloc.len() as u32 - 1, 5);
    w.write(u32::from(mode), 1);
    for &a in alloc {
        w.write(a, 3);
    }
    for (i, &a) in alloc.iter().enumerate() {
        if a != 0 {
            w.write(u32::from(sce.scaled[i].sfi), 6);
        }
    }
    for (i, &a) in alloc.iter().enumerate() {
        if a == 0 {
            continue;
        }
        let m = &mt[BLOCK_SIZE_TAB[i]..BLOCK_SIZE_TAB[i + 1]];
        if mode {
            clc_enc(a, m, Some(w));
        } else {
            vlc_enc(a, m, Some(w));
        }
    }
}

fn write_sound_unit(t: &Tables, mode: Mode, sces: &[Sce], loudness: f32) -> Vec<u8> {
    let half = i32::from(mode.frame_size >> 1);
    let mut writers = [BitWriter::new(), BitWriter::new()];
    let mut to_alloc = [-6i32, -6];
    for (ch, sce) in sces.iter().enumerate() {
        let w = &mut writers[ch];
        if mode.joint_stereo && ch == 1 {
            // no weighting, matrix coefficient 3 (M/S) in all four bands, then the unit id
            w.write(0, 1);
            w.write(7, 3);
            for _ in 0..4 {
                w.write(3, 2);
            }
            w.write(3, 2);
        } else {
            w.write(0x28, 6);
        }
        let bands = sce.gain.len().clamp(1, 4);
        w.write(bands as u32 - 1, 2);
        for points in sce.gain.iter().take(bands) {
            let points = &points[..points.len().min(MAX_GAIN_POINTS - 1)];
            w.write(points.len() as u32, 3);
            for p in points {
                w.write(p.level, 4);
                w.write(p.location, 5);
            }
        }
        to_alloc[ch] -= w.len_bits() as i32;
    }
    let shift = if mode.joint_stereo { ms_bytes_shift(i32::from(mode.frame_size), sces, &to_alloc) } else { 0 };
    to_alloc[0] += 8 * (half + shift);
    to_alloc[1] += 8 * (half - shift);
    let mut out = Vec::with_capacity(usize::from(mode.frame_size));
    let mut mt = [0i32; SAMPLES_PER_FRAME];
    for (ch, sce) in sces.iter().enumerate() {
        let (m, alloc) = create_allocation(t, sce, to_alloc[ch], &mut mt, loudness);
        let mut w = std::mem::take(&mut writers[ch]);
        encode_specs(sce, &mut w, m, &alloc, &mt);
        let mut bytes = w.into_bytes();
        if mode.joint_stereo && ch == 1 {
            bytes.resize((half - shift) as usize, 0);
            out.extend(bytes.iter().rev());
        } else {
            bytes.resize((half + shift) as usize, 0);
            out.extend_from_slice(&bytes);
        }
    }
    if sces.len() == 1 && !mode.joint_stereo {
        // no mono mode in ATRAC3: the half-frame twice
        let copy = out.clone();
        out.extend_from_slice(&copy);
    }
    out.resize(usize::from(mode.frame_size), 0);
    out
}

fn ms_ratio(m: f32, s: f32) -> f32 {
    let total = m + s;
    if total > 0.0 { m / total - 0.5 } else { 0.0 }
}

fn ms_bytes_shift(frame: i32, sces: &[Sce], b: &[i32; 2]) -> i32 {
    let used = (-b[0] - b[1]).max(1);
    let max = frame / 2 - (1 + (used - 1) / 8);
    if sces.get(1).is_none_or(|s| s.scaled.is_empty()) {
        return max;
    }
    let ratio = ms_ratio(sces[0].loudness, sces[1].loudness);
    psy::to_int(frame as f32 * ratio).clamp(-max, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_by_rate() {
        assert_eq!(mode_for_kbps(64).frame_size, 192);
        assert_eq!(mode_for_kbps(66).frame_size, 192);
        assert_eq!(mode_for_kbps(105).frame_size, 304);
        assert_eq!(mode_for_kbps(128).frame_size, 384);
        assert_eq!(mode_for_kbps(132).frame_size, 384);
        assert_eq!(mode_for_kbps(1000).frame_size, 384);
    }

    #[test]
    fn relation_index() {
        assert_eq!(relation_to_idx(1.0 / 2.5), 5);
        assert_eq!(relation_to_idx(0.0), 15);
        assert_eq!(relation_to_idx(1.0), 4);
    }

    #[test]
    fn frames_have_the_mode_size() {
        for m in MODES {
            for ch in 1..=2u16 {
                let mut e = Encoder::new(ch, m).unwrap();
                let pcm: Vec<f32> = (0..1024 * usize::from(ch)).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
                for _ in 0..3 {
                    assert_eq!(e.encode(&pcm).len(), usize::from(m.frame_size) / (3 - usize::from(ch)));
                }
                // hostile input never panics
                assert_eq!(e.encode(&[f32::NAN, f32::INFINITY, 1e30]).len(), e.block_align());
            }
        }
    }
}
