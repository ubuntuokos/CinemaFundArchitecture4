//! Validation profiles (plan §5): a versioned set of rules, each required or not. The eight
//! built-in profiles are defined here, with every version they had ([`Profile::builtin_version`]);
//! DCP-11 adds the profile file format `cfa4-dcp-profile/1` ([`Profile::from_json`],
//! [`Profile::load`]), which deserialises into the same [`Profile`].

use std::path::Path;

use cfa4_dcp::Standard;
use serde::{Deserialize, Serialize};

use crate::rules::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRule {
    pub rule: String,
    /// A required rule that is not `Pass` / `NotApplicable` keeps the verdict from `PASS`.
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// `quick-scan`, `full-integrity`, …
    pub id: String,
    pub version: u32,
    pub title: String,
    /// The standard every document must have (`DCP.STD.001`); `None` accepts both.
    pub standard: Option<Standard>,
    pub rules: Vec<ProfileRule>,
}

/// The ids of the built-in profiles.
pub const BUILTIN: &[&str] =
    &["quick-scan", "full-integrity", "smpte-cinema", "interop-cinema", "subtitle-accessibility", "audio-quality", "security", "delivery-preflight"];

const QUICK: &[&str] = &[AM_PRESENT, AM_DOCUMENT, AM_PATH_SAFE, AM_FILE_EXISTS, PKL_DOCUMENT, CPL_DOCUMENT];
const INTEGRITY: &[&str] =
    &[AM_UNIQUE_IDS, AM_HAS_PKL, AM_UNLISTED, PKL_IN_ASSETMAP, PKL_SIZE, PKL_HASH, CPL_IN_PKL, CPL_ASSETS_IN_PKL, CPL_DURATIONS, CPL_REEL_SYNC];
const CINEMA: &[&str] = &[STD_CONSISTENT, MXF_TRACK_FILES, J2K_CODESTREAM, PIC_FORMAT, AUD_PCM, MARK_MARKERS];

/// The version history of the built-in profiles: the latest version of each, and what changed.
/// A report names the profile id and version, so an older run can be repeated with
/// [`Profile::builtin_version`].
pub const HISTORY: &[(&str, u32, &str)] = &[
    ("quick-scan", 1, "first version (DCP-03)"),
    ("full-integrity", 1, "first version (DCP-03)"),
    ("full-integrity", 2, "adds DCP.AM.007, unlisted packing lists and compositions (warning)"),
    ("smpte-cinema", 1, "first version (DCP-03)"),
    ("smpte-cinema", 2, "requires DCP.PIC.001 (DCP-06)"),
    ("smpte-cinema", 3, "adds DCP.AM.007, unlisted packing lists and compositions (warning)"),
    ("interop-cinema", 1, "first version (DCP-03)"),
    ("interop-cinema", 2, "requires DCP.PIC.001 (DCP-06)"),
    ("interop-cinema", 3, "adds DCP.AM.007, unlisted packing lists and compositions (warning)"),
    ("subtitle-accessibility", 1, "first version (DCP-03)"),
    ("audio-quality", 1, "first version (DCP-03)"),
    ("security", 1, "first version (DCP-03)"),
    ("delivery-preflight", 1, "first version (DCP-03)"),
    ("delivery-preflight", 2, "requires DCP.PIC.001 (DCP-06)"),
    ("delivery-preflight", 3, "adds DCP.AM.007, unlisted packing lists and compositions (warning)"),
];

/// The profile file format.
pub const PROFILE_FORMAT: &str = "cfa4-dcp-profile/1";

/// Largest profile file read.
pub const MAX_PROFILE_BYTES: u64 = 1 << 20;

/// Why a profile file is not a usable profile.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("cannot read the profile: {0}")]
    Io(#[from] std::io::Error),
    #[error("the profile file is {0} bytes, over the limit of {MAX_PROFILE_BYTES}")]
    TooLarge(u64),
    #[error("not a profile file: {0}")]
    Syntax(String),
    #[error("the format is {0:?}; this version reads {PROFILE_FORMAT:?}")]
    Format(String),
    #[error("invalid profile: {0}")]
    Invalid(String),
}

