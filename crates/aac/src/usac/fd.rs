//! FD core coder of USAC (ISO/IEC 23003-3 §6.2.x `fd_channel_stream()`, §7.2–7.7): `ics_info`,
//! scale factors, TNS, arithmetic-coded spectrum, inverse quantisation and noise filling, plus the
//! stereo tools of `StereoCoreToolInfo()` (M/S and complex prediction).
//!
//! Ported from libxaac `ixheaacd_ext_ch_ele.c`, `ixheaacd_spectrum_dec.c`, `ixheaacd_arith_dec.c`
//! and `ixheaacd_tns.c` (Apache-2.0, see `super` for the notice), converted to floating point.

use cfa4_bitstream::BitReader;

use super::arith::{ArithContext, decode_spectrum};
use super::synth::WinSeq;
use crate::huffman;
use crate::tns::dequant_coef;
use crate::{Error, Result};

/// Scalefactor bands of one window length.
#[derive(Clone, Copy)]
pub struct Bands {
    /// Band offsets, `len() == num_bands + 1`.
    pub offsets: &'static [u16],
    /// TNS_MAX_BANDS.
    pub tns_max: usize,
}

/// `ics_info()` plus derived grouping.
#[derive(Debug, Clone, Copy, Default)]
pub struct IcsInfo {
    pub seq: WinSeq,
    pub kbd: bool,
    pub max_sfb: usize,
    pub num_groups: usize,
    pub group_len: [usize; 8],
}

impl IcsInfo {
    pub fn num_windows(&self) -> usize {
        if self.seq.is_short() { 8 } else { 1 }
    }

    pub fn parse(br: &mut BitReader, last: WinSeq, num_bands_long: usize, num_bands_short: usize) -> Result<IcsInfo> {
        let seq = WinSeq::select(br.read_bits(2)?, last);
        let kbd = br.read_flag()?;
        let mut info = IcsInfo { seq, kbd, num_groups: 1, group_len: [1, 0, 0, 0, 0, 0, 0, 0], ..Default::default() };
        if seq.is_short() {
            info.max_sfb = br.read_bits(4)? as usize;
            let grouping = br.read_bits(7)?;
            for w in 1..8 {
                if grouping & (1 << (7 - w)) != 0 {
                    info.group_len[info.num_groups - 1] += 1;
                } else {
                    info.num_groups += 1;
                    info.group_len[info.num_groups - 1] = 1;
                }
            }
            if info.max_sfb > num_bands_short {
                return Err(Error::Bitstream("max_sfb exceeds the number of scalefactor bands"));
            }
        } else {
            info.max_sfb = br.read_bits(6)? as usize;
            if info.max_sfb > num_bands_long {
                return Err(Error::Bitstream("max_sfb exceeds the number of scalefactor bands"));
            }
        }
        Ok(info)
    }
}

/// One TNS filter of `tns_data()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TnsFilter {
    pub start_band: usize,
    pub stop_band: usize,
    pub order: usize,
    pub downward: bool,
    pub coef_res4: bool,
    pub coef: [i8; 16],
}

/// `tns_data()` of one channel: up to 8 windows × 4 filters.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tns {
    pub n_filt: [usize; 8],
    pub filt: [[TnsFilter; 4]; 8],
}

impl Tns {
    /// `tns_data()` in USAC (filter order 4 bits for long windows).
    pub fn parse(br: &mut BitReader, info: &IcsInfo, num_bands: usize) -> Result<Tns> {
        let short = info.seq.is_short();
        let (nf_bits, len_bits, order_bits) = if short { (1, 4, 3) } else { (2, 6, 4) };
        let mut t = Tns::default();
        for w in 0..info.num_windows() {
            let n_filt = br.read_bits(nf_bits)? as usize;
            t.n_filt[w] = n_filt;
            if n_filt == 0 {
                continue;
            }
            let coef_res4 = br.read_flag()?;
            let mut top = num_bands;
            for f in 0..n_filt {
                let len = br.read_bits(len_bits)? as usize;
                let filt = &mut t.filt[w][f];
                filt.stop_band = top;
                top = top.saturating_sub(len);
                filt.start_band = top;
                filt.coef_res4 = coef_res4;
                filt.order = br.read_bits(order_bits)? as usize;
                if filt.order > 0 {
                    filt.downward = br.read_flag()?;
                    let compress = br.read_flag()? as u32;
                    let bits = 3 + coef_res4 as u32 - compress;
                    for c in filt.coef.iter_mut().take(filt.order) {
                        let v = br.read_bits(bits)? as i32;
                        *c = if v & (1 << (bits - 1)) != 0 { (v - (1 << bits)) as i8 } else { v as i8 };
                    }
                }
            }
        }
        Ok(t)
    }

