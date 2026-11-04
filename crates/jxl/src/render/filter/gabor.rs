// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-render 0.12.4.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use crate::grid::{AlignedGrid, MutableSubgrid};
use crate::threadpool::JxlThreadPool;

use crate::render::{ImageWithRegion, Region};

use super::impls::generic::gabor::gabor_row_edge;

pub fn apply_gabor_like(
    fb: &mut ImageWithRegion,
    color_padded_region: Region,
    fb_scratch: &mut [AlignedGrid<f32>; 3],
    weights: [[f32; 2]; 3],
    pool: &crate::threadpool::JxlThreadPool,
) -> crate::render::Result<()> {
    crate::tracing::debug!("Running gaborish");
    let Some(&(region, _)) = fb.regions_and_shifts().first() else {
        return Ok(());
    };
    if !region.contains(color_padded_region) {
        return Err(crate::render::Error::Internal("gaborish region outside the frame buffer"));
    }
    let left = region.left.abs_diff(color_padded_region.left) as usize;
    let top = region.top.abs_diff(color_padded_region.top) as usize;
    let right = left + color_padded_region.width as usize;
    let bottom = top + color_padded_region.height as usize;

    let buffers = fb.as_color_floats_mut()?;
    let buffers = buffers.map(|g| g.as_subgrid_mut().subgrid(left..right, top..bottom));

    super::impls::apply_gabor_like(buffers, fb_scratch, weights, pool);

    let left = color_padded_region.left;
    let top = color_padded_region.top;
    for (idx, grid) in fb_scratch.iter_mut().enumerate() {
        let width = grid.width() as u32;
        let height = grid.height() as u32;
        let region = Region { width, height, left, top };
        fb.swap_channel_f32(idx, grid, region);
    }
    Ok(())
}

pub(super) struct GaborRow<'buf> {
    pub input_rows: [&'buf [f32]; 3],
    pub output_row: &'buf mut [f32],
    pub weights: [f32; 2],
}

pub(super) fn run_gabor_rows<'buf>(
    input: MutableSubgrid<'buf, f32>,
    output: &'buf mut AlignedGrid<f32>,
    weights: [f32; 2],
    pool: &JxlThreadPool,
    handle_row: for<'a> fn(GaborRow<'a>),
) {
    let width = input.width();
    let height = input.height();
    let output_buf = output.buf_mut();
    if output_buf.len() != width * height || width == 0 || height == 0 {
        crate::tracing::error!("gaborish buffer size mismatch");
        return;
    }

    if height == 1 {
        let input_buf = input.get_row(0);
        gabor_row_edge(input_buf, None, output_buf, weights);
        return;
    }

    {
        let input_buf_c = input.get_row(0);
        let input_buf_a = input.get_row(1);
        let output_buf = output_buf.get_mut(..width).unwrap_or_default();
        gabor_row_edge(input_buf_c, Some(input_buf_a), output_buf, weights);
    }

    let (inner_rows, bottom_row) = output_buf.get_mut(width..).unwrap_or_default().split_at_mut((height - 2) * width);
    let output_rows = inner_rows.chunks_mut(width * 8).enumerate().collect::<Vec<_>>();

    pool.for_each_vec(output_rows, |(y8, output_rows)| {
        let it = output_rows.chunks_exact_mut(width);
        for (dy, output_row) in it.enumerate() {
            let y_up = y8 * 8 + dy;
            let input_rows = [input.get_row(y_up), input.get_row(y_up + 1), input.get_row(y_up + 2)];
            let row = GaborRow { input_rows, output_row, weights };
            handle_row(row);
        }
    });

    {
        let input_buf_c = input.get_row(height - 1);
        let input_buf_a = input.get_row(height - 2);
        let output_buf = bottom_row;
        gabor_row_edge(input_buf_c, Some(input_buf_a), output_buf, weights);
    }
}

#[allow(unused)]
pub(crate) fn run_gabor_row_generic(row: GaborRow) {
    super::impls::generic::gabor::run_gabor_row_generic(row)
}
