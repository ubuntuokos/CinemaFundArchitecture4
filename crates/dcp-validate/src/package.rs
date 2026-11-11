//! Package discovery (plan DCP-F01) and safe path resolution (plan §9): the asset map of a package
//! root, and the files its chunks name, never outside the root.

use std::path::{Component, Path, PathBuf};

/// The asset map names, in the order they are tried: SMPTE (`ASSETMAP.xml`), then Interop.
pub const ASSET_MAP_NAMES: &[&str] = &["ASSETMAP.xml", "ASSETMAP"];

/// The asset map files present in `root` (regular files only).
pub fn asset_maps(root: &Path) -> Vec<&'static str> {
    ASSET_MAP_NAMES.iter().copied().filter(|n| std::fs::symlink_metadata(root.join(n)).is_ok_and(|m| m.is_file())).collect()
}

/// Where a chunk path leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// A regular file inside the package.
    File(PathBuf),
    /// Nothing at that path.
    Missing,
    /// Something that is not a regular file (a directory, a device, a dangling link).
    NotAFile,
    /// Refused without touching the file system, or because a link leads outside: the reason.
    Unsafe(String),
}

/// Resolves a chunk `Path` against the canonical package `root`. Absolute paths, drive prefixes,
/// `..` components and backslashes are refused before the file system is touched; symbolic links
/// are followed only when the target stays inside `root`.
pub fn resolve(root: &Path, path: &str) -> Resolved {
    if path.is_empty() {
        return Resolved::Unsafe("the path is empty".into());
    }
    if path.contains('\\') {
        return Resolved::Unsafe("the path contains a backslash (paths use '/')".into());
    }
    if path.contains('\0') {
        return Resolved::Unsafe("the path contains a NUL character".into());
    }
    for c in Path::new(path).components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Resolved::Unsafe("the path contains '..'".into()),
            Component::RootDir | Component::Prefix(_) => return Resolved::Unsafe("the path is absolute".into()),
        }
    }
    let full = root.join(path);
    if std::fs::symlink_metadata(&full).is_err() {
        return Resolved::Missing;
    }
    let Ok(canon) = full.canonicalize() else { return Resolved::NotAFile };
    if !canon.starts_with(root) {
        return Resolved::Unsafe("a symbolic link leads outside the package".into());
    }
    match std::fs::metadata(&canon) {
        Ok(m) if m.is_file() => Resolved::File(canon),
        _ => Resolved::NotAFile,
    }
}
