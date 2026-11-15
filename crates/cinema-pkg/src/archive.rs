//! Provenance report and BagIt archive packaging (CINECERT WP13).
//!
//! [`Provenance`] records how a package came to be: the source assets and their hashes, the
//! profile and tool versions, the transcoder, the QC results and external validators with their
//! versions, and when each happened. It is plain JSON, written by the caller's export or archive
//! step.
//!
//! [`make_bag`] puts a finished package into a BagIt bag (RFC 8493, version 1.0) for archiving:
//! the package under `data/`, SHA-256 payload and tag manifests, `bag-info.txt` with the
//! `Payload-Oxum`, and the provenance report as an extra tag file. The payload is copied with
//! [`crate::delivery`], so it is read back and verified, and the bag is built as
//! `BAG.INCOMPLETE` and renamed when complete. [`validate_bag`] checks a bag.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::delivery::{self, INCOMPLETE};
use crate::{Error, Result};

/// Name of the provenance tag file in a bag.
pub const PROVENANCE_FILE: &str = "cfa4-provenance.json";

/// A file and its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHash {
    /// Path or name, as the caller knows it.
    pub path: String,
    /// SHA-256, lower-case hex.
    pub sha256: String,
}

/// A program that took part, with its version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    /// Name (`cfa4`, `asdcplib`, `Photon`…).
    pub name: String,
    /// Version as the tool reports it.
    pub version: String,
}

/// One QC or validation run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// The validator.
    pub tool: Tool,
    /// Profile or rule set used.
    pub profile: String,
    /// `pass`, `warn` or `fail`.
    pub result: String,
    /// When it ran (RFC 3339).
    pub at: String,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
}

/// How a package came to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    /// Package name (the directory name).
    pub package: String,
    /// When the package was made (RFC 3339).
    pub created: String,
    /// The program that made it.
    pub producer: Tool,
    /// Export profile or preset, with its version.
    pub profile: String,
    /// Encoders and transcoders used.
    pub transcoders: Vec<Tool>,
    /// Source assets.
    pub sources: Vec<FileHash>,
    /// QC and validation runs, internal and external.
    pub checks: Vec<Check>,
}

impl Provenance {
    /// Check the timestamps and hashes, then give the JSON text.
    pub fn to_json(&self) -> Result<String> {
        let times = std::iter::once(&self.created).chain(self.checks.iter().map(|c| &c.at));
        if let Some(t) = times.into_iter().find(|t| DateTime::parse_from_rfc3339(t).is_err()) {
            return Err(Error::Delivery(format!("provenance: {t:?} is not an RFC 3339 time")));
        }
        if let Some(s) = self.sources.iter().find(|s| s.sha256.len() != 64 || !s.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) {
            return Err(Error::Delivery(format!("provenance: {} has no lower-case SHA-256", s.path)));
        }
        serde_json::to_string_pretty(self).map(|t| t + "\n").map_err(|e| Error::Delivery(e.to_string()))
    }
}

/// What [`make_bag`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bag {
    /// The bag directory.
    pub path: PathBuf,
    /// Payload files.
    pub files: usize,
    /// Payload bytes.
    pub bytes: u64,
}

