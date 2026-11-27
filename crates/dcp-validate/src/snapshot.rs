//! Exact copy, manifest and repair log of a DCP package (DCP-24, plan DCP-F21 / "pontos, visszakövethető példány").
//!
//! Before anything is done about a package, [`snapshot`] makes a byte-identical copy of it outside the
//! package and writes a [`Manifest`] (`cfa4-dcp-manifest/1`): every file with its size, SHA-256, SHA-1 and
//! modification time. [`verify`] compares a package with a manifest file by file, so it can be shown that the
//! package is the same before and after. [`append_log`] adds one line to the repair log
//! (`cfa4-dcp-repair-log/1`, JSON lines, append only) that ties an action to the copy and to the reports
//! before and after. The package itself is only read: a copy inside the package, or over something that
//! exists, is refused.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_SCHEMA: &str = "cfa4-dcp-manifest/1";
pub const LOG_SCHEMA: &str = "cfa4-dcp-repair-log/1";
/// File names inside the snapshot folder.
pub const MANIFEST_FILE: &str = "manifest.json";
pub const LOG_FILE: &str = "repair-log.jsonl";
pub const PACKAGE_DIR: &str = "package";

/// Bytes read per step; cancellation and progress are checked between steps.
const STEP: usize = 1 << 20;

/// One file of a package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    /// Relative to the package root, with `/`.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub sha1: String,
    /// RFC 3339, UTC; absent when the file system gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

/// The files of a package at one moment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub cfa4_version: String,
    /// The package root as given.
    pub root: String,
    pub files: Vec<ManifestFile>,
    /// Entries that are not regular files (symbolic links, devices): never copied, listed so nothing is hidden.
    #[serde(default)]
    pub skipped: Vec<String>,
}

impl Manifest {
    /// SHA-256 of the manifest text: its identity in the repair log.
    pub fn id(&self) -> String {
        hex(&Sha256::digest(self.to_json().as_bytes()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Manifest, SnapshotError> {
        let m: Manifest = serde_json::from_str(text).map_err(|e| SnapshotError::Manifest(e.to_string()))?;
        if m.schema != MANIFEST_SCHEMA {
            return Err(SnapshotError::Manifest(format!("the schema is {:?}, this version reads {MANIFEST_SCHEMA:?}", m.schema)));
        }
        Ok(m)
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("{0}: not a folder")]
    NotAFolder(String),
    #[error("{0} is inside the package: the copy goes outside it")]
    InsidePackage(String),
    #[error("the package is inside {0}: choose a folder that does not contain it")]
    ContainsPackage(String),
    #[error("{0} already exists: a snapshot never overwrites anything")]
    Exists(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("not a manifest: {0}")]
    Manifest(String),
    #[error("the copy of {0} differs from the original (the file changed while it was copied)")]
    Changed(String),
}

fn io(path: &Path, e: std::io::Error) -> SnapshotError {
    SnapshotError::Io { path: path.to_string_lossy().into_owned(), message: e.to_string() }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn rfc3339(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The regular files under `root` (relative `/` paths, sorted) and the entries that are not files.
fn walk(root: &Path) -> Result<(Vec<String>, Vec<String>), SnapshotError> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let dir = root.join(&rel);
        for e in std::fs::read_dir(&dir).map_err(|e| io(&dir, e))?.flatten() {
            let name = rel.join(e.file_name());
            let text = name.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(name),
                Ok(t) if t.is_file() => files.push(text),
                _ => skipped.push(text),
            }
        }
    }
    files.sort();
    skipped.sort();
    Ok((files, skipped))
}

/// Hashes `path` (and writes the bytes to `out` when given), checking `cancel` between steps.
fn hash_file(path: &Path, mut out: Option<&mut File>, cancel: &AtomicBool, done: &mut u64) -> Result<(u64, String, String), SnapshotError> {
    let mut f = File::open(path).map_err(|e| io(path, e))?;
    let (mut sha256, mut sha1) = (Sha256::new(), cfa4_dcp::sha1::Sha1::new());
    let mut buf = vec![0u8; STEP];
    let mut size = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(SnapshotError::Cancelled);
        }
        let n = f.read(&mut buf).map_err(|e| io(path, e))?;
        if n == 0 {
            break;
        }
        sha256.update(&buf[..n]);
        sha1.update(&buf[..n]);
        if let Some(o) = out.as_deref_mut() {
            o.write_all(&buf[..n]).map_err(|e| io(path, e))?;
        }
        size += n as u64;
        *done += n as u64;
    }
    Ok((size, hex(&sha256.finalize()), hex(&sha1.finish())))
}

