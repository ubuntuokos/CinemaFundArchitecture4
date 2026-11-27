//! ISDCF metadata registries and the Digital Cinema Naming Convention (CINECERT WP09).
//!
//! The registries (<https://github.com/ISDCF/registries>, CC BY 4.0, © InterSociety Digital
//! Cinema Forum) are carried as an offline snapshot, `data/isdcf-registry.json`: the codes and
//! names the naming convention uses, reduced from one pinned upstream commit by [`reduce`], with
//! the commit and the SHA-256 of every upstream file it was made from. Nothing is fetched at run
//! time; `scripts/dev/update-isdcf-snapshot.sh` refreshes the snapshot on explicit request.
//!
//! The rating registry is not in the snapshot: it is derived from MovieLabs data whose licence is
//! not stated, so ratings are checked for form only.
//!
//! [`dcnc`] checks and builds names. The convention is a convention, not a standard: every
//! finding is a warning for the operator, never a reason to refuse a package.

pub mod dcnc;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Upstream repository of the registries.
pub const UPSTREAM: &str = "https://github.com/ISDCF/registries";

/// The upstream files the snapshot is made from, in `src/main/data/`.
pub const UPSTREAM_FILES: [&str; 8] =
    ["contenttypes", "contentmodifiers", "projectoraspectratios", "languages", "territories", "audioconfigs", "studios", "facilities"];

const SNAPSHOT: &str = include_str!("../../data/isdcf-registry.json");

/// One registry entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The code used in names (`FTR`, `EN`, `51`, a studio code…).
    pub code: String,
    /// What it stands for.
    pub name: String,
    /// Earlier codes for the same entry, no longer to be used (`ENG` for `EN`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obsolete_codes: Vec<String>,
    /// The entry itself is obsolete.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub obsolete: bool,
    /// The code that replaces an obsolete entry, when the registry names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
}

/// Where a snapshot comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Upstream repository.
    pub repository: String,
    /// Upstream commit.
    pub commit: String,
    /// Commit date, `YYYY-MM-DD`.
    pub date: String,
    /// Licence of the data (SPDX).
    pub license: String,
    /// SHA-256 of each upstream file read, by file name.
    pub files: BTreeMap<String, String>,
}

/// The registries the naming convention uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    /// Provenance.
    pub source: Source,
    /// Content types (`FTR`, `TLR`…), in registry order.
    pub content_types: Vec<Entry>,
    /// Content type modifiers in their required order; variable ones are written `[FPS]` etc.
    pub content_modifiers: Vec<Entry>,
    /// Projector aspect ratios (`F`, `S`, `C`).
    pub aspect_ratios: Vec<Entry>,
    /// Audio configurations and audio modifiers (`51`, `IAB`, `HI`…).
    pub audio: Vec<Entry>,
    /// Languages (`EN`, `LAS`…).
    pub languages: Vec<Entry>,
    /// Release territories (`US`, `INT`…).
    pub territories: Vec<Entry>,
    /// Studio codes.
    pub studios: Vec<Entry>,
    /// Facility codes.
    pub facilities: Vec<Entry>,
}

/// What a lookup found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// A current code.
    Current(&'a Entry),
    /// An obsolete code; the entry holds the current code or the replacement, if known.
    Obsolete { entry: &'a Entry, use_instead: Option<String> },
    /// Not in the registry.
    Unknown,
}

impl Registry {
    /// The snapshot built into this crate.
    pub fn builtin() -> Result<&'static Registry> {
        static REG: OnceLock<std::result::Result<Registry, String>> = OnceLock::new();
        REG.get_or_init(|| serde_json::from_str(SNAPSHOT).map_err(|e| e.to_string())).as_ref().map_err(|e| Error::Registry(e.clone()))
    }

    /// Look `code` up in `list`, ignoring ASCII case.
    pub fn lookup<'a>(list: &'a [Entry], code: &str) -> Lookup<'a> {
        if let Some(e) = list.iter().find(|e| e.code.eq_ignore_ascii_case(code)) {
            if e.obsolete {
                return Lookup::Obsolete { entry: e, use_instead: e.replaced_by.clone() };
            }
            return Lookup::Current(e);
        }
        if let Some(e) = list.iter().find(|e| e.obsolete_codes.iter().any(|c| c.eq_ignore_ascii_case(code))) {
            return Lookup::Obsolete { entry: e, use_instead: Some(e.code.clone()) };
        }
        Lookup::Unknown
    }
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

/// Reduce an upstream checkout (`dir` is the repository root) at `commit`, dated `date`, to the
/// snapshot's JSON text. Contact details and everything the naming convention does not use are
/// left out.
pub fn reduce(dir: &Path, commit: &str, date: &str) -> Result<String> {
    let mut files = BTreeMap::new();
    let mut data = BTreeMap::new();
    for name in UPSTREAM_FILES {
        let path = dir.join("src/main/data").join(format!("{name}.json"));
        let bytes = std::fs::read(&path).map_err(|e| Error::Registry(format!("{}: {e}", path.display())))?;
        files.insert(format!("{name}.json"), Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect());
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| Error::Registry(format!("{name}.json: {e}")))?;
        data.insert(name, v.as_array().cloned().ok_or_else(|| Error::Registry(format!("{name}.json: not an array")))?);
    }
    let get = |name: &str| data.get(name).cloned().unwrap_or_default();
    let coded = |name: &str| -> Vec<Entry> {
        get(name)
            .iter()
            .filter_map(|v| {
                Some(Entry {
                    code: s(v, "dcncCode")?,
                    name: s(v, "description").unwrap_or_default(),
                    obsolete_codes: Vec::new(),
                    obsolete: false,
                    replaced_by: None,
                })
            })
            .collect()
    };
    let tagged = |name: &str, label: &str| -> Vec<Entry> {
        get(name)
            .iter()
            .filter_map(|v| {
                Some(Entry {
                    code: s(v, "dcncTag")?,
                    name: s(v, label).unwrap_or_default(),
                    obsolete_codes: strings(v, "obsoleteDCNCTags"),
                    obsolete: false,
                    replaced_by: None,
                })
            })
            .collect()
    };
    let orgs = |name: &str| -> Vec<Entry> {
        get(name)
            .iter()
            .filter_map(|v| {
                Some(Entry {
                    code: s(v, "code")?,
                    name: s(v, "description").unwrap_or_default(),
                    obsolete_codes: Vec::new(),
                    obsolete: v.get("obsolete").and_then(Value::as_bool).unwrap_or(false),
                    replaced_by: s(v, "obsoletedBy"),
                })
            })
            .collect()
    };
    let reg = Registry {
        source: Source { repository: UPSTREAM.to_string(), commit: commit.to_string(), date: date.to_string(), license: "CC-BY-4.0".to_string(), files },
        content_types: coded("contenttypes"),
        content_modifiers: coded("contentmodifiers"),
        aspect_ratios: coded("projectoraspectratios"),
        audio: coded("audioconfigs"),
        languages: tagged("languages", "dcncLanguage"),
        territories: tagged("territories", "dcncTerritory"),
        studios: orgs("studios"),
        facilities: orgs("facilities"),
    };
    serde_json::to_string_pretty(&reg).map(|t| t + "\n").map_err(|e| Error::Registry(e.to_string()))
}