fn err(what: impl std::fmt::Display, path: &Path) -> Error {
    Error::Delivery(format!("{}: {what}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| err(e, path))
}

/// Hash every file under `dir`, relative paths prefixed with `prefix`, sorted.
fn manifest(dir: &Path, prefix: &str) -> Result<(String, usize, u64)> {
    let mut text = String::new();
    let (mut n, mut bytes) = (0usize, 0u64);
    for rel in delivery::package_files(dir)? {
        let p = dir.join(&rel);
        text.push_str(&format!("{}  {prefix}{rel}\n", delivery::sha256_file(&p)?));
        n += 1;
        bytes = bytes.saturating_add(fs::metadata(&p).map_err(|e| err(e, &p))?.len());
    }
    Ok((text, n, bytes))
}

/// Bag the package directory `package` as `bag_dir` (which must not exist). `date` is the
/// Bagging-Date, `provenance` the report to include, `description` the External-Description.
pub fn make_bag(package: &Path, bag_dir: &Path, date: NaiveDate, provenance: &Provenance, description: &str) -> Result<Bag> {
    if bag_dir.exists() {
        return Err(err("already exists; not overwritten", bag_dir));
    }
    let prov = provenance.to_json()?;
    if description.contains(['\n', '\r']) {
        return Err(Error::Delivery("bag description must be one line".into()));
    }
    let name = bag_dir.file_name().and_then(|n| n.to_str()).ok_or_else(|| err("bag directory has no usable name", bag_dir))?;
    let staging = bag_dir.with_file_name(format!("{name}{INCOMPLETE}"));
    let data = staging.join("data");
    fs::create_dir_all(&data).map_err(|e| err(e, &data))?;
    let pkg_name = package.file_name().and_then(|n| n.to_str()).ok_or_else(|| err("package directory has no usable name", package))?;
    if !data.join(pkg_name).is_dir() {
        delivery::deliver(package, &data)?;
    }
    // The delivery manifest is not payload (also after a resumed run).
    let delivered_manifest = data.join(format!("{pkg_name}.sha256"));
    if delivered_manifest.exists() {
        fs::remove_file(&delivered_manifest).map_err(|e| err(e, &delivered_manifest))?;
    }
    // Hashing the payload again reads every copied byte back once more.
    let (payload, files, bytes) = manifest(&data, "data/")?;
    write(&staging.join("bagit.txt"), "BagIt-Version: 1.0\nTag-File-Character-Encoding: UTF-8\n")?;
    write(&staging.join("manifest-sha256.txt"), &payload)?;
    let info = format!(
        "Bagging-Date: {}\nBag-Software-Agent: {} {}\nExternal-Description: {description}\nPayload-Oxum: {bytes}.{files}\n",
        date.format("%Y-%m-%d"),
        provenance.producer.name,
        provenance.producer.version
    );
    write(&staging.join("bag-info.txt"), &info)?;
    write(&staging.join(PROVENANCE_FILE), &prov)?;
    let mut tags = String::new();
    for f in ["bag-info.txt", "bagit.txt", PROVENANCE_FILE, "manifest-sha256.txt"] {
        tags.push_str(&format!("{}  {f}\n", delivery::sha256_file(&staging.join(f))?));
    }
    write(&staging.join("tagmanifest-sha256.txt"), &tags)?;
    fs::rename(&staging, bag_dir).map_err(|e| err(e, bag_dir))?;
    Ok(Bag { path: bag_dir.to_path_buf(), files, bytes })
}

fn parse_manifest(text: &str, path: &Path) -> Result<BTreeMap<String, String>> {
    text.lines()
        .map(|l| {
            l.split_once("  ")
                .filter(|(h, _)| h.len() == 64)
                .map(|(h, p)| (p.to_string(), h.to_string()))
                .ok_or_else(|| err(format!("bad manifest line {l:?}"), path))
        })
        .collect()
}

/// Check a bag: declaration, every payload and tag file against its manifest, no payload file
/// missing from the manifest, and the Payload-Oxum. Returns the problems found; empty means valid.
pub fn validate_bag(bag: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let read = |f: &str| fs::read_to_string(bag.join(f)).map_err(|e| err(e, &bag.join(f)));
    let decl = read("bagit.txt")?;
    if !decl.starts_with("BagIt-Version: ") || !decl.contains("Tag-File-Character-Encoding: UTF-8") {
        out.push("bagit.txt: not a BagIt declaration".to_string());
    }
    for (file, base) in [("manifest-sha256.txt", "data/"), ("tagmanifest-sha256.txt", "")] {
        let entries = parse_manifest(&read(file)?, &bag.join(file))?;
        for (p, h) in &entries {
            if !p.starts_with(base) || !delivery::safe_relative(p) {
                out.push(format!("{file}: path {p} outside its place"));
                continue;
            }
            let f = bag.join(p);
            if !f.is_file() {
                out.push(format!("{p}: missing"));
            } else if &delivery::sha256_file(&f)? != h {
                out.push(format!("{p}: changed"));
            }
        }
        if base == "data/" {
            let data = bag.join("data");
            let have = delivery::package_files(&data)?;
            out.extend(have.iter().filter(|p| !entries.contains_key(&format!("data/{p}"))).map(|p| format!("data/{p}: not in the manifest")));
            let bytes: u64 = have.iter().filter_map(|p| fs::metadata(data.join(p)).ok()).map(|m| m.len()).sum();
            let oxum = format!("{bytes}.{}", have.len());
            let info = read("bag-info.txt")?;
            match info.lines().find_map(|l| l.strip_prefix("Payload-Oxum: ")) {
                Some(o) if o.trim() == oxum => {}
                Some(o) => out.push(format!("bag-info.txt: Payload-Oxum {} but the payload is {oxum}", o.trim())),
                None => out.push("bag-info.txt: no Payload-Oxum".to_string()),
            }
        }
    }
    Ok(out)
}
