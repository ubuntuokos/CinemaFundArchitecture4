//! The pure-Rust backend: always available, and the fallback when libobs cannot start.
//!
//! In S0 it composites what S0 has, the empty scene: opaque black frames at the configured
//! frame rate, paced by the monotonic clock. Later phases grow it into the Rust replacement of
//! libobs (plan S12+), behind the same [`StudioBackend`] trait.

use crate::{Frame, PixelFormat, Result, StudioBackend, VideoSettings};
use std::time::{Duration, Instant};

/// Software compositor of the empty scene.
pub struct SoftwareBackend {
    video: VideoSettings,
    start: Instant,
    next: u64,
    empty: Vec<u8>,
}

impl SoftwareBackend {
    /// A backend producing `video`.
    pub fn new(video: VideoSettings) -> Result<Self> {
        video.validate()?;
        let mut empty = vec![0u8; video.frame_len()];
        // opaque black: alpha 255 in every BGRA pixel
        for px in empty.chunks_exact_mut(4) {
            if let Some(a) = px.get_mut(3) {
                *a = 255;
            }
        }
        Ok(Self { video, start: Instant::now(), next: 0, empty })
    }

    fn due(&self, index: u64) -> Duration {
        let ns = u128::from(index) * 1_000_000_000u128 * u128::from(self.video.fps_den) / u128::from(self.video.fps_num.max(1));
        Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
    }
}

impl StudioBackend for SoftwareBackend {
    fn name(&self) -> &'static str {
        "software"
    }

    fn version(&self) -> String {
        format!("CFA4 {}", env!("CARGO_PKG_VERSION"))
    }

    fn video(&self) -> VideoSettings {
        self.video
    }

    fn show_empty_scene(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        let due = self.due(self.next);
        let now = self.start.elapsed();
        if due > now {
            let wait = due - now;
            if wait > timeout {
                std::thread::sleep(timeout);
                return Ok(None);
            }
            std::thread::sleep(wait);
        } else if now.saturating_sub(due) > Duration::from_secs(1) {
            // far behind (the process was stopped): resume from now instead of bursting
            let interval = self.video.frame_interval().as_nanos().max(1);
            self.next = u64::try_from(now.as_nanos() / interval).unwrap_or(self.next);
        }
        let pts_ns = u64::try_from(self.due(self.next).as_nanos()).unwrap_or(u64::MAX);
        self.next = self.next.saturating_add(1);
        Ok(Some(Frame {
            width: self.video.width,
            height: self.video.height,
            stride: self.video.stride(),
            format: PixelFormat::Bgra8,
            pts_ns,
            data: self.empty.clone(),
        }))
    }

    fn shutdown(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scene_frames_are_opaque_black_and_paced() {
        let video = VideoSettings { width: 32, height: 18, fps_num: 100, fps_den: 1 };
        let mut b = SoftwareBackend::new(video).expect("backend");
        b.show_empty_scene().expect("scene");
        let t = Instant::now();
        let mut last = None;
        for _ in 0..5 {
            let f = b.next_frame(Duration::from_secs(1)).expect("ok").expect("frame");
            assert_eq!((f.width, f.height, f.stride), (32, 18, 128));
            assert_eq!(f.data.len(), 32 * 18 * 4);
            assert!(f.data.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
            if let Some(prev) = last {
                assert!(f.pts_ns > prev);
            }
            last = Some(f.pts_ns);
        }
        assert!(t.elapsed() >= Duration::from_millis(35), "paced at 100 fps");
    }

    #[test]
    fn a_short_timeout_returns_none() {
        let video = VideoSettings { width: 16, height: 16, fps_num: 1, fps_den: 1 };
        let mut b = SoftwareBackend::new(video).expect("backend");
        assert!(b.next_frame(Duration::from_millis(1)).expect("ok").is_some(), "frame 0 is due at once");
        assert!(b.next_frame(Duration::from_millis(1)).expect("ok").is_none());
    }
}
