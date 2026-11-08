//! Safe wrapper for `AVBufferPool` — FFmpeg's reference-counted buffer pool.
//!
//! A pool hands out buffers of a fixed size and automatically recycles the
//! ones whose last reference has been dropped, so a steady stream of
//! equally-sized allocations (e.g. destination frames of a scaler) stops
//! hitting the allocator after a couple of frames.
//!
//! Buffers are allocated zero-filled (`av_buffer_allocz`), which mirrors the
//! behaviour of `av_frame_get_buffer` and keeps padding bytes deterministic
//! for consumers that read past the visible data (SIMD code paths).
//! Note that FFmpeg does *not* re-zero a buffer when it is recycled from the
//! pool — callers that require zeroed content on every acquisition must clear
//! the buffer themselves.
//!
//! # Example
//!
//! ```
//! use rsmpeg::avutil::AVBufferPool;
//!
//! let mut pool = AVBufferPool::new(1024).unwrap();
//!
//! let mut a = pool.get().unwrap();
//! assert_eq!(a.size, 1024);
//! assert_eq!(a.get_ref_count(), 1);
//!
//! // While `a` is alive the pool has to grow:
//! let b = pool.get().unwrap();
//! assert_ne!(a.data, b.data);
//!
//! // Dropping `a` returns its buffer to the pool; the next acquisition
//! // recycles it (same data pointer, no new allocation):
//! let a_addr = a.data as usize;
//! drop(a);
//! let c = pool.get().unwrap();
//! assert_eq!(c.data as usize, a_addr);
//!
//! // Give `b` and `c` back before the pool is dropped, or drop them after
//! // the pool — both are safe.
//! drop(b);
//! drop(c);
//! ```

use crate::{
    avutil::AVBufferRef,
    error::{Result, RsmpegError},
    ffi,
    shared::RsmpegPointerUpgrade,
};

wrap!(AVBufferPool: ffi::AVBufferPool, buffer_size: usize = 0);

/// The allocation callback handed to `av_buffer_pool_init`: allocates
/// zero-filled buffers via `av_buffer_allocz`.
///
/// # Safety
///
/// Called by FFmpeg with the `size` the pool was created with; only fails on
/// OOM (returns NULL, which the pool reports to `av_buffer_pool_get`).
unsafe extern "C" fn alloc_zeroed(size: usize) -> *mut ffi::AVBufferRef {
    unsafe { ffi::av_buffer_allocz(size) }
}

impl AVBufferPool {
    /// Create a pool whose buffers are all `buffer_size` bytes and are
    /// zero-filled on (re)allocation.
    ///
    /// Returns [`RsmpegError::AVError`](`ffi::AVERROR(ffi::EINVAL)`) when
    /// `buffer_size` is zero, and
    /// [`RsmpegError::AVError`](`ffi::AVERROR(ffi::ENOMEM)`) when the
    /// underlying allocation fails.
    pub fn new(buffer_size: usize) -> Result<Self> {
        if buffer_size == 0 {
            return Err(RsmpegError::AVError(ffi::AVERROR(ffi::EINVAL)));
        }

        // Safety: plain FFI call; NULL is triaged below. The allocation
        // callback is a bare function with no state, so no cleanup contract
        // beyond `av_buffer_pool_uninit` (see [`Drop`]) is created.
        let pool = unsafe { ffi::av_buffer_pool_init(buffer_size, Some(alloc_zeroed)) }
            .upgrade_or(ffi::AVERROR(ffi::ENOMEM))?;
        // Safety: `pool` comes from a successful FFmpeg allocation.
        let mut pool = unsafe { Self::from_raw(pool) };
        pool.buffer_size = buffer_size;
        Ok(pool)
    }

    /// Get a buffer from the pool, recycling one of the previously returned
    /// buffers when available and allocating a new zero-filled buffer
    /// otherwise.
    ///
    /// The returned reference starts with a refcount of 1; when its last
    /// reference is dropped the buffer goes back to the pool (or is freed if
    /// the pool has already been dropped, see [`AVBufferPool::drop`]).
    pub fn get(&mut self) -> Result<AVBufferRef> {
        // Safety: `self` owns a valid pool for as long as it is not dropped.
        let raw = unsafe { ffi::av_buffer_pool_get(self.as_mut_ptr()) }
            .upgrade_or(ffi::AVERROR(ffi::ENOMEM))?;
        // Safety: `raw` comes from a successful FFmpeg call.
        Ok(unsafe { AVBufferRef::from_raw(raw) })
    }

    /// The size in bytes of every buffer handed out by this pool.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }
}

