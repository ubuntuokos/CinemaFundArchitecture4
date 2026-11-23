//! QDesign Music Codec 2 (QDM2) audio decoder, codec backlog CB-36.2. Ported from FFmpeg n7.1.1
//! (`libavcodec/qdm2.c`, `qdm2data.h`, `qdm2_tablegen.h`, the MPEG audio synthesis filter of
//! `mpegaudiodsp_template.c` and `dct32_template.c`); LGPL-2.1-or-later, see `COPYING.LGPL-2.1`.
//!
//! A packet ("block") of `checksum_size` bytes holds 16 sub-packets of sound. The sound is the
//! sum of an FFT part (tones) and an MPEG-audio-like 32-band synthesis filter part.

mod data;
mod mpa;
mod vlc;

use data::*;
use vlc::{Bits, Vlc};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("QDM2: {0}")]
    Invalid(String),
}

const SOFTCLIP_THRESHOLD: i32 = 27600;
const HARDCLIP_THRESHOLD: i32 = 35716;
const SBLIMIT: usize = 32;
const MAX_FRAME_SIZE: usize = 512;

type SbInt8 = [[[i8; 64]; 30]; 2];

/// `QDM2_SB_USED`: 8, 16 or 30.
fn sb_used_of(sub_sampling: usize) -> usize {
    if sub_sampling >= 2 { 30 } else { 8 << sub_sampling }
}

const SWITCHTABLE: [i32; 23] = [0, 5, 1, 5, 5, 5, 5, 5, 2, 5, 5, 5, 5, 5, 5, 5, 3, 5, 5, 5, 5, 5, 4];

/// The tables that do not depend on the file.
struct Tables {
    softclip: Vec<u16>,
    noise_table: Vec<f32>,
    noise_samples: [f32; 128],
    random_dequant_index: [[u8; 5]; 256],
    random_dequant_type24: [[u8; 3]; 128],
    level: Vlc,
    diff: Vlc,
    run: Vlc,
    fft_level_exp_alt: Vlc,
    fft_level_exp: Vlc,
    fft_stereo_exp: Vlc,
    fft_stereo_phase: Vlc,
    hi1: Vlc,
    mid: Vlc,
    hi2: Vlc,
    type30: Vlc,
    type34: Vlc,
    fft_tone_offset: Vec<Vlc>,
    window: Vec<f32>,
}

