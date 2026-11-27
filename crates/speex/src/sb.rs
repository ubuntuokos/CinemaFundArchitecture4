//! The sub-band decoder for wideband (16 kHz) and ultra-wideband (32 kHz) Speex: `sb_decode`
//! of `libspeex/sb_celp.c` with the high-band submodes of `modes_wb.c` (float build). The low
//! band is a narrowband decoder (wideband) or a wideband one (ultra-wideband); the high band is
//! an 8th-order LPC synthesis of a folded or coded excitation, and a QMF bank joins the two.

use crate::bits::Bits;
use crate::dsp::{self, SplitCb};
use crate::nb::Nb;
use crate::tables;

const ORDER: usize = 8;
const SB_SUBMODE_BITS: u32 = 3;
const VERY_SMALL: f32 = 1e-15;

/// `gc_quant_bound` (single-precision copies of the reference's double literals).
const GC_QUANT_BOUND: [f64; 16] =
    [0.97979, 1.28384, 1.68223, 2.20426, 2.88829, 3.78458, 4.95900, 6.49787, 8.51428, 11.15642, 14.61846, 19.15484, 25.09895, 32.88761, 43.09325, 56.46588];

const HIGH: SplitCb = SplitCb { subvect_size: 8, nb_subvect: 5, shape_cb: &tables::HEXC_TABLE, shape_bits: 7, have_sign: true };
const HIGH_LBR: SplitCb = SplitCb { subvect_size: 10, nb_subvect: 4, shape_cb: &tables::HEXC_10_32_TABLE, shape_bits: 5, have_sign: false };

/// The high-band submode: `None` folds the low-band innovation, otherwise a codebook (and
/// whether it is applied twice).
fn submode(uwb: bool, id: u32) -> Option<Option<(&'static SplitCb, bool)>> {
    match (uwb, id) {
        (_, 1) => Some(None),
        (false, 2) => Some(Some((&HIGH_LBR, false))),
        (false, 3) => Some(Some((&HIGH, false))),
        (false, 4) => Some(Some((&HIGH, true))),
        _ => None,
    }
}

pub(crate) enum Low {
    Nb(Box<Nb>),
    Sb(Box<Sb>),
}

impl Low {
    fn decode(&mut self, bits: &mut Bits<'_>, out: &mut [f32], innov_save: &mut [f32]) -> i32 {
        match self {
            Low::Nb(d) => d.decode(bits, out, Some(innov_save)),
            Low::Sb(d) => d.decode(bits, out, Some(innov_save)),
        }
    }
    fn dtx(&self) -> bool {
        match self {
            Low::Nb(d) => d.dtx,
            Low::Sb(d) => d.low.dtx(),
        }
    }
    fn pi_gain(&self) -> [f32; 4] {
        match self {
            Low::Nb(d) => d.pi_gain(),
            Low::Sb(d) => d.pi_gain,
        }
    }
    fn exc_rms(&self) -> [f32; 4] {
        match self {
            Low::Nb(d) => d.exc_rms(),
            Low::Sb(d) => d.exc_rms,
        }
    }
    pub(crate) fn nb_mut(&mut self) -> &mut Nb {
        match self {
            Low::Nb(d) => d,
            Low::Sb(d) => d.low.nb_mut(),
        }
    }
}

pub(crate) struct Sb {
    pub(crate) low: Low,
    uwb: bool,
    /// Samples of one band (half the output frame).
    frame_size: usize,
    subframe: usize,
    folding_gain: f32,
    submode: u32,
    first: bool,
    g0_mem: [f32; 64],
    g1_mem: [f32; 64],
    exc_buf: Vec<f32>,
    old_qlsp: [f32; ORDER],
    interp_qlpc: [f32; ORDER],
    mem_sp: [f32; ORDER],
    pi_gain: [f32; 4],
    exc_rms: [f32; 4],
    seed: u32,
    last_ener: f32,
}

impl Sb {
    /// A wideband decoder (`uwb` false) or an ultra-wideband one.
    pub(crate) fn new(uwb: bool) -> Self {
        let low = if uwb {
            Low::Sb(Box::new(Sb::new(false)))
        } else {
            let mut nb = Nb::new();
            nb.wideband = true;
            Low::Nb(Box::new(nb))
        };
        let (frame_size, subframe, folding_gain) = if uwb { (320, 80, 0.7) } else { (160, 40, 0.9) };
        Sb {
            low,
            uwb,
            frame_size,
            subframe,
            folding_gain,
            submode: if uwb { 1 } else { 3 },
            first: true,
            g0_mem: [0.0; 64],
            g1_mem: [0.0; 64],
            exc_buf: vec![0.0; subframe],
            old_qlsp: [0.0; ORDER],
            interp_qlpc: [0.0; ORDER],
            mem_sp: [0.0; ORDER],
            pi_gain: [0.0; 4],
            exc_rms: [0.0; 4],
            seed: 1000,
            last_ener: 0.0,
        }
    }

    /// `sb_decode_lost` for a DTX frame: noise at the last energy.
    fn decode_dtx(&mut self, out: &mut [f32]) {
        self.first = true;
        let (_, hi) = out.split_at_mut(self.frame_size);
        for v in hi.iter_mut() {
            *v = dsp::speex_rand(self.last_ener, &mut self.seed);
        }
        dsp::iir_mem16(hi, &self.interp_qlpc, &mut self.mem_sp);
        dsp::qmf_synth(out, &mut self.g0_mem, &mut self.g1_mem);
    }

