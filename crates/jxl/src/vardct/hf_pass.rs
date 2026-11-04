// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-vardct 0.11.1.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use crate::bitstream::{Bitstream, U};
use crate::coding::Decoder;
use crate::common::Bundle;

/// Parameters for decoding `HfPass`.
#[derive(Debug, Copy, Clone)]
pub struct HfPassParams<'a> {
    hf_block_ctx: &'a crate::vardct::HfBlockContext,
    num_hf_presets: u32,
}

impl<'a> HfPassParams<'a> {
    pub fn new(hf_block_ctx: &'a crate::vardct::HfBlockContext, num_hf_presets: u32) -> Self {
        Self { hf_block_ctx, num_hf_presets }
    }
}

/// HF coefficient decoder configuration.
///
/// Includes distribution for the entropy decoder and the order of HF coefficients. This struct is
/// passed as a parameter when [decoding HF coefficients][crate::vardct::write_hf_coeff].
#[derive(Debug)]
pub struct HfPass {
    permutation: [[Vec<(u16, u16)>; 3]; 13],
    hf_dist: Decoder,
}

impl Bundle<HfPassParams<'_>> for HfPass {
    type Error = crate::vardct::Error;

    fn parse(bitstream: &mut Bitstream, params: HfPassParams<'_>) -> crate::vardct::Result<Self> {
        let HfPassParams { hf_block_ctx, num_hf_presets } = params;
        let mut used_orders = bitstream.read_u32(0x5F, 0x13, 0x00, U(13))?;
        let mut decoder = (used_orders != 0).then(|| Decoder::parse(bitstream, 8)).transpose()?;

        let mut permutation: [_; 13] = std::array::from_fn(|_| [Vec::new(), Vec::new(), Vec::new()]);
        if let Some(decoder) = &mut decoder {
            let it = permutation.iter_mut().zip(BLOCK_SIZES).enumerate();
            for (idx, (permutation, (bw, bh))) in it {
                if used_orders & 1 != 0 {
                    let size = (bw * bh) as u32;
                    let skip = size / 64;
                    for permutation in permutation {
                        let perm = crate::coding::read_permutation(bitstream, decoder, size, skip)?;
                        let nat = natural_order_lazy(idx);
                        for idx in perm {
                            permutation.push(nat[idx]);
                        }
                    }
                }

                used_orders >>= 1;
            }
            decoder.finalize()?;
        }

        let hf_dist = Decoder::parse(bitstream, 495 * num_hf_presets * hf_block_ctx.num_block_clusters)?;

        Ok(Self { permutation, hf_dist })
    }
}

impl HfPass {
    #[inline]
    pub(crate) fn clone_decoder(&self) -> Decoder {
        self.hf_dist.clone()
    }

    #[inline]
    pub(crate) fn order(&self, order_id: usize, channel: usize) -> &[(u16, u16)] {
        let permutation = &self.permutation[order_id][channel];
        if permutation.is_empty() { natural_order_lazy(order_id) } else { permutation }
    }
}

const BLOCK_SIZES: [(usize, usize); 13] =
    [(8, 8), (8, 8), (16, 16), (32, 32), (16, 8), (32, 8), (32, 16), (64, 64), (64, 32), (128, 128), (128, 64), (256, 256), (256, 128)];

