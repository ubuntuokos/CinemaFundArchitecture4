//! Layer II encoder (MP2), written from ISO/IEC 11172-3 (§2.4.1 syntax, §2.4.2 semantics,
//! Annex B tables shared with the decoder) and ISO/IEC 13818-3 (the lower sampling frequencies).
//!
//! The standard's encoding annexes (C and D) are informative; this encoder follows their
//! structure but not their text:
//!
//! - Analysis subband filter: the mirror of the synthesis filter (§2.4.3.2), a 512-tap window
//!   `C[i] = D[i] / 32` and a 64 → 32 cosine matrix, giving 36 samples per subband per frame.
//! - Scalefactors per third of the frame (the smallest one at least the peak), merged by the
//!   scalefactor selection information when neighbours differ by at most two steps (2 dB).
//! - A small psychoacoustic model on the subband energies: a spreading function in the Bark
//!   domain plus the absolute threshold of hearing gives a signal-to-mask ratio per subband.
//! - Bit allocation: greedy, always raising the subband with the lowest mask-to-noise ratio that
//!   still fits the frame.
//! - Quantization as the inverse of the decoder's `(2v + 1 - steps) / steps`, grouping for 3, 5
//!   and 9 steps, optional CRC (header bits 16-31, bit allocation and scalefactor selection),
//!   padding at 44.1 / 22.05 kHz so the average bit rate is exact.
//!
//! Mono, stereo (two channels coded independently) and dual channel. Joint (intensity) stereo
//! is not written yet (#878).

use std::sync::OnceLock;

use cfa4_bitstream::BitWriter;

use crate::tables::{self, AllocTable, SUBBANDS};
use crate::{Error, Result};

/// Samples per channel in one Layer II frame.
pub const FRAME_SAMPLES: usize = 1152;

/// Delay of the analysis filter followed by the decoder's synthesis filter, in samples: decoded
/// sample `n + DELAY` reproduces input sample `n`.
pub const DELAY: usize = 481;

/// How two channels are coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StereoMode {
    /// Left and right, each with its own allocation.
    Stereo,
    /// Two independent mono programmes (bilingual).
    DualChannel,
}

/// Encoder settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// 32 000, 44 100 or 48 000 (MPEG-1); 16 000, 22 050 or 24 000 (MPEG-2 lower sampling
    /// frequencies).
    pub sample_rate: u32,
    /// 1 or 2.
    pub channels: usize,
    /// A Layer II bit rate of the version (Table in §2.4.2.3); MPEG-1 allows 32-192 kbit/s for
    /// mono and 64-384 kbit/s (not 80) for two channels.
    pub bitrate_kbps: u32,
    /// Two channels only.
    pub stereo: StereoMode,
    /// Write the CRC after each header.
    pub crc: bool,
}

impl Config {
    /// Stereo for two channels, no CRC.
    pub fn new(sample_rate: u32, channels: usize, bitrate_kbps: u32) -> Self {
        Self { sample_rate, channels, bitrate_kbps, stereo: StereoMode::Stereo, crc: false }
    }
}

/// The bit rates (kbit/s) a Layer II stream may use with this sampling frequency and channel
/// count (empty for an unsupported sampling frequency).
pub fn bitrates(sample_rate: u32, channels: usize) -> Vec<u32> {
    let Some((lsf, _)) = rate_index(sample_rate) else { return Vec::new() };
    let table: &[u32] = if lsf { &tables::BITRATES_LSF_L23 } else { &tables::BITRATES_MPEG1_L2 };
    table.iter().copied().filter(|&k| k != 0 && (lsf || allowed_mpeg1(k, channels))).collect()
}

/// ISO/IEC 11172-3 §2.4.2.3: the Layer II bit rate / mode combinations allowed.
fn allowed_mpeg1(kbps: u32, channels: usize) -> bool {
    if channels == 1 { kbps <= 192 } else { kbps >= 64 && kbps != 80 }
}

/// (`lsf`, `sampling_frequency` code) of a sampling frequency.
fn rate_index(sample_rate: u32) -> Option<(bool, u32)> {
    match sample_rate {
        44_100 => Some((false, 0)),
        48_000 => Some((false, 1)),
        32_000 => Some((false, 2)),
        22_050 => Some((true, 0)),
        24_000 => Some((true, 1)),
        16_000 => Some((true, 2)),
        _ => None,
    }
}

