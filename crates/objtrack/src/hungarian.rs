//! Optimal assignment on a rectangular cost matrix (Kuhn–Munkres with potentials, O(n²m)).

/// Costs are clamped to this magnitude and NaN becomes this value, so the arithmetic stays finite.
const COST_LIMIT: f64 = 1.0e12;

/// Matrices larger than this on a side are refused (a tracker never has this many boxes).
pub const MAX_DIM: usize = 1 << 14;

/// Assign rows to columns at minimum total cost. `cost` is row-major, `rows * cols` long.
/// Returns, per row, the assigned column (`None` when there are more rows than columns). A matrix
/// of the wrong length, or one with a side over [`MAX_DIM`], gives an empty result.
pub fn assign(cost: &[f64], rows: usize, cols: usize) -> Vec<Option<usize>> {
    if rows > MAX_DIM || cols > MAX_DIM || rows.checked_mul(cols) != Some(cost.len()) {
        return Vec::new();
    }
    let mut out = vec![None; rows];
    if rows == 0 || cols == 0 {
        return out;
    }
    let at = |r: usize, c: usize| -> f64 {
        let v = cost.get(r * cols + c).copied().unwrap_or(COST_LIMIT);
        if v.is_nan() { COST_LIMIT } else { v.clamp(-COST_LIMIT, COST_LIMIT) }
    };
    if rows <= cols {
        for (col, row) in solve(rows, cols, at).into_iter().enumerate() {
            if let Some(o) = row.and_then(|row| out.get_mut(row)) {
                *o = Some(col);
            }
        }
    } else {
        // more rows than columns: solve the transpose (every column gets a row)
        for (row, col) in solve(cols, rows, |c, r| at(r, c)).into_iter().enumerate() {
            if let (Some(col), Some(o)) = (col, out.get_mut(row)) {
                *o = Some(col);
            }
        }
    }
    out
}

/// `n <= m`. Returns, per column, the row assigned to it.
fn solve(n: usize, m: usize, cost: impl Fn(usize, usize) -> f64) -> Vec<Option<usize>> {
    let mut u = vec![0.0f64; n + 1];
    let mut v = vec![0.0f64; m + 1];
    let mut p = vec![0usize; m + 1];
    let mut way = vec![0usize; m + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![f64::INFINITY; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let (mut delta, mut j1) = (f64::INFINITY, 0usize);
            for j in 1..=m {
                if !used[j] {
                    let cur = cost(i0 - 1, j - 1) - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            if j1 == 0 {
                break; // cannot happen with finite costs; never loop forever
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    (1..=m).map(|j| if p[j] != 0 { Some(p[j] - 1) } else { None }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(cost: &[f64], cols: usize, a: &[Option<usize>]) -> f64 {
        a.iter().enumerate().filter_map(|(r, c)| c.map(|c| cost[r * cols + c])).sum()
    }

    /// Cheapest assignment by trying every injective mapping of the smaller side into the larger.
    fn brute(cost: &[f64], rows: usize, cols: usize) -> f64 {
        fn go(cost: &[f64], cols: usize, rows: usize, r: usize, used: &mut Vec<bool>, acc: f64, best: &mut f64) {
            if r == rows {
                *best = best.min(acc);
                return;
            }
            for c in 0..cols {
                if !used[c] {
                    used[c] = true;
                    go(cost, cols, rows, r + 1, used, acc + cost[r * cols + c], best);
                    used[c] = false;
                }
            }
        }
        let mut best = f64::INFINITY;
        if rows <= cols {
            go(cost, cols, rows, 0, &mut vec![false; cols], 0.0, &mut best);
        } else {
            // fewer columns than rows: assign every column a distinct row, on the transposed matrix
            let t: Vec<f64> = (0..cols).flat_map(|c| (0..rows).map(move |r| cost[r * cols + c])).collect();
            go(&t, rows, cols, 0, &mut vec![false; rows], 0.0, &mut best);
        }
        best
    }

    #[test]
    fn known_square_case() {
        // classic 3x3: optimum 5 (0->1, 1->0, 2->2)
        let c = [4.0, 1.0, 3.0, 2.0, 0.0, 5.0, 3.0, 2.0, 2.0];
        let a = assign(&c, 3, 3);
        assert_eq!(total(&c, 3, &a), 5.0);
        assert!(a.iter().all(Option::is_some));
    }

    #[test]
    fn matches_brute_force_on_random_matrices() {
        let mut seed = 0x1234_5678_9abc_def0_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..400 {
            let (rows, cols) = ((next() % 5 + 1) as usize, (next() % 5 + 1) as usize);
            let c: Vec<f64> = (0..rows * cols).map(|_| (next() % 100) as f64 / 10.0 - 3.0).collect();
            let a = assign(&c, rows, cols);
            assert_eq!(a.iter().filter(|x| x.is_some()).count(), rows.min(cols));
            let mut seen = std::collections::BTreeSet::new();
            assert!(a.iter().flatten().all(|col| seen.insert(*col)), "columns are used once");
            let got = total(&c, cols, &a);
            let want = brute(&c, rows, cols);
            assert!((got - want).abs() < 1e-9, "{rows}x{cols}: {got} vs {want}");
        }
    }

    #[test]
    fn degenerate_and_hostile_inputs() {
        assert!(assign(&[], 0, 0).is_empty());
        assert_eq!(assign(&[], 2, 0), vec![None, None]);
        assert!(assign(&[1.0, 2.0, 3.0], 2, 2).is_empty(), "wrong length");
        assert!(assign(&[1.0], usize::MAX, 2).is_empty(), "overflow guarded");
        assert!(assign(&[], usize::MAX, 0).is_empty(), "huge side refused before allocating");
        assert!(assign(&[], MAX_DIM + 1, 0).is_empty());
        let c = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0];
        let a = assign(&c, 2, 2);
        assert_eq!(a.iter().filter(|x| x.is_some()).count(), 2);
    }
}