/// Progress of a snapshot: bytes done of bytes total, and the file being copied.
pub type Progress<'a> = &'a dyn Fn(u64, u64, &str);

/// Copies the package at `root` to `dest/package/`, writes `dest/manifest.json` and starts
/// `dest/repair-log.jsonl`. `dest` must not exist. The copy is read back and compared with the manifest.
pub fn snapshot(root: &Path, dest: &Path, cancel: &AtomicBool, progress: Progress<'_>) -> Result<Manifest, SnapshotError> {
    let root_canon = root.canonicalize().map_err(|_| SnapshotError::NotAFolder(root.to_string_lossy().into_owned()))?;
    if !root_canon.is_dir() {
        return Err(SnapshotError::NotAFolder(root.to_string_lossy().into_owned()));
    }
    if dest.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        // `a/../b` cannot be placed relative to the package without resolving it on disk first
        return Err(SnapshotError::Io { path: dest.to_string_lossy().into_owned(), message: "the path contains '..'".into() });
    }
    let dest_abs = std::path::absolute(dest).map_err(|e| io(dest, e))?;
    if std::fs::symlink_metadata(&dest_abs).is_ok() {
        return Err(SnapshotError::Exists(dest.to_string_lossy().into_owned()));
    }
    // resolve the nearest existing ancestor, so a link into the package cannot hide the copy inside it
    let anchor = dest_abs.ancestors().find(|a| a.exists()).map(Path::to_path_buf).unwrap_or_default();
    let resolved = anchor.canonicalize().map_err(|e| io(&anchor, e))?.join(dest_abs.strip_prefix(&anchor).unwrap_or(Path::new("")));
    if resolved.starts_with(&root_canon) {
        return Err(SnapshotError::InsidePackage(dest.to_string_lossy().into_owned()));
    }
    if root_canon.starts_with(&resolved) {
        return Err(SnapshotError::ContainsPackage(dest.to_string_lossy().into_owned()));
    }
    let (names, skipped) = walk(&root_canon)?;
    let total: u64 = names.iter().filter_map(|n| std::fs::metadata(root_canon.join(n)).ok()).map(|m| m.len()).sum();
    let pkg = dest_abs.join(PACKAGE_DIR);
    std::fs::create_dir_all(&pkg).map_err(|e| io(&pkg, e))?;
    let mut done = 0u64;
    let mut files = Vec::with_capacity(names.len());
    let result = (|| {
        for name in &names {
            let from = root_canon.join(name);
            let to = pkg.join(name);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
            }
            progress(done, total, name);
            let mut out = File::create_new(&to).map_err(|e| io(&to, e))?;
            let (size, sha256, sha1) = hash_file(&from, Some(&mut out), cancel, &mut done)?;
            out.sync_all().map_err(|e| io(&to, e))?;
            let modified = std::fs::metadata(&from).and_then(|m| m.modified()).ok().map(rfc3339);
            files.push(ManifestFile { path: name.clone(), size, sha256, sha1, modified });
        }
        // read the copy back: it must be the bytes the manifest names
        let mut scratch = 0u64;
        for f in &files {
            let (size, sha256, _) = hash_file(&pkg.join(&f.path), None, cancel, &mut scratch)?;
            if size != f.size || sha256 != f.sha256 {
                return Err(SnapshotError::Changed(f.path.clone()));
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        // a failed or cancelled snapshot leaves nothing that could pass for a copy
        let _ = std::fs::remove_dir_all(&dest_abs);
        return Err(e);
    }
    let manifest = Manifest {
        schema: MANIFEST_SCHEMA.into(),
        created_at: rfc3339(std::time::SystemTime::now()),
        cfa4_version: env!("CARGO_PKG_VERSION").into(),
        root: root.to_string_lossy().into_owned(),
        files,
        skipped,
    };
    let mpath = dest_abs.join(MANIFEST_FILE);
    std::fs::write(&mpath, manifest.to_json()).map_err(|e| io(&mpath, e))?;
    progress(total, total, "");
    Ok(manifest)
}

/// How one file compares with the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileState {
    Same,
    Changed,
    Missing,
    /// In the package, not in the manifest.
    Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileCheck {
    pub path: String,
    pub state: FileState,
}

/// The result of [`verify`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Verification {
    /// Every file is `same` and nothing is extra.
    pub identical: bool,
    pub counts: std::collections::BTreeMap<String, usize>,
    /// Everything that is not `same`, then nothing else (a large package would drown the answer).
    pub differences: Vec<FileCheck>,
    pub checked: usize,
}

