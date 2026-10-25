//! Optional GStreamer backend for CFA4 (layer L5).
//!
//! CFA4's own Rust media engine stays the primary path. GStreamer supplements it where the system's
//! media stack can do things the engine does not do yet: vendor-neutral hardware decoding and
//! encoding (VA-API on AMD and Intel, Vulkan Video), a Qt6 preview sink, PipeWire / V4L2 capture
//! and formats the engine cannot open. The decision and its rules are in
//! `docs/adr/ADR-0002-gstreamer-backend.md`.
//!
//! This first step gives the rest of CFA4 two safe entry points:
//!
//! - [`probe`]: is GStreamer installed, which version, and which of the elements CFA4 plans to use
//!   are present (with their plugin, package and licence, so licence policy can be checked at run
//!   time instead of assumed);
//! - [`discover`]: container, duration, seekability and stream list of a media file, read by
//!   GStreamer's `Discoverer`. This is how files the Rust engine cannot open will be identified.
//!
//! The crate builds and its API exists without GStreamer: the `gstreamer` Cargo feature is off by
//! default, and then every entry point returns [`Error::Unavailable`]. With the feature on, a
//! machine without the GStreamer libraries (or with a broken installation) also gets
//! [`Error::Unavailable`] instead of a crash. The crate contains no `unsafe` code: it uses the safe
//! API of the gstreamer-rs bindings.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::path::Path;
use std::time::Duration;

use cfa4_time::{FrameRate, Tick};

#[cfg(feature = "gstreamer")]
mod backend;
#[cfg(feature = "gstreamer")]
mod source;

/// Errors of the GStreamer backend. Every failure is reported, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// GStreamer cannot be used: the crate was built without the `gstreamer` feature, or the
    /// libraries are missing or fail to initialise. The Rust engine should be used instead.
    #[error("GStreamer is not available: {0}")]
    Unavailable(String),
    /// The input was rejected before GStreamer saw it (missing file, not a regular file…).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// GStreamer could not read the file. `missing_plugins` lists installer details of elements
    /// GStreamer reported as missing, when that was the reason.
    #[error("GStreamer could not read the media: {message}")]
    Discover { message: String, missing_plugins: Vec<String> },
    /// A panic inside the bindings was caught and turned into an error (AGENTS.md §0.6).
    #[error("internal error in the GStreamer backend: {0}")]
    Internal(String),
}

/// What CFA4 wants a GStreamer element for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    /// Hardware video decoding without vendor lock-in (VA-API, Vulkan Video).
    HardwareDecode,
    /// Hardware video encoding (VA-API).
    HardwareEncode,
    /// Showing video in the Qt6 user interface.
    Preview,
    /// Recording from cameras, screens and audio inputs.
    Capture,
    /// General pipeline building blocks (automatic decoding, demuxing, muxing).
    Pipeline,
}

/// One element CFA4 looks for, and what it is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementSpec {
    pub area: Area,
    /// GStreamer element factory name (`gst-inspect-1.0 <name>`).
    pub factory: &'static str,
    /// Short English description of what CFA4 would use it for.
    pub purpose: &'static str,
}

/// The elements [`probe`] checks. Only elements whose plugins are LGPL or more permissive are
/// listed; GPL-only or patent-encumbered plugin sets (`ugly`, `libav`) are deliberately not part of
/// CFA4's plan, and [`ElementStatus::license`] lets the caller verify the licence on the machine.
pub const CATALOGUE: &[ElementSpec] = &[
    ElementSpec { area: Area::HardwareDecode, factory: "vah264dec", purpose: "H.264 decoding through VA-API (AMD, Intel)" },
    ElementSpec { area: Area::HardwareDecode, factory: "vah265dec", purpose: "H.265 / HEVC decoding through VA-API" },
    ElementSpec { area: Area::HardwareDecode, factory: "vavp9dec", purpose: "VP9 decoding through VA-API" },
    ElementSpec { area: Area::HardwareDecode, factory: "vaav1dec", purpose: "AV1 decoding through VA-API" },
    ElementSpec { area: Area::HardwareDecode, factory: "vulkanh264dec", purpose: "H.264 decoding through Vulkan Video" },
    ElementSpec { area: Area::HardwareDecode, factory: "vulkanh265dec", purpose: "H.265 / HEVC decoding through Vulkan Video" },
    ElementSpec { area: Area::HardwareEncode, factory: "vah264enc", purpose: "H.264 encoding through VA-API" },
    ElementSpec { area: Area::HardwareEncode, factory: "vah265enc", purpose: "H.265 / HEVC encoding through VA-API" },
    ElementSpec { area: Area::HardwareEncode, factory: "vaav1enc", purpose: "AV1 encoding through VA-API" },
    ElementSpec { area: Area::Preview, factory: "qml6glsink", purpose: "video preview inside a Qt6 / QML interface" },
    ElementSpec { area: Area::Preview, factory: "glimagesink", purpose: "OpenGL video output (fallback preview)" },
    ElementSpec { area: Area::Capture, factory: "pipewiresrc", purpose: "screen, camera and audio capture through PipeWire" },
    ElementSpec { area: Area::Capture, factory: "v4l2src", purpose: "camera and capture-card input (Video4Linux2)" },
    ElementSpec { area: Area::Pipeline, factory: "uridecodebin3", purpose: "automatic demuxing and decoding of any supported file" },
    ElementSpec { area: Area::Pipeline, factory: "matroskamux", purpose: "Matroska / WebM writing" },
    ElementSpec { area: Area::Pipeline, factory: "mp4mux", purpose: "MP4 writing" },
];

