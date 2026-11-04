// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-render 0.12.4.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use crate::grid::{AlignedGrid, AllocTracker, SharedSubgrid};
use crate::modular::ChannelShift;

use crate::render::{Region, Result};

pub fn apply_jpeg_upsampling_single(
    grid: SharedSubgrid<f32>,
    shift: ChannelShift,
    target_region: Region,
    tracker: Option<&AllocTracker>,
) -> Result<AlignedGrid<f32>> {
    fn interpolate(left: f32, center: f32, right: f32) -> (f32, f32) {
        (0.25 * left + 0.75 * center, 0.75 * center + 0.25 * right)
    }

    let height = grid.height();
    let target_width = target_region.width as usize;
    let target_height = target_region.height as usize;

    let mut out = AlignedGrid::with_alloc_tracker(target_width, target_height, tracker)?;
    let buf = out.buf_mut();

    let h_upsampled = shift.hshift() == 0;
    let v_upsampled = shift.vshift() == 0;

    for (y, out_row) in buf.chunks_exact_mut(target_width).take(height).enumerate() {
        let row = grid.get_row(y);
        if h_upsampled {
            out_row.copy_from_slice(&row[..target_width]);
            continue;
        }

        let (Some(&first_sample), Some(&last_sample)) = (row.first(), row.last()) else {
            continue;
        };
        let last_item = [last_sample; 2];
        let adj_samples = row.windows(2).chain(std::iter::once(last_item.as_ref()));

        let mut prev_sample = first_sample;
        let it = adj_samples.zip(out_row.chunks_mut(2));
        for (input, output) in it {
            let [curr, next] = *input else { continue };
            let (left, right) = interpolate(prev_sample, curr, next);
            match output {
                [a] => {
                    *a = left;
                }
                [a, b] => {
                    *a = left;
                    *b = right;
                }
                _ => {}
            }
            prev_sample = curr;
        }
    }

    // image is horizontally upsampled here
    if !v_upsampled {
        let mut prev_row = buf[(height - 1) * target_width..][..target_width].to_vec();
        for y in (0..height).rev() {
            let idx_base = y * target_width;
            let top_base = idx_base.saturating_sub(target_width);
            for x in 0..target_width {
                let curr_sample = buf[idx_base + x];

                // We're interpolating bottom-to-top.
                let (bottom, top) = interpolate(prev_row[x], curr_sample, buf[top_base + x]);
                buf[idx_base * 2 + x] = top;
                if y * 2 + 1 < target_height {
                    buf[idx_base * 2 + target_width + x] = bottom;
                }

                prev_row[x] = curr_sample;
            }
        }
    }

    Ok(out)
}
