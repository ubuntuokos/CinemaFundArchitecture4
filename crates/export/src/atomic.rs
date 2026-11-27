//! Atomic export output (CFA4-SCE-001 v1.5, decision 6, §20): every byte goes to a temp file in the
//! destination directory, and only [`AtomicOutput::commit`] publishes it, after the caller has
//! validated it: the temp file is synced, renamed onto the final path and the directory is synced.
//! Dropping an uncommitted output (an error, a cancel, a panic unwinding) removes the temp file, so
//! a failed export never leaves a partial file and never touches an existing one.
//!
//! The same pattern as `cfa4_format::atomic` (project saves), but streaming (`Write + Seek + Read`:
//! the MP4 writer seeks back and its faststart pass reads the file again) and with an explicit
//! overwrite policy.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What to do when the final path already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overwrite {
    /// Refuse: [`AtomicOutput::commit`] fails and the existing file stays (the default, §20:
    /// overwriting needs the user's approval).
    Refuse,
    /// Replace the existing file (the user approved it).
    Replace,
}

/// A temp file that becomes `path` on [`AtomicOutput::commit`].
#[derive(Debug)]
pub struct AtomicOutput {
    staged: tempfile::NamedTempFile,
    path: PathBuf,
    overwrite: Overwrite,
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Best-effort directory sync; directory handles are not uniformly readable or syncable.
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

impl AtomicOutput {
    /// Stage an output for `path`. With [`Overwrite::Refuse`] an existing `path` is refused here
    /// already, before any work, and again at commit time.
    pub fn create(path: &Path, overwrite: Overwrite) -> io::Result<AtomicOutput> {
        if overwrite == Overwrite::Refuse && path.symlink_metadata().is_ok() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", path.display())));
        }
        let dir = parent_dir(path);
        #[cfg(unix)]
        let staged = {
            use std::os::unix::fs::PermissionsExt;
            // like an ordinary file creation: 0666 minus the process umask
            tempfile::Builder::new().prefix(".cfa4-export-").permissions(std::fs::Permissions::from_mode(0o666)).tempfile_in(&dir)?
        };
        #[cfg(not(unix))]
        let staged = tempfile::Builder::new().prefix(".cfa4-export-").tempfile_in(&dir)?;
        Ok(AtomicOutput { staged, path: path.to_path_buf(), overwrite })
    }

    /// The final path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The temp file being written (for reading it back before the commit).
    pub fn file(&self) -> &File {
        self.staged.as_file()
    }

    /// Sync the temp file and publish it as the final path; returns its size. On failure the temp
    /// file is removed and the final path is unchanged.
    pub fn commit(mut self) -> io::Result<u64> {
        self.staged.as_file_mut().flush()?;
        self.staged.as_file().sync_all()?;
        let size = self.staged.as_file().metadata()?.len();
        let dir = parent_dir(&self.path);
        match self.overwrite {
            Overwrite::Replace => {
                self.staged.persist(&self.path).map_err(|e| e.error)?;
            }
            Overwrite::Refuse => {
                self.staged.persist_noclobber(&self.path).map_err(|e| e.error)?;
            }
        }
        sync_dir(&dir);
        Ok(size)
    }
}

/// Prefix of the staged temp files of [`AtomicOutput`].
pub const STAGED_PREFIX: &str = ".cfa4-export-";

/// Remove staged temp files a killed writer left in `dir` (a SIGKILL runs no destructor).
///
/// Only regular files whose name starts with [`STAGED_PREFIX`] and whose last write is at least
/// `min_age` ago are removed; symlinks, directories and everything else are left alone. A live
/// writer keeps touching its temp file, so a conservative `min_age` (an hour or more) never takes
/// one from under it; call this for a directory with no export running when in doubt. Returns the
/// removed paths. A file that cannot be removed is skipped, not an error.
pub fn sweep_orphans(dir: &Path, min_age: std::time::Duration) -> io::Result<Vec<PathBuf>> {
    let now = std::time::SystemTime::now();
    let mut removed = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(STAGED_PREFIX) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        if !meta.is_file() {
            continue;
        }
        let age = meta.modified().ok().and_then(|m| now.duration_since(m).ok());
        if age.is_some_and(|a| a >= min_age) && std::fs::remove_file(entry.path()).is_ok() {
            removed.push(entry.path());
        }
    }
    removed.sort();
    Ok(removed)
}

impl Write for AtomicOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.staged.as_file_mut().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.staged.as_file_mut().flush()
    }
}

