//! The frame ring: studio frames in shared memory, from the host process to its readers.
//!
//! The supervisor creates the ring as a private (0600) file in `$XDG_RUNTIME_DIR` (a tmpfs, so
//! the file is shared memory), outside the host, so it survives host restarts. The host writes
//! each program frame into the next slot; the engine reads the newest one (and the Qt6 GUI will
//! map the same file, plan V.4). Plain positioned reads and writes are used (no `mmap`), so the
//! Rust side needs no `unsafe`.
//!
//! Layout (little-endian):
//!
//! ```text
//! header (64 B)  magic "CFA4LIV1" | version u32 | slots u32 | slot_size u64 | latest u64 |
//!                writer_pid u32 | generation u32 | reserved
//! slot i at 64 + i * slot_size:
//!   slot header (64 B)  seq u64 | width u32 | height u32 | stride u32 | format u32 | pts_ns u64 |
//!                       len u64 | reserved
//!   pixel data (len B)
//! ```
//!
//! Frame `n` (counted from 1) goes to slot `n % slots`. The writer sets the slot's `seq` to 0,
//! writes the pixels, then the slot header with `seq = n`, then `latest = n`. A reader takes
//! `latest`, reads that slot, and reads `seq` again afterwards: if either is not `n` the writer
//! went round the ring meanwhile and the frame is dropped (a torn frame is never shown).

use crate::{Frame, PixelFormat, Result, StudioError, VideoSettings};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// File magic.
pub const MAGIC: &[u8; 8] = b"CFA4LIV1";
/// Layout version.
pub const VERSION: u32 = 1;
/// Header bytes.
pub const HEADER: u64 = 64;
/// Slot header bytes.
pub const SLOT_HEADER: u64 = 64;
/// Slots per ring.
pub const SLOTS: u32 = 3;
/// Largest ring file (three 8K BGRA frames are 768 MiB; refuse beyond 1 GiB).
pub const MAX_FILE: u64 = 1 << 30;

fn io(what: &str) -> impl FnOnce(std::io::Error) -> StudioError + '_ {
    move |e| StudioError::Io(what.to_owned(), e)
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

/// Geometry of a ring, from its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    slots: u32,
    slot_size: u64,
}

impl Geometry {
    fn for_video(video: &VideoSettings) -> Result<Self> {
        video.validate()?;
        let slot_size = SLOT_HEADER + video.frame_len() as u64;
        let total = HEADER + slot_size * u64::from(SLOTS);
        if total > MAX_FILE {
            return Err(StudioError::Invalid(format!("frames of {}x{} do not fit a frame ring", video.width, video.height)));
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
        if h.get(0..8) != Some(MAGIC.as_slice()) || u32_at(&h, 8) != VERSION {
            return Err(StudioError::Ring("not a CFA4 frame ring (magic or version)".into()));
        }
        let g = Self { slots: u32_at(&h, 12), slot_size: u64_at(&h, 16) };
        let len = file.metadata().map_err(io("reading the frame ring size"))?.len();
        if g.slots == 0 || g.slots > 16 || g.slot_size <= SLOT_HEADER || g.file_len() > MAX_FILE || len < g.file_len() {
            return Err(StudioError::Ring("frame ring header is inconsistent".into()));
        }
        Ok(g)
    }
}

/// Creates a ring for `video` in `dir`: a new private file, never reused. Returns its path; the
/// caller removes the file when done ([`remove`]).
pub fn create(dir: &Path, video: &VideoSettings) -> Result<PathBuf> {
    let g = Geometry::for_video(video)?;
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
            Err(e) => return Err(StudioError::Io(format!("creating a frame ring in {}", dir.display()), e)),
        }
    }
    Err(StudioError::Ring("no free frame ring name".into()))
}

/// Removes a ring file (ignores a file already gone).
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// The host's side of a ring.
pub struct RingWriter {
    file: File,
    g: Geometry,
    seq: u64,
}

impl RingWriter {
    /// Opens the ring at `path` (created by [`create`]) and continues its frame numbering, so
    /// readers see increasing numbers across host restarts. `generation` is the host start
    /// count, stored for diagnostics.
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

