//! Moving pictures for the live studio host (CFA4-LIVE-STUDIO-001 S1b).
//!
//! A media source and the live graphics source change every frame, so the host cannot get them
//! as one picture file. Each gets a frame ring of its own (`cfa4_obs::ring::create_sized`, a
//! private file in the layer directory) that this side keeps writing straight RGBA frames to;
//! the host shows the newest frame of every feed before each program frame
//! (`cfa4_obs::scene::Feeds`).
//!
//! - [`MediaFeed`]: a thread plays a video file with CFA4's decoders (the media pool's opener) on
//!   its own clock, from the start again at the end when looping, and drops frames when decoding
//!   falls behind instead of slowing down.
//! - [`LiveFeed`]: the C10 live graphics program, drawn on the engine's thread by the serve loop
//!   (`cfa4_engine::live::program_picture`), one frame per program frame; transparent while
//!   nothing is on air.
//!
//! A feed's thread never takes the engine down: a panic in a decoder ends that feed only.

use cfa4_media::{FrameRequest, SharedSource};
use cfa4_obs::ring::{self, RingWriter};
use cfa4_obs::{Frame, PixelFormat};
use cfa4_time::Tick;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A frame of `width × height` straight RGBA pixels for a feed ring.
fn rgba_frame(width: u32, height: u32, rgba: Vec<u8>, pts_ns: u64) -> Frame {
    Frame { width, height, stride: width * 4, format: PixelFormat::Rgba8, pts_ns, data: rgba }
}

fn checked_side(what: &str, w: u32, h: u32) -> Result<(), String> {
    let ok = |v: u32| (cfa4_obs::MIN_SIDE..=cfa4_obs::MAX_SIDE).contains(&v);
    if ok(w) && ok(h) { Ok(()) } else { Err(format!("{what}: {w}x{h} is outside {}..={} pixels", cfa4_obs::MIN_SIDE, cfa4_obs::MAX_SIDE)) }
}

/// A video file playing into a frame ring.
pub struct MediaFeed {
    pub ring: PathBuf,
    pub width: u32,
    pub height: u32,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MediaFeed {
    /// Starts playing `source` (its first video stream) into a new ring in `dir`.
    pub fn start(dir: &Path, name: &str, source: SharedSource, looping: bool) -> Result<Self, String> {
        let info = source.info();
        let v = info.video.as_ref().ok_or_else(|| format!("{name} has no video"))?;
        let (width, height) = (v.width, v.height);
        checked_side(name, width, height)?;
        let rate = v.frame_rate;
        let duration = info.duration;
        let ring = ring::create_sized(dir, width, height).map_err(|e| e.to_string())?;
        let mut writer = match RingWriter::open(&ring, 1) {
            Ok(w) => w,
            Err(e) => {
                ring::remove(&ring);
                return Err(e.to_string());
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let label = name.to_owned();
        let thread = std::thread::Builder::new().name("cfa4-studio-media".into()).spawn(move || {
            let run = std::panic::AssertUnwindSafe(|| play(&*source, &mut writer, (width, height), rate, duration, looping, &stop2));
            if std::panic::catch_unwind(run).is_err() {
                log::warn!("live studio: playing {label} failed; the source stops");
            }
        });
        let thread = match thread {
            Ok(t) => t,
            Err(e) => {
                ring::remove(&ring);
                return Err(format!("starting the player: {e}"));
            }
        };
        Ok(Self { ring, width, height, stop, thread: Some(thread) })
    }
}

impl Drop for MediaFeed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        ring::remove(&self.ring);
    }
}

/// The player: the frame due on the wall clock, decoded and written; frames that are late are
/// skipped. Ends when `stop` is set (or, without looping, holds the last frame).
fn play(
    source: &dyn cfa4_media::MediaSource,
    writer: &mut RingWriter,
    (width, height): (u32, u32),
    rate: cfa4_time::FrameRate,
    duration: Tick,
    looping: bool,
    stop: &AtomicBool,
) {
    let fd = rate.frame_duration().0.max(1);
    let frames = (duration.0 / fd).max(1);
    let start = Instant::now();
    let mut shown: Option<i64> = None;
    let mut failures = 0u32;
    while !stop.load(Ordering::SeqCst) {
        let elapsed = Tick(i64::try_from(start.elapsed().as_nanos() * cfa4_time::TICKS_PER_SECOND as u128 / 1_000_000_000).unwrap_or(i64::MAX));
        let n = rate.frame_at(elapsed);
        let index = if looping { n.rem_euclid(frames) } else { n.min(frames - 1) };
        if shown != Some(index) {
            let t = rate.tick_of(index);
            match source.video_frame(FrameRequest::full(t)) {
                Ok(f) if f.width == width && f.height == height => {
                    let rgba = f.to_rgba8();
                    if rgba.len() == width as usize * height as usize * 4 {
                        let pts = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        if writer.write(&rgba_frame(width, height, rgba, pts)).is_err() {
                            return;
                        }
                    }
                    failures = 0;
                }
                Ok(f) => log::debug!("live studio: a {}x{} frame in a {width}x{height} video, skipped", f.width, f.height),
                Err(e) => {
                    failures += 1;
                    if failures == 1 || failures.is_multiple_of(100) {
                        log::warn!("live studio: decoding frame {index}: {e}");
                    }
                }
            }
            shown = Some(index);
        }
        // until the next frame is due (short sleeps: `stop` is seen quickly)
        let next = rate.tick_of(n + 1);
        let now = Tick(i64::try_from(start.elapsed().as_nanos() * cfa4_time::TICKS_PER_SECOND as u128 / 1_000_000_000).unwrap_or(i64::MAX));
        let wait_ns = (next.0 - now.0).max(0) as u128 * 1_000_000_000 / cfa4_time::TICKS_PER_SECOND as u128;
        std::thread::sleep(Duration::from_nanos(u64::try_from(wait_ns).unwrap_or(0).clamp(1_000_000, 50_000_000)));
    }
}

/// The live graphics program feeding a frame ring, drawn on the engine's thread.
pub struct LiveFeed {
    pub ring: PathBuf,
    pub width: u32,
    pub height: u32,
    writer: RingWriter,
    interval: Duration,
    last: Option<Instant>,
    /// A transparent frame was written last (nothing on air): not repeated.
    blank: bool,
    start: Instant,
}

impl LiveFeed {
    /// A ring in `dir` for program pictures of `width × height`, written at most once per
    /// `interval` (the host's frame interval).
    pub fn new(dir: &Path, width: u32, height: u32, interval: Duration) -> Result<Self, String> {
        checked_side("live graphics", width, height)?;
        let ring = ring::create_sized(dir, width, height).map_err(|e| e.to_string())?;
        let writer = match RingWriter::open(&ring, 1) {
            Ok(w) => w,
            Err(e) => {
                ring::remove(&ring);
                return Err(e.to_string());
            }
        };
        Ok(Self { ring, width, height, writer, interval, last: None, blank: false, start: Instant::now() })
    }

