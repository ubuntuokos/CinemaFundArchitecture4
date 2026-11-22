//! ATRAC1 (Sony's Adaptive TRansform Acoustic Coding, as on MiniDisc; `.aea`, Matroska
//! `A_ATRAC/AT1`): a three-band QMF split (0 to 5.5, 5.5 to 11 and 11 to 22 kHz), a long or
//! short MDCT per band and block floating units. A port of FFmpeg n7.1.1 `libavcodec/atrac1.c`,
//! `atrac1data.h` and `atrac.c` (Copyright (c) 2009 Maxim Poliakovski, Benjamin Larsson;
//! LGPL-2.1-or-later). Output: planar `f32` in `[-1, 1]`, 512 samples per channel and sound unit.
//! A packet is one 212-byte sound unit per channel (mono or stereo).

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::bits::Bits;
use crate::mdct::Imdct;
use crate::{Error, Result};

/// Bytes of one sound unit (one channel).
pub const SU_SIZE: usize = 212;
/// Samples of one sound unit.
pub const SU_SAMPLES: usize = 512;
const MAX_BFU: usize = 52;
const SU_MAX_BITS: usize = SU_SIZE * 8;
const MAX_CHANNELS: usize = 2;
const HIGH_BAND: usize = 2;

const BFU_AMOUNT_TAB1: [u8; 8] = [20, 28, 32, 36, 40, 44, 48, 52];
const BFU_AMOUNT_TAB2: [u16; 4] = [0, 112, 176, 208];
const BFU_AMOUNT_TAB3: [u16; 8] = [0, 24, 36, 48, 72, 108, 132, 156];
/// Number of BFUs in each QMF band (cumulative).
const BFU_BANDS_T: [usize; 4] = [0, 20, 36, 52];
/// Spectral lines in each block floating unit.
const SPECS_PER_BFU: [u8; MAX_BFU] = [
    8, 8, 8, 8, 4, 4, 4, 4, 8, 8, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6, // low band
    6, 6, 6, 6, 7, 7, 7, 7, 9, 9, 9, 9, 10, 10, 10, 10, // middle band
    12, 12, 12, 12, 12, 12, 12, 12, 20, 20, 20, 20, 20, 20, 20, 20, // high band
];
/// Start of each BFU in the MDCT spectrum, long mode.
const BFU_START_LONG: [u16; MAX_BFU] = [
    0, 8, 16, 24, 32, 36, 40, 44, 48, 56, 64, 72, 80, 86, 92, 98, 104, 110, 116, 122, //
    128, 134, 140, 146, 152, 159, 166, 173, 180, 189, 198, 207, 216, 226, 236, 246, //
    256, 268, 280, 292, 304, 316, 328, 340, 352, 372, 392, 412, 432, 452, 472, 492,
];
/// Start of each BFU in the MDCT spectrum, short mode.
const BFU_START_SHORT: [u16; MAX_BFU] = [
    0, 32, 64, 96, 8, 40, 72, 104, 12, 44, 76, 108, 20, 52, 84, 116, 26, 58, 90, 122, //
    128, 160, 192, 224, 134, 166, 198, 230, 141, 173, 205, 237, 150, 182, 214, 246, //
    256, 288, 320, 352, 384, 416, 448, 480, 268, 300, 332, 364, 396, 428, 460, 492,
];
/// Transform size in samples in long mode for each QMF band.
const SAMPLES_PER_BAND: [usize; 3] = [128, 128, 256];
const MDCT_LONG_NBITS: [u32; 3] = [7, 7, 8];

/// `qmf_48tap_half`.
#[allow(clippy::excessive_precision)]
const QMF_48TAP_HALF: [f32; 24] = [
    -0.000_014_619_07,
    -0.000_092_054_79,
    -0.000_056_157_569,
    0.000_301_172_69,
    0.000_242_251_9,
    -0.000_852_938_97,
    -0.000_520_557_4,
    0.002_034_016_9,
    0.000_783_338_91,
    -0.004_215_386_2,
    -0.000_756_149_88,
    0.007_840_294_4,
    -0.000_061_169_922,
    -0.013_441_62,
    0.002_462_682_1,
    0.021_736_089,
    -0.007_801_671,
    -0.034_090_221,
    0.018_809_49,
    0.054_326_009,
    -0.043_596_379,
    -0.099_384_367,
    0.132_079_09,
    0.464_241_59,
];

