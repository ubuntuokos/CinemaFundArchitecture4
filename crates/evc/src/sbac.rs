// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! Binary arithmetic decoder (SBAC) of the EVC Baseline profile and the syntax
//! elements it reads (`xevd_eco.c`).

use crate::Error;
use crate::bits::Bits;

const PROB_INIT: u16 = 512;

/// Context models. Baseline initialises every model to the same state; the
/// context-selection flags of the Main profile are always 0.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub(crate) split_cu_flag: [u16; 1],
    pub(crate) run: [u16; 24],
    pub(crate) last: [u16; 2],
    pub(crate) level: [u16; 24],
    pub(crate) cbf_luma: [u16; 1],
    pub(crate) cbf_cb: [u16; 1],
    pub(crate) cbf_cr: [u16; 1],
    pub(crate) cbf_all: [u16; 1],
    pub(crate) pred_mode: [u16; 3],
    pub(crate) direct_mode_flag: [u16; 1],
    pub(crate) inter_dir: [u16; 2],
    pub(crate) intra_dir: [u16; 2],
    pub(crate) mvp_idx: [u16; 3],
    pub(crate) mvd: [u16; 1],
    pub(crate) refi: [u16; 2],
    pub(crate) delta_qp: [u16; 1],
    pub(crate) skip_flag: [u16; 2],
}

impl Ctx {
    fn new() -> Self {
        Self {
            split_cu_flag: [PROB_INIT; 1],
            run: [PROB_INIT; 24],
            last: [PROB_INIT; 2],
            level: [PROB_INIT; 24],
            cbf_luma: [PROB_INIT; 1],
            cbf_cb: [PROB_INIT; 1],
            cbf_cr: [PROB_INIT; 1],
            cbf_all: [PROB_INIT; 1],
            pred_mode: [PROB_INIT; 3],
            direct_mode_flag: [PROB_INIT; 1],
            inter_dir: [PROB_INIT; 2],
            intra_dir: [PROB_INIT; 2],
            mvp_idx: [PROB_INIT; 3],
            mvd: [PROB_INIT; 1],
            refi: [PROB_INIT; 2],
            delta_qp: [PROB_INIT; 1],
            skip_flag: [PROB_INIT; 2],
        }
    }
}

/// Longest unary code accepted before the stream is called malformed. Valid
/// codes are far shorter (a run is below 64 × 64).
const MAX_UNARY: u32 = 1 << 13;

pub(crate) struct Sbac<'a> {
    pub(crate) bs: Bits<'a>,
    range: u32,
    value: u32,
    pub(crate) ctx: Ctx,
}

impl<'a> Sbac<'a> {
    /// `xevd_eco_sbac_reset`: reads the first 14 bits and resets all contexts.
    pub(crate) fn new(mut bs: Bits<'a>) -> Self {
        let mut value = 0u32;
        for _ in 0..14 {
            value = ((value << 1) | bs.bit()) & 0xFFFF;
        }
        Self { bs, range: 16384, value, ctx: Ctx::new() }
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.bs.overrun { Err(Error::Truncated) } else { Ok(()) }
    }

    pub(crate) fn bin(&mut self, model: &mut u16) -> u32 {
        let mut state = u32::from(*model >> 1);
        let mut mps = u32::from(*model & 1);
        let lps = ((state * self.range) >> 9).max(437);
        let bin;
        self.range = self.range.wrapping_sub(lps);
        if self.value >= self.range {
            bin = 1 - mps;
            self.value = self.value.wrapping_sub(self.range);
            self.range = lps;
            state += (512 - state + 16) >> 5;
            if state > 256 {
                mps = 1 - mps;
                state = 512 - state;
            }
        } else {
            bin = mps;
            state -= (state + 16) >> 5;
        }
        *model = ((state << 1) + mps) as u16;
        while self.range < 8192 {
            self.range <<= 1;
            self.value = ((self.value << 1) | self.bs.bit()) & 0xFFFF;
        }
        bin
    }

    pub(crate) fn bin_ep(&mut self) -> u32 {
        self.range >>= 1;
        let bin = if self.value >= self.range {
            self.value -= self.range;
            1
        } else {
            0
        };
        self.range <<= 1;
        self.value = ((self.value << 1) | self.bs.bit()) & 0xFFFF;
        bin
    }

    /// Terminating bin; on 1 the remaining bits up to byte alignment must be 0.
    pub(crate) fn bin_trm(&mut self) -> Result<u32, Error> {
        self.range = self.range.wrapping_sub(1);
        if self.value >= self.range {
            while !self.bs.is_byte_aligned() {
                if self.bs.bit() != 0 {
                    return Err(Error::Malformed("non-zero bits after the end of the tile"));
                }
            }
            Ok(1)
        } else {
            while self.range < 8192 {
                self.range <<= 1;
                self.value = ((self.value << 1) | self.bs.bit()) & 0xFFFF;
            }
            Ok(0)
        }
    }

    /// `sbac_read_unary_sym` over `models` (the last model repeats).
    pub(crate) fn unary(&mut self, which: Unary) -> Result<u32, Error> {
        let (base, num) = which.slot();
        if self.bin_unary(which, base) == 0 {
            return Ok(0);
        }
        let mut symbol = 0u32;
        let mut idx = 0usize;
        loop {
            if idx + 1 < num {
                idx += 1;
            }
            let t = self.bin_unary(which, base + idx);
            symbol += 1;
            if t == 0 {
                return Ok(symbol);
            }
            if symbol > MAX_UNARY || self.bs.overrun {
                return Err(Error::Malformed("unary code too long"));
            }
        }
    }

