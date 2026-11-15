//! Verified delivery of a finished package (CINECERT WP13).
//!
//! [`deliver`] copies a package directory `<src>/NAME` into a target directory:
//!
//! 1. Every file name is checked first ([`check_names`]); a package that would not survive the
//!    copy is refused before anything is written.
//! 2. Files are copied, in sorted order, into `NAME.INCOMPLETE/`. Each one is written to a
//!    temporary name, synced, renamed, then **read back** and its SHA-256 compared with the
//!    source's. A verified file is recorded in `NAME.INCOMPLETE/.cfa4-delivery-progress`.
//! 3. When all files are verified the manifest `NAME.sha256` (`sha256sum` format, sorted, so the
//!    same package always gives the same manifest) is written, the progress file removed, and
//!    `NAME.INCOMPLETE` renamed to `NAME` in one step. A directory called `NAME` is therefore
//!    always complete; anything else carries `.INCOMPLETE`.
//!
//! A failed or cancelled delivery leaves `NAME.INCOMPLETE`. Running [`deliver`] again resumes:
//! recorded files whose copy still matches are kept, everything else is copied again. An existing
//! `NAME` is never overwritten. [`verify`] checks a delivered package against its manifest.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Suffix of a delivery in progress.
pub const INCOMPLETE: &str = ".INCOMPLETE";
/// Progress file inside an incomplete delivery.
pub const PROGRESS: &str = ".cfa4-delivery-progress";
/// Longest file or directory name accepted: the ext2 / ext3 limit, in bytes.
pub const MAX_NAME: usize = 255;

const CHUNK: usize = 1 << 20;

/// A file name that would not survive delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameIssue {
    /// Path relative to the package.
    pub path: String,
    /// What is wrong.
    pub problem: String,
}

/// What a delivery did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The completed package directory.
    pub package: PathBuf,
    /// Its manifest.
    pub manifest: PathBuf,
    /// Files in the package.
    pub files: usize,
    /// Bytes in the package.
    pub bytes: u64,
    /// Files kept from an earlier, interrupted delivery after checking them again.
    pub resumed: usize,
}

/// A file that does not match the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// Path relative to the package.
    pub path: String,
    /// `missing`, `unexpected` (not in the manifest) or `changed`.
    pub kind: &'static str,
}