    /// All-pole TNS synthesis on `spec` (window stride `n_win`).
    pub fn apply(&self, spec: &mut [f32], info: &IcsInfo, bands: &Bands, n_win: usize) {
        let off = bands.offsets;
        let nb = off.len() - 1;
        for w in 0..info.num_windows() {
            for f in 0..self.n_filt[w].min(4) {
                let filt = &self.filt[w][f];
                if filt.order == 0 {
                    continue;
                }
                let lim = bands.tns_max.min(info.max_sfb).min(nb);
                let start = off[filt.start_band.min(lim)] as usize;
                let stop = off[filt.stop_band.min(lim)] as usize;
                if stop <= start {
                    continue;
                }
                // reflection → direct-form coefficients
                let order = filt.order;
                let mut a = [0f64; 17];
                a[0] = 1.0;
                let mut b = [0f64; 17];
                for m in 1..=order {
                    let k = dequant_coef(filt.coef[m - 1], filt.coef_res4);
                    for i in 1..m {
                        b[i] = a[i] + k * a[m - i];
                    }
                    a[1..m].copy_from_slice(&b[1..m]);
                    a[m] = k;
                }
                let Some(s) = spec.get_mut(w * n_win..(w + 1) * n_win) else { continue };
                let mut state = [0f64; 16];
                for m in 0..stop - start {
                    let idx = if filt.downward { stop - 1 - m } else { start + m };
                    let mut y = s[idx] as f64;
                    for i in 0..order {
                        y -= a[i + 1] * state[i];
                    }
                    for i in (1..order).rev() {
                        state[i] = state[i - 1];
                    }
                    state[0] = y;
                    s[idx] = y as f32;
                }
            }
        }
    }
}

/// Which TNS data a channel carries (`tns_data_present` of the core coder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TnsPresent {
    No,
    /// Read inside `fd_channel_stream()`.
    Own,
    /// Common TNS data already read in `StereoCoreToolInfo()`.
    Common,
}

/// Decoded FD channel: dequantised spectrum and side info.
pub struct FdChannel {
    pub info: IcsInfo,
    pub spec: Vec<f32>,
    pub tns: Option<Tns>,
    /// `fac_data` (gain + quantised coefficients) when the previous frame was LPD.
    pub fac: Option<Vec<i32>>,
}

/// Persistent per-channel state of the FD decoder.
#[derive(Clone)]
pub struct FdState {
    pub arith: ArithContext,
    pub seed: u32,
}

fn random_sign(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(69069).wrapping_add(5);
    if *seed & 0x10000 != 0 { -1.0 } else { 1.0 }
}

/// Parameters shared by the channels of an element.
pub struct FdParams<'a> {
    pub n: usize,
    pub long: &'a Bands,
    pub short: &'a Bands,
    pub noise_filling: bool,
    pub indep: bool,
    pub tw_mdct: bool,
}

