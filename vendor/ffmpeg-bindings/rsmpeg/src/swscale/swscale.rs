use crate::{
    avutil::{AVFrame, AVPixelFormat},
    error::*,
    ffi,
    shared::*,
};
use std::ptr;

wrap!(SwsContext: ffi::SwsContext);
#[cfg(feature = "ffmpeg8")]
settable!(SwsContext {
    src_w: i32,
    src_h: i32,
    src_format: ffi::AVPixelFormat,
    dst_w: i32,
    dst_h: i32,
    dst_format: ffi::AVPixelFormat,
    /// specify which algorithm and options to use for rescaling
    flags: u32,
    threads: i32,
    intent: i32,
    dither: ffi::SwsDither,
    alpha_blend: ffi::SwsAlphaBlend,
    #[cfg(feature = "ffmpeg9")]
    scaler: ffi::SwsScaler,
    #[cfg(feature = "ffmpeg9")]
    backends: ffi::SwsBackend,
});

impl SwsContext {
    /// Allocate and return an [`SwsContext`]. You need it to perform
    /// scaling/conversion operations using [`Self::scale()`].
    ///
    /// Return `None` when input is invalid. Parameter `flags` can be set to
    /// `rsmpeg::ffi::SWS_FAST_BILINEAR` etc.
    ///
    /// Note: since FFmpeg 8, `ffi::SWS_*` constants are typed as
    /// `ffi::SwsFlags`, whose underlying type is target-dependent (`i32` on
    /// MSVC, `u32` elsewhere); a plain `as u32` cast works on all platforms.
    #[allow(clippy::too_many_arguments)]
    pub fn get_context(
        src_w: i32,
        src_h: i32,
        src_format: AVPixelFormat,
        dst_w: i32,
        dst_h: i32,
        dst_format: AVPixelFormat,
        flags: u32,
        src_filter: Option<&ffi::SwsFilter>,
        dst_filter: Option<&ffi::SwsFilter>,
        param: Option<&[f64; 2]>,
    ) -> Option<Self> {
        let context = unsafe {
            ffi::sws_getContext(
                src_w,
                src_h,
                src_format,
                dst_w,
                dst_h,
                dst_format,
                flags as i32,
                src_filter
                    .map(|x| x as *const _ as *mut _)
                    .unwrap_or_else(ptr::null_mut),
                dst_filter
                    .map(|x| x as *const _ as *mut _)
                    .unwrap_or_else(ptr::null_mut),
                param.map(|x| x.as_ptr()).unwrap_or_else(ptr::null),
            )
        }
        .upgrade()?;
        unsafe { Some(Self::from_raw(context)) }
    }

    /// Check if context can be reused, otherwise reallocate a new one.
    ///
    /// Checks if the parameters are the ones already
    /// saved in context. If that is the case, returns the current
    /// context. Otherwise, frees context and gets a new context with
    /// the new parameters.
    ///
    /// Be warned that `src_filter` and `dst_filter` are not checked, they
    /// are assumed to remain the same.
    ///
    /// Returns `None` when context allocation or initiation failed.
    ///
    /// Note: since FFmpeg 8, `ffi::SWS_*` constants are typed as
    /// `ffi::SwsFlags`, whose underlying type is target-dependent (`i32` on
    /// MSVC, `u32` elsewhere); a plain `as u32` cast works on all platforms.
    #[allow(clippy::too_many_arguments)]
    pub fn get_cached_context(
        self,
        src_w: i32,
        src_h: i32,
        src_format: AVPixelFormat,
        dst_w: i32,
        dst_h: i32,
        dst_format: AVPixelFormat,
        flags: u32,
        src_filter: Option<&ffi::SwsFilter>,
        dst_filter: Option<&ffi::SwsFilter>,
        param: Option<&[f64; 2]>,
    ) -> Option<Self> {
        // Note that if sws_getCachedContext fails, context is freed, so we use into_raw here.
        let context = unsafe {
            ffi::sws_getCachedContext(
                self.into_raw().as_ptr(),
                src_w,
                src_h,
                src_format,
                dst_w,
                dst_h,
                dst_format,
                flags as i32,
                src_filter
                    .map(|x| x as *const _ as *mut _)
                    .unwrap_or_else(ptr::null_mut),
                dst_filter
                    .map(|x| x as *const _ as *mut _)
                    .unwrap_or_else(ptr::null_mut),
                param.map(|x| x.as_ptr()).unwrap_or_else(ptr::null),
            )
        }
        .upgrade()?;
        Some(unsafe { Self::from_raw(context) })
    }

