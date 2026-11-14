//! Live studio core (layer L5, CFA4-LIVE-STUDIO-001 phases S0–S1).
//!
//! The live studio composites scenes in real time. For now the work is done by **libobs** (the
//! OBS Studio core, C, GPL-2.0-or-later) and later, part by part, by CFA4's own Rust code
//! (plan §I.3, S12+). Both sit behind one trait, [`StudioBackend`], so a replacement changes
//! the implementation and not the callers.
//!
//! - [`StudioBackend`]: what a compositor does (video settings, a scene, frames out).
//! - [`scene`]: the scene the engine sends (layers: a picture and its placement) and the software
//!   compositor of it.
//! - [`software::SoftwareBackend`]: the pure-Rust backend: always available, and the fallback
//!   when libobs is missing or cannot start (no display, no GPU, wrong version).
//! - [`libobs::LibObsBackend`]: the binding of the pinned CFA4 libobs build
//!   (`scripts/dev/build-libobs-33.0.1.sh`), loaded at run time.
//! - [`ring`]: the shared-memory frame ring the host writes and the engine (and later the Qt6
//!   GUI) reads; a torn frame is dropped, never shown.
//! - [`host`]: the supervisor of the `cfa4-studio-host` process. libobs never runs inside the
//!   editor or the engine (plan D2): the engine starts the host, watches it and restarts it after
//!   a crash, and the editor carries on.
//!
//! `unsafe` is confined to the `libobs::sys` module (ADR 0001 addendum, 2026-11-07); everything else is
//! safe code and every public function returns a `Result`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod host;
pub mod libobs;
pub mod ring;
pub mod scene;
pub mod software;

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The libobs release the CFA4 build pins (plan D1).
pub const LIBOBS_VERSION: &str = "33.0.1";

/// Where `scripts/dev/build-libobs-33.0.1.sh` installs it unless `CFA4_LIBOBS_DIR` says otherwise.
pub const LIBOBS_DEFAULT_DIR: &str = "/opt/cfa4/libobs-33.0.1";

/// Largest frame side the studio accepts (8K).
pub const MAX_SIDE: u32 = 8192;
/// Smallest frame side.
pub const MIN_SIDE: u32 = 16;
/// Highest frame rate the studio accepts.
pub const MAX_FPS: u32 = 240;

/// Errors of the live studio. Every failure is one of these; nothing in this crate panics.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    /// A backend cannot run on this machine (library missing, no display, wrong version).
    #[error("studio backend unavailable: {0}")]
    Unavailable(String),
    /// A parameter is out of range.
    #[error("invalid parameter: {0}")]
    Invalid(String),
    /// The backend failed while running.
    #[error("studio backend failed: {0}")]
    Backend(String),
    /// The frame ring could not be read or written.
    #[error("frame ring: {0}")]
    Ring(String),
    /// The host process could not be started or controlled.
    #[error("studio host: {0}")]
    Host(String),
    /// I/O error with context.
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
}

/// Result alias of this crate.
pub type Result<T> = std::result::Result<T, StudioError>;

/// Pixel layout of a studio frame. S0 uses one: 8-bit BGRA, as libobs outputs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixelFormat {
    /// 8-bit B, G, R, A per pixel, rows `stride` bytes apart.
    Bgra8,
}

impl PixelFormat {
    /// Code stored in the frame ring.
    pub fn code(self) -> u32 {
        match self {
            PixelFormat::Bgra8 => 1,
        }
    }

    /// Format from a ring code.
    pub fn from_code(code: u32) -> Option<Self> {
        (code == 1).then_some(PixelFormat::Bgra8)
    }

    /// Bytes per pixel.
    pub fn bytes_per_pixel(self) -> u32 {
        4
    }
}

/// Output video of the studio: the canvas size and the frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSettings {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps_num: 60, fps_den: 1 }
    }
}

impl VideoSettings {
    /// Checks the settings: sides in `MIN_SIDE..=MAX_SIDE` and even (4:2:0 outputs need it), a
    /// frame rate above 0 and at most `MAX_FPS`.
    pub fn validate(&self) -> Result<()> {
        for (name, v) in [("width", self.width), ("height", self.height)] {
            if !(MIN_SIDE..=MAX_SIDE).contains(&v) || v % 2 != 0 {
                return Err(StudioError::Invalid(format!("{name} must be an even number in {MIN_SIDE}..={MAX_SIDE}, got {v}")));
            }
        }
        if self.fps_num == 0 || self.fps_den == 0 {
            return Err(StudioError::Invalid("the frame rate must be above 0".into()));
        }
        if self.fps_num > MAX_FPS.saturating_mul(self.fps_den) {
            return Err(StudioError::Invalid(format!("the frame rate must be at most {MAX_FPS} fps")));
        }
        Ok(())
    }

    /// Bytes per row of a BGRA frame.
    pub fn stride(&self) -> u32 {
        self.width.saturating_mul(PixelFormat::Bgra8.bytes_per_pixel())
    }

    /// Bytes of one BGRA frame.
    pub fn frame_len(&self) -> usize {
        (self.stride() as usize).saturating_mul(self.height as usize)
    }