/// `fd_channel_stream()`. `common` is the `ics_info` of a common window; `tns_present` the TNS
/// signalling of the core coder (`Common` data is passed in `common_tns`).
#[allow(clippy::too_many_arguments)]
pub fn fd_channel_stream(
    br: &mut BitReader,
    p: &FdParams,
    st: &mut FdState,
    last: WinSeq,
    common: Option<IcsInfo>,
    tns_present: TnsPresent,
    common_tns: Option<Tns>,
    max_sfb_override: Option<usize>,
) -> Result<FdChannel> {
    let global_gain = br.read_bits(8)? as i32;
    let (noise_level, noise_offset) = if p.noise_filling { (br.read_bits(3)? as i32, br.read_bits(5)? as i32) } else { (0, 0) };
    let mut info = match common {
        Some(i) => i,
        None => IcsInfo::parse(br, last, p.long.offsets.len() - 1, p.short.offsets.len() - 1)?,
    };
    if let Some(m) = max_sfb_override {
        info.max_sfb = m;
    }
    if p.tw_mdct {
        return Err(Error::Unsupported("USAC time-warped MDCT"));
    }
    let short = info.seq.is_short();
    let bands = if short { p.short } else { p.long };
    let off = bands.offsets;
    let nb = off.len() - 1;
    let max_sfb = info.max_sfb.min(nb);
    let n_win = if short { p.n / 8 } else { p.n };

    // scale_factor_data(): one scalefactor per (group, band < max_sfb); the first is global_gain
    let mut sf = [[0i32; 64]; 8];
    let mut factor = global_gain;
    for g in 0..info.num_groups {
        for (b, s) in sf[g].iter_mut().enumerate().take(max_sfb) {
            if !(g == 0 && b == 0) {
                factor += huffman::decode_sf(br)?;
            }
            *s = factor;
        }
    }

    let tns = match tns_present {
        TnsPresent::No => None,
        TnsPresent::Own => Some(Tns::parse(br, &info, nb)?),
        TnsPresent::Common => common_tns,
    };

    let arith_reset = if p.indep { true } else { br.read_flag()? };
    st.arith.map(n_win, arith_reset);
    let mut quant = vec![0i32; p.n];
    let max_coef = if max_sfb > 0 { off[max_sfb] as usize } else { 0 };
    if max_coef > 0 {
        for w in 0..info.num_windows() {
            decode_spectrum(br, &mut st.arith, max_coef, n_win, &mut quant[w * n_win..(w + 1) * n_win])?;
        }
    } else {
        st.arith.set_all_zero(n_win);
    }

    // inverse quantisation and noise filling (ixheaacd_apply_scfs_and_nf)
    let mut spec = vec![0f32; p.n];
    let nf_start = match (p.n == 768, short) {
        (true, false) => 120,
        (true, true) => 15,
        (false, false) => 160,
        (false, true) => 20,
    };
    let noise_val = if p.noise_filling && noise_level > 0 { (2f64).powf((noise_level - 14) as f64 / 3.0) as f32 } else { 0.0 };
    let mut w0 = 0;
    for g in 0..info.num_groups {
        let glen = info.group_len[g];
        for sfb in 0..max_sfb {
            let (lo, hi) = (off[sfb] as usize, off[sfb + 1] as usize);
            let mut fac = sf[g][sfb] - 100;
            if p.noise_filling {
                let zero = (w0..w0 + glen).all(|w| quant[w * n_win + lo..w * n_win + hi].iter().all(|&q| q == 0));
                if zero {
                    fac += noise_offset - 16;
                }
            }
            let gain = (0.25 * fac.clamp(-400, 400) as f64).exp2() as f32;
            let with_noise = p.noise_filling && lo >= nf_start;
            for w in w0..w0 + glen {
                for k in w * n_win + lo..w * n_win + hi {
                    let q = quant[k];
                    if with_noise && q == 0 {
                        spec[k] = random_sign(&mut st.seed) * noise_val * gain;
                        continue;
                    }
                    let m = crate::tables::pow43(q.unsigned_abs().min(8191)) * gain;
                    spec[k] = if q < 0 { -m } else { m };
                }
            }
        }
        w0 += glen;
    }

    let fac = if br.read_flag()? {
        let lfac = if short { p.n / 16 } else { p.n / 8 };
        let mut v = vec![0i32; lfac + 1];
        v[0] = br.read_bits(7)? as i32;
        super::lpd::fac_decoding(br, lfac, &mut v[1..])?;
        Some(v)
    } else {
        None
    };
    Ok(FdChannel { info, spec, tns, fac })
}