struct Tables {
    /// `ff_atrac_sf_table`.
    sf: [f32; 64],
    /// `qmf_window`.
    qmf: [f32; 48],
    /// `ff_sine_32`.
    sine32: [f32; 32],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut sf = [0f32; 64];
        for (i, s) in sf.iter_mut().enumerate() {
            *s = 2f64.powf((i as f64 - 15.0) / 3.0) as f32;
        }
        let mut qmf = [0f32; 48];
        for (i, &h) in QMF_48TAP_HALF.iter().enumerate() {
            let s = (f64::from(h) * 2.0) as f32;
            if let Some(a) = qmf.get_mut(i) {
                *a = s;
            }
            if let Some(a) = qmf.get_mut(47 - i) {
                *a = s;
            }
        }
        let mut sine32 = [0f32; 32];
        for (i, s) in sine32.iter_mut().enumerate() {
            *s = ((i as f64 + 0.5) / 64.0 * PI).sin() as f32;
        }
        Tables { sf, qmf, sine32 }
    })
}

fn at<T: Copy + Default>(a: &[T], i: usize) -> T {
    a.get(i).copied().unwrap_or_default()
}

fn put<T>(a: &mut [T], i: usize, v: T) {
    if let Some(x) = a.get_mut(i) {
        *x = v;
    }
}

/// `vector_fmul_window`: `2 · len` samples overlapping `src0` and `src1` under `win`.
fn fmul_window(dst: &mut [f32], src0: &[f32], src1: &[f32], win: &[f32], len: usize) {
    for k in 0..len {
        let j = len - 1 - k;
        let s0 = at(src0, k);
        let s1 = at(src1, j);
        let wi = at(win, k);
        let wj = at(win, len + j);
        put(dst, k, s0 * wj - s1 * wi);
        put(dst, len + j, s0 * wi + s1 * wj);
    }
}

/// `ff_atrac_iqmf`: combine a low and a high band of `n_in` samples each into `2 · n_in`.
fn iqmf(t: &Tables, inlo: &[f32], inhi: &[f32], n_in: usize, out: &mut [f32], delay: &mut [f32; 46]) {
    let mut temp = vec![0f32; 46 + 2 * n_in];
    temp[..46].copy_from_slice(delay);
    for i in (0..n_in).step_by(2) {
        let (l0, h0, l1, h1) = (at(inlo, i), at(inhi, i), at(inlo, i + 1), at(inhi, i + 1));
        put(&mut temp, 46 + 2 * i, l0 + h0);
        put(&mut temp, 46 + 2 * i + 1, l0 - h0);
        put(&mut temp, 46 + 2 * i + 2, l1 + h1);
        put(&mut temp, 46 + 2 * i + 3, l1 - h1);
    }
    for j in 0..n_in {
        let mut s1 = 0f32;
        let mut s2 = 0f32;
        for i in (0..48).step_by(2) {
            s1 += at(&temp, j * 2 + i) * at(&t.qmf, i);
            s2 += at(&temp, j * 2 + i + 1) * at(&t.qmf, i + 1);
        }
        put(out, 2 * j, s2);
        put(out, 2 * j + 1, s1);
    }
    delay.copy_from_slice(&temp[n_in * 2..n_in * 2 + 46]);
}

/// The state of one channel's sound unit (`AT1SUCtx`).
#[derive(Clone)]
struct Unit {
    log2_block_count: [usize; 3],
    /// The MDCT output of this and of the previous frame (`spectrum[0]`, `spectrum[1]`).
    spectrum: [Vec<f32>; 2],
    fst_qmf_delay: [f32; 46],
    snd_qmf_delay: [f32; 46],
    last_qmf_delay: Vec<f32>,
}

impl Unit {
    fn new() -> Self {
        Self {
            log2_block_count: [0; 3],
            spectrum: [vec![0.0; SU_SAMPLES], vec![0.0; SU_SAMPLES]],
            fst_qmf_delay: [0.0; 46],
            snd_qmf_delay: [0.0; 46],
            last_qmf_delay: vec![0.0; 256 + 39],
        }
    }
}

/// An ATRAC1 decoder (the overlap and filter bank memories carry from unit to unit).
#[derive(Clone)]
pub struct Decoder {
    channels: usize,
    units: Vec<Unit>,
    /// The spectrum buffer shared by the channels (`q->spec`).
    spec: Vec<f32>,
    /// `low`, `mid` and `high` (with room for the long mode's copy past the band).
    bands: [Vec<f32>; 3],
    imdct32: Imdct,
    imdct128: Imdct,
    imdct256: Imdct,
}

