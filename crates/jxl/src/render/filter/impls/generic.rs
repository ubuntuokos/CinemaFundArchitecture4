// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-render 0.12.4.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use crate::frame::{FrameHeader, filter::EpfParams};
use crate::grid::{AlignedGrid, MutableSubgrid};
use crate::threadpool::JxlThreadPool;

use crate::render::{
    Region,
    filter::{epf::run_epf_rows, gabor::run_gabor_rows},
};

pub(crate) mod epf;
pub(crate) mod gabor;

pub fn epf<const STEP: usize>(
    input: &mut [MutableSubgrid<f32>; 3],
    output: &mut [MutableSubgrid<f32>; 3],
    color_padded_region: Region,
    frame_header: &FrameHeader,
    sigma_grid_map: &[Option<&AlignedGrid<f32>>],
    epf_params: &EpfParams,
    pool: &JxlThreadPool,
) {
    run_epf_rows(input, output, color_padded_region, frame_header, sigma_grid_map, epf_params, pool, epf::epf_row::<STEP>)
}

pub fn apply_gabor_like(fb: [MutableSubgrid<f32>; 3], fb_scratch: &mut [AlignedGrid<f32>; 3], weights: [[f32; 2]; 3], pool: &crate::threadpool::JxlThreadPool) {
    for ((input, output), weights) in fb.into_iter().zip(fb_scratch).zip(weights) {
        run_gabor_rows(input, output, weights, pool, gabor::run_gabor_row_generic);
    }
}
