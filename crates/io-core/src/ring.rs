//! The frame ring: frames (and their sound) in shared memory, from a producer process to its
//! readers (moved here from `cfa4-obs` by CFA4-DECKLINK-READY-001 V2.1; layout version 2, V2.2).
//!
//! The supervisor creates the ring as a private (0600) file in `$XDG_RUNTIME_DIR` (a tmpfs, so
//! the file is shared memory), outside the producer, so it survives producer restarts. The
//! producer writes each frame into the next slot; the engine reads the newest one (and the Qt6
//! GUI will map the same file). Plain positioned reads and writes are used (no `mmap`), so the
//! Rust side needs no `unsafe`.
//!
//! Layout (little-endian), version 2:
//!
//! | where | bytes | field |
//! |---|---|---|
//! | header 0 | 8 | magic `CFA4RNG2` |
//! | 8 | 4 | `version` u32 = 2 |
//! | 12 | 4 | `slots` u32 |
//! | 16 | 8 | `slot_size` u64 |
//! | 24 | 8 | `latest` u64 (number of the newest published frame, 0 = none) |
//! | 32 | 4 | `writer_pid` u32 |
//! | 36 | 4 | `generation` u32 (producer start count) |
//! | 40..64 | 24 | reserved (zero) |
//! | slot `i` at `64 + i × slot_size`, slot header 0 | 8 | `seq` u64 (0 while being written) |
//! | 8 | 4 | `width` u32 |
//! | 12 | 4 | `height` u32 |
//! | 16 | 4 | `stride` u32 |
//! | 20 | 4 | `format` u32 ([`PixelFormat::code`]) |
//! | 24 | 8 | `pts_ns` u64 |
//! | 32 | 8 | `len` u64 (picture bytes) |
//! | 40 | 4 | `timecode` u32, SMPTE 12M BCD `HH MM SS FF` (0 = none) |
//! | 44 | 4 | `audio_len` u32 (audio bytes, 0 = none) |
//! | 48 | 2 | `audio_channels` u16 |
//! | 50 | 2 | `audio_format` u16 ([`AudioFormat::code`]) |
//! | 52 | 4 | `flags` u32 ([`FLAG_INTERLACED`], [`FLAG_BOTTOM_FIELD_FIRST`]; other bits reserved) |
//! | 56..64 | 8 | reserved (zero) |
//! | slot 64 | `len` | picture |
//! | 64 + `len` | `audio_len` | audio block ([`Audio`]) |
//!
//! The slot size is `64 +` the picture bytes of the ring's format `+` the audio bytes the ring
//! was created for. Frame `n` (counted from 1) goes to slot `n % slots`. The writer sets the
//! slot's `seq` to 0, writes picture and audio, then the slot header with `seq = n`, then
//! `latest = n`. A reader takes `latest`, reads that slot, and reads `seq` again afterwards: if
//! either is not `n` the writer went round the ring meanwhile and the frame is dropped (a torn
//! frame is never shown).
//!
//! Version 1 files (magic `CFA4LIV1`) are refused with the error `ring version 1`. A frame with
//! a pixel or audio format code this version does not know is dropped and counted
//! ([`RingReader::dropped`]), never an error and never a panic.

use crate::frame::{Audio, AudioFormat, Frame, MAX_AUDIO_CHANNELS, PixelFormat};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Errors of the ring. Nothing here panics.
#[derive(Debug, thiserror::Error)]
pub enum RingError {
    /// A parameter is out of range.
    #[error("invalid parameter: {0}")]
    Invalid(String),
    /// The ring could not be read or written, or is not a version 2 ring.
    #[error("frame ring: {0}")]
    Ring(String),
    /// I/O error with context.
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
}

/// Result alias of the ring.
pub type Result<T> = std::result::Result<T, RingError>;