impl Decoder {
    /// A decoder for 1 or 2 channels; `block_align` is the packet size (212 bytes per channel).
    pub fn new(channels: usize, block_align: usize) -> Result<Self> {
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(Error::Unsupported("ATRAC1 channel count"));
        }
        if block_align == 0 {
            return Err(Error::Unsupported("ATRAC1 block alignment"));
        }
        // FFmpeg passes -1/32768 to av_tx, whose sign convention is the opposite of ours
        let scale = 1.0 / 32768.0;
        Ok(Self {
            channels,
            units: (0..channels).map(|_| Unit::new()).collect(),
            spec: vec![0.0; SU_SAMPLES],
            bands: [vec![0.0; 256 + 16], vec![0.0; 256 + 16], vec![0.0; 512]],
            imdct32: Imdct::new(32, scale),
            imdct128: Imdct::new(128, scale),
            imdct256: Imdct::new(256, scale),
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Forgets the overlap and filter bank memories, as a new decoder has them.
    pub fn reset(&mut self) {
        self.units = (0..self.channels).map(|_| Unit::new()).collect();
    }

    /// Bytes of one packet.
    pub fn block_align(&self) -> usize {
        SU_SIZE * self.channels
    }

    pub fn samples_per_channel(&self) -> usize {
        SU_SAMPLES
    }

    /// Decodes one packet (`212 · channels` bytes or more) into planar samples. A damaged unit
    /// is an error; the channels before it have already advanced, as in FFmpeg.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<Vec<f32>>> {
        if packet.len() < SU_SIZE * self.channels {
            return Err(Error::Invalid("ATRAC1 packet too short"));
        }
        let t = tables();
        let mut out = vec![vec![0f32; SU_SAMPLES]; self.channels];
        for ch in 0..self.channels {
            let Some(data) = packet.get(SU_SIZE * ch..SU_SIZE * (ch + 1)) else { return Err(Error::Invalid("ATRAC1 packet too short")) };
            let mut b = Bits::new(data, SU_SIZE * 8);
            let Some(su) = self.units.get_mut(ch) else { return Err(Error::Invalid("ATRAC1 channel")) };
            parse_bsm(&mut b, &mut su.log2_block_count)?;
            unpack_dequant(t, &mut b, su, &mut self.spec)?;
            // the three-band IMDCT with overlap
            let mut ref_pos = 0usize;
            let mut pos = 0usize;
            for band in 0..3 {
                let band_samples = at(&SAMPLES_PER_BAND, band);
                let log2 = su.log2_block_count[band];
                let num_blocks = 1usize << log2;
                let (block_size, nbits) = if num_blocks == 1 { (band_samples, at(&MDCT_LONG_NBITS, band)) } else { (32, 5) };
                let mut start_pos = 0usize;
                let mut prev: Vec<f32> = su.spectrum[1].get(ref_pos + band_samples - 16..ref_pos + band_samples).unwrap_or(&[]).to_vec();
                for _ in 0..num_blocks {
                    // the spectrum block, reversed for the odd bands
                    let size = 1usize << nbits;
                    let mut blk: Vec<f32> = self.spec.get(pos..pos + size).unwrap_or(&[]).to_vec();
                    if band != 0 {
                        blk.reverse();
                    }
                    let tx = match nbits {
                        5 => &self.imdct32,
                        7 => &self.imdct128,
                        _ => &self.imdct256,
                    };
                    let mut full = vec![0f32; 2 * size];
                    tx.inverse(&blk, &mut full);
                    // the half inverse MDCT: the middle `size` samples of the full one
                    let half = full.get(size / 2..size / 2 + size).unwrap_or(&[]);
                    let dst_off = ref_pos + start_pos;
                    if let Some(d) = su.spectrum[0].get_mut(dst_off..dst_off + size) {
                        d.copy_from_slice(half);
                    }
                    let cur = su.spectrum[0].get(dst_off..dst_off + 32).unwrap_or(&[]).to_vec();
                    let Some(bd) = self.bands.get_mut(band) else { return Err(Error::Invalid("ATRAC1 band")) };
                    if let Some(dst) = bd.get_mut(start_pos..start_pos + 32) {
                        fmul_window(dst, &prev, &cur, &t.sine32, 16);
                    }
                    prev = su.spectrum[0].get(dst_off + 16..dst_off + 32).unwrap_or(&[]).to_vec();
                    start_pos += block_size;
                    pos += block_size;
                }
                if num_blocks == 1 {
                    // 240 samples, as far as the band's buffer reaches
                    let n = 240.min(su.spectrum[0].len().saturating_sub(ref_pos + 16));
                    if let (Some(bd), Some(src)) = (self.bands.get_mut(band), su.spectrum[0].get(ref_pos + 16..ref_pos + 16 + n)) {
                        let n = n.min(bd.len().saturating_sub(32));
                        if let Some(d) = bd.get_mut(32..32 + n) {
                            d.copy_from_slice(src.get(..n).unwrap_or(&[]));
                        }
                    }
                }
                ref_pos += band_samples;
            }
            su.spectrum.swap(0, 1);
            // subband synthesis
            let mut temp = vec![0f32; 256];
            iqmf(t, &self.bands[0], &self.bands[1], 128, &mut temp, &mut su.fst_qmf_delay);
            su.last_qmf_delay.copy_within(256..256 + 39, 0);
            if let Some(d) = su.last_qmf_delay.get_mut(39..39 + 256) {
                d.copy_from_slice(self.bands.get(HIGH_BAND).and_then(|h| h.get(..256)).unwrap_or(&[0.0; 256]));
            }
            if let Some(o) = out.get_mut(ch) {
                iqmf(t, &temp, &su.last_qmf_delay, 256, o, &mut su.snd_qmf_delay);
            }
        }
        Ok(out)
    }
}

/// `at1_parse_bsm`: the block size mode byte.
fn parse_bsm(b: &mut Bits<'_>, log2: &mut [usize; 3]) -> Result<()> {
    for i in 0..2 {
        let t = b.get(2) as usize;
        if t & 1 != 0 {
            return Err(Error::Invalid("ATRAC1 block size mode"));
        }
        put(log2, i, 2 - t);
    }
    let t = b.get(2) as usize;
    if t != 0 && t != 3 {
        return Err(Error::Invalid("ATRAC1 block size mode"));
    }
    put(log2, HIGH_BAND, 3 - t);
    b.skip(2);
    Ok(())
}

/// `at1_unpack_dequant`: word lengths, scale factors and the quantized spectrum of one unit.
fn unpack_dequant(t: &Tables, b: &mut Bits<'_>, su: &Unit, spec: &mut [f32]) -> Result<()> {
    let num_bfus = usize::from(at(&BFU_AMOUNT_TAB1, b.get(3) as usize));
    let mut bits_used = num_bfus * 10 + 32 + usize::from(at(&BFU_AMOUNT_TAB2, b.get(2) as usize)) + (usize::from(at(&BFU_AMOUNT_TAB3, b.get(3) as usize)) << 1);
    let mut idwls = [0u32; MAX_BFU];
    let mut idsfs = [0usize; MAX_BFU];
    for w in idwls.iter_mut().take(num_bfus) {
        *w = b.get(4);
    }
    for s in idsfs.iter_mut().take(num_bfus) {
        *s = b.get(6) as usize;
    }
    for band in 0..3 {
        for bfu in at(&BFU_BANDS_T, band)..at(&BFU_BANDS_T, band + 1) {
            let num_specs = usize::from(at(&SPECS_PER_BFU, bfu));
            let idwl = at(&idwls, bfu);
            let word_len = if idwl != 0 { idwl + 1 } else { 0 };
            let scale_factor = at(&t.sf, at(&idsfs, bfu));
            bits_used += word_len as usize * num_specs;
            if bits_used > SU_MAX_BITS {
                return Err(Error::Invalid("ATRAC1 sound unit overflow"));
            }
            let start = usize::from(if at(&su.log2_block_count, band) != 0 { at(&BFU_START_SHORT, bfu) } else { at(&BFU_START_LONG, bfu) });
            if word_len != 0 {
                let max_quant = (1.0 / f64::from((1u32 << (word_len - 1)) - 1)) as f32;
                for i in 0..num_specs {
                    // a signed `word_len`-bit value
                    let raw = b.get(word_len);
                    let v = ((raw << (32 - word_len)) as i32) >> (32 - word_len);
                    put(spec, start + i, v as f32 * scale_factor * max_quant);
                }
            } else {
                for i in 0..num_specs {
                    put(spec, start + i, 0.0);
                }
            }
        }
    }
    Ok(())
}