/// Compares the package at `root` with `manifest` (size and SHA-256 of every file; extra files named).
pub fn verify(root: &Path, manifest: &Manifest, cancel: &AtomicBool) -> Result<Verification, SnapshotError> {
    let root_canon = root.canonicalize().map_err(|_| SnapshotError::NotAFolder(root.to_string_lossy().into_owned()))?;
    let (names, _) = walk(&root_canon)?;
    let mut differences = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    let mut bump = |s: FileState| *counts.entry(format!("{s:?}").to_lowercase()).or_insert(0usize) += 1;
    let mut scratch = 0u64;
    for f in &manifest.files {
        let state = match std::fs::metadata(root_canon.join(&f.path)) {
            Err(_) => FileState::Missing,
            Ok(m) if !m.is_file() => FileState::Missing,
            Ok(m) if m.len() != f.size => FileState::Changed,
            Ok(_) => match hash_file(&root_canon.join(&f.path), None, cancel, &mut scratch) {
                Ok((_, sha256, _)) if sha256 == f.sha256 => FileState::Same,
                Ok(_) => FileState::Changed,
                Err(SnapshotError::Cancelled) => return Err(SnapshotError::Cancelled),
                Err(_) => FileState::Changed,
            },
        };
        bump(state);
        if state != FileState::Same {
            differences.push(FileCheck { path: f.path.clone(), state });
        }
    }
    let known: std::collections::BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    for n in names.iter().filter(|n| !known.contains(n.as_str())) {
        bump(FileState::Extra);
        differences.push(FileCheck { path: n.clone(), state: FileState::Extra });
    }
    Ok(Verification { identical: differences.is_empty(), counts, differences, checked: manifest.files.len() })
}

/// One line of the repair log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub schema: String,
    /// RFC 3339, UTC.
    pub at: String,
    /// `snapshot` or `reexport`.
    pub action: String,
    pub package: String,
    /// The rule this action answers, when it answers one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// SHA-256 of the manifest of the copy made before the action.
    pub manifest: String,
    /// SHA-256 of the report before and after, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_after: Option<String>,
    /// What was made (a new package folder, for a re-export).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

impl LogEntry {
    pub fn new(action: &str, package: &str, manifest: &Manifest) -> LogEntry {
        LogEntry {
            schema: LOG_SCHEMA.into(),
            at: rfc3339(std::time::SystemTime::now()),
            action: action.into(),
            package: package.into(),
            rule: None,
            manifest: manifest.id(),
            report_before: None,
            report_after: None,
            result: None,
        }
    }
}

/// Adds `entry` as one JSON line to the log at `path`; the log is never rewritten.
pub fn append_log(path: &Path, entry: &LogEntry) -> Result<(), SnapshotError> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| io(path, e))?;
    let line = serde_json::to_string(entry).map_err(|e| SnapshotError::Manifest(e.to_string()))?;
    f.write_all(line.as_bytes()).and_then(|()| f.write_all(b"\n")).map_err(|e| io(path, e))
}

/// The entries of the log at `path`; a line that is no entry is an error, so a damaged log is not trusted.
pub fn read_log(path: &Path) -> Result<Vec<LogEntry>, SnapshotError> {
    let text = std::fs::read_to_string(path).map_err(|e| io(path, e))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<LogEntry>(l).map_err(|e| SnapshotError::Manifest(format!("repair log: {e}"))))
        .collect()
}

/// SHA-256 of a report's JSON: its identity in the repair log.
pub fn report_id(json: &str) -> String {
    hex(&Sha256::digest(json.as_bytes()))
}
