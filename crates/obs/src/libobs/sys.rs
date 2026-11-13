//! The only module of `cfa4-obs` with `unsafe` code: loading libobs (and the display library it
//! needs) at run time and calling it. Everything here is wrapped in safe functions taking and
//! returning Rust types; raw pointers stay inside the handle types of this module
//! (ADR 0001 addendum, 2026-11-07: `unsafe` in the binding module, `// SAFETY:` on every block,
//! safe `Result` API, no panic across the FFI boundary).
//!
//! This code runs only in the `cfa4-studio-host` process.

#![allow(unsafe_code)]

use super::ffi;
use crate::{Frame, PixelFormat, Result, StudioError};
use libloading::Library;
use std::ffi::{CStr, CString, c_void};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

fn unavailable(what: impl std::fmt::Display) -> StudioError {
    StudioError::Unavailable(what.to_string())
}

/// Copies one function pointer out of `lib`.
///
/// # Safety
/// `T` must be the exact C signature of the symbol `name` in `lib`.
unsafe fn symbol<T: Copy>(lib: &Library, name: &[u8]) -> Result<T> {
    // SAFETY: the caller guarantees `T` matches the symbol's C signature; the pointer is used
    // only while `lib` stays loaded (it is kept next to the pointers in the same struct).
    let s = unsafe { lib.get::<T>(name) }.map_err(|e| unavailable(format!("libobs symbol {} missing: {e}", String::from_utf8_lossy(name))))?;
    Ok(*s)
}

/// The libobs entry points the S0 binding uses, with the library that holds them.
pub struct Api {
    get_version_string: ffi::FnGetVersionString,
    set_nix_platform: ffi::FnSetNixPlatform,
    set_nix_platform_display: ffi::FnSetNixPlatformDisplay,
    startup: ffi::FnStartup,
    shutdown: ffi::FnShutdown,
    reset_video: ffi::FnResetVideo,
    scene_create: ffi::FnSceneCreate,
    scene_get_source: ffi::FnSceneGetSource,
    scene_release: ffi::FnSceneRelease,
    set_output_source: ffi::FnSetOutputSource,
    add_raw_video_callback: ffi::FnAddRawVideoCallback,
    remove_raw_video_callback: ffi::FnRemoveRawVideoCallback,
    // last: dropped after nothing can call the pointers above any more
    _lib: Library,
}

// SAFETY: `Api` holds a loaded library and plain C function pointers; libobs's API functions may be
// called from any thread (libobs takes its own locks), and the host calls them from one thread.
unsafe impl Send for Api {}