impl Tables {
    fn new() -> Self {
        // softclip_table_init
        let dfl = f64::from(SOFTCLIP_THRESHOLD - 32767);
        let delta = (1.0 / -dfl) as f32;
        let softclip = (0..=(HARDCLIP_THRESHOLD - SOFTCLIP_THRESHOLD))
            .map(|i| (SOFTCLIP_THRESHOLD - ((((i as f32 * delta) as f64).sin() * dfl) as i32 & 0xFFFF)) as u16)
            .collect();
        // rnd_table_init
        let mut noise_table = vec![0.0f32; 4096 + 20];
        let mut seed = 0u64;
        let delta = 1.0f32 / 16384.0;
        for v in noise_table.iter_mut().take(4096) {
            seed = seed.wrapping_mul(214013).wrapping_add(2531011);
            *v = (((f64::from(delta * (((seed as i32) >> 16) & 0x7FFF) as f32)) - 1.0) * 1.3) as f32;
        }
        let mut random_dequant_index = [[0u8; 5]; 256];
        for (i, row) in random_dequant_index.iter_mut().enumerate() {
            let (mut seed, mut ldw) = (81u32, i as u32);
            for v in row.iter_mut() {
                *v = (ldw / seed) as u8;
                ldw %= seed;
                seed /= 3;
            }
        }
        let mut random_dequant_type24 = [[0u8; 3]; 128];
        for (i, row) in random_dequant_type24.iter_mut().enumerate() {
            let (mut seed, mut ldw) = (25u32, i as u32);
            for v in row.iter_mut() {
                *v = (ldw / seed) as u8;
                ldw %= seed;
                seed /= 5;
            }
        }
        // init_noise_samples
        let mut noise_samples = [0.0f32; 128];
        let mut seed = 0u32;
        for v in &mut noise_samples {
            seed = seed.wrapping_mul(214013).wrapping_add(2531011);
            *v = (f64::from(delta * ((seed >> 16) & 0x7fff) as f32) - 1.0) as f32;
        }
        // qdm2_init_vlc
        let mut off = 0usize;
        let mut fft_tone_offset = Vec::new();
        for &n in &TAB_FFT_TONE_OFFSET_SIZES {
            fft_tone_offset.push(Vlc::new(&TAB_FFT_TONE_OFFSET[off..off + usize::from(n)]));
            off += usize::from(n);
        }
        Self {
            softclip,
            noise_table,
            noise_samples,
            random_dequant_index,
            random_dequant_type24,
            level: Vlc::new(&TAB_LEVEL),
            diff: Vlc::new(&TAB_DIFF),
            run: Vlc::new(&TAB_RUN),
            fft_level_exp_alt: Vlc::new(&FFT_LEVEL_EXP_ALT),
            fft_level_exp: Vlc::new(&FFT_LEVEL_EXP),
            fft_stereo_exp: Vlc::new(&FFT_STEREO_EXP),
            fft_stereo_phase: Vlc::new(&FFT_STEREO_PHASE),
            hi1: Vlc::new(&TAB_TONE_LEVEL_IDX_HI1),
            mid: Vlc::new(&TAB_TONE_LEVEL_IDX_MID),
            hi2: Vlc::new(&TAB_TONE_LEVEL_IDX_HI2),
            type30: Vlc::new(&TAB_TYPE30),
            type34: Vlc::new(&TAB_TYPE34),
            fft_tone_offset,
            window: mpa::synth_window(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct SubPacket {
    kind: i32,
    size: usize,
    /// Offset of the data in the packet.
    start: usize,
}

#[derive(Clone, Copy, Default)]
struct FftTone {
    level: f32,
    ch: usize,
    offset: usize,
    table: (usize, usize),
    phase: i32,
    phase_shift: i32,
    duration: usize,
    time_index: i16,
    cutoff: i16,
}

#[derive(Clone, Copy, Default)]
struct FftCoef {
    sub_packet: i16,
    channel: u8,
    offset: i16,
    exp: i16,
    phase: u8,
}

/// A QDM2 decoder.
pub struct Decoder {
    t: Tables,
    pub channels: usize,
    pub sample_rate: u32,
    pub bit_rate: u32,
    group_size: usize,
    fft_size: usize,
    /// Bytes of one packet.
    pub checksum_size: usize,
    group_order: i32,
    frame_size: usize,
    frequency_range: i32,
    sub_sampling: usize,
    coeff_per_sb_select: usize,
    cm_table_select: usize,

    sub_packets: [SubPacket; 16],
    list_b: Vec<usize>,
    list_d: Vec<usize>,

    fft_tones: Vec<FftTone>,
    fft_tone_start: usize,
    fft_tone_end: usize,
    fft_coefs: Vec<FftCoef>,
    fft_coefs_index: usize,
    fft_coefs_min_index: [i32; 5],
    fft_coefs_max_index: [i32; 5],
    fft_level_exp: [i32; 6],
    /// complex[ch][0..=256] as (re, im)
    fft_complex: [[(f32, f32); 257]; 2],
    output_buffer: Vec<f32>,

    synth_buf: [[f32; 1024]; 2],
    synth_buf_offset: [usize; 2],
    sb_samples: Box<[[[f32; SBLIMIT]; 128]; 2]>,
    samples: Vec<f32>,

    tone_level: Box<[[[f32; 64]; 30]; 2]>,
    coding_method: Box<SbInt8>,
    quantized_coeffs: [[[i8; 8]; 10]; 2],
    tone_level_idx_base: [[[i8; 8]; 30]; 2],
    tone_level_idx_hi1: [[[[i8; 8]; 8]; 3]; 2],
    tone_level_idx_mid: [[[i8; 8]; 26]; 2],
    tone_level_idx_hi2: [[i8; 26]; 2],
    tone_level_idx: Box<SbInt8>,

    has_errors: bool,
    superblocktype_2_3: bool,
    do_synth_filter: bool,
    sub_packet: usize,
    noise_idx: usize,
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn log2(x: u32) -> i32 {
    31 - x.max(1).leading_zeros() as i32
}

impl Decoder {
    /// From the codec's extradata (the QuickTime `wave` atom contents: `frma`, `QDCA`, `QDCP`).
    pub fn new(extradata: &[u8]) -> Result<Self, Error> {
        let bad = |s: &str| Error::Invalid(s.into());
        if extradata.len() < 48 {
            return Err(bad("extradata missing or truncated"));
        }
        // find "frmaQDM2"
        let mut p = 0usize;
        while extradata.len() - p > 8 {
            if &extradata[p..p + 8] == b"frmaQDM2" {
                break;
            }
            p += 1;
        }
        if extradata.len() - p < 12 {
            return Err(bad("not enough extradata"));
        }
        p += 8;
        let size = be32(extradata, p).ok_or_else(|| bad("truncated"))? as usize;
        p += 4;
        if size > extradata.len() - p {
            return Err(bad("extradata size too small"));
        }
        if be32(extradata, p) != Some(u32::from_be_bytes(*b"QDCA")) {
            return Err(bad("invalid extradata, expecting QDCA"));
        }
        p += 8;
        let f = |i: usize| be32(extradata, p + 4 * i).ok_or_else(|| bad("truncated QDCA"));
        let channels = f(0)? as i32;
        if channels <= 0 || channels > 2 {
            return Err(bad("invalid number of channels"));
        }
        let channels = channels as usize;
        let sample_rate = f(1)?;
        let bit_rate = f(2)?;
        let group_size = f(3)? as usize;
        let fft_size = f(4)? as usize;
        let checksum_size = f(5)?;
        if checksum_size >= 1 << 28 || checksum_size <= 1 {
            return Err(bad("data block size invalid"));
        }
        if fft_size == 0 || fft_size > 1 << 20 || group_size == 0 || group_size > 1 << 20 {
            return Err(bad("invalid sizes"));
        }
        let fft_order = log2(fft_size as u32) + 1;
        if !(7..=9).contains(&fft_order) {
            return Err(bad("unknown FFT order"));
        }
        let group_order = log2(group_size as u32) + 1;
        let frame_size = group_size / 16;
        if frame_size > MAX_FRAME_SIZE {
            return Err(bad("frame too large"));
        }
        let sub_sampling = (fft_order - 7) as usize;
        let frequency_range = 255 / (1 << (2 - sub_sampling));
        if (frame_size * 4) >> sub_sampling > 1152 {
            return Err(bad("large frames"));
        }
        let tmp = match sub_sampling * 2 + channels - 1 {
            0 => 40,
            1 => 48,
            2 => 56,
            3 => 72,
            4 => 80,
            5 => 100,
            _ => sub_sampling as u32,
        };
        let mut cm = 0;
        if tmp * 1000 < bit_rate {
            cm = 1;
        }
        if tmp * 1440 < bit_rate {
            cm = 2;
        }
        if tmp * 1760 < bit_rate {
            cm = 3;
        }
        if tmp * 2240 < bit_rate {
            cm = 4;
        }
        let coeff_per_sb_select = if bit_rate <= 8000 {
            0
        } else if bit_rate < 16000 {
            1
        } else {
            2
        };
        if fft_size != 1 << (fft_order - 1) {
            return Err(bad("FFT size not a power of 2"));
        }
        Ok(Self {
            t: Tables::new(),
            channels,
            sample_rate,
            bit_rate,
            group_size,
            fft_size,
            checksum_size: checksum_size as usize,
            group_order,
            frame_size,
            frequency_range,
            sub_sampling,
            coeff_per_sb_select,
            cm_table_select: cm,
            sub_packets: [SubPacket::default(); 16],
            list_b: Vec::new(),
            list_d: Vec::new(),
            fft_tones: vec![FftTone::default(); 1000],
            fft_tone_start: 0,
            fft_tone_end: 0,
            fft_coefs: vec![FftCoef::default(); 1000],
            fft_coefs_index: 0,
            fft_coefs_min_index: [0; 5],
            fft_coefs_max_index: [0; 5],
            fft_level_exp: [0; 6],
            fft_complex: [[(0.0, 0.0); 257]; 2],
            output_buffer: vec![0.0; MAX_FRAME_SIZE * 2 * 2],
            synth_buf: [[0.0; 1024]; 2],
            synth_buf_offset: [0; 2],
            sb_samples: Box::new([[[0.0; SBLIMIT]; 128]; 2]),
            samples: vec![0.0; 2 * 1152],
            tone_level: Box::new([[[0.0; 64]; 30]; 2]),
            coding_method: Box::new([[[0; 64]; 30]; 2]),
            quantized_coeffs: [[[0; 8]; 10]; 2],
            tone_level_idx_base: [[[0; 8]; 30]; 2],
            tone_level_idx_hi1: [[[[0; 8]; 8]; 3]; 2],
            tone_level_idx_mid: [[[0; 8]; 26]; 2],
            tone_level_idx_hi2: [[0; 26]; 2],
            tone_level_idx: Box::new([[[0; 64]; 30]; 2]),
            has_errors: false,
            superblocktype_2_3: false,
            do_synth_filter: false,
            sub_packet: 0,
            noise_idx: 0,
        })
    }

    /// Samples per channel in one decoded packet.
    pub fn samples_per_packet(&self) -> usize {
        16 * self.frame_size
    }

    /// Decodes one packet (at least `checksum_size` bytes) into interleaved 16-bit samples.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>, Error> {
        if packet.len() < self.checksum_size {
            return Err(Error::Invalid("short packet".into()));
        }
        let packet = &packet[..self.checksum_size];
        let fs = self.frame_size * self.channels;
        let mut out = Vec::with_capacity(16 * fs);
        for _ in 0..16 {
            self.decode_sub(packet, &mut out)?;
        }
        Ok(out)
    }

    fn get_vlc(&self, gb: &mut Bits<'_>, which: fn(&Tables) -> &Vlc, flag: bool) -> i32 {
        let mut value = which(&self.t).get(gb);
        // stage-2, 3 bits exponent escape sequence
        if value < 0 {
            let n = gb.get(3) + 1;
            value = gb.get(n) as i32;
        }
        // stage-3, optional
        if flag {
            if value >= 60 {
                return 0;
            }
            let mut tmp = VLC_STAGE3_VALUES[value as usize];
            if (value & !3) > 0 {
                tmp += gb.get((value >> 2) as u32) as i32;
            }
            value = tmp;
        }
        value
    }

    fn get_se_vlc(&self, which: fn(&Tables) -> &Vlc, gb: &mut Bits<'_>) -> i32 {
        let value = self.get_vlc(gb, which, false);
        if value & 1 != 0 { (value + 1) >> 1 } else { -(value >> 1) }
    }

    fn average_quantized_coeffs(&mut self) {
        let n = usize::from(COEFF_PER_SB_FOR_AVG[self.coeff_per_sb_select][sb_used_of(self.sub_sampling) - 1]) + 1;
        for ch in 0..self.channels {
            for i in 0..n {
                let mut sum: i32 = self.quantized_coeffs[ch][i].iter().map(|&v| i32::from(v)).sum();
                sum /= 8;
                if sum > 0 {
                    sum -= 1;
                }
                self.quantized_coeffs[ch][i] = [sum as i8; 8];
            }
        }
    }

    /// `SB_DITHERING_NOISE`
    fn dither(&mut self, sb: usize) -> f32 {
        let v = self.t.noise_table[self.noise_idx] * SB_NOISE_ATTENUATION[sb];
        self.noise_idx += 1;
        v
    }

    fn fix_noise_idx(&mut self) {
        if self.noise_idx >= 3840 {
            self.noise_idx -= 3840;
        }
    }

    fn build_sb_samples_from_noise(&mut self, sb: usize) {
        self.fix_noise_idx();
        if self.channels == 0 {
            return;
        }
        for ch in 0..self.channels {
            for j in 0..64 {
                let a = self.dither(sb) * self.tone_level[ch][sb][j];
                self.sb_samples[ch][j * 2][sb] = a;
                let b = self.dither(sb) * self.tone_level[ch][sb][j];
                self.sb_samples[ch][j * 2 + 1][sb] = b;
            }
        }
    }

    /// `fix_coding_method_array` (on `self.coding_method`).
    fn fix_coding_method_array(&mut self, sb: usize, channels: usize) -> bool {
        for ch in 0..channels {
            let mut j = 0usize;
            while j < 64 {
                let cm = i32::from(self.coding_method[ch][sb][j]);
                if cm < 8 {
                    return true;
                }
                let (run, case_val): (usize, i8) = if cm - 8 > 22 {
                    (1, 8)
                } else {
                    match SWITCHTABLE[(cm - 8) as usize] {
                        0 => (10, 10),
                        1 => (1, 16),
                        2 => (5, 24),
                        3 => (3, 30),
                        4 => (1, 30),
                        _ => (1, 8),
                    }
                };
                for k in 0..run {
                    if j + k < 128 {
                        let sbjk = sb + (j + k) / 64;
                        if sbjk > 29 {
                            continue;
                        }
                        if self.coding_method[ch][sbjk][(j + k) % 64] > self.coding_method[ch][sb][j] && k > 0 {
                            // "not debugged, almost never used": the second memset wins
                            let base = j + k;
                            for m in 0..k.max(3) {
                                if let Some(x) = self.coding_method[ch][sb].get_mut(base + m) {
                                    *x = case_val;
                                }
                            }
                        }
                    }
                }
                j += run;
            }
        }
        false
    }

    fn fill_tone_level_array(&mut self, flag: bool) {
        let sel = self.coeff_per_sb_select;
        for ch in 0..self.channels {
            for sb in 0..30 {
                for i in 0..8 {
                    let tab = usize::from(COEFF_PER_SB_FOR_DEQUANT[sel][sb]);
                    let q = |t: usize| i32::from(self.quantized_coeffs[ch][t][i]) as u32;
                    let mut tmp: i32 = if tab < usize::from(LAST_COEFF[sel]) - 1 {
                        q(tab + 1).wrapping_mul(DEQUANT_TABLE[sel][tab + 1][sb]).wrapping_add(q(tab).wrapping_mul(DEQUANT_TABLE[sel][tab][sb])) as i32
                    } else {
                        q(tab).wrapping_mul(DEQUANT_TABLE[sel][tab][sb]) as i32
                    };
                    if tmp < 0 {
                        tmp = tmp.wrapping_add(0xff);
                    }
                    self.tone_level_idx_base[ch][sb][i] = ((tmp / 256) & 0xff) as u8 as i8;
                }
            }
        }
        let sb_used = sb_used_of(self.sub_sampling);
        let t23 = self.superblocktype_2_3;
        if t23 && !flag {
            for sb in 0..sb_used {
                for ch in 0..self.channels {
                    for i in 0..64 {
                        let v = self.tone_level_idx_base[ch][sb][i / 8];
                        self.tone_level_idx[ch][sb][i] = v;
                        self.tone_level[ch][sb][i] = if v < 0 { 0.0 } else { FFT_TONE_LEVEL_TABLE[0][(v & 0x3f) as usize] };
                    }
                }
            }
        } else {
            let tab = usize::from(!t23);
            for sb in 0..sb_used {
                if (4..=23).contains(&sb) {
                    for ch in 0..self.channels {
                        for i in 0..64 {
                            let tmp = i32::from(self.tone_level_idx_base[ch][sb][i / 8])
                                - i32::from(self.tone_level_idx_hi1[ch][sb / 8][i / 8][i % 8])
                                - i32::from(self.tone_level_idx_mid[ch][sb - 4][i / 8])
                                - i32::from(self.tone_level_idx_hi2[ch][sb - 4]);
                            self.tone_level_idx[ch][sb][i] = (tmp & 0xff) as u8 as i8;
                            self.tone_level[ch][sb][i] = if tmp < 0 || (!t23 && tmp == 0) { 0.0 } else { FFT_TONE_LEVEL_TABLE[tab][(tmp & 0x3f) as usize] };
                        }
                    }
                } else if sb > 4 {
                    for ch in 0..self.channels {
                        for i in 0..64 {
                            let tmp = i32::from(self.tone_level_idx_base[ch][sb][i / 8])
                                - i32::from(self.tone_level_idx_hi1[ch][2][i / 8][i % 8])
                                - i32::from(self.tone_level_idx_hi2[ch][sb - 4]);
                            self.tone_level_idx[ch][sb][i] = (tmp & 0xff) as u8 as i8;
                            self.tone_level[ch][sb][i] = if tmp < 0 || (!t23 && tmp == 0) { 0.0 } else { FFT_TONE_LEVEL_TABLE[tab][(tmp & 0x3f) as usize] };
                        }
                    }
                } else {
                    for ch in 0..self.channels {
                        for i in 0..64 {
                            let v = self.tone_level_idx_base[ch][sb][i / 8];
                            self.tone_level_idx[ch][sb][i] = v;
                            let tmp = i32::from(v);
                            self.tone_level[ch][sb][i] = if tmp < 0 || (!t23 && tmp == 0) { 0.0 } else { FFT_TONE_LEVEL_TABLE[tab][(tmp & 0x3f) as usize] };
                        }
                    }
                }
            }
        }
    }

    /// `fill_coding_method_array` for the only case FFmpeg runs (`superblocktype_2_3`): a
    /// table per subband. Without it FFmpeg asks for a sample and does nothing.
    fn fill_coding_method_array(&mut self) {
        if !self.superblocktype_2_3 {
            return;
        }
        for ch in 0..self.channels {
            for sb in 0..30 {
                for j in 0..64 {
                    self.coding_method[ch][sb][j] = CODING_METHOD_TABLE[self.cm_table_select][sb];
                }
            }
        }
    }

    /// `synthfilt_build_sb_samples`; an `Err` is FFmpeg's `AVERROR_INVALIDDATA` (the caller ignores it).
    fn synthfilt_build_sb_samples(&mut self, gb: &mut Bits<'_>, length: usize, sb_min: usize, sb_max: usize) -> Result<(), ()> {
        let mut samples = [0.0f32; 10];
        let mut sign_bits = [0i32; 16];
        if length == 0 {
            for sb in sb_min..sb_max {
                self.build_sb_samples_from_noise(sb);
            }
            return Ok(());
        }
        for sb in sb_min..sb_max {
            let mut channels = self.channels;
            let joined_stereo = if self.channels <= 1 || sb < 12 {
                false
            } else if sb >= 24 {
                true
            } else if gb.left() >= 1 {
                gb.get1() != 0
            } else {
                false
            };
            if joined_stereo {
                if gb.left() >= 16 {
                    for s in &mut sign_bits {
                        *s = gb.get1() as i32;
                    }
                }
                for j in 0..64 {
                    if self.coding_method[1][sb][j] > self.coding_method[0][sb][j] {
                        self.coding_method[0][sb][j] = self.coding_method[1][sb][j];
                    }
                }
                if self.fix_coding_method_array(sb, self.channels) {
                    self.build_sb_samples_from_noise(sb);
                    continue;
                }
                channels = 1;
            }
            let js = usize::from(joined_stereo);
            for ch in 0..channels {
                self.fix_noise_idx();
                let zero_encoding = if gb.left() >= 1 { gb.get1() != 0 } else { false };
                let mut type34_predictor = 0.0f32;
                let mut type34_first = true;
                let mut type34_div = 0.0f32;
                let mut j = 0usize;
                while j < 128 {
                    let run: usize;
                    match self.coding_method[ch][sb][j / 2] {
                        8 => {
                            if gb.left() >= 10 {
                                if zero_encoding {
                                    for k in 0..5 {
                                        if j + 2 * k >= 128 {
                                            break;
                                        }
                                        samples[2 * k] = if gb.get1() != 0 {
                                            let i2 = 2 * gb.get1() as usize;
                                            DEQUANT_1BIT[js][i2]
                                        } else {
                                            0.0
                                        };
                                    }
                                } else {
                                    let n = gb.get(8) as usize;
                                    if n >= 243 {
                                        return Err(());
                                    }
                                    for k in 0..5 {
                                        samples[2 * k] = DEQUANT_1BIT[js][usize::from(self.t.random_dequant_index[n][k])];
                                    }
                                }
                                for k in 0..5 {
                                    samples[2 * k + 1] = self.dither(sb);
                                }
                            } else {
                                for s in samples.iter_mut() {
                                    *s = 0.0;
                                }
                                for k in 0..10 {
                                    samples[k] = self.dither(sb);
                                }
                            }
                            run = 10;
                        }
                        10 => {
                            if gb.left() >= 1 {
                                let mut f = 0.81f32;
                                if gb.get1() != 0 {
                                    f = -f;
                                }
                                let idx = ((sb + 1) * (j + 5 * ch + 1)) & 127;
                                f = (f64::from(f) - f64::from(self.t.noise_samples[idx]) * 9.0 / 40.0) as f32;
                                samples[0] = f;
                            } else {
                                samples[0] = self.dither(sb);
                            }
                            run = 1;
                        }
                        16 => {
                            if gb.left() >= 10 {
                                if zero_encoding {
                                    for k in 0..5 {
                                        if j + k >= 128 {
                                            break;
                                        }
                                        samples[k] = if gb.get1() == 0 {
                                            0.0
                                        } else {
                                            let i2 = 2 * gb.get1() as usize;
                                            DEQUANT_1BIT[js][i2]
                                        };
                                    }
                                } else {
                                    let n = gb.get(8) as usize;
                                    if n >= 243 {
                                        return Err(());
                                    }
                                    for k in 0..5 {
                                        samples[k] = DEQUANT_1BIT[js][usize::from(self.t.random_dequant_index[n][k])];
                                    }
                                }
                            } else {
                                for k in 0..5 {
                                    samples[k] = self.dither(sb);
                                }
                            }
                            run = 5;
                        }
                        24 => {
                            if gb.left() >= 7 {
                                let n = gb.get(7) as usize;
                                if n >= 125 {
                                    return Err(());
                                }
                                for k in 0..3 {
                                    samples[k] = ((f64::from(self.t.random_dequant_type24[n][k]) - 2.0) * 0.5) as f32;
                                }
                            } else {
                                for k in 0..3 {
                                    samples[k] = self.dither(sb);
                                }
                            }
                            run = 3;
                        }
                        30 => {
                            if gb.left() >= 4 {
                                let index = self.get_vlc(gb, |t| &t.type30, false) as u32 as usize;
                                if index >= TYPE30_DEQUANT.len() {
                                    return Err(());
                                }
                                samples[0] = TYPE30_DEQUANT[index];
                            } else {
                                samples[0] = self.dither(sb);
                            }
                            run = 1;
                        }
                        34 => {
                            if gb.left() >= 7 {
                                if type34_first {
                                    type34_div = (1u32 << gb.get(2)) as f32;
                                    samples[0] = ((f64::from(gb.get(5) as f32) - 16.0) / 15.0) as f32;
                                    type34_predictor = samples[0];
                                    type34_first = false;
                                } else {
                                    let index = self.get_vlc(gb, |t| &t.type34, false) as u32 as usize;
                                    if index >= TYPE34_DELTA.len() {
                                        return Err(());
                                    }
                                    samples[0] = TYPE34_DELTA[index] / type34_div + type34_predictor;
                                    type34_predictor = samples[0];
                                }
                            } else {
                                samples[0] = self.dither(sb);
                            }
                            run = 1;
                        }
                        _ => {
                            samples[0] = self.dither(sb);
                            run = 1;
                        }
                    }
                    if joined_stereo {
                        let mut k = 0;
                        while k < run && j + k < 128 {
                            self.sb_samples[0][j + k][sb] = self.tone_level[0][sb][(j + k) / 2] * samples[k];
                            if self.channels == 2 {
                                self.sb_samples[1][j + k][sb] = if sign_bits[(j + k) / 8] != 0 {
                                    self.tone_level[1][sb][(j + k) / 2] * -samples[k]
                                } else {
                                    self.tone_level[1][sb][(j + k) / 2] * samples[k]
                                };
                            }
                            k += 1;
                        }
                    } else {
                        for k in 0..run {
                            if j + k < 128 {
                                self.sb_samples[ch][j + k][sb] = self.tone_level[ch][sb][(j + k) / 2] * samples[k];
                            }
                        }
                    }
                    j += run;
                }
            }
        }
        Ok(())
    }

    /// `init_quantized_coeffs_elem0`
    fn init_quantized_coeffs_elem0(&self, ch: usize, gb: &mut Bits<'_>, out: &mut [[[i8; 8]; 10]; 2]) -> Result<(), ()> {
        if gb.left() < 16 {
            return Err(());
        }
        let level0 = self.get_vlc(gb, |t| &t.level, false);
        out[ch][0][0] = level0 as i8;
        let mut level = level0;
        let mut i = 0usize;
        while i < 7 {
            if gb.left() < 16 {
                return Err(());
            }
            let run = self.get_vlc(gb, |t| &t.run, false) + 1;
            if i as i32 + run >= 8 {
                return Err(());
            }
            if gb.left() < 16 {
                return Err(());
            }
            let diff = self.get_se_vlc(|t| &t.diff, gb);
            for k in 1..=run {
                out[ch][0][i + k as usize] = (level + (k * diff) / run) as i8;
            }
            level += diff;
            i += run as usize;
        }
        Ok(())
    }

    fn init_tone_level_dequantization(&mut self, gb: &mut Bits<'_>) {
        let mut q = self.quantized_coeffs;
        for ch in 0..self.channels {
            let _ = self.init_quantized_coeffs_elem0(ch, gb, &mut q);
            if gb.left() < 16 {
                q[ch][0] = [0; 8];
                break;
            }
        }
        self.quantized_coeffs = q;
        let n = self.sub_sampling + 1;
        for sb in 0..n {
            for ch in 0..self.channels {
                for j in 0..8 {
                    if gb.left() < 1 {
                        break;
                    }
                    if gb.get1() != 0 {
                        for k in 0..8 {
                            if gb.left() < 16 {
                                break;
                            }
                            self.tone_level_idx_hi1[ch][sb][j][k] = self.get_vlc(gb, |t| &t.hi1, false) as i8;
                        }
                    } else {
                        for k in 0..8 {
                            self.tone_level_idx_hi1[ch][sb][j][k] = 0;
                        }
                    }
                }
            }
        }
        let n = sb_used_of(self.sub_sampling) - 4;
        for sb in 0..n {
            for ch in 0..self.channels {
                if gb.left() < 16 {
                    break;
                }
                self.tone_level_idx_hi2[ch][sb] = self.get_vlc(gb, |t| &t.hi2, false) as i8;
                if sb > 19 {
                    self.tone_level_idx_hi2[ch][sb] = self.tone_level_idx_hi2[ch][sb].wrapping_sub(16);
                } else {
                    for j in 0..8 {
                        self.tone_level_idx_mid[ch][sb][j] = -16;
                    }
                }
            }
        }
        let n = sb_used_of(self.sub_sampling) - 5;
        for sb in 0..n {
            for ch in 0..self.channels {
                for j in 0..8 {
                    if gb.left() < 16 {
                        break;
                    }
                    self.tone_level_idx_mid[ch][sb][j] = (self.get_vlc(gb, |t| &t.mid, false) - 32) as i8;
                }
            }
        }
    }

    fn process_subpacket_9(&mut self, mut gb: Bits<'_>) -> Result<(), ()> {
        let n = usize::from(COEFF_PER_SB_FOR_AVG[self.coeff_per_sb_select][sb_used_of(self.sub_sampling) - 1]) + 1;
        for i in 1..n {
            for ch in 0..self.channels {
                let mut level = self.get_vlc(&mut gb, |t| &t.level, false);
                self.quantized_coeffs[ch][i][0] = level as i8;
                let mut j = 0usize;
                while j < 7 {
                    let run = self.get_vlc(&mut gb, |t| &t.run, false) + 1;
                    let diff = self.get_se_vlc(|t| &t.diff, &mut gb);
                    if j as i32 + run >= 8 {
                        return Err(());
                    }
                    for k in 1..=run {
                        self.quantized_coeffs[ch][i][j + k as usize] = (level + (k * diff) / run) as i8;
                    }
                    level += diff;
                    j += run as usize;
                }
            }
        }
        for ch in 0..self.channels {
            self.quantized_coeffs[ch][0] = [0; 8];
        }
        Ok(())
    }

    fn process_subpacket_10(&mut self, data: Option<&[u8]>) {
        if let Some(d) = data {
            let mut gb = Bits::new(d);
            self.init_tone_level_dequantization(&mut gb);
            self.fill_tone_level_array(true);
        } else {
            self.fill_tone_level_array(false);
        }
    }

    fn process_subpacket_11(&mut self, data: Option<&[u8]>) {
        let length = data.map_or(0, |d| d.len() * 8);
        let mut gb = Bits::new(data.unwrap_or(&[]));
        if length >= 32 {
            let c = gb.get(13);
            if c > 3 {
                self.fill_coding_method_array();
            }
        }
        let _ = self.synthfilt_build_sb_samples(&mut gb, length, 0, 8);
    }

    fn process_subpacket_12(&mut self, data: Option<&[u8]>) {
        let length = data.map_or(0, |d| d.len() * 8);
        let mut gb = Bits::new(data.unwrap_or(&[]));
        let _ = self.synthfilt_build_sb_samples(&mut gb, length, 8, sb_used_of(self.sub_sampling));
    }

    fn find_in_d(&self, kind: i32) -> Option<usize> {
        self.list_d.iter().copied().find(|&i| self.sub_packets[i].kind == kind)
    }

    fn sub_data<'p>(&self, packet: &'p [u8], i: usize) -> &'p [u8] {
        let sp = self.sub_packets[i];
        let start = sp.start.min(packet.len());
        &packet[start..start.saturating_add(sp.size).min(packet.len())]
    }

    fn process_synthesis_subpackets(&mut self, packet: &[u8]) {
        let n9 = self.find_in_d(9);
        if let Some(i) = n9 {
            let d = self.sub_data(packet, i);
            let _ = self.process_subpacket_9(Bits::new(d));
        }
        let n10 = self.find_in_d(10);
        match n10 {
            Some(i) => {
                let d = self.sub_data(packet, i);
                self.process_subpacket_10(Some(d));
            }
            None => self.process_subpacket_10(None),
        }
        let n11 = self.find_in_d(11);
        match (n9, n10, n11) {
            (Some(_), Some(_), Some(i)) => {
                let d = self.sub_data(packet, i);
                self.process_subpacket_11(Some(d));
            }
            _ => self.process_subpacket_11(None),
        }
        let n12 = self.find_in_d(12);
        match (n9, n10, n12) {
            (Some(_), Some(_), Some(i)) => {
                let d = self.sub_data(packet, i);
                self.process_subpacket_12(Some(d));
            }
            _ => self.process_subpacket_12(None),
        }
    }

    /// `qdm2_decode_sub_packet_header` on `gb` (its data starts at byte `base` of the packet).
    fn sub_packet_header(gb: &mut Bits<'_>, base: usize) -> SubPacket {
        let mut sp = SubPacket { kind: gb.get(8) as i32, ..SubPacket::default() };
        if sp.kind == 0 {
            sp.size = 0;
            sp.start = 0;
        } else {
            sp.size = gb.get(8) as usize;
            if sp.kind & 0x80 != 0 {
                sp.size <<= 8;
                sp.size |= gb.get(8) as usize;
                sp.kind &= 0x7f;
            }
            if sp.kind == 0x7f {
                sp.kind |= (gb.get(8) as i32) << 8;
            }
            sp.start = base + gb.count() / 8;
        }
        sp
    }

    fn decode_super_block(&mut self, packet: &[u8]) {
        self.tone_level_idx_hi1 = [[[[0; 8]; 8]; 3]; 2];
        self.tone_level_idx_mid = [[[0; 8]; 26]; 2];
        self.tone_level_idx_hi2 = [[0; 26]; 2];
        self.list_b.clear();
        self.list_d.clear();
        self.average_quantized_coeffs();

        let mut gb = Bits::new(packet);
        let header = Self::sub_packet_header(&mut gb, 0);
        if header.kind < 2 || header.kind >= 8 {
            self.has_errors = true;
            return;
        }
        self.superblocktype_2_3 = header.kind == 2 || header.kind == 3;
        let mut packet_bytes = self.checksum_size as i64 - (gb.count() / 8) as i64;
        let hdata_start = header.start.min(packet.len());
        let hdata = &packet[hdata_start..hdata_start.saturating_add(header.size).min(packet.len())];
        let mut gb = Bits::new(hdata);
        if header.kind == 2 || header.kind == 4 || header.kind == 5 {
            let mut csum = 257 * gb.get(8) as i32;
            csum += 2 * gb.get(8) as i32;
            for &b in packet.iter().take(self.checksum_size) {
                csum -= i32::from(b);
            }
            if (csum & 0xffff) as u16 != 0 {
                self.has_errors = true;
                return;
            }
        }
        for e in &mut self.fft_level_exp {
            *e -= 1;
            if *e < 0 {
                *e = 0;
            }
        }
        let mut next_index = 0usize;
        let mut i = 0usize;
        while packet_bytes > 0 {
            if i >= 16 {
                return;
            }
            if i > 0 {
                gb = Bits::new(hdata);
                gb.skip(next_index * 8);
                if next_index >= header.size {
                    break;
                }
            }
            let mut sp = Self::sub_packet_header(&mut gb, hdata_start);
            next_index = sp.size + gb.count() / 8;
            let sub_packet_size = i64::from(sp.size > 0xff) + sp.size as i64 + 2;
            if sp.kind == 0 {
                self.sub_packets[i] = sp;
                break;
            }
            if sub_packet_size > packet_bytes {
                if sp.kind != 10 && sp.kind != 11 && sp.kind != 12 {
                    self.sub_packets[i] = sp;
                    break;
                }
                sp.size = (sp.size as i64 + packet_bytes - sub_packet_size).max(0) as usize;
            }
            packet_bytes -= sub_packet_size;
            self.sub_packets[i] = sp;
            if sp.kind == 8 {
                return;
            } else if (9..=12).contains(&sp.kind) {
                self.list_d.push(i);
            } else if sp.kind == 13 {
                for j in 0..6 {
                    self.fft_level_exp[j] = gb.get(6) as i32;
                }
            } else if sp.kind == 14 {
                for j in 0..6 {
                    self.fft_level_exp[j] = self.get_vlc(&mut gb, |t| &t.fft_level_exp, false);
                }
            } else if sp.kind == 15 {
                return;
            } else if (16..48).contains(&sp.kind) && FFT_SUBPACKETS[(sp.kind - 16) as usize] == 0 {
                self.list_b.push(i);
            }
            i += 1;
        }
        if !self.list_d.is_empty() {
            self.process_synthesis_subpackets(packet);
            self.do_synth_filter = true;
        } else if self.do_synth_filter {
            self.process_subpacket_10(None);
            self.process_subpacket_11(None);
            self.process_subpacket_12(None);
        }
    }

    fn fft_init_coefficient(&mut self, sub_packet: i32, offset: i32, duration: usize, channel: i32, exp: i32, phase: i32) {
        if self.fft_coefs_min_index[duration] < 0 {
            self.fft_coefs_min_index[duration] = self.fft_coefs_index as i32;
        }
        self.fft_coefs[self.fft_coefs_index] = FftCoef {
            sub_packet: (if sub_packet >= 16 { sub_packet - 16 } else { sub_packet }) as i16,
            channel: channel as u8,
            offset: offset as i16,
            exp: exp as i16,
            phase: phase as u8,
        };
        self.fft_coefs_index += 1;
    }

    fn fft_decode_tones(&mut self, duration: usize, gb: &mut Bits<'_>, b: bool) {
        let mut local_int_4 = 0i32;
        let mut local_int_28 = 0i32;
        let local_int_20 = 2i32;
        let local_int_8 = 4 - duration as i32;
        let local_int_10 = 1i32 << (self.group_order - duration as i32 - 1);
        let mut offset = 1i32;
        while gb.left() > 0 {
            if self.superblocktype_2_3 {
                loop {
                    let n = self.get_vlc_idx(gb, local_int_8 as usize);
                    if n >= 2 {
                        offset += n - 2;
                        break;
                    }
                    if gb.left() < 0 {
                        return;
                    }
                    offset = 1;
                    if n == 0 {
                        local_int_4 += local_int_10;
                        local_int_28 += 1 << local_int_8;
                    } else {
                        local_int_4 += 8 * local_int_10;
                        local_int_28 += 8 << local_int_8;
                    }
                }
            } else {
                if local_int_10 <= 2 {
                    return;
                }
                offset += self.get_vlc_idx(gb, local_int_8 as usize);
                while offset >= local_int_10 - 1 {
                    offset += 1 - (local_int_10 - 1);
                    local_int_4 += local_int_10;
                    local_int_28 += 1 << local_int_8;
                }
            }
            if local_int_4 >= self.group_size as i32 {
                return;
            }
            let local_int_14 = offset >> local_int_8;
            if local_int_14 < 0 || local_int_14 as usize >= FFT_LEVEL_INDEX_TABLE.len() {
                return;
            }
            let (channel, stereo) = if self.channels > 1 { (gb.get1() as i32, gb.get1() as i32) } else { (0, 0) };
            let mut exp = if b { self.get_vlc(gb, |t| &t.fft_level_exp, false) } else { self.get_vlc(gb, |t| &t.fft_level_exp_alt, false) };
            exp += self.fft_level_exp[FFT_LEVEL_INDEX_TABLE[local_int_14 as usize] as usize];
            exp = exp.max(0);
            let phase = gb.get(3) as i32;
            let mut stereo_exp = 0;
            let mut stereo_phase = 0;
            if stereo != 0 {
                stereo_exp = exp - self.get_vlc(gb, |t| &t.fft_stereo_exp, false);
                stereo_phase = phase - self.get_vlc(gb, |t| &t.fft_stereo_phase, false);
                if stereo_phase < 0 {
                    stereo_phase += 8;
                }
            }
            if self.frequency_range > local_int_14 + 1 {
                let sub_packet = local_int_20 + local_int_28;
                if self.fft_coefs_index + stereo as usize >= self.fft_coefs.len() {
                    return;
                }
                self.fft_init_coefficient(sub_packet, offset, duration, channel, exp, phase);
                if stereo != 0 {
                    self.fft_init_coefficient(sub_packet, offset, duration, 1 - channel, stereo_exp, stereo_phase);
                }
            }
            offset += 1;
        }
    }

    /// `qdm2_get_vlc(gb, &vlc_tab_fft_tone_offset[i], 1, 2)`
    fn get_vlc_idx(&self, gb: &mut Bits<'_>, i: usize) -> i32 {
        let mut value = self.t.fft_tone_offset[i].get(gb);
        if value < 0 {
            let n = gb.get(3) + 1;
            value = gb.get(n) as i32;
        }
        if value >= 60 {
            return 0;
        }
        let mut tmp = VLC_STAGE3_VALUES[value as usize];
        if (value & !3) > 0 {
            tmp += gb.get((value >> 2) as u32) as i32;
        }
        tmp
    }

    fn decode_fft_packets(&mut self, packet: &[u8]) {
        if self.list_b.is_empty() {
            return;
        }
        self.fft_coefs_index = 0;
        self.fft_coefs_min_index = [-1; 5];
        let mut max = 256;
        for i in 0..self.list_b.len() {
            // the subpacket with the largest type below max
            let (mut min, mut found) = (0, None);
            for &j in &self.list_b {
                let v = self.sub_packets[j].kind;
                if v > min && v < max {
                    min = v;
                    found = Some(j);
                }
            }
            max = min;
            let Some(pi) = found else { return };
            let kind = self.sub_packets[pi].kind;
            if i == 0 && (!(16..48).contains(&kind) || FFT_SUBPACKETS[(kind - 16) as usize] != 0) {
                return;
            }
            let data = self.sub_data(packet, pi);
            let mut gb = Bits::new(data);
            let unknown_flag = (32..48).contains(&kind) && FFT_SUBPACKETS[(kind - 16) as usize] == 0;
            if (17..24).contains(&kind) || (33..40).contains(&kind) {
                let duration = self.sub_sampling as i32 + 5 - (kind & 15);
                if (0..4).contains(&duration) {
                    self.fft_decode_tones(duration as usize, &mut gb, unknown_flag);
                }
            } else if kind == 31 {
                for j in 0..4 {
                    self.fft_decode_tones(j, &mut gb, unknown_flag);
                }
            } else if kind == 46 {
                for j in 0..6 {
                    self.fft_level_exp[j] = gb.get(6) as i32;
                }
                for j in 0..4 {
                    self.fft_decode_tones(j, &mut gb, unknown_flag);
                }
            }
        }
        let mut j: i32 = -1;
        for i in 0..5 {
            if self.fft_coefs_min_index[i] >= 0 {
                if j >= 0 {
                    self.fft_coefs_max_index[j as usize] = self.fft_coefs_min_index[i];
                }
                j = i as i32;
            }
        }
        if j >= 0 {
            self.fft_coefs_max_index[j as usize] = self.fft_coefs_index as i32;
        }
    }

    fn fft_generate_tone(&mut self, tone: &mut FftTone) {
        let iscale = 2.0 * std::f64::consts::PI / 512.0;
        tone.phase = tone.phase.wrapping_add(tone.phase_shift);
        let level = FFT_TONE_ENVELOPE_TABLE[tone.duration][tone.time_index as usize] * tone.level;
        let c_im = (f64::from(level) * (f64::from(tone.phase) * iscale).sin()) as f32;
        let c_re = (f64::from(level) * (f64::from(tone.phase) * iscale).cos()) as f32;
        let cx = &mut self.fft_complex[tone.ch];
        let base = tone.offset;
        if tone.duration >= 3 || tone.cutoff >= 3 {
            cx[base].1 += c_im;
            cx[base].0 += c_re;
            cx[base + 1].1 -= c_im;
            cx[base + 1].0 -= c_re;
        } else {
            let t = &FFT_TONE_SAMPLE_TABLE[tone.table.0][tone.table.1];
            let mut f = [0.0f32; 6];
            f[1] = -t[4];
            f[0] = t[3] - t[0];
            f[2] = (1.0 - f64::from(t[2]) - f64::from(t[3])) as f32;
            f[3] = (f64::from(t[1] + t[4]) - 1.0) as f32;
            f[4] = t[0] - t[1];
            f[5] = t[2];
            for i in 0..2 {
                let idx = FFT_CUTOFF_INDEX_TABLE[tone.cutoff as usize][i] as usize;
                cx[base + idx].0 += c_re * f[i];
                cx[base + idx].1 += c_im * if (tone.cutoff as usize) <= i { -f[i] } else { f[i] };
            }
            for i in 0..4 {
                cx[base + i].0 += c_re * f[i + 2];
                cx[base + i].1 += c_im * f[i + 2];
            }
        }
        tone.time_index += 1;
        if i32::from(tone.time_index) < (1 << (5 - tone.duration)) - 1 {
            self.fft_tones[self.fft_tone_end] = *tone;
            self.fft_tone_end = (self.fft_tone_end + 1) % 1000;
        }
    }

    fn fft_tone_synthesizer(&mut self, sub_packet: usize) {
        let iscale = 0.25 * std::f64::consts::PI;
        for ch in 0..self.channels {
            for c in &mut self.fft_complex[ch][..self.fft_size] {
                *c = (0.0, 0.0);
            }
        }
        let tab = usize::from(!self.superblocktype_2_3);
        if self.fft_coefs_min_index[4] >= 0 {
            for i in self.fft_coefs_min_index[4]..self.fft_coefs_max_index[4] {
                let co = self.fft_coefs[i as usize];
                if co.sub_packet as usize != sub_packet {
                    break;
                }
                let ch = if self.channels == 1 { 0 } else { usize::from(co.channel) };
                let level = if co.exp < 0 { 0.0 } else { FFT_TONE_LEVEL_TABLE[tab][(co.exp & 63) as usize] };
                let c_re = (f64::from(level) * (f64::from(co.phase) * iscale).cos()) as f32;
                let c_im = (f64::from(level) * (f64::from(co.phase) * iscale).sin()) as f32;
                let o = co.offset as usize;
                self.fft_complex[ch][o].0 += c_re;
                self.fft_complex[ch][o].1 += c_im;
                self.fft_complex[ch][o + 1].0 -= c_re;
                self.fft_complex[ch][o + 1].1 -= c_im;
            }
        }
        // existing tones
        let stop = self.fft_tone_end;
        while stop != self.fft_tone_start {
            let mut t = self.fft_tones[self.fft_tone_start];
            self.fft_generate_tone(&mut t);
            self.fft_tone_start = (self.fft_tone_start + 1) % 1000;
        }
        // new tones with duration 0 (long) to 3 (short)
        for i in 0..4usize {
            if self.fft_coefs_min_index[i] >= 0 {
                let mut j = self.fft_coefs_min_index[i];
                while j < self.fft_coefs_max_index[i] {
                    let co = self.fft_coefs[j as usize];
                    if co.sub_packet as usize != sub_packet {
                        break;
                    }
                    let four_i = 4 - i;
                    let offset = (co.offset as usize) >> four_i;
                    let ch = if self.channels == 1 { 0 } else { usize::from(co.channel) };
                    if (offset as i32) < self.frequency_range {
                        let cutoff = if offset < 2 {
                            offset as i16
                        } else if offset >= 60 {
                            3
                        } else {
                            2
                        };
                        let level = if co.exp < 0 { 0.0 } else { FFT_TONE_LEVEL_TABLE[tab][(co.exp & 63) as usize] };
                        let mut tone = FftTone {
                            level,
                            ch,
                            offset,
                            table: (i, co.offset as usize - (offset << four_i)),
                            phase: 64 * i32::from(co.phase) - ((offset as i32) << 8) - 128,
                            phase_shift: (2 * i32::from(co.offset) + 1) << (7 - four_i),
                            duration: i,
                            time_index: 0,
                            cutoff,
                        };
                        self.fft_generate_tone(&mut tone);
                    }
                    j += 1;
                }
                self.fft_coefs_min_index[i] = j;
            }
        }
    }

    fn calculate_fft(&mut self, channel: usize) {
        let m = self.fft_size;
        self.fft_complex[channel][0].0 *= 2.0;
        self.fft_complex[channel][0].1 = 0.0;
        self.fft_complex[channel][m] = (0.0, 0.0);
        let x = mpa::inverse_rdft(&self.fft_complex[channel][..=m], 0.5);
        let ch = self.channels;
        let mut o = channel;
        for i in 0..m.next_multiple_of(8).min(m) {
            self.output_buffer[o] += x[2 * i];
            self.output_buffer[o + ch] += x[2 * i + 1];
            o += 2 * ch;
        }
    }

    fn synthesis_filter(&mut self, index: usize) {
        let sb_used = sb_used_of(self.sub_sampling);
        for ch in 0..self.channels {
            for i in 0..8 {
                for k in sb_used..SBLIMIT {
                    self.sb_samples[ch][8 * index + i][k] = 0.0;
                }
            }
        }
        let mut dither_state = 0i32;
        let nb = self.channels;
        for ch in 0..nb {
            let mut ptr = ch;
            for i in 0..8 {
                mpa::synth_filter(
                    &mut self.synth_buf[ch],
                    &mut self.synth_buf_offset[ch],
                    &self.t.window,
                    &mut dither_state,
                    &mut self.samples,
                    ptr,
                    nb,
                    &self.sb_samples[ch][8 * index + i],
                );
                ptr += 32 * nb;
            }
        }
        let sub_sampling = 4 >> self.sub_sampling;
        for ch in 0..self.channels {
            for i in 0..self.frame_size {
                let s = self.samples[nb * sub_sampling * i + ch];
                self.output_buffer[self.channels * i + ch] += (1 << 23) as f32 * s;
            }
        }
    }

    fn decode_sub(&mut self, packet: &[u8], out: &mut Vec<i16>) -> Result<(), Error> {
        let frame_size = self.frame_size * self.channels;
        if frame_size > self.output_buffer.len() / 2 {
            return Err(Error::Invalid("frame too large".into()));
        }
        self.output_buffer.copy_within(frame_size..2 * frame_size, 0);
        for v in &mut self.output_buffer[frame_size..2 * frame_size] {
            *v = 0.0;
        }
        if self.sub_packet == 0 {
            self.has_errors = false;
            self.decode_super_block(packet);
        }
        if !self.has_errors {
            if self.sub_packet == 2 {
                self.decode_fft_packets(packet);
            }
            self.fft_tone_synthesizer(self.sub_packet);
        }
        for ch in 0..self.channels {
            self.calculate_fft(ch);
        }
        if !self.has_errors && self.do_synth_filter {
            self.synthesis_filter(self.sub_packet);
        }
        self.sub_packet = (self.sub_packet + 1) % 16;
        for i in 0..frame_size {
            let mut value = self.output_buffer[i] as i32;
            if value > SOFTCLIP_THRESHOLD {
                value = if value > HARDCLIP_THRESHOLD { 32767 } else { i32::from(self.t.softclip[(value - SOFTCLIP_THRESHOLD) as usize]) };
            } else if value < -SOFTCLIP_THRESHOLD {
                value = if value < -HARDCLIP_THRESHOLD { -32767 } else { -i32::from(self.t.softclip[(-value - SOFTCLIP_THRESHOLD) as usize]) };
            }
            out.push(value as i16);
        }
        Ok(())
    }
}
