// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-modular 0.11.3.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use std::num::Wrapping;

use crate::grid::MutableSubgrid;
use crate::threadpool::JxlThreadPool;

use crate::modular::Sample;

type FnRow<S> = fn(&mut [&mut [S]; 3]);

pub fn inverse_rct<S: Sample, const TYPE: u32>(permutation: u32, mut grids: [&mut MutableSubgrid<S>; 3], pool: &JxlThreadPool) {
    let grid16 = grids.each_mut().map(|g| S::try_as_mutable_subgrid_i16(g));
    if let [Some(a), Some(b), Some(c)] = grid16 {
        let grids = [a, b, c];

        run_rows(permutation, grids, inverse_row_i16_base::<TYPE>, pool);
        return;
    }

    let grid32 = grids.each_mut().map(|g| S::try_as_mutable_subgrid_i32(g));
    if let [Some(a), Some(b), Some(c)] = grid32 {
        let grids = [a, b, c];

        run_rows(permutation, grids, inverse_row_i32_base::<TYPE>, pool);
    }
}

#[inline(never)]
fn run_rows<S: Sample>(permutation: u32, grids: [&mut MutableSubgrid<S>; 3], f: FnRow<S>, pool: &JxlThreadPool) {
    struct RctJob<'g, S: Sample> {
        grids: [MutableSubgrid<'g, S>; 3],
    }

    let width = grids[0].width();
    let height = grids[0].height();
    if grids.iter().any(|g| g.width() != width || g.height() != height) {
        crate::tracing::error!("RCT channels differ in size");
        return;
    }
    if width == 0 || height == 0 {
        return;
    }

    let [mut a, mut b, mut c] = grids.map(|g| g.borrow_mut().into_groups(width, 16));
    let mut jobs = Vec::new();
    while let (Some(a), Some(b), Some(c)) = (a.pop(), b.pop(), c.pop()) {
        jobs.push(RctJob { grids: [a, b, c] });
    }

    pool.for_each_vec(jobs, |job| {
        let mut grids = job.grids;
        let height = grids[0].height();
        for y in 0..height {
            let mut rows = grids.each_mut().map(|g| g.get_row_mut(y));
            f(&mut rows);
            inverse_permute(permutation, rows);
        }
    });
}

#[inline]
fn inverse_row_i16_base<const TYPE: u32>(rows: &mut [&mut [i16]; 3]) {
    let [a, b, c] = rows;

    for ((ra, rb), rc) in a.iter_mut().zip(&mut **b).zip(&mut **c) {
        let a = Wrapping(*ra);
        let b = Wrapping(*rb);
        let c = Wrapping(*rc);
        let d;
        let e;
        let f;
        if TYPE == 6 {
            let tmp = a - (c >> 1);
            e = c + tmp;
            f = tmp - (b >> 1);
            d = f + b;
        } else {
            d = a;
            f = if TYPE & 1 != 0 { c + a } else { c };
            e = if (TYPE >> 1) == 1 {
                b + a
            } else if (TYPE >> 1) == 2 {
                b + ((a + f) >> 1)
            } else {
                b
            };
        }
        *ra = d.0;
        *rb = e.0;
        *rc = f.0;
    }
}

#[inline]
fn inverse_row_i32_base<const TYPE: u32>(rows: &mut [&mut [i32]; 3]) {
    let [a, b, c] = rows;

    for ((ra, rb), rc) in a.iter_mut().zip(&mut **b).zip(&mut **c) {
        let a = Wrapping(*ra);
        let b = Wrapping(*rb);
        let c = Wrapping(*rc);
        let d;
        let e;
        let f;
        if TYPE == 6 {
            let tmp = a - (c >> 1);
            e = c + tmp;
            f = tmp - (b >> 1);
            d = f + b;
        } else {
            d = a;
            f = if TYPE & 1 != 0 { c + a } else { c };
            e = if (TYPE >> 1) == 1 {
                b + a
            } else if (TYPE >> 1) == 2 {
                b + ((a + f) >> 1)
            } else {
                b
            };
        }
        *ra = d.0;
        *rb = e.0;
        *rc = f.0;
    }
}

#[inline(always)]
fn inverse_permute<S: Sample>(permutation: u32, rows: [&mut [S]; 3]) {
    let [a, b, c] = rows;
    match permutation {
        1 => {
            a.swap_with_slice(b);
            a.swap_with_slice(c);
        }
        2 => {
            a.swap_with_slice(b);
            b.swap_with_slice(c);
        }
        3 => {
            b.swap_with_slice(c);
        }
        4 => {
            a.swap_with_slice(b);
        }
        5 => {
            a.swap_with_slice(c);
        }
        _ => {}
    }
}
