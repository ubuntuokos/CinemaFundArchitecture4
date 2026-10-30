// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-grid 0.6.2.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.
//
// CFA4: a subgrid is a list of disjoint row slices instead of a raw pointer with a stride, so
// splitting into groups hands out real `&mut [V]` borrows. The in-place horizontal merge of the
// squeeze transform is replaced by hidden columns / rows (`shrink_width`, `extend_width_from`).

use std::ops::RangeBounds;

use crate::grid::SharedSubgrid;

/// A mutable subgrid of the underlying buffer.
///
/// Invariant: `rows.len() >= height` and every row is at least `width` long. Rows and columns
/// past `width` / `height` are hidden storage owned by this subgrid (left behind by
/// [`shrink_width`](Self::shrink_width) / [`shrink_height`](Self::shrink_height)).
#[derive(Debug)]
pub struct MutableSubgrid<'g, V = f32> {
    rows: Vec<&'g mut [V]>,
    width: usize,
    height: usize,
}

impl<'g, V> From<&'g mut crate::grid::AlignedGrid<V>> for MutableSubgrid<'g, V> {
    fn from(grid: &'g mut crate::grid::AlignedGrid<V>) -> Self {
        let width = grid.width();
        let height = grid.height();
        Self::from_buf(grid.buf_mut(), width, height, width)
    }
}

pub(crate) fn resolve_range(range: impl RangeBounds<usize>, len: usize) -> (usize, usize) {
    use std::ops::Bound;

    let start = match range.start_bound() {
        Bound::Included(&v) => v,
        Bound::Excluded(&v) => v.saturating_add(1),
        Bound::Unbounded => 0,
    };
    let end = match range.end_bound() {
        Bound::Included(&v) => v.saturating_add(1),
        Bound::Excluded(&v) => v,
        Bound::Unbounded => len,
    };
    let end = end.min(len);
    (start.min(end), end)
}

impl<'g, V> MutableSubgrid<'g, V> {
    /// Creates a empty, zero-sized subgrid.
    pub fn empty() -> Self {
        Self { rows: Vec::new(), width: 0, height: 0 }
    }

    /// Create a `MutableSubgrid` from buffer slice, width, height and stride.
    ///
    /// A buffer too small for the area yields a grid clipped to the rows and columns it holds.
    pub fn from_buf(buf: &'g mut [V], width: usize, height: usize, stride: usize) -> Self {
        if width == 0 || height == 0 || stride == 0 {
            return Self::empty();
        }
        let width = width.min(stride);
        let mut rows = Vec::with_capacity(height.min(buf.len() / stride + 1));
        let mut rest = buf;
        while rows.len() < height {
            let take = rest.len().min(stride);
            let (row, tail) = std::mem::take(&mut rest).split_at_mut(take);
            rest = tail;
            if row.len() < width {
                break;
            }
            rows.push(&mut row[..width]);
        }
        let height = rows.len();
        Self { rows, width, height }
    }

    /// Returns the width of the subgrid.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the subgrid.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns a reference to the sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of bounds.
    #[inline]
    pub fn get_ref(&self, x: usize, y: usize) -> &V {
        &self.get_row(y)[x]
    }