/// `cfa4-dcp-profile/1` as written.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    format: String,
    id: String,
    version: u32,
    title: String,
    /// `smpte`, `interop` or absent (both).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    standard: Option<String>,
    /// A built-in profile id, optionally `@version`, whose rules come first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extends: Option<String>,
    /// Rules to add, or to change the `required` flag of an inherited one.
    #[serde(default)]
    rules: Vec<ProfileRule>,
    /// Inherited rules to leave out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    remove: Vec<String>,
}

impl Profile {
    /// The latest version of a built-in profile.
    pub fn builtin(id: &str) -> Option<Profile> {
        let latest = HISTORY.iter().filter(|(p, _, _)| *p == id).map(|(_, v, _)| *v).max()?;
        Profile::builtin_version(id, latest)
    }

    /// A built-in profile as it was at `version`.
    pub fn builtin_version(id: &str, version: u32) -> Option<Profile> {
        if !HISTORY.iter().any(|(p, v, _)| *p == id && *v == version) {
            return None;
        }
        // version 1 of the cinema profiles had no DCP.PIC.001
        let cinema: Vec<&str> = CINEMA.iter().copied().filter(|r| version >= 2 || *r != PIC_FORMAT).collect();
        // DCP.AM.007 came with full-integrity v2 and with v3 of the profiles that also had DCP.PIC.001
        let unlisted_from = if id == "full-integrity" { 2 } else { 3 };
        let integrity: Vec<&str> = INTEGRITY.iter().copied().filter(|r| version >= unlisted_from || *r != AM_UNLISTED).collect();
        let (title, standard, required, optional): (&str, Option<Standard>, Vec<&str>, Vec<&str>) = match id {
            "quick-scan" => ("Quick Scan", None, QUICK.to_vec(), vec![]),
            "full-integrity" => ("Full Integrity", None, [QUICK, &integrity].concat(), vec![]),
            "smpte-cinema" => ("SMPTE Cinema", Some(Standard::Smpte), [QUICK, &integrity, &cinema].concat(), vec![IAB_TRACK, SUB_DOCUMENTS, NAME_ISDCF]),
            "interop-cinema" => ("Interop Cinema", Some(Standard::Interop), [QUICK, &integrity, &cinema].concat(), vec![SUB_DOCUMENTS, NAME_ISDCF]),
            "subtitle-accessibility" => ("Subtitle & Accessibility", None, [QUICK, &[SUB_DOCUMENTS]].concat(), vec![]),
            "audio-quality" => ("Audio Quality", None, [QUICK, &[AUD_PCM, AUD_LOUDNESS]].concat(), vec![IAB_TRACK]),
            "security" => ("Security", None, [QUICK, &[SIG_SIGNATURE, SIG_CERTIFICATES]].concat(), vec![]),
            "delivery-preflight" => (
                "Delivery Preflight",
                None,
                [QUICK, &integrity, &cinema].concat(),
                vec![AUD_LOUDNESS, IAB_TRACK, SUB_DOCUMENTS, NAME_ISDCF, SIG_SIGNATURE, SIG_CERTIFICATES],
            ),
            _ => return None,
        };
        let rules = required
            .iter()
            .map(|r| ProfileRule { rule: (*r).to_string(), required: true })
            .chain(optional.iter().map(|r| ProfileRule { rule: (*r).to_string(), required: false }))
            .collect();
        Some(Profile { id: id.to_string(), version, title: title.to_string(), standard, rules })
    }

    /// Reads a `cfa4-dcp-profile/1` file (at most [`MAX_PROFILE_BYTES`]).
    pub fn load(path: &Path) -> Result<Profile, ProfileError> {
        let len = std::fs::metadata(path)?.len();
        if len > MAX_PROFILE_BYTES {
            return Err(ProfileError::TooLarge(len));
        }
        let text = std::fs::read_to_string(path)?;
        Profile::from_json(&text)
    }