    /// Whether a frame is due.
    pub fn due(&self) -> bool {
        self.last.is_none_or(|t| t.elapsed() >= self.interval)
    }

    /// Writes the program picture (`None`: nothing on air, a transparent frame once). A picture
    /// of another size is refused (`Err`): the program changed size and the feed is rebuilt.
    pub fn write(&mut self, picture: Option<(Vec<u8>, u32, u32)>) -> Result<(), String> {
        self.last = Some(Instant::now());
        let pts = u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let (rgba, blank) = match picture {
            Some((rgba, w, h)) => {
                if (w, h) != (self.width, self.height) || rgba.len() != w as usize * h as usize * 4 {
                    return Err(format!("the program is {w}x{h} now, the feed {}x{}", self.width, self.height));
                }
                (rgba, false)
            }
            None if self.blank => return Ok(()),
            None => (vec![0u8; self.width as usize * self.height as usize * 4], true),
        };
        self.writer.write(&rgba_frame(self.width, self.height, rgba, pts)).map_err(|e| e.to_string())?;
        self.blank = blank;
        Ok(())
    }
}

impl Drop for LiveFeed {
    fn drop(&mut self) {
        ring::remove(&self.ring);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_obs::ring::RingReader;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfa4-feeds-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).expect("dir");
        d
    }

    #[test]
    fn the_live_feed_writes_pictures_and_one_blank() {
        let d = dir("live");
        let mut f = LiveFeed::new(&d, 20, 16, Duration::from_millis(10)).expect("feed");
        let r = RingReader::open(&f.ring).expect("reader");
        assert!(f.due());
        f.write(None).expect("blank");
        f.write(None).expect("blank again");
        assert_eq!(r.latest_seq().expect("seq"), 1, "a blank frame is written once");
        f.write(Some((vec![7; 20 * 16 * 4], 20, 16))).expect("picture");
        let got = r.latest().expect("read").expect("frame");
        assert_eq!((got.seq, got.frame.format, got.frame.data[0]), (2, PixelFormat::Rgba8, 7));
        assert!(f.write(Some((vec![0; 30 * 16 * 4], 30, 16))).is_err(), "another size");
        assert!(LiveFeed::new(&d, 4, 16, Duration::ZERO).is_err());
        let ring = f.ring.clone();
        drop(f);
        assert!(!ring.exists(), "the ring goes with the feed");
        let _ = std::fs::remove_dir_all(&d);
    }
}