    /// Scale the image slice in `src_slice` and put the resulting scaled
    /// slice in the image in `dst`. A slice is a sequence of consecutive
    /// rows in an image.
    ///
    /// Slices have to be provided in sequential order, either in
    /// top-bottom or bottom-top order. If slices are provided in
    /// non-sequential order the behavior of the function is undefined.
    ///
    /// # Safety
    /// The `src_slice` should be valid with the `src_stride`, `src_slice_y` and
    /// `src_slice_h`. The `dst` should be valid with the `dst_stride`.
    pub unsafe fn scale(
        &mut self,
        src_slice: *const *const u8,
        src_stride: *const i32,
        src_slice_y: i32,
        src_slice_h: i32,
        dst: *const *mut u8,
        dst_stride: *const i32,
    ) -> Result<()> {
        // ATTENTION, ffmpeg's documentation doesn't say `sws_scale` could
        // return negative number, but after checking it's implementation, you
        // will find it returns negative number on error.
        unsafe {
            ffi::sws_scale(
                self.as_mut_ptr(),
                src_slice,
                src_stride,
                src_slice_y,
                src_slice_h,
                dst,
                dst_stride,
            )
        }
        .upgrade()?;
        Ok(())
    }

    /// A wrapper of [`Self::scale`], check it's documentation.
    pub fn scale_frame(
        &mut self,
        src_frame: &AVFrame,
        src_slice_y: i32,
        src_slice_h: i32,
        dst_frame: &mut AVFrame,
    ) -> Result<()> {
        unsafe {
            self.scale(
                src_frame.data.as_ptr() as _,
                src_frame.linesize.as_ptr(),
                src_slice_y,
                src_slice_h,
                dst_frame.data.as_ptr(),
                dst_frame.linesize.as_ptr(),
            )
        }
    }
}

/// Modern libswscale API. `sws_scale_frame()` was introduced in FFmpeg 5.0
/// (libswscale 6.1.100), and became the officially recommended API in
/// FFmpeg 8.0 (libswscale 8.12.100), which allowed fully dynamic usage
/// without `sws_init_context()` (deprecated since then) and reclassified
/// the classic `sws_getContext()` + `sws_scale()` workflow as the
/// "Legacy (stateful) API".
///
/// Gated behind `ffmpeg8` rather than `ffmpeg7` because `sws_scale_frame()`
/// crashes on FFmpeg 7.x due to a buffer pool bug, fixed in FFmpeg 8 by
/// <https://github.com/FFmpeg/FFmpeg/commit/6b402cdbf46e4398b3285277f3ff7c3654d57ce6>.
///
/// Since
/// <https://github.com/FFmpeg/FFmpeg/commit/47f89ea88ba1ae9a9ac5b1b9bfa6063dfbd8c73a>
/// (post-FFmpeg 9.0), libswscale explicitly tracks whether a context was
/// initialized through the legacy or the modern API, and rejects any mixing
/// of the two with `AVERROR(EINVAL)`: a context created by [`Self::alloc()`]
/// must only be used with [`Self::scale_full_frame()`], never with the
/// legacy `sws_getContext()`/`sws_scale()` family, and vice versa.
#[cfg(feature = "ffmpeg8")]
impl SwsContext {
    /// Allocate an uninitialized [`SwsContext`] for use with the modern
    /// [`Self::scale_full_frame()`] API.
    ///
    /// [`Self::scale_full_frame()`] can be called directly on such a context
    /// in a fully dynamic mode, deriving all parameters from the frame
    /// properties, without ever calling `sws_init_context()`.
    ///
    /// Return `None` on allocation failure.
    pub fn alloc() -> Option<Self> {
        let context = unsafe { ffi::sws_alloc_context() };
        unsafe { context.upgrade().map(|x| Self::from_raw(x)) }
    }

    /// Scale the image data of `src` and write the output to `dst`.
    ///
    /// This is the modern libswscale API (introduced in FFmpeg 5.0, and the
    /// officially recommended way to use libswscale since FFmpeg 8.0,
    /// which deprecated `sws_init_context()` and the classic
    /// `sws_getContext()` + `sws_scale()` "Legacy (stateful) API").
    ///
    /// It can be used directly on a context created with
    /// [`Self::alloc()`], without setting up any frame properties or
    /// initializing the context. Such usage is fully dynamic and does not
    /// require reallocation if the frame properties change.
    ///
    /// The `dst` frame must have its format, width and height set; its data
    /// buffers may either be allocated by the caller or left clear, in
    /// which case they will be allocated by the scaler.
    ///
    /// Return `Ok(())` on success, `Err(_)` with a negative AVERROR code on
    /// failure.
    pub fn scale_full_frame(&mut self, dst: &mut AVFrame, src: &AVFrame) -> Result<()> {
        unsafe { ffi::sws_scale_frame(self.as_mut_ptr(), dst.as_mut_ptr(), src.as_ptr()) }
            .upgrade()?;
        Ok(())
    }
}