/// Whether one [`CATALOGUE`] element is installed, and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementStatus {
    pub spec: ElementSpec,
    pub available: bool,
    /// Plugin that provides the element (`va`, `qml6`…), when available.
    pub plugin: Option<String>,
    /// Licence the plugin declares (`LGPL`, `MIT/X11`…), when available.
    pub license: Option<String>,
    /// Package the plugin was built in (`GStreamer Bad Plug-ins (Ubuntu)`…), when available.
    pub package: Option<String>,
}

/// Result of [`probe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// GStreamer version of the libraries actually loaded, e.g. `1.24.2`.
    pub version: String,
    /// One entry per [`CATALOGUE`] element, in catalogue order.
    pub elements: Vec<ElementStatus>,
}

impl Status {
    /// The available elements of one area.
    pub fn available(&self, area: Area) -> impl Iterator<Item = &ElementStatus> {
        self.elements.iter().filter(move |e| e.available && e.spec.area == area)
    }

    /// True when at least one element of `area` is installed.
    pub fn has(&self, area: Area) -> bool {
        self.available(area).next().is_some()
    }
}

/// Kind-specific stream details reported by [`discover`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamKind {
    Video {
        width: u32,
        height: u32,
        /// `None` for still images and variable or unknown frame rates.
        frame_rate: Option<FrameRate>,
        /// Bits per pixel as GStreamer reports it (0 when unknown).
        depth: u32,
        is_image: bool,
        /// Pixel aspect ratio, `None` when unknown or invalid.
        par: Option<(u32, u32)>,
    },
    Audio {
        channels: u32,
        sample_rate: u32,
        language: Option<String>,
    },
    Subtitle {
        language: Option<String>,
    },
    /// A stream GStreamer found but did not classify.
    Other,
}

/// One elementary stream of a discovered file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub kind: StreamKind,
    /// Human-readable codec description from GStreamer (`H.264 (High Profile)`…).
    pub codec: Option<String>,
    /// The stream's GStreamer caps as a string (for diagnostics and later element selection).
    pub caps: Option<String>,
}

/// Result of [`discover`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    pub uri: String,
    /// Human-readable container description (`Matroska`, `ISO MP4/M4A`…), if the file has one.
    pub container: Option<String>,
    /// Duration in exact engine ticks, when GStreamer knows it.
    pub duration: Option<Tick>,
    pub seekable: bool,
    pub streams: Vec<StreamInfo>,
}

impl MediaInfo {
    /// The video streams, in file order.
    pub fn video(&self) -> impl Iterator<Item = &StreamInfo> {
        self.streams.iter().filter(|s| matches!(s.kind, StreamKind::Video { .. }))
    }

    /// The audio streams, in file order.
    pub fn audio(&self) -> impl Iterator<Item = &StreamInfo> {
        self.streams.iter().filter(|s| matches!(s.kind, StreamKind::Audio { .. }))
    }
}

/// Default time [`discover`] lets GStreamer spend on one file.
pub const DEFAULT_DISCOVER_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound for the `timeout` of [`discover`]; longer requests are capped.
pub const MAX_DISCOVER_TIMEOUT: Duration = Duration::from_secs(600);