    /// Returns a reference to the sample at the given location, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_ref(&self, x: usize, y: usize) -> Option<&V> {
        self.try_get_row(y)?.get(x)
    }

    /// Returns a slice of a row of samples.
    ///
    /// # Panics
    /// Panics if the row index is out of bounds.
    #[inline]
    pub fn get_row(&self, row: usize) -> &[V] {
        &self.rows[..self.height][row][..self.width]
    }

    /// Returns a slice of a row of samples, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_row(&self, row: usize) -> Option<&[V]> {
        if row >= self.height {
            return None;
        }
        self.rows.get(row)?.get(..self.width)
    }

    /// Returns a mutable reference to the sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of bounds.
    #[inline]
    pub fn get_mut(&mut self, x: usize, y: usize) -> &mut V {
        &mut self.get_row_mut(y)[x]
    }

    /// Returns a mutable reference to the sample at the given location, or `None` if it is out of
    /// bounds.
    #[inline]
    pub fn try_get_mut(&mut self, x: usize, y: usize) -> Option<&mut V> {
        self.try_get_row_mut(y)?.get_mut(x)
    }

    /// Returns a mutable slice of a row of samples.
    ///
    /// # Panics
    /// Panics if the row index is out of bounds.
    #[inline]
    pub fn get_row_mut(&mut self, row: usize) -> &mut [V] {
        let width = self.width;
        &mut self.rows[..self.height][row][..width]
    }

    /// Returns a mutable slice of a row of samples, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_row_mut(&mut self, row: usize) -> Option<&mut [V]> {
        if row >= self.height {
            return None;
        }
        let width = self.width;
        self.rows.get_mut(row)?.get_mut(..width)
    }

    /// Swaps two samples at the given locations. Out-of-bounds locations are ignored.
    #[inline]
    pub fn swap(&mut self, (ax, ay): (usize, usize), (bx, by): (usize, usize)) {
        if ax >= self.width || bx >= self.width || ay >= self.height || by >= self.height {
            return;
        }
        if ay == by {
            if let Some(row) = self.rows.get_mut(ay) {
                row.swap(ax, bx);
            }
            return;
        }
        let (lo, hi, lo_x, hi_x) = if ay < by { (ay, by, ax, bx) } else { (by, ay, bx, ax) };
        let (top, bottom) = self.rows.split_at_mut(hi);
        if let (Some(a), Some(b)) = (top.get_mut(lo).and_then(|r| r.get_mut(lo_x)), bottom.first_mut().and_then(|r| r.get_mut(hi_x))) {
            std::mem::swap(a, b);
        }
    }

    /// Reborrows the mutable subgrid, and returns a mutable subgrid with shorter lifetime.
    ///
    /// Hidden rows and columns stay with the reborrow.
    pub fn borrow_mut(&mut self) -> MutableSubgrid<'_, V> {
        MutableSubgrid { rows: self.rows.iter_mut().map(|row| &mut **row).collect(), width: self.width, height: self.height }
    }

    /// Reborrows the mutable subgrid, and returns a shared subgrid.
    pub fn as_shared(&self) -> SharedSubgrid<'_, V> {
        SharedSubgrid::from_rows(&self.rows, self.width, self.height)
    }

    /// Creates a subgrid of this subgrid. The ranges are clipped to the grid.
    pub fn subgrid(self, range_x: impl RangeBounds<usize>, range_y: impl RangeBounds<usize>) -> MutableSubgrid<'g, V> {
        let (left, right) = resolve_range(range_x, self.width);
        let (top, bottom) = resolve_range(range_y, self.height);
        let rows: Vec<_> = self.rows.into_iter().skip(top).take(bottom - top).filter_map(|row| row.get_mut(left..right)).collect();
        let height = rows.len();
        MutableSubgrid { rows, width: right - left, height }
    }

    /// Creates a subgrid of this subgrid, borrowing it. The ranges are clipped to the grid.
    pub fn subgrid_mut(&mut self, range_x: impl RangeBounds<usize>, range_y: impl RangeBounds<usize>) -> MutableSubgrid<'_, V> {
        let (left, right) = resolve_range(range_x, self.width);
        let (top, bottom) = resolve_range(range_y, self.height);
        let rows: Vec<&mut [V]> = self.rows.get_mut(top..bottom).unwrap_or_default().iter_mut().filter_map(|row| row.get_mut(left..right)).collect();
        let height = rows.len();
        MutableSubgrid { rows, width: right - left, height }
    }

    /// Split the grid horizontally at an index (clipped to the width).
    pub fn split_horizontal(&mut self, x: usize) -> (MutableSubgrid<'_, V>, MutableSubgrid<'_, V>) {
        let width = self.width;
        let x = x.min(width);
        let mut left = Vec::with_capacity(self.height);
        let mut right = Vec::with_capacity(self.height);
        for row in self.rows.iter_mut().take(self.height) {
            let row: &mut [V] = row;
            if let Some(row) = row.get_mut(..width) {
                let (l, r) = row.split_at_mut(x);
                left.push(l);
                right.push(r);
            }
        }
        let height = left.len();
        (MutableSubgrid { rows: left, width: x, height }, MutableSubgrid { rows: right, width: width - x, height })
    }

    /// Split the grid vertically at an index (clipped to the height).
    pub fn split_vertical(&mut self, y: usize) -> (MutableSubgrid<'_, V>, MutableSubgrid<'_, V>) {
        let width = self.width;
        let height = self.height;
        let y = y.min(height);
        let mut rows = self.rows.iter_mut().take(height).filter_map(|row| row.get_mut(..width));
        let top: Vec<_> = rows.by_ref().take(y).collect();
        let bottom: Vec<_> = rows.collect();
        (MutableSubgrid { height: top.len(), rows: top, width }, MutableSubgrid { height: bottom.len(), rows: bottom, width })
    }

    /// Split the grid vertically at an index (clipped to the height) in-place, returning the
    /// bottom part. Hidden rows of `self` are dropped from both parts.
    pub fn split_vertical_in_place(&mut self, y: usize) -> MutableSubgrid<'g, V> {
        let y = y.min(self.height);
        self.rows.truncate(self.height);
        let bottom = self.rows.split_off(y);
        let bottom_height = self.height - y;
        self.height = y;
        MutableSubgrid { rows: bottom, width: self.width, height: bottom_height }
    }

    /// Hides the columns from `width` on; they keep their storage and come back with
    /// [`extend_width_from`](Self::extend_width_from). `width` is clipped to the current width.
    pub fn shrink_width(&mut self, width: usize) {
        self.width = width.min(self.width);
    }

    /// Hides the rows from `height` on; they keep their storage and come back with
    /// [`extend_height_from`](Self::extend_height_from). `height` is clipped to the current height.
    pub fn shrink_height(&mut self, height: usize) {
        self.height = height.min(self.height);
    }

    /// Returns a list of subgrids split into groups of given size, in row-first order.
    ///
    /// Groups at the edge of the grid may have smaller sizes. A zero group size yields no groups.
    pub fn into_groups(self, group_width: usize, group_height: usize) -> Vec<MutableSubgrid<'g, V>> {
        if group_width == 0 || group_height == 0 {
            return Vec::new();
        }

        let num_cols = self.width.div_ceil(group_width);
        let num_rows = self.height.div_ceil(group_height);
        self.into_groups_with_fixed_count(group_width, group_height, num_cols, num_rows)
    }

    /// Returns a list of subgrids split into groups of given size, with `num_rows` rows and
    /// `num_cols` columns, in row-first order.
    ///
    /// Unlike `into_groups`, this method will always return vector of length
    /// `num_cols * num_rows`. Remaining rows or columns will be truncated, and groups out of
    /// bounds will be zero-sized.
    pub fn into_groups_with_fixed_count(self, group_width: usize, group_height: usize, num_cols: usize, num_rows: usize) -> Vec<MutableSubgrid<'g, V>> {
        let MutableSubgrid { rows, width, height } = self;

        let group_count = num_cols.saturating_mul(num_rows);
        let mut groups: Vec<MutableSubgrid<'g, V>> = Vec::with_capacity(group_count);
        for gy in 0..num_rows {
            let y = gy.saturating_mul(group_height).min(height);
            let gh = (height - y).min(group_height);
            for gx in 0..num_cols {
                let x = gx.saturating_mul(group_width).min(width);
                let gw = (width - x).min(group_width);
                let (gw, gh) = if gw == 0 || gh == 0 { (0, 0) } else { (gw, gh) };
                groups.push(MutableSubgrid { rows: Vec::with_capacity(gh), width: gw, height: gh });
            }
        }

        for (y, row) in rows.into_iter().take(height).enumerate() {
            let Some(gy) = y.checked_div(group_height) else {
                break;
            };
            if gy >= num_rows {
                break;
            }
            let Some(mut rest) = row.get_mut(..width) else {
                continue;
            };
            for gx in 0..num_cols {
                let Some(group) = groups.get_mut(gy.saturating_mul(num_cols).saturating_add(gx)) else {
                    break;
                };
                if group.width == 0 {
                    continue;
                }
                let take = group.width.min(rest.len());
                let (chunk, tail) = std::mem::take(&mut rest).split_at_mut(take);
                rest = tail;
                group.rows.push(chunk);
            }
        }

        for group in &mut groups {
            if group.rows.len() < group.height {
                group.height = group.rows.len();
            }
        }
        groups
    }
}

