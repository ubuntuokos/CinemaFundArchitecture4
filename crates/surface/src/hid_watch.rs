//! inotify on `/dev` (F0 spec §6, decision D5).
//!
//! Plugging or unplugging a hidraw node is noticed here first. The two-second scan in the
//! session stays as a safety net for a missed event, a permission that appears later, and a
//! watch that could not be opened.

use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use rustix::io::{Errno, read};
use std::os::fd::OwnedFd;
use std::path::Path;

/// A watch of one directory. Dropping it closes the watch.
pub struct DevWatch {
    fd: OwnedFd,
}

impl DevWatch {
    /// Watch `dir` for nodes appearing and disappearing. An error when inotify cannot be opened
    /// (the two-second scan still runs).
    pub fn open(dir: &Path) -> Result<Self, String> {
        let fd = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).map_err(|e| format!("HID: inotify is not available ({e})"))?;
        let flags = WatchFlags::CREATE | WatchFlags::DELETE | WatchFlags::MOVED_FROM | WatchFlags::MOVED_TO | WatchFlags::ONLYDIR;
        inotify::add_watch(&fd, dir, flags).map_err(|e| format!("HID: cannot watch {}: {e}", dir.display()))?;
        Ok(Self { fd })
    }

    /// Whether the directory changed since the last call. A queue overflow counts as a change,
    /// so a missed name still causes a scan. `false` when nothing happened.
    pub fn changed(&self) -> bool {
        let mut buf = [0u8; 4096];
        let mut any = false;
        loop {
            match read(&self.fd, &mut buf) {
                Ok(0) => return any,
                Ok(n) => {
                    any = true;
                    if n < buf.len() {
                        return true;
                    }
                }
                Err(Errno::AGAIN | Errno::INTR) => return any,
                Err(_) => return true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_directory_is_an_error() {
        let e = DevWatch::open(Path::new("/nonexistent/cfa4/dev")).err().unwrap_or_default();
        assert!(e.contains("inotify") || e.contains("watch"), "{e}");
    }
}