const NATURAL_ORDER: [&[(u16, u16)]; 9] = [
    &const_compute_natural_order::<{ BLOCK_SIZES[0].0 * BLOCK_SIZES[0].1 }>(BLOCK_SIZES[0]),
    &const_compute_natural_order::<{ BLOCK_SIZES[1].0 * BLOCK_SIZES[1].1 }>(BLOCK_SIZES[1]),
    &const_compute_natural_order::<{ BLOCK_SIZES[2].0 * BLOCK_SIZES[2].1 }>(BLOCK_SIZES[2]),
    &const_compute_natural_order::<{ BLOCK_SIZES[3].0 * BLOCK_SIZES[3].1 }>(BLOCK_SIZES[3]),
    &const_compute_natural_order::<{ BLOCK_SIZES[4].0 * BLOCK_SIZES[4].1 }>(BLOCK_SIZES[4]),
    &const_compute_natural_order::<{ BLOCK_SIZES[5].0 * BLOCK_SIZES[5].1 }>(BLOCK_SIZES[5]),
    &const_compute_natural_order::<{ BLOCK_SIZES[6].0 * BLOCK_SIZES[6].1 }>(BLOCK_SIZES[6]),
    &const_compute_natural_order::<{ BLOCK_SIZES[7].0 * BLOCK_SIZES[7].1 }>(BLOCK_SIZES[7]),
    &const_compute_natural_order::<{ BLOCK_SIZES[8].0 * BLOCK_SIZES[8].1 }>(BLOCK_SIZES[8]),
];

pub const DCT8_NATURAL_ORDER: &[(u16, u16)] = NATURAL_ORDER[0];

fn natural_order_lazy(idx: usize) -> &'static [(u16, u16)] {
    let Some(&block_size) = BLOCK_SIZES.get(idx) else {
        return &[];
    };
    let Some(large_idx) = idx.checked_sub(NATURAL_ORDER.len()) else {
        return NATURAL_ORDER.get(idx).copied().unwrap_or(&[]);
    };

    static LARGE_NATURAL_ORDER: [std::sync::OnceLock<Vec<(u16, u16)>>; 4] =
        [std::sync::OnceLock::new(), std::sync::OnceLock::new(), std::sync::OnceLock::new(), std::sync::OnceLock::new()];
    let Some(cell) = LARGE_NATURAL_ORDER.get(large_idx) else {
        return &[];
    };
    cell.get_or_init(|| {
        let mut natural_order = vec![(0, 0); block_size.0 * block_size.1];
        fill_natural_order(block_size, &mut natural_order);
        natural_order
    })
}

const fn const_compute_natural_order<const N: usize>((bw, bh): (usize, usize)) -> [(u16, u16); N] {
    let y_scale = bw / bh;

    let mut ret = [(0u16, 0u16); N];
    let mut idx = 0usize;
    let lbw = bw / 8;
    let lbh = bh / 8;

    while idx < lbw * lbh {
        let x = idx % lbw;
        let y = idx / lbw;
        ret[idx] = (x as u16, y as u16);
        idx += 1;
    }

    let mut dist = 1usize;
    while dist < 2 * bw {
        let margin = dist.saturating_sub(bw);
        let mut order = margin;
        while order < dist - margin {
            let (x, y) = if dist % 2 == 1 { (order, dist - 1 - order) } else { (dist - 1 - order, order) };
            order += 1;

            if x < lbw && y < lbw {
                continue;
            }
            if y % y_scale != 0 {
                continue;
            }
            ret[idx] = (x as u16, (y / y_scale) as u16);
            idx += 1;
        }
        dist += 1;
    }

    ret
}

fn fill_natural_order((bw, bh): (usize, usize), output: &mut [(u16, u16)]) {
    let y_scale = bw / bh;

    let mut idx = 0usize;
    let lbw = bw / 8;
    let lbh = bh / 8;

    while idx < lbw * lbh {
        let x = idx % lbw;
        let y = idx / lbw;
        output[idx] = (x as u16, y as u16);
        idx += 1;
    }

    for dist in 1..(2 * bw) {
        let margin = dist.saturating_sub(bw);
        for order in margin..(dist - margin) {
            let (x, y) = if dist % 2 == 1 { (order, dist - 1 - order) } else { (dist - 1 - order, order) };

            if x < lbw && y < lbw {
                continue;
            }
            if y % y_scale != 0 {
                continue;
            }
            output[idx] = (x as u16, (y / y_scale) as u16);
            idx += 1;
        }
    }
}
