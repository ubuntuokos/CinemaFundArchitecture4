//! FFmpeg supplementary media backend for CFA4 (layer L5).
//!
//! CFA4's own Rust media engine is the base. FFmpeg (at least 9.0.2, GPL-3 build) supplements it
//! where the engine does not decode a format yet: 10-bit and 4:2:2 H.264 / HEVC camera media
//! without hardware (`docs/gaps.md` G3), AVI, DV / DVCPRO, WMV / ASF, MPEG-4 Part 2 and other
//! containers and codecs (G10). The decision and its rules are in
//! `docs/adr/ADR-0005-media-backends.md`.
//!
//! - [`probe`]: the FFmpeg this process linked: version, licence, configuration, hardware
//!   acceleration types, and whether the patent-restricted encoders of the Ubuntu model
//!   (`docs/licensing.md` §7) are present.
//! - [`discover`]: container, duration and streams of a file.
//! - [`open`]: an engine `MediaSource` decoded by FFmpeg, at the stream's **native** bit depth and
//!   chroma (8-bit and 9–16-bit planar Y'CbCr go to the engine as `Yuv8` / `Yuv16`, other layouts
//!   as RGBA 8-bit), audio as 32-bit float at the rate the engine asks for.
//! - [`register`]: makes FFmpeg the engine's first fallback backend (`cfa4_media::fallback`),
//!   before GStreamer; [`set_enabled`] switches it off and on at run time (Settings ▸ Media
//!   Backends).
//! - [`route_backend`]: the `ffmpeg` backend of the compute router (`cfa4-route`, ADR 0004).
//!
//! The crate builds and its API exists without FFmpeg: the `ffmpeg` Cargo feature is off by
//! default, and then every entry point returns [`Error::Unavailable`]. The crate contains no
//! `unsafe` code (it uses rsmedia's safe API); every call into FFmpeg runs under
//! `catch_unwind`, and strings are checked before they reach the C library (a NUL byte in a path
//! panics inside rsmedia).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use cfa4_time::Tick;
use serde::Serialize;

#[cfg(feature = "ffmpeg")]
mod backend;
mod route;
#[cfg(feature = "ffmpeg")]
mod source;

pub use route::{ROUTE_ID, route_backend};

/// Errors of the FFmpeg backend. Every failure is reported, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// FFmpeg cannot be used: the crate was built without the `ffmpeg` feature, the linked FFmpeg
    /// is older than [`MIN_VERSION`], or it is not a GPL-3 build without non-free parts.
    #[error("FFmpeg is not available: {0}")]
    Unavailable(String),
    /// The input was rejected before FFmpeg saw it (missing file, not a regular file, NUL byte…).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// FFmpeg could not open or decode the media.
    #[error("FFmpeg could not read the media: {0}")]
    Media(String),
    /// The backend is switched off (Settings ▸ Media Backends).
    #[error("the FFmpeg backend is switched off in Settings ▸ Media Backends")]
    Disabled,
    /// A panic inside the bindings was caught and turned into an error (AGENTS.md §0.6).
    #[error("internal error in the FFmpeg backend: {0}")]
    Internal(String),
}

/// Lowest FFmpeg release the backend accepts (ADR 0005).
pub const MIN_VERSION: (u32, u32, u32) = (9, 0, 2);

/// A patent-restricted component of the Ubuntu model (`docs/licensing.md` §7): never in the base
/// package, installed on request with `cfa4-restricted-extras`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RestrictedSpec {
    /// FFmpeg encoder name.
    pub encoder: &'static str,
    /// What CFA4 would use it for (English; the GUI translates it).
    pub purpose: &'static str,
}

/// The restricted encoders [`probe`] looks for.
pub const RESTRICTED: &[RestrictedSpec] = &[
    RestrictedSpec { encoder: "libx264", purpose: "H.264 software encoding (x264)" },
    RestrictedSpec { encoder: "libx265", purpose: "HEVC software encoding (x265)" },
];

/// Name of the package that brings the [`RESTRICTED`] components (Ubuntu model).
pub const RESTRICTED_PACKAGE: &str = "cfa4-restricted-extras";

/// Whether one [`RESTRICTED`] component is present in the linked FFmpeg.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictedStatus {
    pub encoder: String,
    pub purpose: String,
    pub available: bool,
}

