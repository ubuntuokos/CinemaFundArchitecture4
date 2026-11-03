// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-grid 0.6.2.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use std::ops::RangeBounds;

use crate::grid::mutable_subgrid::resolve_range;

enum Source<'g, V> {
    Flat { buf: &'g [V], stride: usize },
    Rows(&'g [&'g mut [V]]),
}

impl<V> Clone for Source<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for Source<'_, V> {}

/// An immutable shared subgrid of the underlying buffer.
pub struct SharedSubgrid<'g, V = f32> {
    src: Source<'g, V>,
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

impl<V> Clone for SharedSubgrid<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for SharedSubgrid<'_, V> {}

impl<V> std::fmt::Debug for SharedSubgrid<'_, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedSubgrid").field("width", &self.width).field("height", &self.height).finish_non_exhaustive()
    }
}

impl<'g, V> From<&'g crate::grid::AlignedGrid<V>> for SharedSubgrid<'g, V> {
    fn from(value: &'g crate::grid::AlignedGrid<V>) -> Self {
        SharedSubgrid::from_buf(value.buf(), value.width(), value.height(), value.width())
    }
}

impl<'g, V> SharedSubgrid<'g, V> {
    /// Create a `SharedSubgrid` from buffer slice, width, height and stride.
    ///
    /// A buffer too small for the area yields a grid clipped to the rows it holds.
    pub fn from_buf(buf: &'g [V], width: usize, height: usize, stride: usize) -> Self {
        let width = width.min(stride);
        let full_rows = buf.len().checked_div(stride).unwrap_or(0);
        let partial = buf.len().checked_rem(stride).unwrap_or(0) >= width && width > 0;
        let height = height.min(full_rows + usize::from(partial));
        let (width, height) = if width == 0 || height == 0 { (0, 0) } else { (width, height) };
        Self { src: Source::Flat { buf, stride }, left: 0, top: 0, width, height }
    }

    pub(crate) fn from_rows(rows: &'g [&'g mut [V]], width: usize, height: usize) -> Self {
        let height = height.min(rows.len());
        Self { src: Source::Rows(rows), left: 0, top: 0, width, height }
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
    pub fn get_ref(&self, x: usize, y: usize) -> &'g V {
        &self.get_row(y)[x]
    }

    /// Returns a reference to the sample at the given location, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_ref(&self, x: usize, y: usize) -> Option<&'g V> {
        self.try_get_row(y)?.get(x)
    }

    /// Returns a slice of a row of samples, or an empty slice if the row is out of bounds.
    #[inline]
    pub fn get_row(&self, row: usize) -> &'g [V] {
        self.try_get_row(row).unwrap_or(&[])
    }

    /// Returns a slice of a row of samples, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_row(&self, row: usize) -> Option<&'g [V]> {
        if row >= self.height {
            return None;
        }
        let y = self.top.checked_add(row)?;
        let end = self.left.checked_add(self.width)?;
        match self.src {
            Source::Flat { buf, stride } => {
                let start = y.checked_mul(stride)?.checked_add(self.left)?;
                buf.get(start..start.checked_add(self.width)?)
            }
            Source::Rows(rows) => {
                let row: &'g [V] = rows.get(y)?;
                row.get(self.left..end)
            }
        }
    }

    /// Split the grid horizontally at an index (clipped to the width).
    pub fn split_horizontal(&self, x: usize) -> (SharedSubgrid<'g, V>, SharedSubgrid<'g, V>) {
        let x = x.min(self.width);
        (self.subgrid(..x, ..), self.subgrid(x.., ..))
    }

    /// Split the grid vertically at an index (clipped to the height).
    pub fn split_vertical(&self, y: usize) -> (SharedSubgrid<'g, V>, SharedSubgrid<'g, V>) {
        let y = y.min(self.height);
        (self.subgrid(.., ..y), self.subgrid(.., y..))
    }

    /// Creates a subgrid of this subgrid. The ranges are clipped to the grid.
    pub fn subgrid(&self, range_x: impl RangeBounds<usize>, range_y: impl RangeBounds<usize>) -> SharedSubgrid<'g, V> {
        let (left, right) = resolve_range(range_x, self.width);
        let (top, bottom) = resolve_range(range_y, self.height);
        SharedSubgrid { src: self.src, left: self.left.saturating_add(left), top: self.top.saturating_add(top), width: right - left, height: bottom - top }
    }
}

impl<V: Copy> SharedSubgrid<'_, V> {
    /// Returns a copy of sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of range.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> V {
        *self.get_ref(x, y)
    }
}
