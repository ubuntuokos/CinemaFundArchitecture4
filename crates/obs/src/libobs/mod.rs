//! The libobs backend: the pinned CFA4 libobs 33.0.1 build behind [`StudioBackend`].
//!
//! libobs is loaded at run time from `<dir>/lib/libobs.so.30` (libobs keeps the ABI soname of
//! OBS 30), where `<dir>` is `CFA4_LIBOBS_DIR` or `/opt/cfa4/libobs-33.0.1`. Only that build is
//! used, never the system's libobs (Ubuntu ships an older one), and a version other than
//! [`crate::LIBOBS_VERSION`] is refused.
//!
//! libobs renders with OpenGL through EGL, so it needs a display connection (X11 or Wayland);
//! without one, or without the library, [`LibObsBackend::open`] returns
//! [`StudioError::Unavailable`] and `BackendChoice::Auto` uses the software backend.
//!
//! This backend runs only inside the `cfa4-studio-host` process (plan D2): a libobs crash ends
//! that process, the engine's supervisor restarts it, and the editor carries on.

pub mod ffi;
mod sys;

use crate::{Frame, Result, StudioBackend, StudioError, VideoSettings};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The libobs soname the build installs.
pub const LIBOBS_SONAME: &str = "libobs.so.30";

/// The directory of the CFA4 libobs build: `CFA4_LIBOBS_DIR` or the default prefix.
pub fn default_dir() -> PathBuf {
    std::env::var_os("CFA4_LIBOBS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(crate::LIBOBS_DEFAULT_DIR))
}

/// Path of the libobs library in `dir`.
pub fn library_path(dir: &Path) -> PathBuf {
    dir.join("lib").join(LIBOBS_SONAME)
}

/// Whether the libobs build in `dir` is the pinned version, without starting it.
pub fn probe(dir: &Path) -> Result<String> {
    let api = sys::Api::load(&library_path(dir))?;
    let v = api.version();
    check_version(&v)?;
    Ok(v)
}

fn check_version(v: &str) -> Result<()> {
    if v == crate::LIBOBS_VERSION {
        Ok(())
    } else {
        Err(StudioError::Unavailable(format!("libobs {v} found, CFA4 needs the pinned {} build", crate::LIBOBS_VERSION)))
    }
}

/// libobs, started, with an empty scene once [`StudioBackend::show_empty_scene`] ran.
pub struct LibObsBackend {
    video: VideoSettings,
    version: String,
    sink: Arc<sys::FrameSink>,
    raw: Option<sys::RawVideo>,
    scene: Option<sys::Scene>,
    started: bool,
    // field order matters for drop: `shutdown` runs first (Drop impl), then the display is
    // closed, then the library is unloaded.
    api: sys::Api,
    display: Option<sys::Display>,
}

impl LibObsBackend {
    /// Loads libobs from `dir`, opens the display, starts libobs and its video at `video`.
    pub fn open(dir: &Path, video: VideoSettings) -> Result<Self> {
        video.validate()?;
        let api = sys::Api::load(&library_path(dir))?;
        let version = api.version();
        check_version(&version)?;
        let display = sys::Display::open()?;
        api.set_platform(&display);
        let module = CString::new(dir.join("lib").join("libobs-opengl").to_string_lossy().into_owned())
            .map_err(|_| StudioError::Invalid("libobs directory contains NUL".into()))?;
        let mut me = Self {
            video,
            version,
            sink: Arc::new(sys::FrameSink::new(video.width, video.height)),
            raw: None,
            scene: None,
            started: false,
            api,
            display: Some(display),
        };
        me.api.startup()?;
        me.started = true;
        me.api.reset_video(&module, &video)?;
        me.raw = Some(me.api.add_raw_video(me.sink.clone()));
        log::info!(
            "live studio: libobs {} started ({}x{} @ {}/{}, {})",
            me.version,
            video.width,
            video.height,
            video.fps_num,
            video.fps_den,
            me.display.as_ref().map_or("?", sys::Display::kind)
        );
        Ok(me)
    }
}

impl StudioBackend for LibObsBackend {
    fn name(&self) -> &'static str {
        "libobs"
    }

    fn version(&self) -> String {
        self.version.clone()
    }

    fn video(&self) -> VideoSettings {
        self.video
    }

    fn show_empty_scene(&mut self) -> Result<()> {
        if let Some(old) = self.scene.take() {
            self.api.release_scene(old);
        }
        self.scene = Some(self.api.show_empty_scene("CFA4 empty scene")?);
        Ok(())
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        if self.raw.is_none() {
            return Err(StudioError::Backend("libobs video is not running".into()));
        }
        Ok(self.sink.take(timeout))
    }

    fn shutdown(&mut self) {
        if let Some(raw) = self.raw.take() {
            self.api.remove_raw_video(raw);
        }
        if let Some(scene) = self.scene.take() {
            self.api.release_scene(scene);
        }
        if self.started {
            self.api.shutdown();
            self.started = false;
        }
        // the display outlives libobs's use of it
        self.display = None;
    }
}

impl Drop for LibObsBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_build_is_unavailable_not_a_crash() {
        let dir = Path::new("/nonexistent/cfa4-libobs");
        assert!(matches!(probe(dir), Err(StudioError::Unavailable(_))));
        assert!(matches!(LibObsBackend::open(dir, VideoSettings::default()), Err(StudioError::Unavailable(_))));
    }

    #[test]
    fn only_the_pinned_version_is_accepted() {
        assert!(check_version(crate::LIBOBS_VERSION).is_ok());
        assert!(check_version("32.1.0").is_err());
        assert!(check_version("").is_err());
    }

    /// Runs when the CFA4 libobs build is installed: the library loads and reports the pinned
    /// version. (Starting libobs needs a display; the host's tests do that under Xvfb.)
    #[test]
    fn the_installed_build_reports_the_pinned_version() {
        let dir = default_dir();
        if !library_path(&dir).is_file() {
            eprintln!("skipped: no libobs build in {}", dir.display());
            return;
        }
        assert_eq!(probe(&dir).expect("probe"), crate::LIBOBS_VERSION);
    }
}