/// Complex-prediction state of a CPE (`alpha_q_*_prev`, previous downmix).
#[derive(Clone)]
pub struct CplxState {
    pub alpha_re_prev: [i32; 64],
    pub alpha_im_prev: [i32; 64],
    /// Previous frame's final L/R spectra (`coef_save`).
    pub save: [Vec<f32>; 2],
}

impl CplxState {
    pub fn new(n: usize) -> CplxState {
        CplxState { alpha_re_prev: [0; 64], alpha_im_prev: [0; 64], save: [vec![0.0; n], vec![0.0; n]] }
    }
    pub fn reset_alpha(&mut self) {
        self.alpha_re_prev = [0; 64];
        self.alpha_im_prev = [0; 64];
    }
}

/// `ms_mask_present` and its data.
pub enum StereoMask {
    None,
    /// `ms_used[group][sfb]`.
    Ms(Box<[[bool; 64]; 8]>),
    Cplx(Box<CplxPred>),
}

/// `cplx_pred_data()`.
pub struct CplxPred {
    pub used: [[bool; 64]; 8],
    pub alpha_re: [[i32; 64]; 8],
    pub alpha_im: [[i32; 64]; 8],
    pub pred_dir: bool,
    pub complex_coef: bool,
    pub use_prev_frame: bool,
}

/// `ms_mask_present` + `ms_used` / `cplx_pred_data()`.
pub fn read_stereo_mask(br: &mut BitReader, info: &IcsInfo, max_sfb_ste: usize, num_bands: usize, indep: bool, cs: &mut CplxState) -> Result<StereoMask> {
    let mask = br.read_bits(2)?;
    let res = match mask {
        0 => StereoMask::None,
        1 => {
            let mut m = Box::new([[false; 64]; 8]);
            for row in m.iter_mut().take(info.num_groups) {
                for v in row.iter_mut().take(max_sfb_ste.min(64)) {
                    *v = br.read_flag()?;
                }
            }
            StereoMask::Ms(m)
        }
        2 => {
            let mut m = Box::new([[false; 64]; 8]);
            for row in m.iter_mut().take(info.num_groups) {
                for v in row.iter_mut().take(num_bands.min(64)) {
                    *v = true;
                }
            }
            StereoMask::Ms(m)
        }
        _ => {
            let ste = max_sfb_ste.min(64);
            let mut cp = Box::new(CplxPred {
                used: [[false; 64]; 8],
                alpha_re: [[0; 64]; 8],
                alpha_im: [[0; 64]; 8],
                pred_dir: false,
                complex_coef: false,
                use_prev_frame: false,
            });
            let all = br.read_flag()?;
            for g in 0..info.num_groups {
                if all {
                    for v in cp.used[g].iter_mut().take(ste) {
                        *v = true;
                    }
                } else {
                    let mut sfb = 0;
                    while sfb < ste {
                        let u = br.read_flag()?;
                        cp.used[g][sfb] = u;
                        if sfb + 1 < ste {
                            cp.used[g][sfb + 1] = u;
                        }
                        sfb += 2;
                    }
                }
            }
            cp.pred_dir = br.read_flag()?;
            cp.complex_coef = br.read_flag()?;
            if cp.complex_coef {
                cp.use_prev_frame = if indep { false } else { br.read_flag()? };
            }
            let delta_time = if indep { false } else { br.read_flag()? };
            for g in 0..info.num_groups {
                let mut sfb = 0;
                while sfb < ste {
                    let (last_re, last_im) = if delta_time {
                        (cs.alpha_re_prev[sfb], cs.alpha_im_prev[sfb])
                    } else if sfb > 0 {
                        (cp.alpha_re[g][sfb - 1], cp.alpha_im[g][sfb - 1])
                    } else {
                        (0, 0)
                    };
                    if cp.used[g][sfb] {
                        cp.alpha_re[g][sfb] = last_re - huffman::decode_sf(br)?;
                        cp.alpha_im[g][sfb] = if cp.complex_coef { last_im - huffman::decode_sf(br)? } else { 0 };
                    }
                    if sfb + 1 < ste {
                        cp.alpha_re[g][sfb + 1] = cp.alpha_re[g][sfb];
                        cp.alpha_im[g][sfb + 1] = cp.alpha_im[g][sfb];
                    }
                    cs.alpha_re_prev[sfb] = cp.alpha_re[g][sfb];
                    cs.alpha_im_prev[sfb] = cp.alpha_im[g][sfb];
                    sfb += 2;
                }
                for sfb in ste..64 {
                    cs.alpha_re_prev[sfb] = 0;
                    cs.alpha_im_prev[sfb] = 0;
                }
            }
            return Ok(StereoMask::Cplx(cp));
        }
    };
    cs.reset_alpha();
    Ok(res)
}