/// Result of [`probe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// FFmpeg version of the libraries actually loaded (`9.0.2`, or a git description).
    pub version: String,
    /// The version as numbers, when it can be read.
    pub version_numbers: Option<(u32, u32, u32)>,
    /// Licence the libraries report (`GPL version 3 or later`).
    pub license: String,
    /// The `configure` line the libraries were built with.
    pub configuration: String,
    /// Prefix of the loaded libraries, read from `--prefix=` in the configuration.
    pub prefix: Option<String>,
    /// At least [`MIN_VERSION`].
    pub meets_minimum: bool,
    /// A GPL version 3 build.
    pub gpl3: bool,
    /// Built with `--enable-nonfree` (never acceptable: such a build cannot be distributed).
    pub nonfree: bool,
    /// Hardware acceleration types this FFmpeg was built with (`vaapi`, `vulkan`, `cuda`…).
    pub hwaccels: Vec<String>,
    /// The patent-restricted components (Ubuntu model).
    pub restricted: Vec<RestrictedStatus>,
}

impl Status {
    /// Why the backend refuses this FFmpeg, or `None` when it is usable.
    pub fn refusal(&self) -> Option<String> {
        if self.nonfree {
            return Some("this FFmpeg was built with --enable-nonfree and cannot be used".into());
        }
        if !self.gpl3 {
            return Some(format!("CFA4 needs a GPL version 3 build of FFmpeg, this one is '{}'", self.license));
        }
        if !self.meets_minimum {
            let (a, b, c) = MIN_VERSION;
            return Some(format!("CFA4 needs FFmpeg {a}.{b}.{c} or later, this one is '{}'", self.version));
        }
        None
    }
}

/// Kind-specific stream details reported by [`discover`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum StreamKind {
    Video {
        width: u32,
        height: u32,
        /// FFmpeg pixel format name (`yuv422p10le`).
        pixel_format: String,
        /// `None` when unknown.
        frame_rate: Option<(i64, i64)>,
    },
    Audio {
        channels: u32,
        sample_rate: u32,
    },
    Subtitle,
    Other,
}

/// One elementary stream of a discovered file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub index: usize,
    #[serde(flatten)]
    pub kind: StreamKind,
    /// FFmpeg codec name (`h264`, `hevc`, `dvvideo`…).
    pub codec: String,
}

/// Result of [`discover`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// FFmpeg container (demuxer) name (`mov,mp4,m4a,3gp,3g2,mj2`, `avi`…).
    pub container: String,
    /// Duration in exact engine ticks, when known.
    pub duration: Option<Tick>,
    pub streams: Vec<StreamInfo>,
}

/// Whether this build contains the FFmpeg backend at all (the `ffmpeg` feature).
pub const fn compiled_in() -> bool {
    cfg!(feature = "ffmpeg")
}

/// Switches the backend off (files it would open fall through to the next backend) or on again.
/// Takes effect for the next file opened; sources already open keep working. The switch lives in
/// `cfa4_media::backends`, where the engine sets it from Settings ▸ Media Backends.
pub fn set_enabled(enabled: bool) {
    cfa4_media::set_backend_enabled(ROUTE_ID, enabled);
}

/// Whether the backend is switched on (the default).
pub fn enabled() -> bool {
    cfa4_media::backend_enabled(ROUTE_ID)
}

/// Registers this backend's report for Settings ▸ Media Backends (`media.backends`). The apps call
/// it at startup whether or not [`register`] succeeds, so the panel can say why FFmpeg is not used.
pub fn register_status() {
    cfa4_media::register_backend(cfa4_media::BackendEntry { id: ROUTE_ID, report });
}

/// The report of [`register_status`].
pub fn report() -> cfa4_media::BackendReport {
    let mut r = cfa4_media::BackendReport {
        id: ROUTE_ID.into(),
        name: "FFmpeg".into(),
        role: "Supplementary decoder for formats the own engine does not read yet (10-bit and 4:2:2 camera media, AVI, DV, WMV, MPEG-4 Part 2…)".into(),
        compiled_in: compiled_in(),
        enabled: enabled(),
        restricted_package: Some(RESTRICTED_PACKAGE.into()),
        ..Default::default()
    };
    match probe() {
        Ok(status) => {
            r.problem = status.refusal().or_else(|| (!registered()).then(|| "FFmpeg was found but not registered as a backend".into()));
            r.available = r.problem.is_none();
            r.version = Some(status.version);
            r.license = Some(status.license);
            r.location = status.prefix;
            r.capabilities = status.hwaccels.iter().map(|h| format!("hwaccel: {h}")).collect();
            r.restricted =
                status.restricted.into_iter().map(|x| cfa4_media::RestrictedItem { name: x.encoder, purpose: x.purpose, available: x.available }).collect();
        }
        Err(e) => {
            r.problem = Some(e.to_string());
            r.restricted =
                RESTRICTED.iter().map(|x| cfa4_media::RestrictedItem { name: x.encoder.into(), purpose: x.purpose.into(), available: false }).collect();
        }
    }
    r
}