    /// `sb_decode`: one frame (`2 * frame_size` samples) into `out`.
    pub(crate) fn decode(&mut self, bits: &mut Bits<'_>, out: &mut [f32], mut innov_save: Option<&mut [f32]>) -> i32 {
        let full = 2 * self.frame_size;
        let Some(out) = out.get_mut(..full) else { return -2 };
        let (lo, hi) = out.split_at_mut(self.frame_size);
        let ret = self.low.decode(bits, lo, hi);
        let dtx = self.low.dtx();
        if ret != 0 {
            return ret;
        }

        let wideband = if bits.remaining() > 0 { bits.peek1() } else { 0 };
        if wideband != 0 {
            bits.unpack(1);
            self.submode = bits.unpack(SB_SUBMODE_BITS);
        } else {
            self.submode = 0;
        }
        let mode = if self.submode == 0 {
            None
        } else {
            match submode(self.uwb, self.submode) {
                Some(m) => Some(m),
                None => return -2,
            }
        };
        let Some(mode) = mode else {
            if dtx {
                self.decode_dtx(out);
                return 0;
            }
            hi.fill(VERY_SMALL);
            self.first = true;
            dsp::iir_mem16(hi, &self.interp_qlpc, &mut self.mem_sp);
            dsp::qmf_synth(out, &mut self.g0_mem, &mut self.g1_mem);
            return 0;
        };

        let low_pi_gain = self.low.pi_gain();
        let low_exc_rms = self.low.exc_rms();
        let mut qlsp = [0f32; ORDER];
        dsp::lsp_unquant_high(&mut qlsp, bits);
        if self.first {
            self.old_qlsp = qlsp;
        }

        let sf = self.subframe;
        let mut interp_qlsp = [0f32; ORDER];
        let mut ak = [0f32; ORDER];
        let mut exc_ener_sum = 0f32;
        for sub in 0..4 {
            let offset = sf * sub;
            if let Some(save) = innov_save.as_deref_mut()
                && let Some(s) = save.get_mut(2 * offset..2 * offset + 2 * sf)
            {
                s.fill(0.0);
            }
            dsp::lsp_interpolate(&self.old_qlsp, &qlsp, &mut interp_qlsp, sub, 4, 0.05f64 as f32);
            dsp::lsp_to_lpc(&interp_qlsp, &mut ak);

            let mut pi = 1f32;
            let mut rh = 1f32;
            for i in (0..ORDER).step_by(2) {
                rh += ak[i + 1] - ak[i];
                pi += ak[i] + ak[i + 1];
            }
            self.pi_gain[sub] = pi;
            let rl = low_pi_gain[sub];
            let filter_ratio = ((f64::from(rl) + 0.01) / (f64::from(rh) + 0.01)) as f32;

            let mut exc = vec![0f32; sf];
            match mode {
                None => {
                    let quant = bits.unpack(5) as i32;
                    let g = f64::from(0.125f32 * (quant - 10) as f32).exp() as f32;
                    let g = g / filter_ratio;
                    let fg = self.folding_gain;
                    for i in (0..sf).step_by(2) {
                        exc[i] = (fg * hi[offset + i]) * g;
                        exc[i + 1] = -((fg * hi[offset + i + 1]) * g);
                    }
                }
                Some((cb, double)) => {
                    let qgc = bits.unpack(4) as usize;
                    let el = low_exc_rms[sub];
                    let bound = GC_QUANT_BOUND[qgc & 15] as f32;
                    let mut gc = (0.87360 * f64::from(bound)) as f32;
                    if sf == 80 {
                        // `QCONST16(1.4142f, 14)`: the reference rounds √2 to 1.4142.
                        #[allow(clippy::approx_constant)]
                        let sqrt2 = 1.4142f32;
                        gc *= sqrt2;
                    }
                    let scale = (gc * el) / filter_ratio;
                    dsp::split_cb_unquant(&mut exc, cb, bits);
                    for v in &mut exc {
                        *v *= scale;
                    }
                    if double {
                        let mut innov2 = vec![0f32; sf];
                        dsp::split_cb_unquant(&mut innov2, cb, bits);
                        let g = 0.4f32 * scale;
                        for (a, b) in exc.iter_mut().zip(innov2) {
                            *a += g * b;
                        }
                    }
                }
            }
            if let Some(save) = innov_save.as_deref_mut() {
                for (i, &v) in exc.iter().enumerate() {
                    if let Some(s) = save.get_mut(2 * offset + 2 * i) {
                        *s = v;
                    }
                }
            }
            // The previous subframe's excitation through the previous subframe's filter.
            let sp = &mut hi[offset..offset + sf];
            sp.copy_from_slice(&self.exc_buf);
            dsp::iir_mem16(sp, &self.interp_qlpc, &mut self.mem_sp);
            self.exc_buf.copy_from_slice(&exc);
            self.interp_qlpc = ak;
            let rms = dsp::compute_rms(&self.exc_buf);
            self.exc_rms[sub] = rms;
            exc_ener_sum += (rms * rms) / 4.0;
        }
        self.last_ener = f64::from(exc_ener_sum).sqrt() as f32;
        dsp::qmf_synth(out, &mut self.g0_mem, &mut self.g1_mem);
        self.old_qlsp = qlsp;
        self.first = false;
        0
    }
}