impl Drop for AVBufferPool {
    fn drop(&mut self) {
        // Safety: `self` owns a valid pool pointer that has not been
        // uninitialized yet. `av_buffer_pool_uninit` marks the pool for
        // destruction: it is freed once every outstanding buffer is released,
        // and buffers dropped afterwards are freed instead of recycled.
        let mut ptr = self.as_mut_ptr();
        unsafe { ffi::av_buffer_pool_uninit(&mut ptr) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh buffer has the requested size, refcount 1 and writable data.
    #[test]
    fn new_pool_hands_out_sized_buffers() {
        let mut pool = AVBufferPool::new(1024).unwrap();

        let a = pool.get().unwrap();
        assert_eq!(a.size, 1024);
        assert_eq!(pool.buffer_size(), 1024);
        assert_eq!(a.get_ref_count(), 1);
        assert!(a.is_writable());
    }

    /// Buffers are zero-filled on allocation, matching
    /// `av_frame_get_buffer`'s behaviour.
    #[test]
    fn buffers_are_zero_filled_on_allocation() {
        let mut pool = AVBufferPool::new(4096).unwrap();
        let a = pool.get().unwrap();
        let data = unsafe { std::slice::from_raw_parts(a.data, a.size) };
        assert!(data.iter().all(|&byte| byte == 0));
    }

    /// Dropping a buffer returns it to the pool; the next acquisition
    /// recycles the very same memory instead of allocating anew.
    #[test]
    fn dropped_buffers_are_recycled() {
        let mut pool = AVBufferPool::new(1024).unwrap();

        let a = pool.get().unwrap();
        let a_addr = a.data as usize;
        drop(a);

        let b = pool.get().unwrap();
        assert_eq!(b.data as usize, a_addr);
    }

    /// While previous buffers are still referenced, the pool has to grow:
    /// concurrently held buffers never share memory.
    #[test]
    fn held_buffers_do_not_share_memory() {
        let mut pool = AVBufferPool::new(1024).unwrap();

        let a = pool.get().unwrap();
        let b = pool.get().unwrap();
        let c = pool.get().unwrap();
        assert_ne!(a.data, b.data);
        assert_ne!(b.data, c.data);
        assert_ne!(a.data, c.data);
    }

    /// Buffer data pointers keep the alignment `av_malloc()` promises.
    ///
    /// `av_buffer_pool` hands out `av_buffer_allocz()` buffers, which go
    /// through `av_mallocz()` → `av_malloc()`. That allocates with `ALIGN`,
    /// which `libavutil/mem.c` defines as
    /// `HAVE_SIMD_ALIGN_64 ? 64 : (HAVE_SIMD_ALIGN_32 ? 32 : 16)`.
    ///
    /// Only the portable floor can be asserted here: 32/64 depend on how the
    /// linked FFmpeg was configured, and a build without 32-byte SIMD
    /// alignment really does return 16-byte aligned memory (Linux aarch64 is
    /// one such build). Plane layout via `av_image_fill_arrays` and SIMD
    /// consumers rely on getting at least this much.
    #[test]
    fn buffer_data_is_aligned() {
        // The floor `av_malloc()` guarantees on every platform FFmpeg
        // supports, see the note above.
        const AV_MALLOC_MIN_ALIGN: usize = 16;

        let mut pool = AVBufferPool::new(1024).unwrap();
        for _ in 0..8 {
            let buffer = pool.get().unwrap();
            assert_eq!(buffer.data as usize % AV_MALLOC_MIN_ALIGN, 0);
        }
    }

    /// Refcounting across cloned references behaves as expected: cloning
    /// increments, dropping the clone restores.
    #[test]
    fn refcounts_are_tracked() {
        let mut pool = AVBufferPool::new(1024).unwrap();
        let a = pool.get().unwrap();

        let cloned = a.clone();
        assert_eq!(a.get_ref_count(), 2);
        assert!(!a.is_writable(), "shared buffers are not writable");

        drop(cloned);
        assert_eq!(a.get_ref_count(), 1);
        assert!(a.is_writable());
    }

    /// Dropping the pool while buffers are still checked out is safe: those
    /// buffers are freed on their last unref instead of being recycled.
    #[test]
    fn pool_can_be_dropped_with_outstanding_buffers() {
        let mut pool = AVBufferPool::new(512).unwrap();
        let outstanding = pool.get().unwrap();

        drop(pool);
        drop(outstanding);
    }

    /// A zero-sized pool is rejected with `EINVAL` instead of deferring to
    /// FFmpeg's behaviour on empty buffers.
    #[test]
    fn zero_sized_pool_is_rejected() {
        match AVBufferPool::new(0) {
            Ok(_) => panic!("a zero-sized pool should be rejected"),
            Err(err) => assert_eq!(err.raw_error(), Some(ffi::AVERROR(ffi::EINVAL))),
        }
    }
}