/// M/S: `l' = l + r`, `r' = l - r` in the flagged bands below `nbands`.
pub fn apply_ms(l: &mut [f32], r: &mut [f32], info: &IcsInfo, bands: &Bands, n_win: usize, used: &[[bool; 64]; 8], nbands: usize) {
    let off = bands.offsets;
    let mut w = 0;
    for g in 0..info.num_groups {
        for _ in 0..info.group_len[g] {
            for sfb in 0..nbands.min(off.len() - 1).min(64) {
                if !used[g][sfb] {
                    continue;
                }
                for k in w * n_win + off[sfb] as usize..w * n_win + off[sfb + 1] as usize {
                    let (a, b) = (l[k], r[k]);
                    l[k] = a + b;
                    r[k] = a - b;
                }
            }
            w += 1;
        }
    }
}

// MDST estimation filters (Q15) of `ixheaacd_ext_ch_ele.c`, indexed [prev_shape][cur_shape].
const MDST_LONG: [[[i16; 7]; 2]; 2] = [
    [[0, 0, -16384, 0, 16384, 0, 0], [-1499, -1876, -17718, 0, 17718, 1876, 1499]],
    [[-1499, 1876, -17718, 0, 17718, -1876, 1499], [-2998, 0, -19052, 0, 19052, 0, 2998]],
];
const MDST_START: [[[i16; 7]; 2]; 2] = [
    [[-3364, -3401, -18584, 0, 18584, 3401, 3364], [-3433, -3447, -18608, 0, 18608, 3447, 3433]],
    [[-4863, -1525, -19918, 0, 19918, 1525, 4863], [-4932, -1572, -19942, 0, 19942, 1572, 4932]],
];
const MDST_STOP: [[[i16; 7]; 2]; 2] = [
    [[-3364, 3401, -18584, 0, 18584, -3401, 3364], [-4863, 1525, -19918, 0, 19918, -1525, 4863]],
    [[-3433, 3447, -18608, 0, 18608, -3447, 3433], [-4932, 1572, -19942, 0, 19942, -1572, 4932]],
];
const MDST_STOPSTART: [[[i16; 7]; 2]; 2] = [
    [[-6728, 0, -20785, 0, 20785, 0, 6728], [-6797, -46, -20808, 0, 20808, 46, 6797]],
    [[-6797, 46, -20808, 0, 20808, 46, 6797], [-6866, 0, -20831, 0, 20831, 0, 6866]],
];
const MDST_PREV_LONG_SHORT_START: [[i16; 7]; 2] = [[0, 3477, 8192, 10430, 8192, 3477, 0], [1950, 4054, 6114, 6982, 6114, 4054, 1950]];
const MDST_PREV_STOP_STOPSTART: [[i16; 7]; 2] = [[1262, 1285, 1299, 1304, 1299, 1285, 1262], [857, 866, 871, 873, 871, 866, 857]];