/// A Layer II encoder. Feed planar samples at full scale ±1 with [`Encoder::encode`]; every
/// 1152 samples per channel make one frame. [`Encoder::flush`] pads the rest (and the filter
/// delay) with silence.
pub struct Encoder {
    cfg: Config,
    lsf: bool,
    rate_code: u32,
    bitrate_index: u32,
    table: AllocTable,
    /// Bytes of a frame without padding, and the padding accumulator (in units of 1/rate bytes).
    base_bytes: usize,
    pad_rest: u64,
    pad_acc: u64,
    /// The analysis filter's input history per channel, newest sample first.
    x: [[f32; 512]; 2],
    /// Samples waiting for a whole frame, per channel.
    pending: [Vec<f32>; 2],
}

impl Encoder {
    pub fn new(cfg: Config) -> Result<Self> {
        let (lsf, rate_code) = rate_index(cfg.sample_rate).ok_or(Error::Unsupported("Layer II encoding at this sampling frequency"))?;
        if !(1..=2).contains(&cfg.channels) {
            return Err(Error::Unsupported("Layer II encodes one or two channels"));
        }
        let rates: &[u32] = if lsf { &tables::BITRATES_LSF_L23 } else { &tables::BITRATES_MPEG1_L2 };
        let bitrate_index =
            rates.iter().position(|&k| k == cfg.bitrate_kbps && k != 0).ok_or(Error::Unsupported("not a Layer II bit rate for this sampling frequency"))?;
        if !lsf && !allowed_mpeg1(cfg.bitrate_kbps, cfg.channels) {
            return Err(Error::Unsupported("ISO/IEC 11172-3 does not allow this bit rate with this channel count"));
        }
        let per_channel = cfg.bitrate_kbps / cfg.channels as u32;
        let table = AllocTable::select(lsf, cfg.sample_rate, per_channel);
        // §2.4.3.1: 144 bytes per 1152 samples per bit/s over the sampling frequency
        let num = 144 * u64::from(cfg.bitrate_kbps) * 1000;
        let rate = u64::from(cfg.sample_rate);
        Ok(Self {
            cfg,
            lsf,
            rate_code,
            bitrate_index: bitrate_index as u32,
            table,
            base_bytes: (num / rate) as usize,
            pad_rest: num % rate,
            pad_acc: 0,
            x: [[0.0; 512]; 2],
            pending: [Vec::new(), Vec::new()],
        })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Encode planar samples (one slice per channel, equal lengths). Returns the frames that are
    /// complete, each starting with its header.
    pub fn encode(&mut self, channels: &[&[f32]]) -> Result<Vec<Vec<u8>>> {
        if channels.len() != self.cfg.channels {
            return Err(Error::Invalid("channel count differs from the configuration"));
        }
        let n = channels.first().map_or(0, |c| c.len());
        if channels.iter().any(|c| c.len() != n) {
            return Err(Error::Invalid("channels of different lengths"));
        }
        for (p, c) in self.pending.iter_mut().zip(channels) {
            p.extend_from_slice(c);
        }
        let mut out = Vec::new();
        while self.pending.first().is_some_and(|p| p.len() >= FRAME_SAMPLES) {
            out.push(self.frame());
        }
        Ok(out)
    }

    /// Encode what is left: the pending samples and the filter delay, padded with silence to
    /// whole frames.
    pub fn flush(&mut self) -> Vec<Vec<u8>> {
        let nch = self.cfg.channels;
        let have = self.pending.first().map_or(0, Vec::len);
        let total = (have + DELAY).div_ceil(FRAME_SAMPLES) * FRAME_SAMPLES;
        for p in self.pending.iter_mut().take(nch) {
            p.resize(total, 0.0);
        }
        let mut out = Vec::new();
        while self.pending.first().is_some_and(|p| p.len() >= FRAME_SAMPLES) {
            out.push(self.frame());
        }
        out
    }

    /// Encode the first [`FRAME_SAMPLES`] pending samples per channel.
    fn frame(&mut self) -> Vec<u8> {
        let nch = self.cfg.channels;
        // subband samples [channel][time slot][subband]
        let mut sb = [[[0f32; SUBBANDS]; 36]; 2];
        for ch in 0..nch {
            let (Some(p), Some(x), Some(out)) = (self.pending.get_mut(ch), self.x.get_mut(ch), sb.get_mut(ch)) else { continue };
            let take = FRAME_SAMPLES.min(p.len());
            for (slot, chunk) in out.iter_mut().zip(p[..take].chunks(32)) {
                analyse(x, chunk, slot);
            }
            p.drain(..take);
        }
        // padding keeps the average frame length at 144 · bit rate / sampling frequency
        self.pad_acc += self.pad_rest;
        let padding = self.pad_acc >= u64::from(self.cfg.sample_rate);
        if padding {
            self.pad_acc -= u64::from(self.cfg.sample_rate);
        }
        let frame_bytes = self.base_bytes + usize::from(padding);
        self.write_frame(&sb, padding, frame_bytes)
    }

    fn write_frame(&self, sb: &[[[f32; SUBBANDS]; 36]; 2], padding: bool, frame_bytes: usize) -> Vec<u8> {
        let nch = self.cfg.channels;
        let table = self.table;
        let sblimit = table.sblimit().min(SUBBANDS);
        // scalefactors per third and their selection
        let mut scf = [[[0u32; 3]; SUBBANDS]; 2];
        let mut scfsi = [[0u32; SUBBANDS]; 2];
        for ch in 0..nch {
            for s in 0..sblimit {
                let parts: [u32; 3] = std::array::from_fn(|p| {
                    let peak = (0..12).map(|k| sb[ch][12 * p + k][s].abs()).fold(0f32, f32::max);
                    scalefactor_index(peak)
                });
                let (sel, merged) = select_scalefactors(parts);
                scfsi[ch][s] = sel;
                scf[ch][s] = merged;
            }
        }
        let smr = psychoacoustics(sb, nch, self.cfg.sample_rate, sblimit);
        // bits: header, CRC, allocation fields, then scalefactor selection, scalefactors and
        // samples out of what is left
        let nbal: Vec<u32> = (0..sblimit).map(|s| table.row(s).len().trailing_zeros()).collect();
        let fixed = 32 + if self.cfg.crc { 16 } else { 0 } + nch as u32 * nbal.iter().sum::<u32>();
        let mut avail = (frame_bytes as i64 * 8) - i64::from(fixed);
        let mut alloc = [[0usize; SUBBANDS]; 2];
        let scf_bits = |ch: usize, s: usize| -> i64 { 2 + 6 * i64::from(scf_count(scfsi[ch][s])) };
        loop {
            let mut best: Option<(f32, usize, usize, i64)> = None;
            for ch in 0..nch {
                for s in 0..sblimit {
                    let row = table.row(s);
                    let a = alloc[ch][s];
                    let Some(&next) = row.get(a + 1) else { continue };
                    let cur = row.get(a).copied().unwrap_or(0);
                    let cost = sample_bits(next) - sample_bits(cur) + if a == 0 { scf_bits(ch, s) } else { 0 };
                    if cost > avail {
                        continue;
                    }
                    let mnr = snr_db(cur) - smr[ch][s];
                    if best.is_none_or(|b| mnr < b.0) {
                        best = Some((mnr, ch, s, cost));
                    }
                }
            }
            let Some((_, ch, s, cost)) = best else { break };
            alloc[ch][s] += 1;
            avail -= cost;
        }
        // the frame
        let mut w = BitWriter::new();
        let header = self.header(padding);
        w.write_bits(header, 32);
        let mut protected = BitWriter::new();
        protected.write_bits(header & 0xFFFF, 16);
        for (s, &nb) in nbal.iter().enumerate() {
            for a in alloc.iter().take(nch) {
                protected.write_bits(a[s] as u32, nb);
            }
        }
        for s in 0..sblimit {
            for ch in 0..nch {
                if alloc[ch][s] != 0 {
                    protected.write_bits(scfsi[ch][s], 2);
                }
            }
        }
        let protected_bits = protected.bit_len();
        let protected = protected.finish();
        if self.cfg.crc {
            w.write_bits(crc16(&protected, protected_bits), 16);
        }
        // the protected fields after the header's 16 bits
        let mut r = cfa4_bitstream::BitReader::new(&protected);
        let _ = r.skip(16);
        let mut left = protected_bits.saturating_sub(16);
        while left > 0 {
            let n = left.min(16) as u32;
            w.write_bits(r.read_bits(n).unwrap_or(0), n);
            left -= n as usize;
        }
        for s in 0..sblimit {
            for ch in 0..nch {
                if alloc[ch][s] == 0 {
                    continue;
                }
                let [a, b, c] = scf[ch][s];
                match scfsi[ch][s] {
                    0 => [a, b, c].iter().for_each(|&v| w.write_bits(v, 6)),
                    1 => [a, c].iter().for_each(|&v| w.write_bits(v, 6)),
                    2 => w.write_bits(a, 6),
                    _ => [a, b].iter().for_each(|&v| w.write_bits(v, 6)),
                }
            }
        }
        for gr in 0..12 {
            let part = gr / 4;
            for s in 0..sblimit {
                let row = table.row(s);
                for ch in 0..nch {
                    let steps = row.get(alloc[ch][s]).copied().unwrap_or(0);
                    if steps == 0 {
                        continue;
                    }
                    let scale = tables::scalefactor(scf[ch][s][part]);
                    let v: [u32; 3] = std::array::from_fn(|k| quantize(sb[ch][3 * gr + k][s], scale, steps));
                    let st = u32::from(steps);
                    match tables::grouping(steps) {
                        Some(bits) => w.write_bits(v[0] + st * (v[1] + st * v[2]), bits),
                        None => {
                            let nb = (st + 1).trailing_zeros();
                            v.iter().for_each(|&x| w.write_bits(x, nb));
                        }
                    }
                }
            }
        }
        let mut out = w.finish();
        out.resize(frame_bytes, 0);
        out
    }

    /// The 32-bit header (§2.4.1.3): Layer II, no private / copyright bits, original, no
    /// emphasis.
    fn header(&self, padding: bool) -> u32 {
        let mode = match (self.cfg.channels, self.cfg.stereo) {
            (1, _) => 3,
            (_, StereoMode::Stereo) => 0,
            (_, StereoMode::DualChannel) => 2,
        };
        (0xFFF << 20)
            | (u32::from(!self.lsf) << 19)
            | (2 << 17)
            | (u32::from(!self.cfg.crc) << 16)
            | (self.bitrate_index << 12)
            | (self.rate_code << 10)
            | (u32::from(padding) << 9)
            | (mode << 6)
            | (1 << 2)
    }
}

/// The analysis window `C[i]`: the synthesis window `D[i]` (Table 3-B.3) over 32.
fn analysis_window() -> &'static [f32; 512] {
    static C: OnceLock<[f32; 512]> = OnceLock::new();
    C.get_or_init(|| tables::window().map(|d| d / 32.0))
}