impl<V: Copy> MutableSubgrid<'_, V> {
    /// Returns a copy of sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of range.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> V {
        *self.get_ref(x, y)
    }

    /// Brings back hidden columns after the visible ones and fills them from `right` (the
    /// inverse of [`shrink_width`](Self::shrink_width)). Returns `false`, leaving the grid as it
    /// is, if the hidden storage or `right` does not fit.
    pub fn extend_width_from(&mut self, right: &SharedSubgrid<'_, V>) -> bool {
        let width = self.width;
        let new_width = width.saturating_add(right.width());
        if right.height() != self.height || self.rows.iter().take(self.height).any(|row| row.len() < new_width) {
            return false;
        }
        for (y, row) in self.rows.iter_mut().take(self.height).enumerate() {
            if let (Some(dst), Some(src)) = (row.get_mut(width..new_width), right.try_get_row(y)) {
                dst.copy_from_slice(src);
            }
        }
        self.width = new_width;
        true
    }

    /// Brings back hidden rows after the visible ones and fills them from `bottom` (the inverse
    /// of [`shrink_height`](Self::shrink_height)). Returns `false`, leaving the grid as it is, if
    /// the hidden storage or `bottom` does not fit.
    pub fn extend_height_from(&mut self, bottom: &SharedSubgrid<'_, V>) -> bool {
        let width = self.width;
        let height = self.height;
        let new_height = height.saturating_add(bottom.height());
        if bottom.width() != width || self.rows.len() < new_height {
            return false;
        }
        for (y, row) in self.rows.iter_mut().enumerate().skip(height).take(bottom.height()) {
            if let (Some(dst), Some(src)) = (row.get_mut(..width), bottom.try_get_row(y - height)) {
                dst.copy_from_slice(src);
            }
        }
        self.height = new_height;
        true
    }
}