impl Drop for SwsContext {
    fn drop(&mut self) {
        unsafe { ffi::sws_freeContext(self.as_mut_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cached_sws_context() {
        let context = SwsContext::get_context(
            10,
            10,
            ffi::AV_PIX_FMT_RGB24,
            10,
            10,
            ffi::AV_PIX_FMT_RGB24,
            (ffi::SWS_FULL_CHR_H_INT | ffi::SWS_BICUBIC) as _,
            None,
            None,
            Some(&[ffi::SWS_PARAM_DEFAULT as f64, ffi::SWS_PARAM_DEFAULT as f64]),
        )
        .unwrap();
        let old_ptr = context.as_ptr();
        let context = context
            .get_cached_context(
                10,
                10,
                ffi::AV_PIX_FMT_RGB24,
                10,
                10,
                ffi::AV_PIX_FMT_RGB24,
                (ffi::SWS_FULL_CHR_H_INT | ffi::SWS_BICUBIC) as _,
                None,
                None,
                None,
            )
            .unwrap();
        let new_ptr = context.as_ptr();
        assert_eq!(old_ptr, new_ptr);
    }

    #[cfg(feature = "ffmpeg8")]
    #[test]
    fn test_scale_full_frame() {
        use crate::avutil::AVImage;

        // Modern dynamic mode: allocate a context and scale frames directly,
        // deriving all parameters from the frame properties. The source
        // frame buffers are pre-allocated here, while the destination
        // buffers are left clear so the scaler allocates them itself.
        let src_img = AVImage::new(ffi::AV_PIX_FMT_YUV420P, 64, 64, 1).unwrap();
        let mut src = AVFrame::new();
        src.data_mut().clone_from(src_img.data());
        src.linesize_mut().clone_from(src_img.linesizes());
        src.set_format(ffi::AV_PIX_FMT_YUV420P as _);
        src.set_width(64);
        src.set_height(64);

        let mut dst = AVFrame::new();
        dst.set_width(32);
        dst.set_height(32);
        dst.set_format(ffi::AV_PIX_FMT_RGB24 as _);

        let mut context = SwsContext::alloc().unwrap();
        context.scale_full_frame(&mut dst, &src).unwrap();

        assert_eq!(dst.width, 32);
        assert_eq!(dst.height, 32);
        assert_eq!(dst.format, ffi::AV_PIX_FMT_RGB24 as _);
        assert!(!dst.data[0].is_null());
        assert!(dst.linesize[0] >= 32 * 3);
    }

    #[test]
    #[cfg(feature = "ffmpeg8")]
    fn test_alloc_setters_and_scale_full_frame() {
        use crate::{
            avutil::AVImage,
            ffi::{SWS_ACCURATE_RND, SWS_BITEXACT, SWS_DITHER_AUTO},
        };

        // Verify the modern-API field setters on an allocated context.
        let src_img = AVImage::new(ffi::AV_PIX_FMT_YUV420P, 64, 64, 1).unwrap();
        let mut src = AVFrame::new();
        src.data_mut().clone_from(src_img.data());
        src.linesize_mut().clone_from(src_img.linesizes());
        src.set_format(ffi::AV_PIX_FMT_YUV420P as _);
        src.set_width(64);
        src.set_height(64);

        let mut dst = AVFrame::new();
        dst.set_width(32);
        dst.set_height(32);
        dst.set_format(ffi::AV_PIX_FMT_RGB24 as _);

        let mut context = SwsContext::alloc().unwrap();
        context.set_flags((SWS_ACCURATE_RND | SWS_BITEXACT | ffi::SWS_BICUBIC) as _);
        context.set_threads(0);
        context.set_dither(SWS_DITHER_AUTO);
        #[cfg(feature = "ffmpeg9")]
        context.set_scaler(ffi::SWS_SCALE_BICUBIC);
        context.scale_full_frame(&mut dst, &src).unwrap();

        assert_eq!(dst.width, 32);
        assert_eq!(dst.height, 32);
        assert!(!dst.data[0].is_null());
    }
}
