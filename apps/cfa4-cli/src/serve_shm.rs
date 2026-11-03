//! Frames through shared memory: `frame.render {shm: "<ring>"}` writes the frame into a slot of a
//! file in shared memory (`$XDG_RUNTIME_DIR`, else `/dev/shm`) instead of the reply's binary tail,
//! and the reply names it (`shm: {path, offset, len, slots, slotSize}`). The GUI maps the file
//! once and shows each frame straight from the mapping: no copy through the pipe, the reader
//! thread and a QImage copy per frame (`docs/qt-parity.md` 5.5).
//!
//! Each ring (one per monitor) has [`SLOTS`] slots used in turn, so a frame the GUI shows is not
//! overwritten while it asks for the next ones one at a time. A frame larger than the ring's
//! slots starts a new file (the old one stays valid for whoever still maps it). The files are
//! created private (0600, never reused names); the GUI removes a file once it has mapped it, and
//! the engine removes the ones left when it ends.

use serde_json::{Value, json};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::PathBuf;

/// Slots per ring.
pub const SLOTS: u64 = 4;
/// Slot sizes are whole MiB, so a growing monitor does not start a new file for every pixel.
const SLOT_ROUND: u64 = 1 << 20;
/// The largest frame sent this way (an 8K RGBA frame is 127 MiB).
pub const MAX_FRAME: usize = 256 << 20;

struct Ring {
    name: String,
    path: PathBuf,
    file: File,
    slot: u64,
    next: u64,
}

thread_local! {
    static RINGS: RefCell<Vec<Ring>> = const { RefCell::new(Vec::new()) };
}

/// Whether `name` can name a ring: 1–16 ASCII letters, digits or `-`.
pub fn valid_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn dir() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir());
    runtime.or_else(|| Some(PathBuf::from("/dev/shm")).filter(|p| p.is_dir())).unwrap_or_else(std::env::temp_dir)
}

fn create(name: &str, slot: u64) -> Result<Ring, String> {
    let size = slot.checked_mul(SLOTS).ok_or("frame too large")?;
    let base = dir();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for attempt in 0..8 {
        let path = base.join(format!("cfa4-frames-{}-{name}-{stamp:x}-{attempt}", std::process::id()));
        match OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path) {
            Ok(file) => {
                if let Err(e) = file.set_len(size) {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("shared memory unavailable: {e}"));
                }
                return Ok(Ring { name: name.to_owned(), path, file, slot, next: 0 });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("shared memory unavailable: {e}")),
        }
    }
    Err("shared memory unavailable: no free name".into())
}

/// Writes `bytes` into the next slot of ring `name` and says where: `{path, offset, len, slots,
/// slotSize}`. An error leaves the caller to send the frame as a binary tail.
pub fn put(name: &str, bytes: &[u8]) -> Result<Value, String> {
    if !valid_name(name) {
        return Err("`shm` must name a ring: 1–16 letters, digits or -".into());
    }
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("frame size unsuitable for shared memory".into());
    }
    let len = bytes.len() as u64;
    RINGS.with(|cell| {
        let mut rings = cell.borrow_mut();
        let i = match rings.iter().position(|r| r.name == name) {
            Some(i) if rings.get(i).is_some_and(|r| r.slot >= len) => i,
            found => {
                let ring = create(name, len.div_ceil(SLOT_ROUND) * SLOT_ROUND)?;
                match found {
                    Some(i) => {
                        if let Some(old) = rings.get_mut(i) {
                            let _ = std::fs::remove_file(&old.path);
                            *old = ring;
                        }
                        i
                    }
                    None => {
                        rings.push(ring);
                        rings.len() - 1
                    }
                }
            }
        };
        let ring = rings.get_mut(i).ok_or("shared memory ring lost")?;
        let index = ring.next % SLOTS;
        let offset = index * ring.slot;
        ring.file.write_all_at(bytes, offset).map_err(|e| format!("shared memory write failed: {e}"))?;
        ring.next = ring.next.wrapping_add(1);
        Ok(json!({"path": ring.path.to_string_lossy(), "offset": offset, "len": len, "slots": SLOTS, "slotSize": ring.slot}))
    })
}

/// Removes the ring files left (the engine is ending).
pub fn cleanup() {
    RINGS.with(|cell| {
        for ring in cell.borrow_mut().drain(..) {
            let _ = std::fs::remove_file(&ring.path);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(info: &Value) -> Vec<u8> {
        let data = std::fs::read(info["path"].as_str().unwrap_or_default()).unwrap_or_default();
        let (o, n) = (info["offset"].as_u64().unwrap_or(0) as usize, info["len"].as_u64().unwrap_or(0) as usize);
        data.get(o..o + n).map(<[u8]>::to_vec).unwrap_or_default()
    }

    #[test]
    fn frames_go_round_the_slots_of_one_private_file() {
        let a: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let first = put("t-ring", &a).expect("shm");
        assert_eq!(read(&first), a);
        let path = first["path"].as_str().unwrap_or_default().to_owned();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
        assert_eq!(mode, 0o600, "private to the user");
        assert_eq!(first["slots"], json!(SLOTS));
        let mut offsets = vec![first["offset"].as_u64()];
        for k in 1..SLOTS + 1 {
            let b = vec![k as u8; 1000];
            let r = put("t-ring", &b).expect("shm");
            assert_eq!(r["path"].as_str(), Some(path.as_str()), "the same file");
            assert_eq!(read(&r), b);
            offsets.push(r["offset"].as_u64());
        }
        assert_eq!(offsets[0], offsets[SLOTS as usize], "round the ring");
        let distinct: std::collections::HashSet<_> = offsets[..SLOTS as usize].iter().collect();
        assert_eq!(distinct.len(), SLOTS as usize, "each slot once");
        // a frame larger than a slot: a new file; the old one is gone from the directory
        let big = vec![7u8; (SLOT_ROUND + 1) as usize];
        let r = put("t-ring", &big).expect("shm");
        assert_ne!(r["path"].as_str(), Some(path.as_str()));
        assert!(!std::path::Path::new(&path).exists());
        assert_eq!(read(&r), big);
        cleanup();
        assert!(!std::path::Path::new(r["path"].as_str().unwrap_or_default()).exists(), "removed at the end");
    }

    #[test]
    fn bad_names_and_sizes_are_refused() {
        for name in ["", "../x", "a/b", "seventeen-chars-x", "sp ace"] {
            assert!(put(name, &[1, 2, 3]).is_err(), "{name:?}");
        }
        assert!(put("ok", &[]).is_err());
        assert!(valid_name("program") && valid_name("source"));
    }
}
