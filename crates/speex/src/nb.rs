//! The narrowband (8 kHz) CELP decoder: `nb_decode` of `libspeex/nb_celp.c` with the submodes
//! of `modes.c`, `pitch_unquant_3tap` / `forced_pitch_unquant` of `ltp.c` and the innovation
//! codebooks of `cb_search.c` (float build). Packet-loss concealment is not needed: a file
//! decode never loses a packet.

use crate::bits::Bits;
use crate::dsp::{self, SplitCb, at};
use crate::tables;

pub(crate) const FRAME: usize = 160;
const SUBFRAME: usize = 40;
const ORDER: usize = 10;
const PITCH_START: i32 = 17;
const PITCH_END: i32 = 144;
/// `NB_DEC_BUFFER` and the offset of the current frame's excitation in it.
const EXC_BUF: usize = FRAME + 2 * PITCH_END as usize + SUBFRAME + 12;
const EXC: usize = 2 * PITCH_END as usize + SUBFRAME + 6;
const SB_SUBMODE_BITS: u32 = 3;
const WB_SKIP: [i64; 8] = [0, 36, 112, 192, 352, 0, 0, 0];
const INBAND_STEREO: u32 = 9;

enum Ltp {
    Forced,
    ThreeTap { gain_cdbk: &'static [i8], gain_bits: u32, pitch_bits: u32 },
}

enum Innov {
    Noise,
    Split(SplitCb),
}

struct Submode {
    lbr_pitch: i32,
    forced_pitch_gain: bool,
    subframe_gain: u8,
    double_codebook: bool,
    lsp_nb: bool,
    ltp: Ltp,
    innov: Innov,
    comb_gain: f32,
}

const LTP_NB: Ltp = Ltp::ThreeTap { gain_cdbk: &tables::GAIN_CDBK_NB, gain_bits: 7, pitch_bits: 7 };
const LTP_VLBR: Ltp = Ltp::ThreeTap { gain_cdbk: &tables::GAIN_CDBK_LBR, gain_bits: 5, pitch_bits: 0 };
const LTP_LBR: Ltp = Ltp::ThreeTap { gain_cdbk: &tables::GAIN_CDBK_LBR, gain_bits: 5, pitch_bits: 7 };

const fn split(subvect_size: usize, nb_subvect: usize, shape_cb: &'static [i8], shape_bits: u32) -> Innov {
    Innov::Split(SplitCb { subvect_size, nb_subvect, shape_cb, shape_bits, have_sign: false })
}

fn submode(id: u32) -> Option<Submode> {
    let s = |lbr_pitch, forced_pitch_gain, subframe_gain, double_codebook, lsp_nb, ltp, innov, comb_gain: f64| Submode {
        lbr_pitch,
        forced_pitch_gain,
        subframe_gain,
        double_codebook,
        lsp_nb,
        ltp,
        innov,
        comb_gain: comb_gain as f32,
    };
    Some(match id {
        1 => s(0, true, 0, false, false, Ltp::Forced, Innov::Noise, -1.0),
        2 => s(0, false, 0, false, false, LTP_VLBR, split(10, 4, &tables::EXC_10_16_TABLE, 4), 0.6),
        3 => s(-1, false, 1, false, false, LTP_LBR, split(10, 4, &tables::EXC_10_32_TABLE, 5), 0.55),
        4 => s(-1, false, 1, false, false, LTP_LBR, split(8, 5, &tables::EXC_8_128_TABLE, 7), 0.45),
        5 => s(-1, false, 3, false, true, LTP_NB, split(5, 8, &tables::EXC_5_64_TABLE, 6), 0.25),
        6 => s(-1, false, 3, false, true, LTP_NB, split(5, 8, &tables::EXC_5_256_TABLE, 8), 0.15),
        7 => s(-1, false, 3, true, true, LTP_NB, split(5, 8, &tables::EXC_5_64_TABLE, 6), 0.05),
        8 => s(0, true, 0, false, false, Ltp::Forced, split(20, 2, &tables::EXC_20_32_TABLE, 5), 0.5),
        _ => return None,
    })
}

const SCAL3: [f32; 8] = [0.061_13, 0.163_546, 0.310_413, 0.428_22, 0.555_887, 0.719_055, 0.938_694, 1.326_874];
const SCAL1: [f32; 2] = [0.704_69, 1.051_27];

/// In-band stereo parameters (`speex_std_stereo_request_handler`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StereoParams {
    pub balance: f32,
    pub e_ratio: f32,
}