/// File magic.
pub const MAGIC: &[u8; 8] = b"CFA4RNG2";
/// Magic of the version 1 layout, which is refused.
pub const MAGIC_V1: &[u8; 8] = b"CFA4LIV1";
/// Layout version.
pub const VERSION: u32 = 2;
/// Header bytes.
pub const HEADER: u64 = 64;
/// Slot header bytes.
pub const SLOT_HEADER: u64 = 64;
/// Slots per ring.
pub const SLOTS: u32 = 3;
/// Largest ring file (three 8K BGRA frames are 768 MiB; refuse beyond 1 GiB).
pub const MAX_FILE: u64 = 1 << 30;
/// Largest frame side.
pub const MAX_SIDE: u32 = 8192;
/// Smallest frame side.
pub const MIN_SIDE: u32 = 16;
/// `flags`: the picture is interlaced (two fields).
pub const FLAG_INTERLACED: u32 = 1;
/// `flags`: the bottom field comes first (only with [`FLAG_INTERLACED`]).
pub const FLAG_BOTTOM_FIELD_FIRST: u32 = 2;

fn io(what: &str) -> impl FnOnce(std::io::Error) -> RingError + '_ {
    move |e| RingError::Io(what.to_owned(), e)
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2).and_then(|s| s.try_into().ok()).map_or(0, u16::from_le_bytes)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).and_then(|s| s.try_into().ok()).map_or(0, u32::from_le_bytes)
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 8).and_then(|s| s.try_into().ok()).map_or(0, u64::from_le_bytes)
}

/// The directory rings are created in: `$XDG_RUNTIME_DIR`, else `/dev/shm`, else the temp dir.
pub fn default_dir() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir());
    runtime.or_else(|| Some(PathBuf::from("/dev/shm")).filter(|p| p.is_dir())).unwrap_or_else(std::env::temp_dir)
}

/// What travels with a picture besides its pixels: the time code, the flags and the audio.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    /// SMPTE 12M time code as BCD `0xHHMMSSFF` (0: none).
    pub timecode: u32,
    /// [`FLAG_INTERLACED`] and [`FLAG_BOTTOM_FIELD_FIRST`].
    pub flags: u32,
    /// The sound of this frame, if any.
    pub audio: Option<Audio>,
}

/// Geometry of a ring, from its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    slots: u32,
    slot_size: u64,
}

impl Geometry {
    fn for_format(width: u32, height: u32, format: PixelFormat, audio_bytes: u64) -> Result<Self> {
        if !(MIN_SIDE..=MAX_SIDE).contains(&width) || !(MIN_SIDE..=MAX_SIDE).contains(&height) {
            return Err(RingError::Invalid(format!("frames of {width}x{height} are outside {MIN_SIDE}..={MAX_SIDE}")));
        }
        if format.needs_even_width() && !width.is_multiple_of(2) {
            return Err(RingError::Invalid(format!("{format:?} frames need an even width, got {width}")));
        }
        let slot_size = SLOT_HEADER.saturating_add(format.bytes_per_frame(width, height)).saturating_add(audio_bytes);
        let total = HEADER.saturating_add(slot_size.saturating_mul(u64::from(SLOTS)));
        if total > MAX_FILE {
            return Err(RingError::Invalid(format!("frames of {width}x{height} do not fit a frame ring")));
        }
        Ok(Self { slots: SLOTS, slot_size })
    }

    fn file_len(&self) -> u64 {
        HEADER.saturating_add(self.slot_size.saturating_mul(u64::from(self.slots)))
    }

    fn slot_offset(&self, seq: u64) -> u64 {
        HEADER + (seq % u64::from(self.slots.max(1))) * self.slot_size
    }