    fn bin_unary(&mut self, which: Unary, i: usize) -> u32 {
        let mut m = *self.model(which, i);
        let b = self.bin(&mut m);
        *self.model(which, i) = m;
        b
    }

    fn model(&mut self, which: Unary, i: usize) -> &mut u16 {
        let c = &mut self.ctx;
        let slot = match which {
            Unary::Run(_) => c.run.get_mut(i),
            Unary::Level(_) => c.level.get_mut(i),
            Unary::IntraDir => c.intra_dir.get_mut(i),
            Unary::DeltaQp => c.delta_qp.get_mut(i),
        };
        // Indices come from `Unary::slot` and stay inside the arrays; the
        // fallback model only exists so that no index can panic.
        slot.unwrap_or(&mut c.split_cu_flag[0])
    }

    /// `sbac_read_truncate_unary_sym` for the MVP index (3 models, max 4).
    pub(crate) fn mvp_idx(&mut self) -> u32 {
        let mut idx = 0u32;
        while idx < 3 {
            let mut m = self.ctx.mvp_idx[idx.min(2) as usize];
            let b = self.bin(&mut m);
            self.ctx.mvp_idx[idx.min(2) as usize] = m;
            if b == 0 {
                break;
            }
            idx += 1;
        }
        idx
    }

    pub(crate) fn bin_ctx(&mut self, sel: Model) -> u32 {
        let mut m = *self.ctx_ref(sel);
        let b = self.bin(&mut m);
        *self.ctx_ref(sel) = m;
        b
    }

    fn ctx_ref(&mut self, sel: Model) -> &mut u16 {
        let c = &mut self.ctx;
        match sel {
            Model::SplitCu => &mut c.split_cu_flag[0],
            Model::Last(chroma) => &mut c.last[usize::from(chroma)],
            Model::CbfLuma => &mut c.cbf_luma[0],
            Model::CbfCb => &mut c.cbf_cb[0],
            Model::CbfCr => &mut c.cbf_cr[0],
            Model::CbfAll => &mut c.cbf_all[0],
            Model::PredMode => &mut c.pred_mode[0],
            Model::DirectMode => &mut c.direct_mode_flag[0],
            Model::InterDir(i) => &mut c.inter_dir[usize::from(i)],
            Model::Refi(i) => &mut c.refi[usize::from(i)],
            Model::Mvd => &mut c.mvd[0],
            Model::Skip => &mut c.skip_flag[0],
        }
    }

    /// `xevd_eco_abs_mvd`: an Exp-Golomb-like code whose first bin uses the
    /// context model.
    pub(crate) fn abs_mvd(&mut self) -> Result<u32, Error> {
        let mut code = self.bin_ctx(Model::Mvd);
        if code != 0 {
            return Ok(0);
        }
        let mut len = 0u32;
        while code & 1 == 0 {
            code = if len == 0 { self.bin_ctx(Model::Mvd) } else { self.bin_ep() };
            len += 1;
            if len > 30 {
                return Err(Error::Malformed("motion vector difference too long"));
            }
        }
        let mut val = (1u32 << len) - 1;
        while len != 0 {
            len -= 1;
            val += self.bin_ep() << len;
        }
        Ok(val)
    }

    pub(crate) fn mvd(&mut self) -> Result<[i16; 2], Error> {
        let mut out = [0i16; 2];
        for v in &mut out {
            let t = self.abs_mvd()? as i16;
            if t != 0 {
                *v = if self.bin_ep() != 0 { t.wrapping_neg() } else { t };
            }
        }
        Ok(out)
    }

    /// `xevd_eco_refi`.
    pub(crate) fn refi(&mut self, num_refp: i32) -> i32 {
        let mut ref_num = 0;
        if num_refp > 1 && self.bin_ctx(Model::Refi(0)) != 0 {
            ref_num += 1;
            if num_refp > 2 && self.bin_ctx(Model::Refi(1)) != 0 {
                ref_num += 1;
                while ref_num < num_refp - 1 {
                    if self.bin_ep() == 0 {
                        break;
                    }
                    ref_num += 1;
                }
            }
        }
        ref_num
    }

    pub(crate) fn dqp(&mut self) -> Result<i32, Error> {
        let dqp = self.unary(Unary::DeltaQp)? as i32;
        if dqp > 0 && self.bin_ep() != 0 {
            return Ok(-dqp);
        }
        Ok(dqp)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Unary {
    /// Coefficient run; `true` for chroma.
    Run(bool),
    Level(bool),
    IntraDir,
    DeltaQp,
}

impl Unary {
    /// First model index and number of models.
    fn slot(self) -> (usize, usize) {
        match self {
            Unary::Run(c) | Unary::Level(c) => (if c { 2 } else { 0 }, 2),
            Unary::IntraDir => (0, 2),
            Unary::DeltaQp => (0, 1),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Model {
    SplitCu,
    Last(bool),
    CbfLuma,
    CbfCb,
    CbfCr,
    CbfAll,
    PredMode,
    DirectMode,
    InterDir(u8),
    Refi(u8),
    Mvd,
    Skip,
}