/// Reads the linked FFmpeg's version, licence, configuration, hardware acceleration types and
/// restricted components. Returns [`Error::Unavailable`] when the backend is not compiled in;
/// never panics. It does not judge the build: see [`Status::refusal`].
pub fn probe() -> Result<Status, Error> {
    #[cfg(feature = "ffmpeg")]
    {
        guarded(|| Ok(backend::probe()))
    }
    #[cfg(not(feature = "ffmpeg"))]
    {
        Err(not_compiled_in())
    }
}

/// Reads container, duration and streams of the file at `path` through FFmpeg.
pub fn discover(path: &Path) -> Result<MediaInfo, Error> {
    #[cfg(feature = "ffmpeg")]
    {
        let path = checked_path(path)?;
        guarded(move || backend::discover(&path))
    }
    #[cfg(not(feature = "ffmpeg"))]
    {
        let _ = path;
        Err(not_compiled_in())
    }
}

/// Opens the movie or audio file at `path` as an engine media source decoded by FFmpeg.
///
/// Still images are declined (the engine's still decoders handle those).
pub fn open(path: &Path) -> Result<cfa4_media::SharedSource, Error> {
    #[cfg(feature = "ffmpeg")]
    {
        if !enabled() {
            return Err(Error::Disabled);
        }
        let path = checked_path(path)?;
        guarded(move || source::FfSource::open(&path).map(|s| std::sync::Arc::new(s) as cfa4_media::SharedSource))
    }
    #[cfg(not(feature = "ffmpeg"))]
    {
        let _ = path;
        Err(not_compiled_in())
    }
}

/// Video file extensions the FFmpeg fallback makes importable, beyond the engine's own.
pub const EXTRA_VIDEO_EXTENSIONS: &[&str] = &["dv", "dif", "wmv", "asf", "flv", "f4v", "ogv", "divx", "xvid", "rm", "rmvb", "3g2", "mpe", "nut", "vob"];
/// Audio file extensions the FFmpeg fallback makes importable, beyond the engine's own.
pub const EXTRA_AUDIO_EXTENSIONS: &[&str] = &["wma", "ape", "wv", "amr", "au", "caf", "spx", "mka", "oga", "tta", "dts", "ac3", "eac3", "mlp", "thd"];

/// Makes FFmpeg the engine's fallback backend (`cfa4_media::fallback`). The apps call it before
/// the GStreamer backend's `register`, so the order is: built-in openers → FFmpeg → GStreamer.
///
/// Returns the [`probe`] result. Registers nothing, and returns [`Error::Unavailable`], when the
/// backend is not compiled in or the linked FFmpeg is refused ([`Status::refusal`]). Safe to call
/// more than once.
pub fn register() -> Result<Status, Error> {
    let status = probe()?;
    if let Some(reason) = status.refusal() {
        return Err(Error::Unavailable(reason));
    }
    cfa4_media::register_fallback(cfa4_media::Fallback {
        name: "FFmpeg",
        video_extensions: EXTRA_VIDEO_EXTENSIONS,
        audio_extensions: EXTRA_AUDIO_EXTENSIONS,
        open: fallback_open,
    });
    REGISTERED.store(true, Ordering::Relaxed);
    log::info!("FFmpeg {} ({}) registered as a fallback media backend", status.version, status.license);
    Ok(status)
}

static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Whether [`register`] succeeded in this process.
pub fn registered() -> bool {
    REGISTERED.load(Ordering::Relaxed)
}

fn fallback_open(path: &Path) -> cfa4_media::Result<cfa4_media::SharedSource> {
    open(path).map_err(|e| match e {
        Error::InvalidInput(m) => cfa4_media::MediaError::Io(m),
        other => cfa4_media::MediaError::Unsupported(other.to_string()),
    })
}