impl Api {
    /// Loads libobs from `path`.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(unavailable(format!("{} not found (build it with scripts/dev/build-libobs-33.0.1.sh)", path.display())));
        }
        // SAFETY: loading libobs runs its ELF constructors, which only initialise libobs's own
        // static state; the path is the CFA4 libobs build, not an arbitrary library.
        let lib = unsafe { Library::new(path) }.map_err(|e| unavailable(format!("cannot load {}: {e}", path.display())))?;
        // SAFETY: for every symbol below, the requested type is the C signature transcribed in
        // `ffi` from the libobs 33.0.1 headers (checked by `ffi::abi_tests`).
        unsafe {
            Ok(Self {
                get_version_string: symbol(&lib, b"obs_get_version_string\0")?,
                set_nix_platform: symbol(&lib, b"obs_set_nix_platform\0")?,
                set_nix_platform_display: symbol(&lib, b"obs_set_nix_platform_display\0")?,
                startup: symbol(&lib, b"obs_startup\0")?,
                shutdown: symbol(&lib, b"obs_shutdown\0")?,
                reset_video: symbol(&lib, b"obs_reset_video\0")?,
                scene_create: symbol(&lib, b"obs_scene_create\0")?,
                scene_get_source: symbol(&lib, b"obs_scene_get_source\0")?,
                scene_release: symbol(&lib, b"obs_scene_release\0")?,
                set_output_source: symbol(&lib, b"obs_set_output_source\0")?,
                add_raw_video_callback: symbol(&lib, b"obs_add_raw_video_callback\0")?,
                remove_raw_video_callback: symbol(&lib, b"obs_remove_raw_video_callback\0")?,
                _lib: lib,
            })
        }
    }

    /// `obs_get_version_string()`.
    pub fn version(&self) -> String {
        // SAFETY: takes no arguments; returns a pointer to a static NUL-terminated string (or null).
        let p = unsafe { (self.get_version_string)() };
        if p.is_null() {
            return String::new();
        }
        // SAFETY: non-null and points to libobs's static version string, valid while loaded.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    /// Selects the display platform; must precede [`Api::startup`]. `display` must outlive libobs.
    pub fn set_platform(&self, display: &Display) {
        // SAFETY: plain enum argument; libobs stores it.
        unsafe { (self.set_nix_platform)(display.platform) };
        // SAFETY: the display connection is open and stays open until after `obs_shutdown`
        // (the backend drops `Display` after `Api::shutdown`).
        unsafe { (self.set_nix_platform_display)(display.ptr) };
    }

    /// `obs_startup("en-US", NULL, NULL)`.
    pub fn startup(&self) -> Result<()> {
        // SAFETY: a literal NUL-terminated locale; NULL module config path and profiler store
        // are documented as allowed.
        let ok = unsafe { (self.startup)(c"en-US".as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
        if ok { Ok(()) } else { Err(StudioError::Backend("obs_startup failed".into())) }
    }

    /// `obs_shutdown()`.
    pub fn shutdown(&self) {
        // SAFETY: valid after a successful or failed `obs_startup`; libobs frees its own state.
        unsafe { (self.shutdown)() };
    }

    /// `obs_reset_video` with `video`, output BGRA, sRGB full range.
    pub fn reset_video(&self, graphics_module: &CStr, video: &crate::VideoSettings) -> Result<()> {
        let mut ovi = ffi::obs_video_info {
            graphics_module: graphics_module.as_ptr(),
            fps_num: video.fps_num,
            fps_den: video.fps_den,
            base_width: video.width,
            base_height: video.height,
            output_width: video.width,
            output_height: video.height,
            output_format: ffi::VIDEO_FORMAT_BGRA,
            adapter: 0,
            gpu_conversion: false,
            colorspace: ffi::VIDEO_CS_SRGB,
            range: ffi::VIDEO_RANGE_FULL,
            scale_type: ffi::OBS_SCALE_BICUBIC,
        };
        // SAFETY: `ovi` is a fully initialised `obs_video_info` living across the call; the
        // module name outlives it (libobs copies what it keeps).
        let code = unsafe { (self.reset_video)(&mut ovi) };
        if code == ffi::OBS_VIDEO_SUCCESS { Ok(()) } else { Err(StudioError::Backend(format!("obs_reset_video: {} ({code})", ffi::reset_video_error(code)))) }
    }

    /// Creates an empty scene and puts it on output channel 0.
    pub fn show_empty_scene(&self, name: &str) -> Result<Scene> {
        let name = CString::new(name).map_err(|_| StudioError::Invalid("scene name contains NUL".into()))?;
        // SAFETY: NUL-terminated name, copied by libobs.
        let scene = unsafe { (self.scene_create)(name.as_ptr()) };
        if scene.is_null() {
            return Err(StudioError::Backend("obs_scene_create failed".into()));
        }
        // SAFETY: `scene` is a live scene we own a reference to; the source is borrowed from it.
        let source = unsafe { (self.scene_get_source)(scene) };
        if source.is_null() {
            // SAFETY: releasing the reference `obs_scene_create` gave us.
            unsafe { (self.scene_release)(scene) };
            return Err(StudioError::Backend("obs_scene_get_source failed".into()));
        }
        // SAFETY: the source is alive (the scene holds it); channel 0 is a valid channel. libobs
        // takes its own reference.
        unsafe { (self.set_output_source)(0, source) };
        Ok(Scene { ptr: scene })
    }

    /// Takes the scene off output channel 0 and releases it.
    pub fn release_scene(&self, scene: Scene) {
        // SAFETY: clearing channel 0 is always valid (drops libobs's reference).
        unsafe { (self.set_output_source)(0, std::ptr::null_mut()) };
        // SAFETY: `scene` holds the one reference `obs_scene_create` gave us; it is consumed here.
        unsafe { (self.scene_release)(scene.ptr) };
    }

    /// Registers `sink` for raw program frames (BGRA, the output format).
    pub fn add_raw_video(&self, sink: Arc<FrameSink>) -> RawVideo {
        let param = Arc::into_raw(sink) as *mut c_void;
        // SAFETY: NULL conversion means "the output format"; the callback is `on_raw_video`
        // below; `param` is a leaked `Arc<FrameSink>` that stays valid until `remove_raw_video`
        // has returned (libobs then no longer calls it).
        unsafe { (self.add_raw_video_callback)(std::ptr::null(), on_raw_video, param) };
        RawVideo { param }
    }

    /// Unregisters a raw-video callback and frees its sink reference.
    pub fn remove_raw_video(&self, raw: RawVideo) {
        // SAFETY: the same callback/param pair that was registered; libobs disconnects it under
        // its video input lock, so no call is running or will run once this returns.
        unsafe { (self.remove_raw_video_callback)(on_raw_video, raw.param) };
        // SAFETY: `param` came from `Arc::into_raw` in `add_raw_video` and is released once.
        drop(unsafe { Arc::from_raw(raw.param as *const FrameSink) });
    }
}

/// A scene created by [`Api::show_empty_scene`].
pub struct Scene {
    ptr: *mut ffi::obs_scene_t,
}

// SAFETY: a libobs scene reference may be released from any thread (libobs reference counting is
// atomic); the backend uses it from one thread at a time.
unsafe impl Send for Scene {}

/// A registered raw-video callback.
pub struct RawVideo {
    param: *mut c_void,
}

// SAFETY: `param` is an `Arc<FrameSink>` pointer; `FrameSink` is `Sync`.
unsafe impl Send for RawVideo {}

/// Where the raw-video callback leaves the newest frame.
pub struct FrameSink {
    width: u32,
    height: u32,
    slot: Mutex<(u64, Option<Frame>)>,
    ready: Condvar,
}

impl FrameSink {
    /// A sink for `width` × `height` BGRA frames.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, slot: Mutex::new((0, None)), ready: Condvar::new() }
    }

    /// Waits up to `timeout` for a frame newer than the last one taken.
    pub fn take(&self, timeout: Duration) -> Option<Frame> {
        let guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut guard, _) = self.ready.wait_timeout_while(guard, timeout, |s| s.1.is_none()).unwrap_or_else(PoisonError::into_inner);
        guard.1.take()
    }

    fn put(&self, frame: Frame) {
        let mut guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        guard.0 = guard.0.wrapping_add(1);
        guard.1 = Some(frame);
        self.ready.notify_all();
    }
}