fn err(what: impl std::fmt::Display, path: &Path) -> Error {
    Error::Delivery(format!("{}: {what}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(|e| err(e, path))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf).map_err(|e| err(e, path))?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

/// The regular files under `root`, relative, `/`-separated, sorted. Symbolic links and other
/// special files are refused: a package is plain files.
pub fn package_files(root: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir).map_err(|e| err(e, dir))? {
            let entry = entry.map_err(|e| err(e, dir))?;
            let path = entry.path();
            let ty = entry.file_type().map_err(|e| err(e, &path))?;
            if ty.is_dir() {
                walk(root, &path, out)?;
            } else if ty.is_file() {
                let rel = path.strip_prefix(root).map_err(|e| err(e, &path))?;
                let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                out.push(parts.join("/"));
            } else {
                return Err(err("not a regular file or directory (symbolic links are not delivered)", &path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

/// File and directory names under `root` that a delivery drive or a cinema server may not take:
/// non-ASCII, a space, a character outside `A-Z a-z 0-9 . _ -`, or longer than [`MAX_NAME`].
pub fn check_names(root: &Path) -> Result<Vec<NameIssue>> {
    let mut issues = Vec::new();
    for rel in package_files(root)? {
        for part in rel.split('/') {
            let problem = if part.len() > MAX_NAME {
                Some(format!("name longer than {MAX_NAME} bytes"))
            } else if !part.is_ascii() {
                Some("non-ASCII name".to_string())
            } else if part.contains(' ') {
                Some("space in name".to_string())
            } else {
                part.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))).map(|c| format!("character {c:?} in name"))
            };
            if let Some(problem) = problem {
                issues.push(NameIssue { path: rel.clone(), problem });
                break;
            }
        }
    }
    Ok(issues)
}

fn read_progress(path: &Path) -> BTreeMap<String, String> {
    let Ok(f) = File::open(path) else { return BTreeMap::new() };
    BufReader::new(f)
        .lines()
        .map_while(std::result::Result::ok)
        .filter_map(|l| l.split_once("  ").filter(|(h, _)| h.len() == 64).map(|(h, p)| (p.to_string(), h.to_string())))
        .collect()
}

/// Copy `src` to `dst`, returning the SHA-256 of what was read; `dst` is synced.
fn copy_hashed(src: &Path, dst: &Path, cancel: &mut dyn FnMut(&str) -> bool, rel: &str) -> Result<String> {
    let mut input = File::open(src).map_err(|e| err(e, src))?;
    let mut out = File::create(dst).map_err(|e| err(e, dst))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if !cancel(rel) {
            return Err(Error::Cancelled);
        }
        let n = input.read(&mut buf).map_err(|e| err(e, src))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| err(e, dst))?;
    }
    out.sync_all().map_err(|e| err(e, dst))?;
    Ok(hex(&h.finalize()))
}

/// Deliver the package directory `src` into `target` (see the module documentation).
pub fn deliver(src: &Path, target: &Path) -> Result<Report> {
    deliver_with(src, target, &mut |_| true)
}

/// [`deliver`], asking `go(relative path)` before each read step; `false` stops with
/// [`Error::Cancelled`], leaving a resumable `NAME.INCOMPLETE`.
pub fn deliver_with(src: &Path, target: &Path, go: &mut dyn FnMut(&str) -> bool) -> Result<Report> {
    let name = src.file_name().and_then(|n| n.to_str()).ok_or_else(|| err("package directory has no usable name", src))?.to_string();
    if !src.is_dir() {
        return Err(err("not a directory", src));
    }
    if !target.is_dir() {
        return Err(err("delivery target is not a directory", target));
    }
    let issues = check_names(src)?;
    if let Some(i) = issues.first() {
        return Err(Error::Delivery(format!("{} file name(s) cannot be delivered, first: {}: {}", issues.len(), i.path, i.problem)));
    }
    if bad_name(&name) {
        return Err(err("package name cannot be delivered", src));
    }
    let final_dir = target.join(&name);
    if final_dir.exists() {
        return Err(err("already delivered; not overwritten", &final_dir));
    }
    let staging = target.join(format!("{name}{INCOMPLETE}"));
    fs::create_dir_all(&staging).map_err(|e| err(e, &staging))?;
    let progress_path = staging.join(PROGRESS);
    let done = read_progress(&progress_path);
    let mut progress = fs::OpenOptions::new().create(true).append(true).open(&progress_path).map_err(|e| err(e, &progress_path))?;

    let files = package_files(src)?;
    let mut manifest = String::new();
    let (mut bytes, mut resumed) = (0u64, 0usize);
    for rel in &files {
        let from = src.join(rel);
        let to = staging.join(rel);
        let source_hash = sha256_file(&from)?;
        bytes = bytes.saturating_add(fs::metadata(&from).map_err(|e| err(e, &from))?.len());
        let kept = done.get(rel) == Some(&source_hash) && to.is_file() && sha256_file(&to)? == source_hash;
        if kept {
            resumed += 1;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|e| err(e, parent))?;
            }
            let tmp = PathBuf::from(format!("{}.cfa4-part", to.display()));
            let read = copy_hashed(&from, &tmp, go, rel)?;
            fs::rename(&tmp, &to).map_err(|e| err(e, &to))?;
            let back = sha256_file(&to)?;
            if read != source_hash || back != source_hash {
                return Err(err("copy does not match the source after reading it back", &to));
            }
            writeln!(progress, "{back}  {rel}").and_then(|()| progress.sync_all()).map_err(|e| err(e, &progress_path))?;
        }
        manifest.push_str(&format!("{source_hash}  {rel}\n"));
    }
    // Leftovers of an earlier attempt that are not part of the package.
    for rel in package_files(&staging)? {
        if rel != PROGRESS && !files.contains(&rel) {
            let p = staging.join(&rel);
            fs::remove_file(&p).map_err(|e| err(e, &p))?;
        }
    }
    drop(progress);
    let manifest_path = target.join(format!("{name}.sha256"));
    let tmp = target.join(format!("{name}.sha256.cfa4-part"));
    fs::write(&tmp, &manifest).and_then(|()| File::open(&tmp)?.sync_all()).map_err(|e| err(e, &tmp))?;
    fs::rename(&tmp, &manifest_path).map_err(|e| err(e, &manifest_path))?;
    fs::remove_file(&progress_path).map_err(|e| err(e, &progress_path))?;
    fs::rename(&staging, &final_dir).map_err(|e| err(e, &final_dir))?;
    if let Ok(dir) = File::open(target) {
        let _ = dir.sync_all();
    }
    Ok(Report { package: final_dir, manifest: manifest_path, files: files.len(), bytes, resumed })
}

fn bad_name(part: &str) -> bool {
    part.is_empty() || part.len() > MAX_NAME || !part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Compare the package directory `package` with its manifest (`sha256sum` format).
/// An empty result means every file is present and unchanged and nothing else is there.
pub fn verify(package: &Path, manifest: &Path) -> Result<Vec<Mismatch>> {
    let text = fs::read_to_string(manifest).map_err(|e| err(e, manifest))?;
    let mut want = BTreeMap::new();
    for line in text.lines() {
        let (h, p) = line.split_once("  ").filter(|(h, _)| h.len() == 64).ok_or_else(|| err(format!("bad manifest line {line:?}"), manifest))?;
        want.insert(p.to_string(), h.to_string());
    }
    let have = package_files(package)?;
    let mut out = Vec::new();
    for (p, h) in &want {
        let path = package.join(p);
        if !path.is_file() {
            out.push(Mismatch { path: p.clone(), kind: "missing" });
        } else if &sha256_file(&path)? != h {
            out.push(Mismatch { path: p.clone(), kind: "changed" });
        }
    }
    out.extend(have.into_iter().filter(|p| !want.contains_key(p)).map(|path| Mismatch { path, kind: "unexpected" }));
    Ok(out)
}