/// The analysis matrix `M[i][k] = cos((2i + 1)(k - 16)π / 64)`, `i` in `0..32`, `k` in `0..64`.
fn analysis_matrix() -> &'static [[f32; 64]; 32] {
    static M: OnceLock<[[f32; 64]; 32]> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = [[0f32; 64]; 32];
        for (i, row) in m.iter_mut().enumerate() {
            for (k, c) in row.iter_mut().enumerate() {
                *c = (((2 * i + 1) as f64) * (k as f64 - 16.0) * std::f64::consts::PI / 64.0).cos() as f32;
            }
        }
        m
    })
}

/// One step of the analysis subband filter: 32 new input samples (oldest first) shifted into
/// `x`, 32 subband samples out.
fn analyse(x: &mut [f32; 512], input: &[f32], out: &mut [f32; SUBBANDS]) {
    x.copy_within(0..480, 32);
    for (i, xi) in x.iter_mut().take(32).enumerate() {
        // the newest sample goes to X[0]
        *xi = input.get(31 - i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
    }
    let c = analysis_window();
    let mut y = [0f32; 64];
    for (i, yi) in y.iter_mut().enumerate() {
        let mut acc = 0f32;
        for j in 0..8 {
            acc += c[i + 64 * j] * x[i + 64 * j];
        }
        *yi = acc;
    }
    for (o, row) in out.iter_mut().zip(analysis_matrix()) {
        *o = row.iter().zip(&y).map(|(a, b)| a * b).sum();
    }
}

/// The scalefactor index for a peak: the smallest scalefactor at least the peak (index 0, 2.0,
/// when the peak is beyond it).
fn scalefactor_index(peak: f32) -> u32 {
    let mut i = 62u32;
    while i > 0 && tables::scalefactor(i) < peak {
        i -= 1;
    }
    i
}

/// The scalefactor selection information and the scalefactors each third then uses: thirds
/// within two steps of each other share the larger scalefactor.
fn select_scalefactors([a, b, c]: [u32; 3]) -> (u32, [u32; 3]) {
    let near = |p: u32, q: u32| p.abs_diff(q) <= 2;
    if near(a, b) && near(b, c) && near(a, c) {
        let m = a.min(b).min(c);
        (2, [m, m, m])
    } else if near(a, b) {
        let m = a.min(b);
        (1, [m, m, c])
    } else if near(b, c) {
        let m = b.min(c);
        (3, [a, m, m])
    } else {
        (0, [a, b, c])
    }
}

/// Scalefactors transmitted for a selection value.
fn scf_count(scfsi: u32) -> u32 {
    match scfsi {
        0 => 3,
        2 => 1,
        _ => 2,
    }
}

/// Bits of the 36 samples of a subband at `steps` quantization steps.
fn sample_bits(steps: u16) -> i64 {
    match steps {
        0 => 0,
        s => match tables::grouping(s) {
            Some(bits) => 12 * i64::from(bits),
            None => 36 * i64::from((u32::from(s) + 1).trailing_zeros()),
        },
    }
}

/// The signal-to-noise ratio a uniform quantizer of `steps` levels gives a full-scale signal.
fn snr_db(steps: u16) -> f32 {
    if steps == 0 { 0.0 } else { 20.0 * f32::from(steps).log10() }
}

/// The code of a subband sample: the inverse of the decoder's `(2v + 1 - steps) / steps`.
fn quantize(s: f32, scale: f32, steps: u16) -> u32 {
    let x = if scale > 0.0 { (s / scale).clamp(-1.0, 1.0) } else { 0.0 };
    let v = ((x + 1.0) * f32::from(steps) / 2.0).floor();
    (v.max(0.0) as u32).min(u32::from(steps) - 1)
}

/// CRC-16 (§2.4.3.1: generator x^16 + x^15 + x^2 + 1, initial state all ones) over the first
/// `bits` bits of `data`.
fn crc16(data: &[u8], bits: usize) -> u32 {
    let mut crc = 0xFFFFu32;
    for i in 0..bits {
        let bit = u32::from(data.get(i / 8).is_some_and(|b| b >> (7 - i % 8) & 1 == 1));
        let top = (crc >> 15) & 1;
        crc = (crc << 1) & 0xFFFF;
        if top ^ bit == 1 {
            crc ^= 0x8005;
        }
    }
    crc
}

/// Bark scale (Zwicker): critical-band rate of a frequency in Hz.
fn bark(f: f32) -> f32 {
    13.0 * (0.00076 * f).atan() + 3.5 * (f / 7500.0).powi(2).atan()
}

/// The absolute threshold of hearing (Terhardt) in dB relative to a full-scale sine, taking a
/// full-scale sine as 96 dB SPL.
fn hearing_threshold_db(f: f32) -> f32 {
    let k = (f / 1000.0).max(0.02);
    let spl = 3.64 * k.powf(-0.8) - 6.5 * (-0.6 * (k - 3.3) * (k - 3.3)).exp() + 1e-3 * k.powi(4);
    spl.min(96.0) - 96.0
}

/// Signal-to-mask ratio per channel and subband (dB) from the subband energies: each subband
/// masks itself and its neighbours through a spreading function (25 dB per Bark downwards, 10
/// upwards, 14.5 + z dB below the masker: every masker taken as tonal, the cautious choice),
/// floored by the threshold of hearing.
fn psychoacoustics(sb: &[[[f32; SUBBANDS]; 36]; 2], nch: usize, sample_rate: u32, sblimit: usize) -> [[f32; SUBBANDS]; 2] {
    let width = sample_rate as f32 / 64.0;
    let z: [f32; SUBBANDS] = std::array::from_fn(|s| bark((s as f32 + 0.5) * width));
    // the quietest point of the band's hearing threshold (it falls towards 3-4 kHz)
    let ath: [f32; SUBBANDS] = std::array::from_fn(|s| {
        let (lo, hi) = (s as f32 * width, (s as f32 + 1.0) * width);
        (0..=8).map(|k| hearing_threshold_db(lo + (hi - lo) * k as f32 / 8.0)).fold(f32::MAX, f32::min)
    });
    let mut smr = [[0f32; SUBBANDS]; 2];
    for ch in 0..nch {
        // energy per subband relative to a full-scale sine (power 1/2)
        let level: [f32; SUBBANDS] = std::array::from_fn(|s| {
            let e = (0..36).map(|t| sb[ch][t][s] * sb[ch][t][s]).sum::<f32>() / 36.0;
            10.0 * (e / 0.5 + 1e-12).log10()
        });
        for s in 0..sblimit {
            let mut mask = 0f32;
            for j in 0..sblimit {
                let dz = z[s] - z[j];
                let spread = if dz >= 0.0 { 10.0 * dz } else { -25.0 * dz };
                mask += 10f32.powf((level[j] - (14.5 + z[j]) - spread) / 10.0);
            }
            let mask_db = (10.0 * mask.max(1e-30).log10()).max(ath[s]);
            smr[ch][s] = level[s] - mask_db;
        }
    }
    smr
}

#[cfg(test)]
mod tests;
