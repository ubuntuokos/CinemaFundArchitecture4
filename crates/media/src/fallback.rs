//! Path-based fallback openers: another media backend (GStreamer, ADR 0002) that takes over when
//! the built-in openers cannot open a file, or open it without the video it carries.
//!
//! The built-in openers always go first. A fallback is tried only when
//!
//! - they reject the file as unsupported or undecodable ([`MediaError::Unsupported`] /
//!   [`MediaError::Decode`]); I/O errors and offline media are passed through unchanged; or
//! - they open a file with a video extension but find no video in it (an Ogg Theora + Vorbis file
//!   opened as audio only), and the fallback finds video.
//!
//! When every fallback fails, the built-in result is returned unchanged, so registering a fallback
//! never makes a file that opened before fail. Fallbacks are registered once at startup by the
//! application (layer L5/L6); nothing is registered by default.
//!
//! Their order comes from the compute router (ADR 0004 / ADR 0005): [`set_fallback_order`] takes
//! the supplementary backends of the router's `codec_decode` chain, after CFA4's own engine, and
//! a backend the chain leaves out (a user `routing.toml` without it) is not tried. Until an order
//! is set, the registration order applies.

use std::path::Path;
use std::sync::RwLock;

use crate::{AUDIO_EXTENSIONS, FrameRequest, MediaError, Result, STILL_EXTENSIONS, SharedSource, VIDEO_EXTENSIONS};

/// A fallback media backend.
#[derive(Clone, Copy, Debug)]
pub struct Fallback {
    /// The backend's id in the routing table and in Settings ▸ Media Backends (`"gstreamer"`).
    pub id: &'static str,
    /// Short name for logs and diagnostics (`"GStreamer"`).
    pub name: &'static str,
    /// Extra video file extensions (lower case, no dot) this backend makes importable.
    pub video_extensions: &'static [&'static str],
    /// Extra audio file extensions (lower case, no dot) this backend makes importable.
    pub audio_extensions: &'static [&'static str],
    /// Opens a regular file on the local file system.
    pub open: fn(&Path) -> Result<SharedSource>,
}

fn registry() -> &'static RwLock<Vec<Fallback>> {
    static R: RwLock<Vec<Fallback>> = RwLock::new(Vec::new());
    &R
}

/// Registers `f` after the fallbacks registered before it. Registering the same id twice keeps
/// the first registration.
pub fn register_fallback(f: Fallback) {
    let mut r = registry().write().unwrap_or_else(|e| e.into_inner());
    if !r.iter().any(|x| x.id == f.id) {
        r.push(f);
        drop(r);
        // the router ordered the fallbacks it knew: a new one would be left out until it is asked
        // again (the next Preferences apply), so the registration order applies until then
        set_fallback_order(None);
    }
}

fn order() -> &'static RwLock<Option<Vec<String>>> {
    static O: RwLock<Option<Vec<String>>> = RwLock::new(None);
    &O
}

/// The order the router chose for the fallbacks, by id: only these are tried, in this order.
/// `None` goes back to the registration order (tests, and before the router has run).
pub fn set_fallback_order(ids: Option<Vec<String>>) {
    *order().write().unwrap_or_else(|e| e.into_inner()) = ids;
}

/// The order set by [`set_fallback_order`], if any.
pub fn fallback_order() -> Option<Vec<String>> {
    order().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Every registered fallback, in registration order (for extensions and diagnostics).
pub fn registered_fallbacks() -> Vec<Fallback> {
    registry().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The fallbacks that are tried, in the order they are tried: the router's order when one is set
/// (backends it leaves out are skipped), else the registration order.
pub fn fallbacks() -> Vec<Fallback> {
    let all = registered_fallbacks();
    match fallback_order() {
        None => all,
        Some(ids) => ids.iter().filter_map(|id| all.iter().find(|f| f.id == id).copied()).collect(),
    }
}

fn merged(base: &'static [&'static str], extra: impl Fn(&Fallback) -> &'static [&'static str]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = base.to_vec();
    // a file stays importable while its backend is registered, whatever the order: when the
    // router leaves the backend out, the import reports why instead of hiding the file
    for f in registered_fallbacks() {
        for e in extra(&f) {
            if !v.contains(e) {
                v.push(e);
            }
        }
    }
    v
}

/// Video extensions: the built-in ones plus those of the registered fallbacks.
pub fn video_extensions() -> Vec<&'static str> {
    merged(VIDEO_EXTENSIONS, |f| f.video_extensions)
}

/// Audio extensions: the built-in ones plus those of the registered fallbacks.
pub fn audio_extensions() -> Vec<&'static str> {
    merged(AUDIO_EXTENSIONS, |f| f.audio_extensions)
}

