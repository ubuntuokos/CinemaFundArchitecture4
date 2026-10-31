// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-modular 0.11.3.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use std::num::Wrapping;

use crate::grid::MutableSubgrid;

use crate::modular::Sample;

pub fn inverse_h<S: Sample>(merged: &mut MutableSubgrid<'_, S>) {
    if let Some(merged) = S::try_as_mutable_subgrid_i16(merged) {
        inverse_h_i16(merged)
    } else if let Some(merged) = S::try_as_mutable_subgrid_i32(merged) {
        inverse_h_i32(merged)
    }
}

fn inverse_h_i32(merged: &mut MutableSubgrid<i32>) {
    inverse_h_i32_base(merged)
}

#[allow(unreachable_code)]
fn inverse_h_i16(merged: &mut MutableSubgrid<i16>) {
    inverse_h_i16_base(merged)
}

fn inverse_h_i32_base(merged: &mut MutableSubgrid<'_, i32>) {
    let height = merged.height();
    let width = merged.width();
    let mut scratch = vec![0i32; width];
    let avg_width = width.div_ceil(2);
    for y in 0..height {
        let row_out = merged.get_row_mut(y);
        scratch.copy_from_slice(row_out);

        let (avg_row, residu_row) = scratch.split_at_mut(avg_width);
        let mut avg = avg_row[0];
        let mut left = avg;
        let mut row_out_it = row_out.chunks_exact_mut(2);
        for (x, pair) in (&mut row_out_it).enumerate() {
            let residu = residu_row[x];
            let next_avg = avg_row.get(x + 1).copied().unwrap_or(avg);
            let diff = residu.wrapping_add(tendency_i32(left, avg, next_avg));
            let first = avg.wrapping_add(diff / 2);
            let second = first.wrapping_sub(diff);
            pair[0] = first;
            pair[1] = second;
            avg = next_avg;
            left = second;
        }

        if let [v] = row_out_it.into_remainder() {
            *v = avg_row[avg_width - 1];
        }
    }
}

#[inline(never)]
fn inverse_h_i16_base(merged: &mut MutableSubgrid<'_, i16>) {
    let height = merged.height();
    let width = merged.width();
    let mut scratch = vec![0i16; width];
    let avg_width = width.div_ceil(2);
    for y in 0..height {
        let row_out = merged.get_row_mut(y);
        scratch.copy_from_slice(row_out);

        let (avg_row, residu_row) = scratch.split_at_mut(avg_width);
        let mut avg = avg_row[0];
        let mut left = avg;
        let mut row_out_it = row_out.chunks_exact_mut(2);
        for (x, pair) in (&mut row_out_it).enumerate() {
            let residu = residu_row[x];
            let next_avg = avg_row.get(x + 1).copied().unwrap_or(avg);
            let diff = residu.wrapping_add(tendency_i16(left, avg, next_avg));
            let first = avg.wrapping_add(diff / 2);
            let second = first.wrapping_sub(diff);
            pair[0] = first;
            pair[1] = second;
            avg = next_avg;
            left = second;
        }

        if let [v] = row_out_it.into_remainder() {
            *v = avg_row[avg_width - 1];
        }
    }
}

pub fn inverse_v<S: Sample>(merged: &mut MutableSubgrid<'_, S>) {
    if let Some(merged) = S::try_as_mutable_subgrid_i16(merged) {
        inverse_v_i16(merged)
    } else if let Some(merged) = S::try_as_mutable_subgrid_i32(merged) {
        inverse_v_i32(merged)
    }
}

fn inverse_v_i32(merged: &mut MutableSubgrid<i32>) {
    inverse_v_i32_base(merged)
}

#[allow(unreachable_code)]
fn inverse_v_i16(merged: &mut MutableSubgrid<i16>) {
    inverse_v_i16_base(merged)
}