    /// A profile from `cfa4-dcp-profile/1` JSON. The result is checked ([`Profile::check`]).
    pub fn from_json(text: &str) -> Result<Profile, ProfileError> {
        let f: ProfileFile = serde_json::from_str(text).map_err(|e| ProfileError::Syntax(e.to_string()))?;
        if f.format != PROFILE_FORMAT {
            return Err(ProfileError::Format(f.format));
        }
        let standard = match f.standard.as_deref() {
            None => None,
            Some("smpte") => Some(Standard::Smpte),
            Some("interop") => Some(Standard::Interop),
            Some(s) => return Err(ProfileError::Invalid(format!("standard {s:?} is not \"smpte\" or \"interop\""))),
        };
        let mut rules = match f.extends.as_deref() {
            None => Vec::new(),
            Some(base) => {
                let (bid, ver) = match base.split_once('@') {
                    Some((b, v)) => (b, Some(v.parse::<u32>().map_err(|_| ProfileError::Invalid(format!("extends {base:?}: the version is not a number")))?)),
                    None => (base, None),
                };
                let p = match ver {
                    Some(v) => Profile::builtin_version(bid, v),
                    None => Profile::builtin(bid),
                };
                p.ok_or_else(|| ProfileError::Invalid(format!("extends {base:?}, which is not a built-in profile")))?.rules
            }
        };
        for r in &f.remove {
            let before = rules.len();
            rules.retain(|x| x.rule != *r);
            if rules.len() == before {
                return Err(ProfileError::Invalid(format!("removes {r}, which it does not have")));
            }
        }
        let mut named = std::collections::BTreeSet::new();
        for r in f.rules {
            if !named.insert(r.rule.clone()) {
                return Err(ProfileError::Invalid(format!("names {} twice", r.rule)));
            }
            match rules.iter_mut().find(|x| x.rule == r.rule) {
                Some(x) => x.required = r.required,
                None => rules.push(r),
            }
        }
        let p = Profile { id: f.id, version: f.version, title: f.title, standard, rules };
        p.check()?;
        Ok(p)
    }

    /// The profile as a self-contained `cfa4-dcp-profile/1` file (no `extends`).
    pub fn to_json(&self) -> String {
        let f = ProfileFile {
            format: PROFILE_FORMAT.into(),
            id: self.id.clone(),
            version: self.version,
            title: self.title.clone(),
            standard: self.standard.map(|s| if s == Standard::Interop { "interop" } else { "smpte" }.to_string()),
            extends: None,
            rules: self.rules.clone(),
            remove: Vec::new(),
        };
        serde_json::to_string_pretty(&f).unwrap_or_default()
    }

    /// A usable profile: an id of lower-case letters, digits and `-` (at most 64), a version of 1
    /// or more, a title, at least one rule, every rule in the catalogue and none twice, and
    /// `DCP.REF.001` (the reference engine's own messages) not among them.
    pub fn check(&self) -> Result<(), ProfileError> {
        let bad = |m: String| Err(ProfileError::Invalid(m));
        let id_ok = !self.id.is_empty() && self.id.len() <= 64 && self.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok {
            return bad(format!("id {:?} is not 1-64 lower-case letters, digits and '-'", self.id));
        }
        if self.version == 0 {
            return bad("version 0; versions start at 1".into());
        }
        if self.title.trim().is_empty() {
            return bad("no title".into());
        }
        if self.rules.is_empty() {
            return bad("no rules".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in &self.rules {
            if crate::rules::find(&r.rule).is_none() {
                return bad(format!("{} is not a rule of the catalogue", r.rule));
            }
            if r.rule == REF_ENGINE {
                return bad(format!("{REF_ENGINE} is the reference engine's own and in no profile"));
            }
            if !seen.insert(r.rule.as_str()) {
                return bad(format!("names {} twice", r.rule));
            }
        }
        Ok(())
    }

    pub fn all() -> Vec<Profile> {
        BUILTIN.iter().filter_map(|id| Profile::builtin(id)).collect()
    }

    pub fn contains(&self, rule: &str) -> bool {
        self.rules.iter().any(|r| r.rule == rule)
    }
}