#[cfg(not(feature = "ffmpeg"))]
fn not_compiled_in() -> Error {
    Error::Unavailable("this build of CFA4 was made without the `ffmpeg` feature".into())
}

/// Validates a path up front, so FFmpeg only ever sees existing regular files whose path it can
/// take: a NUL byte panics inside rsmedia (`CString::new(..).unwrap()`), and FFmpeg would read a
/// path that is not valid UTF-8 differently from the file system.
#[cfg(any(feature = "ffmpeg", test))]
fn checked_path(path: &Path) -> Result<std::path::PathBuf, Error> {
    let meta = std::fs::metadata(path).map_err(|e| Error::InvalidInput(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::InvalidInput(format!("{}: not a regular file", path.display())));
    }
    let canonical = std::fs::canonicalize(path).map_err(|e| Error::InvalidInput(format!("{}: {e}", path.display())))?;
    check_path_text(&canonical)?;
    Ok(canonical)
}

/// The text checks of [`checked_path`]: UTF-8, no NUL byte, not taken for a URL or a protocol.
#[cfg(any(feature = "ffmpeg", test))]
fn check_path_text(path: &Path) -> Result<(), Error> {
    let text = path.to_str().ok_or_else(|| Error::InvalidInput(format!("{}: the path is not valid UTF-8", path.display())))?;
    if text.contains('\0') {
        return Err(Error::InvalidInput("the path contains a NUL byte".into()));
    }
    if !text.starts_with('/') {
        // Only absolute file paths: a relative "concat:a|b" or "http:…" would select an FFmpeg
        // protocol instead of a file.
        return Err(Error::InvalidInput(format!("{text}: not an absolute path")));
    }
    Ok(())
}

/// Reads `major.minor.patch` from an FFmpeg version string (`9.0.2`, `n9.0.2`, `9.0`).
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let text = text.trim();
    let text = text.strip_prefix('n').unwrap_or(text);
    let numeric: String = text.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let mut parts = numeric.split('.').filter(|p| !p.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    Some((major, minor, patch))
}

/// `--prefix=` of a configure line.
fn prefix_of(configuration: &str) -> Option<String> {
    configuration.split_whitespace().find_map(|w| w.strip_prefix("--prefix=")).map(|p| p.trim_matches(['\'', '"']).to_string())
}

/// Builds a [`Status`] from what the libraries report (separate from the FFI so it can be tested).
#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
fn status_from(version: String, license: String, configuration: String, hwaccels: Vec<String>, restricted: Vec<RestrictedStatus>) -> Status {
    let version_numbers = parse_version(&version);
    let lower = license.to_ascii_lowercase();
    let nonfree = lower.contains("nonfree") || lower.contains("unredistributable") || configuration.contains("--enable-nonfree");
    Status {
        meets_minimum: version_numbers.is_some_and(|v| v >= MIN_VERSION),
        gpl3: lower.contains("gpl version 3"),
        nonfree,
        prefix: prefix_of(&configuration),
        version,
        version_numbers,
        license,
        configuration,
        hwaccels,
        restricted,
    }
}