/// Copies plane 0 (BGRA) of a libobs frame into the sink. Runs on libobs's video thread; a panic
/// is caught here and the frame dropped, so nothing unwinds into C.
unsafe extern "C" fn on_raw_video(param: *mut c_void, frame: *mut ffi::video_data) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if param.is_null() || frame.is_null() {
            return;
        }
        // SAFETY: `param` is the `Arc<FrameSink>` pointer registered in `add_raw_video`, alive
        // until the callback is removed (which waits for running calls).
        let sink = unsafe { &*(param as *const FrameSink) };
        // SAFETY: libobs passes a valid `video_data` for the duration of the call.
        let vd = unsafe { &*frame };
        let (w, h) = (sink.width as usize, sink.height as usize);
        let row = w * 4;
        let stride = vd.linesize[0] as usize;
        let base = vd.data[0];
        if base.is_null() || stride < row {
            return;
        }
        let Some(total) = stride.checked_mul(h) else { return };
        // SAFETY: libobs's plane 0 of a BGRA output frame is `linesize[0] * output_height` bytes,
        // and the sink's size is the output size the video was reset with.
        let plane = unsafe { std::slice::from_raw_parts(base as *const u8, total) };
        let mut data = Vec::with_capacity(row * h);
        for r in 0..h {
            if let Some(src) = plane.get(r * stride..r * stride + row) {
                data.extend_from_slice(src);
            }
        }
        sink.put(Frame { width: sink.width, height: sink.height, stride: sink.width * 4, format: PixelFormat::Bgra8, pts_ns: vd.timestamp, data });
    }));
}