impl Seek for AtomicOutput {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.staged.as_file_mut().seek(pos)
    }
}

impl Read for AtomicOutput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.staged.as_file_mut().read(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfa4-export-atomic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entries(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn nothing_appears_before_the_commit() {
        let d = tmpdir("commit");
        let p = d.join("out.mp4");
        let mut o = AtomicOutput::create(&p, Overwrite::Refuse).unwrap();
        o.write_all(b"hello").unwrap();
        assert!(!p.exists());
        assert_eq!(o.commit().unwrap(), 5);
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        assert_eq!(entries(&d), vec!["out.mp4".to_string()], "no temp file left");
    }

    #[test]
    fn a_dropped_output_leaves_nothing_and_keeps_the_old_file() {
        let d = tmpdir("drop");
        let p = d.join("out.mp4");
        std::fs::write(&p, b"old").unwrap();
        let mut o = AtomicOutput::create(&p, Overwrite::Replace).unwrap();
        o.write_all(b"partial").unwrap();
        drop(o);
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
        assert_eq!(entries(&d), vec!["out.mp4".to_string()]);
    }

    #[test]
    fn an_existing_file_is_refused_unless_replacing_is_approved() {
        let d = tmpdir("exists");
        let p = d.join("out.mp4");
        std::fs::write(&p, b"old").unwrap();
        let e = AtomicOutput::create(&p, Overwrite::Refuse).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        // a file that appears while the export runs is not replaced either
        let q = d.join("late.mp4");
        let mut o = AtomicOutput::create(&q, Overwrite::Refuse).unwrap();
        o.write_all(b"new").unwrap();
        std::fs::write(&q, b"someone else's").unwrap();
        assert!(o.commit().is_err());
        assert_eq!(std::fs::read(&q).unwrap(), b"someone else's");
        let mut o = AtomicOutput::create(&p, Overwrite::Replace).unwrap();
        o.write_all(b"new").unwrap();
        o.commit().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert_eq!(entries(&d), vec!["late.mp4".to_string(), "out.mp4".to_string()]);
    }

    #[test]
    fn it_seeks_and_reads_back() {
        let d = tmpdir("seek");
        let mut o = AtomicOutput::create(&d.join("x.bin"), Overwrite::Refuse).unwrap();
        o.write_all(b"abcdef").unwrap();
        o.seek(SeekFrom::Start(1)).unwrap();
        o.write_all(b"X").unwrap();
        o.seek(SeekFrom::Start(0)).unwrap();
        let mut s = String::new();
        o.read_to_string(&mut s).unwrap();
        assert_eq!(s, "aXcdef");
    }

    #[test]
    fn a_missing_directory_is_an_error_not_a_panic() {
        let d = tmpdir("missing");
        assert!(AtomicOutput::create(&d.join("no/such/dir/out.mp4"), Overwrite::Refuse).is_err());
    }

    #[test]
    fn the_sweep_removes_only_old_staged_files() {
        use std::time::{Duration, SystemTime};
        let d = tmpdir("sweep");
        let old = d.join(format!("{STAGED_PREFIX}old"));
        let fresh = d.join(format!("{STAGED_PREFIX}fresh"));
        let other = d.join("keep.mp4");
        let dir_like = d.join(format!("{STAGED_PREFIX}dir"));
        for p in [&old, &fresh, &other] {
            std::fs::write(p, b"x").unwrap();
        }
        std::fs::create_dir(&dir_like).unwrap();
        let two_hours_ago = SystemTime::now() - Duration::from_secs(2 * 3600);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(two_hours_ago).unwrap();
        let removed = sweep_orphans(&d, Duration::from_secs(3600)).unwrap();
        assert_eq!(removed, vec![old.clone()]);
        assert!(!old.exists() && fresh.exists() && other.exists() && dir_like.exists());
        // a second sweep finds nothing; a zero age takes the remaining staged file too
        assert!(sweep_orphans(&d, Duration::from_secs(3600)).unwrap().is_empty());
        assert_eq!(sweep_orphans(&d, Duration::ZERO).unwrap(), vec![fresh.clone()]);
    }

    #[cfg(unix)]
    #[test]
    fn the_sweep_never_follows_a_symlink() {
        use std::time::Duration;
        let d = tmpdir("sweep-link");
        let target = d.join("precious.mp4");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, d.join(format!("{STAGED_PREFIX}link"))).unwrap();
        assert!(sweep_orphans(&d, Duration::ZERO).unwrap().is_empty());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }
}