pub(crate) struct Nb {
    exc_buf: [f32; EXC_BUF],
    old_qlsp: [f32; ORDER],
    interp_qlpc: [f32; ORDER],
    mem_sp: [f32; ORDER],
    mem_hp: [f32; 2],
    pi_gain: [f32; 4],
    submode: u32,
    first: bool,
    pitch_gain_buf: [f32; 3],
    pitch_gain_idx: usize,
    seed: u32,
    voc_m1: f32,
    voc_m2: f32,
    voc_mean: f32,
    voc_offset: i32,
    pub(crate) dtx: bool,
    pub(crate) wideband: bool,
    pub(crate) enhance: bool,
    /// Set when the stream is stereo: in-band stereo requests update it.
    pub(crate) stereo: Option<StereoParams>,
}

impl Nb {
    pub(crate) fn new() -> Self {
        Nb {
            exc_buf: [0.0; EXC_BUF],
            old_qlsp: [0.0; ORDER],
            interp_qlpc: [0.0; ORDER],
            mem_sp: [0.0; ORDER],
            mem_hp: [0.0; 2],
            pi_gain: [0.0; 4],
            submode: 5,
            first: true,
            pitch_gain_buf: [0.0; 3],
            pitch_gain_idx: 0,
            seed: 1000,
            voc_m1: 0.0,
            voc_m2: 0.0,
            voc_mean: 0.0,
            voc_offset: 0,
            dtx: false,
            wideband: false,
            enhance: true,
            stereo: None,
        }
    }

    pub(crate) fn pi_gain(&self) -> [f32; 4] {
        self.pi_gain
    }

    /// `SPEEX_GET_EXC`: the RMS of each subframe's excitation.
    pub(crate) fn exc_rms(&self) -> [f32; 4] {
        std::array::from_fn(|i| dsp::compute_rms(&self.exc_buf[EXC + i * SUBFRAME..EXC + (i + 1) * SUBFRAME]))
    }

    /// `speex_inband_handler` for a Speex in-band request (only stereo is acted on).
    fn inband(&mut self, bits: &mut Bits) {
        let id = bits.unpack(4);
        if id == INBAND_STEREO
            && let Some(st) = self.stereo.as_mut()
        {
            const E_RATIO: [f32; 4] = [0.25, 0.315, 0.397, 0.5];
            let sign: f32 = if bits.unpack(1) != 0 { -1.0 } else { 1.0 };
            let dexp = bits.unpack(5) as f32;
            st.balance = (f64::from(sign) * 0.25 * f64::from(dexp)).exp() as f32;
            st.e_ratio = E_RATIO[bits.unpack(2) as usize & 3];
            return;
        }
        let adv = match id {
            0..2 => 1,
            2..8 => 4,
            8..10 => 8,
            10..12 => 16,
            12..14 => 32,
            _ => 64,
        };
        bits.advance(adv);
    }