/// Whether this build contains the GStreamer backend at all (the `gstreamer` feature).
pub const fn compiled_in() -> bool {
    cfg!(feature = "gstreamer")
}

/// Checks the GStreamer installation and the elements of [`CATALOGUE`].
///
/// Returns [`Error::Unavailable`] when the backend is not compiled in or GStreamer cannot be
/// initialised; never panics.
pub fn probe() -> Result<Status, Error> {
    #[cfg(feature = "gstreamer")]
    {
        guarded(backend::probe)
    }
    #[cfg(not(feature = "gstreamer"))]
    {
        Err(not_compiled_in())
    }
}

/// Reads container, duration and streams of the file at `path` through GStreamer.
///
/// `timeout` is capped to [`MAX_DISCOVER_TIMEOUT`] and raised to at least one second. The path must
/// be an existing regular file.
pub fn discover(path: &Path, timeout: Duration) -> Result<MediaInfo, Error> {
    #[cfg(feature = "gstreamer")]
    {
        let path = checked_path(path)?;
        let timeout = clamp_timeout(timeout);
        guarded(move || backend::discover(&path, timeout))
    }
    #[cfg(not(feature = "gstreamer"))]
    {
        let _ = (path, timeout);
        Err(not_compiled_in())
    }
}

/// Video file extensions the GStreamer fallback makes importable, beyond the engine's own.
pub const EXTRA_VIDEO_EXTENSIONS: &[&str] = &["ogv", "flv", "f4v", "wmv", "asf", "dv", "divx", "rm", "rmvb", "3g2", "mpe"];
/// Audio file extensions the GStreamer fallback makes importable, beyond the engine's own.
pub const EXTRA_AUDIO_EXTENSIONS: &[&str] = &["oga", "wma", "ape", "wv", "amr", "au", "caf", "spx", "mka"];

/// Opens the movie or audio file at `path` as an engine media source decoded by GStreamer.
///
/// Video frames come out as RGBA 8-bit, audio as 32-bit float at the rate the engine asks for.
/// Still images are declined (the engine's still decoders handle those).
pub fn open(path: &Path) -> Result<cfa4_media::SharedSource, Error> {
    #[cfg(feature = "gstreamer")]
    {
        let path = checked_path(path)?;
        guarded(move || source::GstSource::open(&path).map(|s| std::sync::Arc::new(s) as cfa4_media::SharedSource))
    }
    #[cfg(not(feature = "gstreamer"))]
    {
        let _ = path;
        Err(not_compiled_in())
    }
}

/// Makes GStreamer the engine's fallback backend (`cfa4_media::fallback`): files the built-in
/// openers cannot open, or open without their video, are opened through [`open`], and the
/// [`EXTRA_VIDEO_EXTENSIONS`] / [`EXTRA_AUDIO_EXTENSIONS`] become importable.
///
/// Returns the [`probe`] result. Registers nothing (and returns the error) when GStreamer is not
/// available or lacks the `playbin` element that decoding needs. Safe to call more than once.
pub fn register() -> Result<Status, Error> {
    let status = probe()?;
    #[cfg(feature = "gstreamer")]
    {
        backend::require_element("playbin")?;
        cfa4_media::register_fallback(cfa4_media::Fallback {
            name: "GStreamer",
            video_extensions: EXTRA_VIDEO_EXTENSIONS,
            audio_extensions: EXTRA_AUDIO_EXTENSIONS,
            open: fallback_open,
        });
        log::info!("GStreamer {} registered as the fallback media backend", status.version);
    }
    Ok(status)
}

#[cfg(feature = "gstreamer")]
fn fallback_open(path: &Path) -> cfa4_media::Result<cfa4_media::SharedSource> {
    open(path).map_err(|e| match e {
        Error::InvalidInput(m) => cfa4_media::MediaError::Io(m),
        other => cfa4_media::MediaError::Unsupported(other.to_string()),
    })
}

#[cfg(not(feature = "gstreamer"))]
fn not_compiled_in() -> Error {
    Error::Unavailable("this build of CFA4 was made without the `gstreamer` feature".into())
}

/// Validates a discover path up front, so GStreamer only ever sees existing regular files.
#[cfg_attr(not(feature = "gstreamer"), allow(dead_code))]
fn checked_path(path: &Path) -> Result<std::path::PathBuf, Error> {
    let meta = std::fs::metadata(path).map_err(|e| Error::InvalidInput(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::InvalidInput(format!("{}: not a regular file", path.display())));
    }
    std::fs::canonicalize(path).map_err(|e| Error::InvalidInput(format!("{}: {e}", path.display())))
}