    fn read(file: &File) -> Result<Self> {
        let mut h = [0u8; HEADER as usize];
        file.read_exact_at(&mut h, 0).map_err(io("reading the frame ring header"))?;
        if h.get(0..8) == Some(MAGIC_V1.as_slice()) {
            return Err(RingError::Ring("ring version 1".into()));
        }
        if h.get(0..8) != Some(MAGIC.as_slice()) || u32_at(&h, 8) != VERSION {
            return Err(RingError::Ring("not a CFA4 frame ring (magic or version)".into()));
        }
        let g = Self { slots: u32_at(&h, 12), slot_size: u64_at(&h, 16) };
        let len = file.metadata().map_err(io("reading the frame ring size"))?.len();
        if g.slots == 0 || g.slots > 16 || g.slot_size <= SLOT_HEADER || g.file_len() > MAX_FILE || len < g.file_len() {
            return Err(RingError::Ring("frame ring header is inconsistent".into()));
        }
        Ok(g)
    }
}

/// Creates a ring for frames of `width × height` pixels in `format`, with room for `audio_bytes`
/// of sound per frame (0: no sound), in `dir`: a new private file, never reused. Returns its path;
/// the caller removes the file when done ([`remove`]).
pub fn create_format(dir: &Path, width: u32, height: u32, format: PixelFormat, audio_bytes: u64) -> Result<PathBuf> {
    create_with(dir, Geometry::for_format(width, height, format, audio_bytes)?)
}

/// Creates a ring for frames of `width × height` 4-byte pixels (BGRA or RGBA; any sides in
/// `MIN_SIDE..=MAX_SIDE`, odd ones too), without sound.
pub fn create_sized(dir: &Path, width: u32, height: u32) -> Result<PathBuf> {
    create_format(dir, width, height, PixelFormat::Bgra8, 0)
}

fn create_with(dir: &Path, g: Geometry) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for attempt in 0..8 {
        let path = dir.join(format!("cfa4-live-{}-{stamp:x}-{attempt}", std::process::id()));
        match OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path) {
            Ok(file) => {
                let init = || -> Result<()> {
                    file.set_len(g.file_len()).map_err(io("sizing the frame ring"))?;
                    let mut h = [0u8; HEADER as usize];
                    h[0..8].copy_from_slice(MAGIC);
                    h[8..12].copy_from_slice(&VERSION.to_le_bytes());
                    h[12..16].copy_from_slice(&g.slots.to_le_bytes());
                    h[16..24].copy_from_slice(&g.slot_size.to_le_bytes());
                    file.write_all_at(&h, 0).map_err(io("writing the frame ring header"))
                };
                if let Err(e) = init() {
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(RingError::Io(format!("creating a frame ring in {}", dir.display()), e)),
        }
    }
    Err(RingError::Ring("no free frame ring name".into()))
}

/// Removes a ring file (ignores a file already gone).
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// The producer's side of a ring.
pub struct RingWriter {
    file: File,
    g: Geometry,
    seq: u64,
}

