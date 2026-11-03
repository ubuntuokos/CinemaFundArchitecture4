// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-grid 0.6.2.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

//! This crate provides [`AlignedGrid`], used in various places involving images.

mod alloc_tracker;
mod mutable_subgrid;
mod shared_subgrid;
pub use alloc_tracker::*;
pub use mutable_subgrid::*;
pub use shared_subgrid::*;

#[cfg(test)]
mod tests;

/// Largest grid CFA4 allocates, in samples (1 GiB of `f32`).
pub const MAX_GRID_SAMPLES: usize = 1 << 28;

/// The error type for failed grid allocation.
#[derive(Debug)]
pub struct OutOfMemory {
    bytes: usize,
}

impl OutOfMemory {
    fn new(bytes: usize) -> Self {
        Self { bytes }
    }

    /// Returns the size of failed allocation in bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl std::error::Error for OutOfMemory {}

impl std::fmt::Display for OutOfMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to allocate {} byte(s)", self.bytes)
    }
}

/// A continuous buffer in the "raster order".
///
/// The beginning of the buffer is aligned so that it can be used in SIMD instructions.
pub struct AlignedGrid<S> {
    width: usize,
    height: usize,
    offset: usize,
    buf: Vec<S>,
    handle: Option<AllocHandle>,
}

impl<S> std::fmt::Debug for AlignedGrid<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlignedGrid").field("width", &self.width).field("height", &self.height).field("offset", &self.offset).finish_non_exhaustive()
    }
}

impl<S> AlignedGrid<S> {
    /// Creates a zero-sized "empty" grid.
    #[inline]
    pub fn empty() -> Self {
        Self { width: 0, height: 0, offset: 0, buf: Vec::new(), handle: None }
    }
}

impl<S: Default + Clone> AlignedGrid<S> {
    /// Create a new buffer, recording the allocation if a tracker is given.
    #[inline]
    pub fn with_alloc_tracker(width: usize, height: usize, tracker: Option<&AllocTracker>) -> Result<Self, OutOfMemory> {
        let len = Self::checked_len(width, height)?;
        let handle = tracker.map(|tracker| tracker.alloc::<S>(len)).transpose()?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(len).map_err(|_| OutOfMemory::new(len.saturating_mul(std::mem::size_of::<S>())))?;
        buf.resize(len, S::default());
        let offset = 0;

        Ok(Self { width, height, offset, buf, handle })
    }

    #[inline]
    fn empty_aligned(width: usize, height: usize, tracker: Option<&AllocTracker>) -> Result<Self, OutOfMemory> {
        let len = Self::checked_len(width, height)?;
        let handle = tracker.map(|tracker| tracker.alloc::<S>(len)).transpose()?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(len).map_err(|_| OutOfMemory::new(len.saturating_mul(std::mem::size_of::<S>())))?;
        let offset = 0;

        Ok(Self { width, height, offset, buf, handle })
    }

    /// Clones the buffer without recording an allocation.
    pub fn clone_untracked(&self) -> Self {
        Self { width: self.width, height: self.height, offset: 0, buf: self.buf().to_vec(), handle: None }
    }

    /// Number of samples of a `width` x `height` grid, refused above [`MAX_GRID_SAMPLES`].
    fn checked_len(width: usize, height: usize) -> Result<usize, OutOfMemory> {
        width.checked_mul(height).filter(|&len| len <= MAX_GRID_SAMPLES).ok_or(OutOfMemory::new(usize::MAX))
    }

    /// Tries to clone the buffer, and records the allocation in the same tracker as the original
    /// buffer.
    pub fn try_clone(&self) -> Result<Self, OutOfMemory> {
        let mut out = Self::empty_aligned(self.width, self.height, self.tracker().as_ref())?;
        out.buf.extend_from_slice(self.buf());
        Ok(out)
    }
}

impl<S> AlignedGrid<S> {
    /// Returns the width of the grid.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the grid.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns allocation tracker associated with the grid.
    #[inline]
    pub fn tracker(&self) -> Option<AllocTracker> {
        self.handle.as_ref().map(|handle| handle.tracker())
    }

    /// Returns a reference to the sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of bounds.
    #[inline]
    pub fn get_ref(&self, x: usize, y: usize) -> &S {
        &self.get_row(y)[x]
    }

    /// Returns a reference to the sample at the given location, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_ref(&self, x: usize, y: usize) -> Option<&S> {
        if x >= self.width || y >= self.height {
            return None;
        }

        self.buf.get(y * self.width + x + self.offset)
    }

    /// Returns a mutable reference to the sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of bounds.
    #[inline]
    pub fn get_mut(&mut self, x: usize, y: usize) -> &mut S {
        &mut self.get_row_mut(y)[x]
    }

    /// Returns a mutable reference to the sample at the given location, or `None` if it is out of
    /// bounds.
    #[inline]
    pub fn try_get_mut(&mut self, x: usize, y: usize) -> Option<&mut S> {
        if x >= self.width || y >= self.height {
            return None;
        }

        self.buf.get_mut(y * self.width + x + self.offset)
    }

    /// Returns a slice of a row of samples.
    ///
    /// # Panics
    /// Panics if the row index is out of bounds.
    #[inline]
    pub fn get_row(&self, row: usize) -> &[S] {
        self.try_get_row(row).unwrap_or(&[])
    }

    /// Returns a slice of a row of samples, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_row(&self, y: usize) -> Option<&[S]> {
        if y >= self.height {
            return None;
        }

        let start = y * self.width + self.offset;
        self.buf.get(start..start + self.width)
    }

    /// Returns a mutable slice of a row of samples.
    ///
    /// # Panics
    /// Panics if the row index is out of bounds.
    #[inline]
    pub fn get_row_mut(&mut self, row: usize) -> &mut [S] {
        self.try_get_row_mut(row).unwrap_or(&mut [])
    }

    /// Returns a mutable slice of a row of samples, or `None` if it is out of bounds.
    #[inline]
    pub fn try_get_row_mut(&mut self, y: usize) -> Option<&mut [S]> {
        if y >= self.height {
            return None;
        }

        let start = y * self.width + self.offset;
        self.buf.get_mut(start..start + self.width)
    }

    /// Returns the immutable slice to the underlying buffer.
    #[inline]
    pub fn buf(&self) -> &[S] {
        self.buf.get(self.offset..).unwrap_or(&[])
    }

    /// Returns the mutable slice to the underlying buffer.
    #[inline]
    pub fn buf_mut(&mut self) -> &mut [S] {
        self.buf.get_mut(self.offset..).unwrap_or(&mut [])
    }

    /// Borrows the grid into a `SharedSubgrid`.
    #[inline]
    pub fn as_subgrid(&self) -> SharedSubgrid<'_, S> {
        SharedSubgrid::from(self)
    }

    /// Borrows the grid into a `MutableSubgrid`.
    #[inline]
    pub fn as_subgrid_mut(&mut self) -> MutableSubgrid<'_, S> {
        MutableSubgrid::from(self)
    }
}

impl<V: Copy> AlignedGrid<V> {
    /// Returns a copy of sample at the given location.
    ///
    /// # Panics
    /// Panics if the coordinate is out of range.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> V {
        *self.get_ref(x, y)
    }
}