    /// Writes `frame` into the next slot and publishes it. Returns its number.
    pub fn write(&mut self, frame: &Frame) -> Result<u64> {
        let len = frame.data.len() as u64;
        let expected = u64::from(frame.stride).saturating_mul(u64::from(frame.height));
        if len != expected || frame.stride < frame.width.saturating_mul(frame.format.bytes_per_pixel()) {
            return Err(StudioError::Ring(format!("frame of {} bytes does not match {}x{} stride {}", len, frame.width, frame.height, frame.stride)));
        }
        if SLOT_HEADER + len > self.g.slot_size {
            return Err(StudioError::Ring(format!("frame of {len} bytes is larger than the ring's slots")));
        }
        let seq = self.seq.checked_add(1).ok_or_else(|| StudioError::Ring("frame numbers exhausted".into()))?;
        let at = self.g.slot_offset(seq);
        // 1. mark the slot as being written, 2. pixels, 3. slot header, 4. publish
        self.file.write_all_at(&0u64.to_le_bytes(), at).map_err(io("writing a frame"))?;
        self.file.write_all_at(&frame.data, at + SLOT_HEADER).map_err(io("writing a frame"))?;
        let mut sh = [0u8; SLOT_HEADER as usize];
        sh[0..8].copy_from_slice(&seq.to_le_bytes());
        sh[8..12].copy_from_slice(&frame.width.to_le_bytes());
        sh[12..16].copy_from_slice(&frame.height.to_le_bytes());
        sh[16..20].copy_from_slice(&frame.stride.to_le_bytes());
        sh[20..24].copy_from_slice(&frame.format.code().to_le_bytes());
        sh[24..32].copy_from_slice(&frame.pts_ns.to_le_bytes());
        sh[32..40].copy_from_slice(&len.to_le_bytes());
        self.file.write_all_at(&sh, at).map_err(io("writing a frame"))?;
        self.file.write_all_at(&seq.to_le_bytes(), 24).map_err(io("publishing a frame"))?;
        self.seq = seq;
        Ok(seq)
    }
}

/// A frame read from a ring, with its number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingFrame {
    /// Frame number (increasing across host restarts).
    pub seq: u64,
    /// The frame.
    pub frame: Frame,
}

/// A reader's side of a ring.
pub struct RingReader {
    file: File,
    g: Geometry,
}

impl RingReader {
    /// Opens the ring at `path` for reading.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(io("opening the frame ring"))?;
        let g = Geometry::read(&file)?;
        Ok(Self { file, g })
    }

    /// Number of the newest published frame (0: none yet).
    pub fn latest_seq(&self) -> Result<u64> {
        let mut b = [0u8; 8];
        self.file.read_exact_at(&mut b, 24).map_err(io("reading the frame ring"))?;
        Ok(u64::from_le_bytes(b))
    }

    /// Process id and start count of the host that last opened the ring.
    pub fn writer(&self) -> Result<(u32, u32)> {
        let mut b = [0u8; 8];
        self.file.read_exact_at(&mut b, 32).map_err(io("reading the frame ring"))?;
        Ok((u32_at(&b, 0), u32_at(&b, 4)))
    }

    /// The newest frame, or `None` when there is none yet or it was overwritten while being read
    /// (tried three times; a torn frame is dropped, never returned).
    pub fn latest(&self) -> Result<Option<RingFrame>> {
        for _ in 0..3 {
            let seq = self.latest_seq()?;
            if seq == 0 {
                return Ok(None);
            }
            if let Some(f) = self.read_slot(seq)? {
                return Ok(Some(f));
            }
        }
        Ok(None)
    }

    fn read_slot(&self, seq: u64) -> Result<Option<RingFrame>> {
        let at = self.g.slot_offset(seq);
        let mut sh = [0u8; SLOT_HEADER as usize];
        self.file.read_exact_at(&mut sh, at).map_err(io("reading a frame"))?;
        if u64_at(&sh, 0) != seq {
            return Ok(None);
        }
        let (width, height, stride, code, pts_ns, len) = (u32_at(&sh, 8), u32_at(&sh, 12), u32_at(&sh, 16), u32_at(&sh, 20), u64_at(&sh, 24), u64_at(&sh, 32));
        let Some(format) = PixelFormat::from_code(code) else {
            return Err(StudioError::Ring(format!("unknown pixel format {code}")));
        };
        let valid = (crate::MIN_SIDE..=crate::MAX_SIDE).contains(&width)
            && (crate::MIN_SIDE..=crate::MAX_SIDE).contains(&height)
            && stride >= width.saturating_mul(format.bytes_per_pixel())
            && len == u64::from(stride).saturating_mul(u64::from(height))
            && SLOT_HEADER.saturating_add(len) <= self.g.slot_size;
        if !valid {
            // a header written by a different (or hostile) writer: report, do not allocate
            return Err(StudioError::Ring("frame header is inconsistent".into()));
        }
        let mut data = vec![0u8; usize::try_from(len).map_err(|_| StudioError::Ring("frame too large".into()))?];
        self.file.read_exact_at(&mut data, at + SLOT_HEADER).map_err(io("reading a frame"))?;
        let mut again = [0u8; 8];
        self.file.read_exact_at(&mut again, at).map_err(io("reading a frame"))?;
        if u64::from_le_bytes(again) != seq {
            return Ok(None);
        }
        Ok(Some(RingFrame { seq, frame: Frame { width, height, stride, format, pts_ns, data } }))
    }
}