impl RingWriter {
    /// Opens the ring at `path` (created by [`create_format`]) and continues its frame numbering,
    /// so readers see increasing numbers across producer restarts. `generation` is the producer
    /// start count, stored for diagnostics.
    pub fn open(path: &Path, generation: u32) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path).map_err(io("opening the frame ring"))?;
        let g = Geometry::read(&file)?;
        let mut h = [0u8; 16];
        file.read_exact_at(&mut h, 24).map_err(io("reading the frame ring header"))?;
        let seq = u64_at(&h, 0);
        let mut ids = [0u8; 8];
        ids[0..4].copy_from_slice(&std::process::id().to_le_bytes());
        ids[4..8].copy_from_slice(&generation.to_le_bytes());
        file.write_all_at(&ids, 32).map_err(io("writing the frame ring header"))?;
        Ok(Self { file, g, seq })
    }

    /// Writes `frame` without time code, flags or sound. Returns its number.
    pub fn write(&mut self, frame: &Frame) -> Result<u64> {
        self.write_with(frame, &Extras::default())
    }

    /// Writes `frame` and its `extras` into the next slot and publishes it. Returns its number.
    pub fn write_with(&mut self, frame: &Frame, extras: &Extras) -> Result<u64> {
        let len = frame.data.len() as u64;
        if !frame.is_consistent() {
            return Err(RingError::Ring(format!(
                "frame of {} bytes does not match {}x{} stride {} in {:?}",
                len, frame.width, frame.height, frame.stride, frame.format
            )));
        }
        let (audio_len, audio_channels, audio_format) = match &extras.audio {
            None => (0u64, 0u16, 0u16),
            Some(a) if a.is_consistent() => (a.data.len() as u64, a.channels, a.format.code()),
            Some(_) => return Err(RingError::Ring("audio block does not match its channel count".into())),
        };
        let audio_len32 = u32::try_from(audio_len).map_err(|_| RingError::Ring("audio block too large".into()))?;
        if SLOT_HEADER + len + audio_len > self.g.slot_size {
            return Err(RingError::Ring(format!("frame of {len} bytes and {audio_len} bytes of sound do not fit the ring's slots")));
        }
        let seq = self.seq.checked_add(1).ok_or_else(|| RingError::Ring("frame numbers exhausted".into()))?;
        let at = self.g.slot_offset(seq);
        // 1. mark the slot as being written, 2. picture and sound, 3. slot header, 4. publish
        self.file.write_all_at(&0u64.to_le_bytes(), at).map_err(io("writing a frame"))?;
        self.file.write_all_at(&frame.data, at + SLOT_HEADER).map_err(io("writing a frame"))?;
        if let Some(a) = &extras.audio {
            self.file.write_all_at(&a.data, at + SLOT_HEADER + len).map_err(io("writing a frame"))?;
        }
        let mut sh = [0u8; SLOT_HEADER as usize];
        sh[0..8].copy_from_slice(&seq.to_le_bytes());
        sh[8..12].copy_from_slice(&frame.width.to_le_bytes());
        sh[12..16].copy_from_slice(&frame.height.to_le_bytes());
        sh[16..20].copy_from_slice(&frame.stride.to_le_bytes());
        sh[20..24].copy_from_slice(&frame.format.code().to_le_bytes());
        sh[24..32].copy_from_slice(&frame.pts_ns.to_le_bytes());
        sh[32..40].copy_from_slice(&len.to_le_bytes());
        sh[40..44].copy_from_slice(&extras.timecode.to_le_bytes());
        sh[44..48].copy_from_slice(&audio_len32.to_le_bytes());
        sh[48..50].copy_from_slice(&audio_channels.to_le_bytes());
        sh[50..52].copy_from_slice(&audio_format.to_le_bytes());
        sh[52..56].copy_from_slice(&extras.flags.to_le_bytes());
        self.file.write_all_at(&sh, at).map_err(io("writing a frame"))?;
        self.file.write_all_at(&seq.to_le_bytes(), 24).map_err(io("publishing a frame"))?;
        self.seq = seq;
        Ok(seq)
    }
}

/// A frame read from a ring, with its number and what travelled with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingFrame {
    /// Frame number (increasing across producer restarts).
    pub seq: u64,
    /// The picture.
    pub frame: Frame,
    /// SMPTE 12M time code as BCD `0xHHMMSSFF` (0: none).
    pub timecode: u32,
    /// [`FLAG_INTERLACED`] and [`FLAG_BOTTOM_FIELD_FIRST`] (reserved bits are returned as written).
    pub flags: u32,
    /// The sound of this frame, if any.
    pub audio: Option<Audio>,
}

/// A reader's side of a ring.
pub struct RingReader {
    file: File,
    g: Geometry,
    dropped: AtomicU64,
}