#[cfg_attr(not(feature = "gstreamer"), allow(dead_code))]
fn clamp_timeout(timeout: Duration) -> Duration {
    timeout.max(Duration::from_secs(1)).min(MAX_DISCOVER_TIMEOUT)
}

/// Converts a nanosecond count to engine ticks exactly (saturating at the tick range).
#[cfg_attr(not(feature = "gstreamer"), allow(dead_code))]
fn ticks_from_nanos(nanos: u64) -> Tick {
    Tick::from_units(i64::try_from(nanos).unwrap_or(i64::MAX), 1_000_000_000)
}

/// A frame rate from a GStreamer fraction; `None` for 0/x (variable or unknown) and invalid values.
#[cfg_attr(not(feature = "gstreamer"), allow(dead_code))]
fn frame_rate(num: i32, den: i32) -> Option<FrameRate> {
    if num <= 0 || den <= 0 {
        return None;
    }
    Some(FrameRate { num: i64::from(num), den: i64::from(den) })
}

/// Runs `f`, turning a panic in the bindings into [`Error::Internal`] (AGENTS.md §0.6).
#[cfg(feature = "gstreamer")]
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic without a message".into());
        log::error!("GStreamer backend panicked: {message}");
        Err(Error::Internal(message))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_names_are_unique_and_cover_every_area() {
        let mut names: Vec<_> = CATALOGUE.iter().map(|e| e.factory).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CATALOGUE.len());
        for area in [Area::HardwareDecode, Area::HardwareEncode, Area::Preview, Area::Capture, Area::Pipeline] {
            assert!(CATALOGUE.iter().any(|e| e.area == area), "{area:?} has no element");
        }
    }

    #[test]
    fn catalogue_avoids_gpl_and_patent_plugin_sets() {
        // x264enc / x265enc (ugly, GPL) and avdec_* / avenc_* (libav) must not become part of the plan.
        for e in CATALOGUE {
            assert!(!e.factory.starts_with("x26"), "{}", e.factory);
            assert!(!e.factory.starts_with("av"), "{}", e.factory);
        }
    }

    #[test]
    fn frame_rate_rejects_hostile_fractions() {
        assert_eq!(frame_rate(25, 1), Some(FrameRate::FPS_25));
        assert_eq!(frame_rate(30000, 1001), Some(FrameRate::FPS_29_97));
        assert_eq!(frame_rate(0, 1), None);
        assert_eq!(frame_rate(25, 0), None);
        assert_eq!(frame_rate(-25, 1), None);
        assert_eq!(frame_rate(i32::MIN, i32::MIN), None);
    }

    #[test]
    fn nanos_convert_exactly_and_saturate() {
        assert_eq!(ticks_from_nanos(1_000_000_000), Tick(cfa4_time::TICKS_PER_SECOND));
        assert_eq!(ticks_from_nanos(40_000_000), Tick::from_units(1, 25));
        // u64::MAX nanoseconds must not panic or wrap.
        assert!(ticks_from_nanos(u64::MAX) > Tick::ZERO);
    }

    #[test]
    fn timeout_is_clamped() {
        assert_eq!(clamp_timeout(Duration::ZERO), Duration::from_secs(1));
        assert_eq!(clamp_timeout(Duration::from_secs(5)), Duration::from_secs(5));
        assert_eq!(clamp_timeout(Duration::MAX), MAX_DISCOVER_TIMEOUT);
    }

    #[test]
    fn checked_path_rejects_missing_files_and_directories() {
        assert!(matches!(checked_path(Path::new("/nonexistent/cfa4/file.mkv")), Err(Error::InvalidInput(_))));
        assert!(matches!(checked_path(&std::env::temp_dir()), Err(Error::InvalidInput(_))));
    }

    #[cfg(not(feature = "gstreamer"))]
    #[test]
    fn without_the_feature_everything_is_unavailable() {
        assert!(!compiled_in());
        assert!(matches!(probe(), Err(Error::Unavailable(_))));
        assert!(matches!(discover(Path::new("/nonexistent"), DEFAULT_DISCOVER_TIMEOUT), Err(Error::Unavailable(_))));
    }
}