/// Every importable extension: video, audio and still images.
pub fn importable_extensions() -> Vec<&'static str> {
    let mut v = video_extensions();
    for e in audio_extensions().into_iter().chain(STILL_EXTENSIONS.iter().copied()) {
        if !v.contains(&e) {
            v.push(e);
        }
    }
    v
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Applies the registered fallbacks to the built-in openers' result `native` for the local file
/// `path` (see the module documentation for when they are tried).
pub fn open_with_fallback(path: &Path, native: Result<SharedSource>) -> Result<SharedSource> {
    let fallbacks = fallbacks();
    if fallbacks.is_empty() || !path.is_file() {
        return native;
    }
    match native {
        Ok(src) => {
            if src.info().has_video() {
                // The built-in openers can read a container whose video codec they have no decoder
                // for (MPEG-4 Part 2 in AVI): the first frame tells, and then a fallback that
                // decodes it takes the file. A damaged file (a decode error) stays with them.
                if let Err(MediaError::Unsupported(why)) = src.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)) {
                    for f in &fallbacks {
                        match (f.open)(path) {
                            Ok(alt) if alt.info().has_video() => {
                                log::info!("{}: opened by {} (the built-in decoders: {why})", path.display(), f.name);
                                return Ok(alt);
                            }
                            Ok(_) => {}
                            Err(e) => log::debug!("{}: {} fallback declined: {e}", path.display(), f.name),
                        }
                    }
                }
                return Ok(src);
            }
            if !video_extensions().contains(&extension(path).as_str()) {
                return Ok(src);
            }
            for f in &fallbacks {
                match (f.open)(path) {
                    Ok(alt) if alt.info().has_video() => {
                        log::info!("{}: opened by {} (the built-in openers found no video)", path.display(), f.name);
                        return Ok(alt);
                    }
                    Ok(_) => {}
                    Err(e) => log::debug!("{}: {} fallback declined: {e}", path.display(), f.name),
                }
            }
            Ok(src)
        }
        Err(native_err @ (MediaError::Unsupported(_) | MediaError::Decode(_))) => {
            for f in &fallbacks {
                match (f.open)(path) {
                    Ok(alt) => {
                        log::info!("{}: opened by {} ({native_err})", path.display(), f.name);
                        return Ok(alt);
                    }
                    Err(e) => log::debug!("{}: {} fallback declined: {e}", path.display(), f.name),
                }
            }
            Err(native_err)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{AudioStreamInfo, FrameRequest, MediaInfo, MediaKind, OfflineSource, VideoStreamInfo};

    // The registry is process-wide, so every test here registers the same fallback and only
    // checks behaviour that holds once it is registered.
    fn info(video: bool) -> MediaInfo {
        MediaInfo {
            name: "test".into(),
            kind: if video { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration: cfa4_time::Tick::from_units(1, 1),
            video: video.then(|| VideoStreamInfo {
                width: 16,
                height: 16,
                frame_rate: cfa4_time::FrameRate::FPS_25,
                par: (1, 1),
                codec: "test".into(),
                pixel_format: "RGBA".into(),
                color: cfa4_color::ColorInfo::SRGB_FULL,
                has_alpha: false,
                bitrate: None,
                hdr: None,
            }),
            audio_streams: vec![AudioStreamInfo { sample_rate: 48_000, channels: 2, codec: "test".into(), bits_per_sample: None }],
            container: "test".into(),
            start_timecode: None,
            file_size: None,
        }
    }

    fn src(video: bool) -> SharedSource {
        Arc::new(OfflineSource { info: info(video) })
    }

    /// Opens files whose name contains "ok" (with video), declines everything else.
    fn test_open(path: &Path) -> Result<SharedSource> {
        if path.to_string_lossy().contains("ok") { Ok(src(true)) } else { Err(MediaError::Unsupported("test fallback".into())) }
    }

    /// The registry and the order are process-wide: tests that register or open hold this lock,
    /// so one test's order does not change another's result.
    fn register() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        register_fallback(Fallback { id: "test", name: "test", video_extensions: &["zzv"], audio_extensions: &["zza"], open: test_open });
        set_fallback_order(None);
        guard
    }

    fn file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cfa4-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p
    }

    #[test]
    fn extensions_include_registered_ones_once() {
        let _g = register();
        drop(_g);
        let _g = register();
        assert_eq!(video_extensions().iter().filter(|e| **e == "zzv").count(), 1);
        assert!(audio_extensions().contains(&"zza"));
        assert!(importable_extensions().contains(&"zzv") && importable_extensions().contains(&"png"));
        assert!(crate::is_importable(Path::new("/a/b.ZZV")));
        assert_eq!(fallbacks().iter().filter(|f| f.name == "test").count(), 1);
    }

    #[test]
    fn unsupported_files_go_to_the_fallback() {
        let _g = register();
        let p = file("ok.zzv");
        let opened = open_with_fallback(&p, Err(MediaError::Unsupported("native".into()))).unwrap();
        assert!(opened.info().has_video());
    }

    #[test]
    fn native_error_is_kept_when_the_fallback_declines() {
        let _g = register();
        let p = file("nope.zzv");
        let err = open_with_fallback(&p, Err(MediaError::Unsupported("native".into()))).err().unwrap();
        assert!(err.to_string().contains("native"), "{err}");
    }

    #[test]
    fn io_and_offline_errors_are_not_retried() {
        let _g = register();
        let p = file("ok.mkv");
        assert!(matches!(open_with_fallback(&p, Err(MediaError::Io("disk".into()))), Err(MediaError::Io(_))));
        assert!(matches!(open_with_fallback(&p, Err(MediaError::Offline("gone".into()))), Err(MediaError::Offline(_))));
    }

    #[test]
    fn audio_only_result_of_a_video_file_is_upgraded() {
        let _g = register();
        let p = file("ok.mkv");
        let opened = open_with_fallback(&p, Ok(src(false))).unwrap();
        assert!(opened.info().has_video());
        // An audio file stays with the built-in opener.
        let a = file("ok.wav");
        assert!(!open_with_fallback(&a, Ok(src(false))).unwrap().info().has_video());
    }

    #[test]
    fn audio_only_result_is_kept_when_the_fallback_finds_no_video() {
        let _g = register();
        let p = file("nope.mkv");
        let opened = open_with_fallback(&p, Ok(src(false))).unwrap();
        assert!(!opened.info().has_video());
        assert!(opened.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)).is_err());
    }

    /// Built-in openers that read the container but have no decoder for its video codec.
    struct NoVideoDecoder(MediaInfo);

    impl crate::MediaSource for NoVideoDecoder {
        fn info(&self) -> &MediaInfo {
            &self.0
        }
        fn video_frame(&self, _req: FrameRequest) -> Result<Arc<crate::VideoFrame>> {
            Err(MediaError::Unsupported("no decoder for MPEG-4 Part 2 (DivX / Xvid) video".into()))
        }
        fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> Result<crate::AudioBuffer> {
            Err(MediaError::NoStream("audio"))
        }
    }

    #[test]
    fn a_video_codec_without_a_built_in_decoder_goes_to_the_fallback() {
        let _g = register();
        let p = file("ok.avi");
        let native: SharedSource = Arc::new(NoVideoDecoder(info(true)));
        let opened = open_with_fallback(&p, Ok(native)).unwrap();
        // the fallback's source (OfflineSource) draws frames; the native one could not
        assert!(opened.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)).is_ok());
        // when no fallback decodes it either, the built-in source and its message stay
        let q = file("nope.avi");
        let kept = open_with_fallback(&q, Ok(Arc::new(NoVideoDecoder(info(true))) as SharedSource)).unwrap();
        assert!(matches!(kept.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)), Err(MediaError::Unsupported(_))));
        // a decodable built-in source is never replaced
        let fine = open_with_fallback(&p, Ok(src(true))).unwrap();
        assert!(fine.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)).is_ok());
    }

    #[test]
    fn the_router_order_selects_and_orders_the_fallbacks() {
        let _g = register();
        let p = file("ok.zzv");
        // an order without the backend: it is not tried, the built-in result stays
        set_fallback_order(Some(vec!["ffmpeg".into()]));
        assert!(fallbacks().iter().all(|f| f.id != "test"));
        assert!(open_with_fallback(&p, Err(MediaError::Unsupported("native".into()))).is_err());
        // an order with it: tried again
        set_fallback_order(Some(vec!["gstreamer".into(), "test".into()]));
        assert_eq!(fallbacks().iter().map(|f| f.id).collect::<Vec<_>>(), vec!["test"]);
        assert!(open_with_fallback(&p, Err(MediaError::Unsupported("native".into()))).is_ok());
        set_fallback_order(None);
        assert!(registered_fallbacks().iter().any(|f| f.id == "test"));
    }

    #[test]
    fn missing_paths_are_not_given_to_fallbacks() {
        let _g = register();
        let missing = Path::new("/nonexistent/cfa4/ok.zzv");
        assert!(matches!(open_with_fallback(missing, Err(MediaError::Unsupported("native".into()))), Err(MediaError::Unsupported(_))));
    }
}