fn inverse_v_i32_base(merged: &mut MutableSubgrid<'_, i32>) {
    let width = merged.width();
    let height = merged.height();
    let mut scratch = vec![0i32; height];
    let avg_height = height.div_ceil(2);
    for x in 0..width {
        for (y, v) in scratch.iter_mut().enumerate() {
            *v = merged.get(x, y);
        }

        let (avg_col, residu_col) = scratch.split_at_mut(avg_height);
        let mut avg = avg_col[0];
        let mut top = avg;
        for (y, &residu) in residu_col.iter().enumerate() {
            let next_avg = avg_col.get(y + 1).copied().unwrap_or(avg);
            let diff = residu.wrapping_add(tendency_i32(top, avg, next_avg));
            let first = avg.wrapping_add(diff / 2);
            let second = first.wrapping_sub(diff);
            *merged.get_mut(x, 2 * y) = first;
            *merged.get_mut(x, 2 * y + 1) = second;
            avg = next_avg;
            top = second;
        }

        if height % 2 == 1 {
            *merged.get_mut(x, height - 1) = avg_col[avg_height - 1];
        }
    }
}

#[inline(never)]
fn inverse_v_i16_base(merged: &mut MutableSubgrid<'_, i16>) {
    let width = merged.width();
    let height = merged.height();
    let mut scratch = vec![0i16; height];
    let avg_height = height.div_ceil(2);
    for x in 0..width {
        for (y, v) in scratch.iter_mut().enumerate() {
            *v = merged.get(x, y);
        }

        let (avg_col, residu_col) = scratch.split_at_mut(avg_height);
        let mut avg = avg_col[0];
        let mut top = avg;
        for (y, &residu) in residu_col.iter().enumerate() {
            let next_avg = avg_col.get(y + 1).copied().unwrap_or(avg);
            let diff = residu.wrapping_add(tendency_i16(top, avg, next_avg));
            let first = avg.wrapping_add(diff / 2);
            let second = first.wrapping_sub(diff);
            *merged.get_mut(x, 2 * y) = first;
            *merged.get_mut(x, 2 * y + 1) = second;
            avg = next_avg;
            top = second;
        }

        if height % 2 == 1 {
            *merged.get_mut(x, height - 1) = avg_col[avg_height - 1];
        }
    }
}

fn tendency_i32(a: i32, b: i32, c: i32) -> i32 {
    let a = Wrapping(a);
    let b = Wrapping(b);
    let c = Wrapping(c);

    let n1 = Wrapping(1);
    let n2 = Wrapping(2);
    let n3 = Wrapping(3);
    let n4 = Wrapping(4);
    let n6 = Wrapping(6);
    let n12 = Wrapping(12);

    if a >= b && b >= c {
        let mut x = (n4 * a - n3 * c - b + n6) / n12;
        if x - (x & n1) > n2 * (a - b) {
            x = n2 * (a - b) + n1;
        }
        if x + (x & n1) > n2 * (b - c) {
            x = n2 * (b - c);
        }
        x.0
    } else if a <= b && b <= c {
        let mut x = (n4 * a - n3 * c - b - n6) / n12;
        if x + (x & n1) < n2 * (a - b) {
            x = n2 * (a - b) - n1;
        }
        if x - (x & n1) < n2 * (b - c) {
            x = n2 * (b - c);
        }
        x.0
    } else {
        0
    }
}

fn tendency_i16(a: i16, b: i16, c: i16) -> i16 {
    let a = Wrapping(a);
    let b = Wrapping(b);
    let c = Wrapping(c);

    let n1 = Wrapping(1);
    let n2 = Wrapping(2);
    let n3 = Wrapping(3);
    let n4 = Wrapping(4);
    let n6 = Wrapping(6);
    let n12 = Wrapping(12);

    if a >= b && b >= c {
        let mut x = (n4 * a - n3 * c - b + n6) / n12;
        if x - (x & n1) > n2 * (a - b) {
            x = n2 * (a - b) + n1;
        }
        if x + (x & n1) > n2 * (b - c) {
            x = n2 * (b - c);
        }
        x.0
    } else if a <= b && b <= c {
        let mut x = (n4 * a - n3 * c - b - n6) / n12;
        if x + (x & n1) < n2 * (a - b) {
            x = n2 * (a - b) - n1;
        }
        if x - (x & n1) < n2 * (b - c) {
            x = n2 * (b - c);
        }
        x.0
    } else {
        0
    }
}