    /// `nb_decode`: one 160-sample frame into `out`; the innovation is copied to `innov_save`
    /// when given. Returns 0, -1 (end of the packet) or -2 (corrupt stream).
    pub(crate) fn decode(&mut self, bits: &mut Bits, out: &mut [f32], mut innov_save: Option<&mut [f32]>) -> i32 {
        let Some(out) = out.get_mut(..FRAME) else { return -2 };
        let m = loop {
            if bits.remaining() < 5 {
                return -1;
            }
            if bits.unpack(1) != 0 {
                let sub = bits.unpack(SB_SUBMODE_BITS) as usize;
                bits.advance(WB_SKIP[sub & 7] - i64::from(SB_SUBMODE_BITS + 1));
                if bits.remaining() < 5 {
                    return -1;
                }
                if bits.unpack(1) != 0 {
                    let sub = bits.unpack(SB_SUBMODE_BITS) as usize;
                    bits.advance(WB_SKIP[sub & 7] - i64::from(SB_SUBMODE_BITS + 1));
                    if bits.unpack(1) != 0 {
                        return -2;
                    }
                }
            }
            if bits.remaining() < 4 {
                return -1;
            }
            let m = bits.unpack(4);
            match m {
                15 => return -1,
                14 => self.inband(bits),
                13 => {
                    let req = bits.unpack(4);
                    bits.advance(5 + 8 * i64::from(req));
                }
                9..=12 => return -2,
                _ => break m,
            }
        };
        self.submode = m;

        self.exc_buf.copy_within(FRAME.., 0);

        let Some(mode) = submode(m) else {
            // Null mode: comfort noise of the previous excitation's level.
            let mut lpc = [0f32; ORDER];
            dsp::bw_lpc(0.93, &self.interp_qlpc, &mut lpc);
            let innov_gain = dsp::compute_rms(&self.exc_buf[EXC..EXC + FRAME]);
            for v in &mut self.exc_buf[EXC..EXC + FRAME] {
                *v = dsp::speex_rand(innov_gain, &mut self.seed);
            }
            self.first = true;
            out.copy_from_slice(&self.exc_buf[EXC..EXC + FRAME]);
            dsp::iir_mem16(out, &lpc, &mut self.mem_sp);
            return 0;
        };

        let mut qlsp = [0f32; ORDER];
        if mode.lsp_nb {
            dsp::lsp_unquant_nb(&mut qlsp, bits);
        } else {
            dsp::lsp_unquant_lbr(&mut qlsp, bits);
        }
        if self.first {
            self.old_qlsp = qlsp;
        }
        let mut ol_pitch = 0i32;
        if mode.lbr_pitch != -1 {
            ol_pitch = PITCH_START + bits.unpack(7) as i32;
        }
        let mut ol_pitch_coef = 0f32;
        if mode.forced_pitch_gain {
            let quant = bits.unpack(4);
            ol_pitch_coef = (0.066667 * f64::from(quant)) as f32;
        }
        let qe = bits.unpack(5);
        let ol_gain = (f64::from(qe) / 3.5).exp() as f32;
        if m == 1 {
            self.dtx = bits.unpack(4) == 15;
        }
        if m > 1 {
            self.dtx = false;
        }

        let mut best_pitch = 40i32;
        let mut best_pitch_gain = 0f32;
        let mut pitch_average = 0f32;
        for sub in 0..4 {
            let offset = SUBFRAME * sub;
            let e = EXC + offset;
            self.exc_buf[e..e + SUBFRAME].fill(0.0);

            // The smallest pitch allowed (the largest only matters to the encoder).
            let pit_min = match mode.lbr_pitch {
                -1 => PITCH_START,
                0 => ol_pitch,
                margin => (ol_pitch - margin + 1).max(PITCH_START),
            };

            let mut exc32 = [0f32; SUBFRAME];
            let (pitch, gain) = match &mode.ltp {
                Ltp::Forced => {
                    let coef = if f64::from(ol_pitch_coef) > 0.99 { 0.99f32 } else { ol_pitch_coef };
                    for i in 0..SUBFRAME {
                        let v = at(&self.exc_buf, e as isize + i as isize - pit_min as isize) * coef;
                        exc32[i] = v;
                        self.exc_buf[e + i] = v;
                    }
                    (pit_min, [0.0, coef, 0.0])
                }
                Ltp::ThreeTap { gain_cdbk, gain_bits, pitch_bits } => {
                    let pitch = bits.unpack(*pitch_bits) as i32 + pit_min;
                    let gi = bits.unpack(*gain_bits) as usize * 4;
                    let g = |k: usize| (0.015625 * f64::from(gain_cdbk.get(gi + k).copied().unwrap_or(0)) + 0.5) as f32;
                    let gain = [g(0), g(1), g(2)];
                    for i in 0..3 {
                        let pp = (pitch + 1 - i as i32) as isize;
                        let gg = gain[2 - i];
                        let tmp1 = (SUBFRAME as isize).min(pp).max(0) as usize;
                        for j in 0..tmp1 {
                            exc32[j] += gg * at(&self.exc_buf, e as isize + j as isize - pp);
                        }
                        let tmp3 = (SUBFRAME as isize).min(pp + pitch as isize).max(0) as usize;
                        for j in tmp1..tmp3 {
                            exc32[j] += gg * at(&self.exc_buf, e as isize + j as isize - pp - pitch as isize);
                        }
                    }
                    (pitch, gain)
                }
            };
            dsp::sanitize(&mut exc32, -32000.0, 32000.0);

            let tmp = gain_3tap_to_1tap(&gain);
            pitch_average += tmp;
            let (bp, p) = (best_pitch, pitch);
            let far = (2 * bp - p).abs() >= 3 && (3 * bp - p).abs() >= 4 && (4 * bp - p).abs() >= 5;
            let sub_multiple = (bp - 2 * p).abs() < 3 || (bp - 3 * p).abs() < 4 || (bp - 4 * p).abs() < 5;
            let multiple = (2 * bp - p).abs() < 3 || (3 * bp - p).abs() < 4 || (4 * bp - p).abs() < 5;
            if (tmp > best_pitch_gain && far)
                || (f64::from(tmp) > 0.6 * f64::from(best_pitch_gain) && sub_multiple)
                || (0.67 * f64::from(tmp) > f64::from(best_pitch_gain) && multiple)
            {
                best_pitch = pitch;
                if tmp > best_pitch_gain {
                    best_pitch_gain = tmp;
                }
            }

            // Innovation.
            let mut innov = [0f32; SUBFRAME];
            let ener = match mode.subframe_gain {
                3 => SCAL3[bits.unpack(3) as usize & 7] * ol_gain,
                1 => SCAL1[bits.unpack(1) as usize & 1] * ol_gain,
                _ => ol_gain,
            };
            match &mode.innov {
                Innov::Noise => {
                    for v in &mut innov {
                        *v = dsp::speex_rand(1.0, &mut self.seed);
                    }
                }
                Innov::Split(p) => dsp::split_cb_unquant(&mut innov, p, bits),
            }
            for v in &mut innov {
                *v *= ener;
            }
            if mode.double_codebook
                && let Innov::Split(p) = &mode.innov
            {
                let mut innov2 = [0f32; SUBFRAME];
                dsp::split_cb_unquant(&mut innov2, p, bits);
                let g = 0.454_545f32 * ener;
                for (a, b) in innov.iter_mut().zip(innov2) {
                    *a += g * b;
                }
            }
            for i in 0..SUBFRAME {
                self.exc_buf[e + i] = exc32[i] + innov[i];
            }
            if let Some(save) = innov_save.as_deref_mut()
                && let Some(s) = save.get_mut(offset..offset + SUBFRAME)
            {
                s.copy_from_slice(&innov);
            }

            if m == 1 {
                // Vocoder mode.
                let mut g = 1.5f32 * (ol_pitch_coef - 0.2);
                g = g.clamp(0.0, 1.0);
                let exc = &mut self.exc_buf[e..e + SUBFRAME];
                exc.fill(0.0);
                let pulse = f64::from(2 * ol_pitch).sqrt() as f32 * (g * ol_gain);
                while self.voc_offset < SUBFRAME as i32 {
                    if self.voc_offset >= 0
                        && let Some(v) = exc.get_mut(self.voc_offset as usize)
                    {
                        *v = pulse;
                    }
                    self.voc_offset += ol_pitch.max(1);
                }
                self.voc_offset -= SUBFRAME as i32;
                for i in 0..SUBFRAME {
                    let exci = exc[i];
                    exc[i] = (0.7f32 * exc[i] + 0.3f32 * self.voc_m1) + ((1.0 - 0.85f32 * g) * innov[i] - (0.15f32 * g) * self.voc_m2);
                    self.voc_m1 = exci;
                    self.voc_m2 = innov[i];
                    self.voc_mean = 0.8f32 * self.voc_mean + 0.2f32 * exc[i];
                    exc[i] -= self.voc_mean;
                }
            }
        }

        if self.enhance && mode.comb_gain > 0.0 {
            dsp::multicomb(&self.exc_buf, (EXC - SUBFRAME) as isize, &mut out[..2 * SUBFRAME], 2 * SUBFRAME, best_pitch, 40, mode.comb_gain);
            dsp::multicomb(&self.exc_buf, (EXC + SUBFRAME) as isize, &mut out[2 * SUBFRAME..], 2 * SUBFRAME, best_pitch, 40, mode.comb_gain);
        } else {
            out.copy_from_slice(&self.exc_buf[EXC - SUBFRAME..EXC - SUBFRAME + FRAME]);
        }

        let mut interp_qlsp = [0f32; ORDER];
        let mut ak = [0f32; ORDER];
        for sub in 0..4 {
            dsp::lsp_interpolate(&self.old_qlsp, &qlsp, &mut interp_qlsp, sub, 4, 0.002);
            dsp::lsp_to_lpc(&interp_qlsp, &mut ak);
            let mut pi_g = 1f32;
            for i in (0..ORDER).step_by(2) {
                pi_g += ak[i + 1] - ak[i];
            }
            self.pi_gain[sub] = pi_g;
            dsp::iir_mem16(&mut out[sub * SUBFRAME..(sub + 1) * SUBFRAME], &self.interp_qlpc, &mut self.mem_sp);
            self.interp_qlpc = ak;
        }
        dsp::highpass(out, self.wideband, &mut self.mem_hp);

        self.old_qlsp = qlsp;
        self.first = false;
        let last_pitch_gain = (0.25 * f64::from(pitch_average)) as f32;
        self.pitch_gain_buf[self.pitch_gain_idx] = last_pitch_gain;
        self.pitch_gain_idx = (self.pitch_gain_idx + 1) % 3;
        0
    }
}

/// `gain_3tap_to_1tap`, in double as the macro's `-.5*g` branches make it.
fn gain_3tap_to_1tap(g: &[f32; 3]) -> f32 {
    let side = |v: f32| if v > 0.0 { f64::from(v) } else { -0.5 * f64::from(v) };
    (f64::from(g[1].abs()) + side(g[0]) + side(g[2])) as f32
}