/// Runs `f`, turning a panic in the bindings into [`Error::Internal`] (AGENTS.md §0.6).
#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic without a message".into());
        log::error!("FFmpeg backend panicked: {message}");
        Err(Error::Internal(message))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(version: &str, license: &str, configuration: &str) -> Status {
        status_from(version.into(), license.into(), configuration.into(), Vec::new(), Vec::new())
    }

    #[test]
    fn versions_are_read_from_release_and_git_tags() {
        assert_eq!(parse_version("9.0.2"), Some((9, 0, 2)));
        assert_eq!(parse_version("n9.0.2"), Some((9, 0, 2)));
        assert_eq!(parse_version("9.1"), Some((9, 1, 0)));
        assert_eq!(parse_version("10"), Some((10, 0, 0)));
        assert_eq!(parse_version("6.1.1-3ubuntu5"), Some((6, 1, 1)));
        assert_eq!(parse_version("N-114985-g1234abcd"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("99999999999.0.0"), None);
    }

    #[test]
    fn only_gpl3_builds_from_9_0_2_without_nonfree_are_accepted() {
        let good = st("9.0.2", "GPL version 3 or later", "--prefix=/opt/cfa4/ffmpeg-9.0.2 --enable-gpl --enable-version3");
        assert_eq!(good.refusal(), None);
        assert_eq!(good.prefix.as_deref(), Some("/opt/cfa4/ffmpeg-9.0.2"));
        assert!(st("9.0.1", "GPL version 3 or later", "").refusal().is_some());
        assert!(st("8.1", "GPL version 3 or later", "").refusal().is_some());
        // Ubuntu 26.04's own FFmpeg is 8.x and LGPL/GPL-2: refused, the engine works without it
        assert!(st("8.0.1", "GPL version 2 or later", "").refusal().is_some());
        assert!(st("9.0.2", "LGPL version 2.1 or later", "").refusal().is_some());
        assert!(st("9.0.2", "nonfree and unredistributable", "--enable-nonfree").refusal().is_some());
        assert!(st("9.0.2", "GPL version 3 or later", "--enable-gpl --enable-version3 --enable-nonfree").refusal().is_some());
        assert!(st("N-1-gabc", "GPL version 3 or later", "").refusal().is_some());
    }

    #[test]
    fn restricted_list_is_the_ubuntu_model() {
        let names: Vec<_> = RESTRICTED.iter().map(|r| r.encoder).collect();
        assert_eq!(names, ["libx264", "libx265"]);
        assert_eq!(RESTRICTED_PACKAGE, "cfa4-restricted-extras");
    }

    #[test]
    fn paths_are_checked_before_ffmpeg_sees_them() {
        assert!(matches!(checked_path(Path::new("/nonexistent/cfa4/file.mov")), Err(Error::InvalidInput(_))));
        assert!(matches!(checked_path(&std::env::temp_dir()), Err(Error::InvalidInput(_))));
        // a NUL byte panicked inside rsmedia before this check (CString::new(..).unwrap())
        assert!(matches!(check_path_text(Path::new("/tmp/klip\0.mp4")), Err(Error::InvalidInput(_))));
        assert!(matches!(check_path_text(Path::new("concat:/a.mp4|/b.mp4")), Err(Error::InvalidInput(_))));
        assert!(matches!(check_path_text(Path::new("http://example.com/a.mp4")), Err(Error::InvalidInput(_))));
        assert!(check_path_text(Path::new("/tmp/klip.mp4")).is_ok());
    }

    #[test]
    fn panics_become_errors() {
        let r: Result<(), Error> = guarded(|| std::panic::panic_any("boom"));
        assert_eq!(r, Err(Error::Internal("boom".into())));
    }

    #[test]
    fn enabling_is_a_runtime_switch() {
        set_enabled(false);
        assert!(!enabled());
        if compiled_in() {
            assert_eq!(open(Path::new("/nonexistent")).err(), Some(Error::Disabled));
        }
        set_enabled(true);
        assert!(enabled());
    }

    #[test]
    fn the_report_explains_itself() {
        register_status();
        let r = cfa4_media::backend_reports().into_iter().find(|r| r.id == ROUTE_ID).unwrap_or_default();
        assert_eq!(r.name, "FFmpeg");
        assert_eq!(r.compiled_in, compiled_in());
        assert_eq!(r.restricted_package.as_deref(), Some(RESTRICTED_PACKAGE));
        assert_eq!(r.restricted.len(), RESTRICTED.len());
        if !r.available {
            assert!(r.problem.is_some(), "an unavailable backend says why");
        }
    }

    #[test]
    fn extra_extensions_are_lower_case_without_dots_and_unique() {
        for list in [EXTRA_VIDEO_EXTENSIONS, EXTRA_AUDIO_EXTENSIONS] {
            let mut v: Vec<_> = list.to_vec();
            for e in &v {
                assert!(!e.is_empty() && !e.contains('.') && e.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{e}");
            }
            v.sort_unstable();
            v.dedup();
            assert_eq!(v.len(), list.len());
        }
    }

    #[cfg(not(feature = "ffmpeg"))]
    #[test]
    fn without_the_feature_everything_is_unavailable() {
        assert!(!compiled_in());
        assert!(matches!(probe(), Err(Error::Unavailable(_))));
        assert!(matches!(register(), Err(Error::Unavailable(_))));
        assert!(matches!(discover(Path::new("/nonexistent")), Err(Error::Unavailable(_))));
        assert!(!registered());
    }
}