    /// Time between two frames.
    pub fn frame_interval(&self) -> Duration {
        if self.fps_num == 0 {
            return Duration::from_secs(1);
        }
        Duration::from_nanos(1_000_000_000u64.saturating_mul(u64::from(self.fps_den)) / u64::from(self.fps_num))
    }
}

/// One output frame of the studio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per row in `data`.
    pub stride: u32,
    /// Pixel layout.
    pub format: PixelFormat,
    /// Presentation time from the backend's clock, in nanoseconds.
    pub pts_ns: u64,
    /// Pixel rows, `stride * height` bytes.
    pub data: Vec<u8>,
}

/// Which backend the host should use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendChoice {
    /// libobs when it can start, else the software backend (the reason is reported).
    #[default]
    Auto,
    /// libobs or an error.
    Libobs,
    /// The pure-Rust software backend.
    Software,
}

impl BackendChoice {
    /// Parses `auto`, `libobs` or `software`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(Self::Auto),
            "libobs" => Ok(Self::Libobs),
            "software" => Ok(Self::Software),
            other => Err(StudioError::Invalid(format!("unknown backend `{other}` (auto, libobs or software)"))),
        }
    }

    /// The command-line spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Libobs => "libobs",
            Self::Software => "software",
        }
    }
}

/// A real-time compositor. Implemented by libobs today and by CFA4's own Rust code part by part
/// later (plan S12+); callers see only this trait.
pub trait StudioBackend: Send {
    /// Short name (`libobs`, `software`).
    fn name(&self) -> &'static str;
    /// Version of the implementation (the libobs version string, or the CFA4 version).
    fn version(&self) -> String;
    /// The video the backend produces.
    fn video(&self) -> VideoSettings;
    /// Shows an empty scene on the program output.
    fn show_empty_scene(&mut self) -> Result<()>;
    /// Shows `scene` on the program output instead of the one shown (S1). On an error the
    /// shown scene stays.
    fn set_scene(&mut self, scene: &scene::LoadedScene) -> Result<()>;
    /// Waits up to `timeout` for the next program frame. `Ok(None)` when none arrived in time.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>>;
    /// Stops the backend and frees what it holds. Called once; also done on drop.
    fn shutdown(&mut self);
}

/// An opened backend and, when `Auto` fell back to software, the reason.
pub struct OpenedBackend {
    /// The backend.
    pub backend: Box<dyn StudioBackend>,
    /// Why libobs was not used, when `Auto` fell back.
    pub fallback_reason: Option<String>,
}

/// Opens the backend `choice` names for `video`, with libobs from `libobs_dir`.
pub fn open_backend(choice: BackendChoice, video: VideoSettings, libobs_dir: &std::path::Path) -> Result<OpenedBackend> {
    video.validate()?;
    match choice {
        BackendChoice::Software => Ok(OpenedBackend { backend: Box::new(software::SoftwareBackend::new(video)?), fallback_reason: None }),
        BackendChoice::Libobs => Ok(OpenedBackend { backend: Box::new(libobs::LibObsBackend::open(libobs_dir, video)?), fallback_reason: None }),
        BackendChoice::Auto => match libobs::LibObsBackend::open(libobs_dir, video) {
            Ok(b) => Ok(OpenedBackend { backend: Box::new(b), fallback_reason: None }),
            Err(e) => {
                log::warn!("live studio: libobs unavailable, using the software backend: {e}");
                Ok(OpenedBackend { backend: Box::new(software::SoftwareBackend::new(video)?), fallback_reason: Some(e.to_string()) })
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_settings_are_checked() {
        assert!(VideoSettings::default().validate().is_ok());
        let bad = [
            VideoSettings { width: 0, ..Default::default() },
            VideoSettings { width: 1921, ..Default::default() },
            VideoSettings { height: MAX_SIDE + 2, ..Default::default() },
            VideoSettings { fps_num: 0, ..Default::default() },
            VideoSettings { fps_den: 0, ..Default::default() },
            VideoSettings { fps_num: 1000, fps_den: 1, ..Default::default() },
            VideoSettings { fps_num: u32::MAX, fps_den: u32::MAX, width: u32::MAX, height: u32::MAX },
        ];
        for v in bad {
            assert!(v.validate().is_err(), "{v:?}");
        }
        let ntsc = VideoSettings { fps_num: 60000, fps_den: 1001, ..Default::default() };
        assert!(ntsc.validate().is_ok());
        assert_eq!(ntsc.frame_interval(), Duration::from_nanos(16_683_333));
        assert_eq!(VideoSettings::default().frame_len(), 1920 * 1080 * 4);
    }

    #[test]
    fn backend_choice_round_trips() {
        for c in [BackendChoice::Auto, BackendChoice::Libobs, BackendChoice::Software] {
            assert_eq!(BackendChoice::parse(c.as_str()).ok(), Some(c));
        }
        assert!(BackendChoice::parse("ndi").is_err());
    }

    #[test]
    fn auto_falls_back_to_software_without_libobs() {
        let dir = std::path::Path::new("/nonexistent/cfa4-libobs");
        let opened = open_backend(BackendChoice::Auto, VideoSettings { width: 64, height: 36, ..Default::default() }, dir).expect("fallback");
        assert_eq!(opened.backend.name(), "software");
        assert!(opened.fallback_reason.is_some());
        assert!(open_backend(BackendChoice::Libobs, VideoSettings::default(), dir).is_err());
    }
}