/// `out[i] += f(i) * Σ_j in[i - 3 + j] * filter[j]` with symmetric extension at both ends;
/// `f` is `even` for even `i`, `odd` for odd `i`.
fn filter_and_add(input: &[f32], filter: &[i16; 7], out: &mut [f32], even: f32, odd: f32) {
    let len = input.len() as isize;
    let at = |i: isize| -> f32 {
        let j = if i < 0 {
            -i - 1
        } else if i >= len {
            2 * len - 1 - i
        } else {
            i
        };
        input.get(j as usize).copied().unwrap_or(0.0)
    };
    for (i, o) in out.iter_mut().enumerate().take(input.len()) {
        let mut sum = 0f32;
        for (j, &c) in filter.iter().enumerate() {
            sum += at(i as isize - 3 + j as isize) * c as f32;
        }
        *o += sum / 32768.0 * if i % 2 == 0 { even } else { odd };
    }
}

/// Complex-prediction upmix (`ixheaacd_cplx_pred_upmixing`): `l` holds the downmix, `r` the
/// residual on entry, L/R on exit.
#[allow(clippy::too_many_arguments)]
pub fn apply_cplx_pred(l: &mut [f32], r: &mut [f32], info: &IcsInfo, bands: &Bands, n: usize, cp: &CplxPred, dmx_re_prev: &[f32], prev_kbd: bool) {
    let short = info.seq.is_short();
    let n_win = if short { n / 8 } else { n };
    let off = bands.offsets;
    let nb = off.len() - 1;
    let factor = if cp.pred_dir { -1.0 } else { 1.0 };
    let mut dmx_re = vec![0f32; n];
    let mut w = 0;
    for g in 0..info.num_groups {
        for _ in 0..info.group_len[g] {
            for sfb in 0..nb.min(64) {
                for k in w * n_win + off[sfb] as usize..w * n_win + off[sfb + 1] as usize {
                    dmx_re[k] = if cp.used[g][sfb] { l[k] } else { 0.5 * (l[k] + factor * r[k]) };
                }
            }
            w += 1;
        }
    }
    let mut dmx_im = vec![0f32; n];
    if cp.complex_coef {
        let (p, c) = (prev_kbd as usize, info.kbd as usize);
        let (cur, prev) = match info.seq {
            WinSeq::OnlyLong | WinSeq::EightShort => (&MDST_LONG[p][c], &MDST_PREV_LONG_SHORT_START[p]),
            WinSeq::LongStart => (&MDST_START[p][c], &MDST_PREV_LONG_SHORT_START[p]),
            WinSeq::LongStop => (&MDST_STOP[p][c], &MDST_PREV_STOP_STOPSTART[p]),
            WinSeq::StopStart => (&MDST_STOPSTART[p][c], &MDST_PREV_STOP_STOPSTART[p]),
        };
        for w in 0..info.num_windows() {
            let range = w * n_win..(w + 1) * n_win;
            filter_and_add(&dmx_re[range.clone()], cur, &mut dmx_im[range.clone()], 1.0, 1.0);
            if w > 0 {
                let pr = (w - 1) * n_win..w * n_win;
                let prev_win: Vec<f32> = dmx_re[pr].to_vec();
                filter_and_add(&prev_win, prev, &mut dmx_im[range], -1.0, 1.0);
            } else if cp.use_prev_frame {
                filter_and_add(&dmx_re_prev[..n_win.min(dmx_re_prev.len())], prev, &mut dmx_im[range], -1.0, 1.0);
            }
        }
    }
    let mut w = 0;
    for g in 0..info.num_groups {
        for _ in 0..info.group_len[g] {
            for sfb in 0..nb.min(64) {
                if !cp.used[g][sfb] {
                    continue;
                }
                let are = cp.alpha_re[g][sfb] as f32 * 0.1;
                let aim = if cp.complex_coef { cp.alpha_im[g][sfb] as f32 * 0.1 } else { 0.0 };
                for k in w * n_win + off[sfb] as usize..w * n_win + off[sfb + 1] as usize {
                    let side = r[k] - are * l[k] - aim * dmx_im[k];
                    r[k] = factor * (l[k] - side);
                    l[k] += side;
                }
            }
            w += 1;
        }
    }
}