/// Converts a BGRA frame to tightly packed RGBA (the format of the engine's `frame.render`).
pub fn bgra_to_rgba(frame: &Frame) -> Vec<u8> {
    let row = frame.width as usize * 4;
    let mut out = Vec::with_capacity(row.saturating_mul(frame.height as usize));
    for r in 0..frame.height as usize {
        let start = r.saturating_mul(frame.stride as usize);
        let Some(src) = frame.data.get(start..start.saturating_add(row)) else { break };
        for px in src.chunks_exact(4) {
            if let [b, g, r, a] = *px {
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> VideoSettings {
        VideoSettings { width: 32, height: 16, fps_num: 30, fps_den: 1 }
    }

    fn frame(fill: u8, pts: u64) -> Frame {
        let v = video();
        Frame { width: v.width, height: v.height, stride: v.stride(), format: PixelFormat::Bgra8, pts_ns: pts, data: vec![fill; v.frame_len()] }
    }

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfa4-obs-ring-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn frames_round_trip_and_numbers_continue_across_writers() {
        let path = create(&dir(), &video()).expect("ring");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).ok(), Some(0o600));
        let reader = RingReader::open(&path).expect("reader");
        assert_eq!(reader.latest().expect("read"), None, "empty at first");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        for k in 1..=5u8 {
            assert_eq!(w.write(&frame(k, u64::from(k) * 10)).expect("write"), u64::from(k));
            let got = reader.latest().expect("read").expect("frame");
            assert_eq!(got.seq, u64::from(k));
            assert_eq!(got.frame, frame(k, u64::from(k) * 10));
        }
        // a restarted host continues the numbering
        drop(w);
        let mut w2 = RingWriter::open(&path, 2).expect("writer");
        assert_eq!(w2.write(&frame(9, 1)).expect("write"), 6);
        assert_eq!(reader.writer().expect("ids"), (std::process::id(), 2));
        remove(&path);
        assert!(!path.exists());
    }

    #[test]
    fn a_torn_frame_is_dropped() {
        let path = create(&dir(), &video()).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        w.write(&frame(1, 0)).expect("write");
        // simulate a writer that went round the ring: the slot of `latest` is marked as written
        let reader = RingReader::open(&path).expect("reader");
        let at = reader.g.slot_offset(1);
        let f = OpenOptions::new().write(true).open(&path).expect("open");
        f.write_all_at(&0u64.to_le_bytes(), at).expect("mark");
        assert_eq!(reader.latest().expect("read"), None);
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
        let path = create(&d, &video()).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        w.write(&frame(1, 0)).expect("write");
        let reader = RingReader::open(&path).expect("reader");
        let at = reader.g.slot_offset(1);
        let f = OpenOptions::new().write(true).open(&path).expect("open");
        f.write_all_at(&u64::MAX.to_le_bytes(), at + 32).expect("corrupt len");
        assert!(reader.latest().is_err());
        f.write_all_at(&7u32.to_le_bytes(), at + 20).expect("corrupt format");
        assert!(reader.latest().is_err());
        // truncated file
        f.set_len(10).expect("truncate");
        assert!(RingReader::open(&path).is_err());
        remove(&path);
        remove(&p);
        // frames that do not fit
        let mut big = frame(0, 0);
        big.data.push(0);
        let path = create(&d, &video()).expect("ring");
        let mut w = RingWriter::open(&path, 1).expect("writer");
        assert!(w.write(&big).is_err());
        remove(&path);
    }

    #[test]
    fn bgra_becomes_rgba() {
        let f = Frame { width: 2, height: 1, stride: 12, format: PixelFormat::Bgra8, pts_ns: 0, data: vec![1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0] };
        assert_eq!(bgra_to_rgba(&f), vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