impl RingReader {
    /// Opens the ring at `path` for reading. A version 1 ring is the error `ring version 1`.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(io("opening the frame ring"))?;
        let g = Geometry::read(&file)?;
        Ok(Self { file, g, dropped: AtomicU64::new(0) })
    }

    /// Frames dropped because their pixel or audio format code is unknown to this version.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Number of the newest published frame (0: none yet).
    pub fn latest_seq(&self) -> Result<u64> {
        let mut b = [0u8; 8];
        self.file.read_exact_at(&mut b, 24).map_err(io("reading the frame ring"))?;
        Ok(u64::from_le_bytes(b))
    }

    /// Process id and start count of the producer that last opened the ring.
    pub fn writer(&self) -> Result<(u32, u32)> {
        let mut b = [0u8; 8];
        self.file.read_exact_at(&mut b, 32).map_err(io("reading the frame ring"))?;
        Ok((u32_at(&b, 0), u32_at(&b, 4)))
    }

    /// The newest frame, or `None` when there is none yet, it was overwritten while being read
    /// (tried three times; a torn frame is dropped, never returned) or its format is unknown to
    /// this version (dropped and counted).
    pub fn latest(&self) -> Result<Option<RingFrame>> {
        for _ in 0..3 {
            let seq = self.latest_seq()?;
            if seq == 0 {
                return Ok(None);
            }
            if let Some(f) = self.read_slot(seq)? {
                return Ok(f);
            }
        }
        Ok(None)
    }

    /// `Ok(None)`: torn, try again. `Ok(Some(None))`: dropped for good (unknown format).
    fn read_slot(&self, seq: u64) -> Result<Option<Option<RingFrame>>> {
        let at = self.g.slot_offset(seq);
        let mut sh = [0u8; SLOT_HEADER as usize];
        self.file.read_exact_at(&mut sh, at).map_err(io("reading a frame"))?;
        if u64_at(&sh, 0) != seq {
            return Ok(None);
        }
        let (width, height, stride, code, pts_ns, len) = (u32_at(&sh, 8), u32_at(&sh, 12), u32_at(&sh, 16), u32_at(&sh, 20), u64_at(&sh, 24), u64_at(&sh, 32));
        let (timecode, audio_len, channels, audio_code, flags) =
            (u32_at(&sh, 40), u64::from(u32_at(&sh, 44)), u16_at(&sh, 48), u16_at(&sh, 50), u32_at(&sh, 52));
        let Some(format) = PixelFormat::from_code(code) else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(None));
        };
        let audio_format = if audio_len > 0 {
            let Some(f) = AudioFormat::from_code(audio_code) else {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(None));
            };
            Some(f)
        } else {
            None
        };
        let valid = (MIN_SIDE..=MAX_SIDE).contains(&width)
            && (MIN_SIDE..=MAX_SIDE).contains(&height)
            && format.frame_len(width, height, stride) == Some(len)
            && SLOT_HEADER.saturating_add(len).saturating_add(audio_len) <= self.g.slot_size
            && (audio_len == 0 || ((1..=MAX_AUDIO_CHANNELS).contains(&channels) && audio_len.is_multiple_of(u64::from(channels) * 4)));
        if !valid {
            // a header written by a different (or hostile) writer: report, do not allocate
            return Err(RingError::Ring("frame header is inconsistent".into()));
        }
        let mut data = vec![0u8; usize::try_from(len).map_err(|_| RingError::Ring("frame too large".into()))?];
        self.file.read_exact_at(&mut data, at + SLOT_HEADER).map_err(io("reading a frame"))?;
        let audio = match audio_format {
            Some(format) => {
                let mut a = vec![0u8; usize::try_from(audio_len).map_err(|_| RingError::Ring("audio too large".into()))?];
                self.file.read_exact_at(&mut a, at + SLOT_HEADER + len).map_err(io("reading a frame"))?;
                Some(Audio { channels, format, data: a })
            }
            None => None,
        };
        let mut again = [0u8; 8];
        self.file.read_exact_at(&mut again, at).map_err(io("reading a frame"))?;
        if u64::from_le_bytes(again) != seq {
            return Ok(None);
        }
        Ok(Some(Some(RingFrame { seq, frame: Frame { width, height, stride, format, pts_ns, data }, timecode, flags, audio })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 32;
    const H: u32 = 16;

    fn frame(fill: u8, pts: u64) -> Frame {
        Frame { width: W, height: H, stride: W * 4, format: PixelFormat::Bgra8, pts_ns: pts, data: vec![fill; (W * H * 4) as usize] }
    }

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfa4-io-core-ring-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    fn plain(frame: Frame, seq: u64) -> RingFrame {
        RingFrame { seq, frame, timecode: 0, flags: 0, audio: None }
    }

    #[test]
    fn frames_round_trip_and_numbers_continue_across_writers() {
        let path = create_sized(&dir(), W, H).expect("ring");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).ok(), Some(0o600));
        let reader = RingReader::open(&path).expect("reader");
        assert_eq!(reader.latest().expect("read"), None, "empty at first");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        for k in 1..=5u8 {
            assert_eq!(w.write(&frame(k, u64::from(k) * 10)).expect("write"), u64::from(k));
            assert_eq!(reader.latest().expect("read"), Some(plain(frame(k, u64::from(k) * 10), u64::from(k))));
        }
        drop(w);
        let mut w2 = RingWriter::open(&path, 2).expect("writer");
        assert_eq!(w2.write(&frame(9, 1)).expect("write"), 6);
        assert_eq!(reader.writer().expect("ids"), (std::process::id(), 2));
        remove(&path);
        assert!(!path.exists());
    }

    #[test]
    fn timecode_flags_and_sound_travel_with_the_picture() {
        let audio = Audio { channels: 2, format: AudioFormat::S32, data: (0..32u8).collect() };
        let path = create_format(&dir(), W, H, PixelFormat::Bgra8, 1024).expect("ring");
        let reader = RingReader::open(&path).expect("reader");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        let extras = Extras { timecode: 0x10_20_30_24, flags: FLAG_INTERLACED | FLAG_BOTTOM_FIELD_FIRST, audio: Some(audio.clone()) };
        w.write_with(&frame(3, 7), &extras).expect("write");
        let got = reader.latest().expect("read").expect("frame");
        assert_eq!((got.timecode, got.flags, got.audio.as_ref()), (0x10_20_30_24, 3, Some(&audio)));
        assert_eq!(got.frame, frame(3, 7));
        // a frame without sound after one with
        w.write(&frame(4, 8)).expect("write");
        assert_eq!(reader.latest().expect("read"), Some(plain(frame(4, 8), 2)));
        // sound that does not fit the slot, and a broken block, are refused
        let big = Audio { channels: 2, format: AudioFormat::S32, data: vec![0; 2048] };
        assert!(w.write_with(&frame(5, 9), &Extras { audio: Some(big), ..Extras::default() }).is_err());
        let odd = Audio { channels: 2, format: AudioFormat::S32, data: vec![0; 12] };
        assert!(w.write_with(&frame(5, 9), &Extras { audio: Some(odd), ..Extras::default() }).is_err());
        remove(&path);
    }

    #[test]
    fn ten_and_twelve_bit_pictures_fit_and_convert() {
        let d = dir();
        for (format, stride) in [(PixelFormat::Yuv422P10, W * 2), (PixelFormat::Rgb12P, W * 2), (PixelFormat::Uyvy8, W * 2)] {
            let path = create_format(&d, W, H, format, 0).expect("ring");
            let len = format.bytes_per_frame(W, H) as usize;
            let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let f = Frame { width: W, height: H, stride, format, pts_ns: 5, data };
            let mut w = RingWriter::open(&path, 1).expect("writer");
            w.write(&f).expect("write");
            let got = RingReader::open(&path).expect("reader").latest().expect("read").expect("frame");
            assert_eq!(got.frame, f);
            assert!(got.frame.to_video_frame().is_some());
            remove(&path);
        }
        assert!(create_format(&d, 17, H, PixelFormat::Yuv422P10, 0).is_err(), "odd width");
        assert!(create_format(&d, 8192, 8192, PixelFormat::Rgb12P, 0).is_err(), "too large for a ring");
    }

    #[test]
    fn a_torn_frame_is_dropped() {
        let path = create_sized(&dir(), W, H).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        w.write(&frame(1, 0)).expect("write");
        let reader = RingReader::open(&path).expect("reader");
        let at = reader.g.slot_offset(1);
        let f = OpenOptions::new().write(true).open(&path).expect("open");
        f.write_all_at(&0u64.to_le_bytes(), at).expect("mark");
        assert_eq!(reader.latest().expect("read"), None);
        remove(&path);
    }

    #[test]
    fn a_version_1_ring_is_a_clear_error() {
        let d = dir();
        let p = d.join("old-ring");
        let mut h = vec![0u8; 4096];
        h[0..8].copy_from_slice(MAGIC_V1);
        h[8..12].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&p, &h).expect("write");
        for e in [RingReader::open(&p).err(), RingWriter::open(&p, 0).err()] {
            assert!(matches!(e, Some(RingError::Ring(ref m)) if m == "ring version 1"), "{e:?}");
        }
        remove(&p);
    }

    #[test]
    fn an_unknown_format_code_drops_the_frame_and_counts_it() {
        let path = create_sized(&dir(), W, H).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        w.write(&frame(1, 0)).expect("write");
        let reader = RingReader::open(&path).expect("reader");
        let at = reader.g.slot_offset(1);
        let f = OpenOptions::new().write(true).open(&path).expect("open");
        f.write_all_at(&99u32.to_le_bytes(), at + 20).expect("unknown pixel format");
        assert_eq!(reader.latest().expect("no error"), None);
        assert_eq!(reader.dropped(), 1);
        f.write_all_at(&PixelFormat::Bgra8.code().to_le_bytes(), at + 20).expect("restore");
        f.write_all_at(&16u32.to_le_bytes(), at + 44).expect("audio_len");
        f.write_all_at(&2u16.to_le_bytes(), at + 48).expect("channels");
        f.write_all_at(&77u16.to_le_bytes(), at + 50).expect("unknown audio format");
        assert_eq!(reader.latest().expect("no error"), None);
        assert_eq!(reader.dropped(), 2);
        // a later frame is read normally
        w.write(&frame(2, 1)).expect("write");
        assert_eq!(reader.latest().expect("read").map(|f| f.seq), Some(2));
        remove(&path);
    }

    #[test]
    fn hostile_files_are_errors_not_crashes() {
        let d = dir();
        let p = d.join("not-a-ring");
        std::fs::write(&p, b"hello").expect("write");
        assert!(RingReader::open(&p).is_err());
        assert!(RingWriter::open(&p, 0).is_err());
        // a valid header with a corrupt slot header
        let path = create_sized(&d, W, H).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        w.write(&frame(1, 0)).expect("write");
        let reader = RingReader::open(&path).expect("reader");
        let at = reader.g.slot_offset(1);
        let f = OpenOptions::new().write(true).open(&path).expect("open");
        f.write_all_at(&u64::MAX.to_le_bytes(), at + 32).expect("corrupt len");
        assert!(reader.latest().is_err());
        f.write_all_at(&(W * H * 4).to_le_bytes(), at + 32).expect("restore len");
        f.write_all_at(&u32::MAX.to_le_bytes(), at + 44).expect("corrupt audio_len");
        f.write_all_at(&2u16.to_le_bytes(), at + 48).expect("channels");
        f.write_all_at(&AudioFormat::S32.code().to_le_bytes(), at + 50).expect("audio format");
        assert!(reader.latest().is_err());
        // truncated file
        f.set_len(10).expect("truncate");
        assert!(RingReader::open(&path).is_err());
        remove(&path);
        remove(&p);
        // frames that do not fit
        let mut big = frame(0, 0);
        big.data.push(0);
        let path = create_sized(&d, W, H).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        assert!(w.write(&big).is_err());
        remove(&path);
    }
}