/// An open display connection libobs renders through (X11 for EGL, or Wayland).
pub struct Display {
    platform: ffi::obs_nix_platform_type,
    ptr: *mut c_void,
    close: Closer,
    // last: the library must outlive the pointer and the close function
    _lib: Library,
}

enum Closer {
    X11(ffi::FnXCloseDisplay),
    Wayland(ffi::FnWlDisplayDisconnect),
}

// SAFETY: the display connection is used by libobs's graphics thread through libobs only and
// closed once, after `obs_shutdown`, from the thread that owns the backend.
unsafe impl Send for Display {}

impl Display {
    /// Opens the session's display: Wayland when `WAYLAND_DISPLAY` is set, else X11 (`DISPLAY`).
    pub fn open() -> Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Self::open_wayland();
        }
        if std::env::var_os("DISPLAY").is_some() {
            return Self::open_x11();
        }
        Err(unavailable("no display (neither WAYLAND_DISPLAY nor DISPLAY is set); libobs needs one for its OpenGL renderer"))
    }

    fn open_x11() -> Result<Self> {
        // SAFETY: loading the system Xlib runs only its own initialisers.
        let lib = unsafe { Library::new("libX11.so.6") }.map_err(|e| unavailable(format!("libX11 not available: {e}")))?;
        // SAFETY: the types are Xlib's `XOpenDisplay` / `XCloseDisplay` signatures.
        let (open, close): (ffi::FnXOpenDisplay, ffi::FnXCloseDisplay) = unsafe { (symbol(&lib, b"XOpenDisplay\0")?, symbol(&lib, b"XCloseDisplay\0")?) };
        // SAFETY: NULL means "the display named by $DISPLAY".
        let ptr = unsafe { open(std::ptr::null()) };
        if ptr.is_null() {
            return Err(unavailable("cannot open the X11 display"));
        }
        Ok(Self { platform: ffi::OBS_NIX_PLATFORM_X11_EGL, ptr, close: Closer::X11(close), _lib: lib })
    }

    fn open_wayland() -> Result<Self> {
        // SAFETY: loading the system libwayland-client runs only its own initialisers.
        let lib = unsafe { Library::new("libwayland-client.so.0") }.map_err(|e| unavailable(format!("libwayland-client not available: {e}")))?;
        // SAFETY: the types are `wl_display_connect` / `wl_display_disconnect` signatures.
        let (connect, disconnect): (ffi::FnWlDisplayConnect, ffi::FnWlDisplayDisconnect) =
            unsafe { (symbol(&lib, b"wl_display_connect\0")?, symbol(&lib, b"wl_display_disconnect\0")?) };
        // SAFETY: NULL means "the display named by $WAYLAND_DISPLAY".
        let ptr = unsafe { connect(std::ptr::null()) };
        if ptr.is_null() {
            return Err(unavailable("cannot connect to the Wayland display"));
        }
        Ok(Self { platform: ffi::OBS_NIX_PLATFORM_WAYLAND, ptr, close: Closer::Wayland(disconnect), _lib: lib })
    }

    /// `x11` or `wayland`.
    pub fn kind(&self) -> &'static str {
        if self.platform == ffi::OBS_NIX_PLATFORM_WAYLAND { "wayland" } else { "x11" }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        match self.close {
            // SAFETY: `ptr` is the connection opened by the matching open call, closed once.
            Closer::X11(close) => unsafe {
                close(self.ptr);
            },
            // SAFETY: as above, for Wayland.
            Closer::Wayland(disconnect) => unsafe { disconnect(self.ptr) },
        }
    }
}
