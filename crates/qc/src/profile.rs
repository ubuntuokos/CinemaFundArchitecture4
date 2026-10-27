//! QC profiles: which checks run, which of them are mandatory, and the bounds that turn a measured
//! value into PASS, WARN or FAIL.
//!
//! A profile is a small, versioned TOML file. There is no global loudness or picture threshold:
//! every bound comes from the profile chosen for the export's purpose, and the report records the
//! profile's id, version and SHA-256 next to every result.
//!
//! ```toml
//! schema = "cfa4-qc-profile/1"
//! id = "delivery-web"
//! version = 1
//! title = "Web delivery"
//! human_review = "mandatory"
//!
//! [checks."audio.loudness"]
//! requirement = "mandatory"
//! pass = { min = -15.0, max = -13.0 }    # inside: PASS
//! accept = { min = -16.0, max = -12.0 }  # inside but outside `pass`: WARN; outside: FAIL
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{self, Measure};
use crate::{Error, hash};

/// Schema id every profile file must carry.
pub const PROFILE_SCHEMA: &str = "cfa4-qc-profile/1";
/// Largest profile file read (a hostile or corrupted file cannot use unbounded memory).
pub const MAX_PROFILE_BYTES: usize = 64 * 1024;
/// Longest profile id.
pub const MAX_ID_LEN: usize = 64;

/// The profiles CFA4 ships, as (id, TOML source).
pub const BUILTIN: &[(&str, &str)] = &[("delivery-web", include_str!("../profiles/delivery-web.toml"))];

/// Whether a check (or the human review) must have a result for the QC to pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Requirement {
    #[default]
    Mandatory,
    Optional,
}

/// A closed range; a missing side is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl Bounds {
    pub fn contains(&self, v: f64) -> bool {
        v.is_finite() && self.min.is_none_or(|m| v >= m) && self.max.is_none_or(|m| v <= m)
    }

    /// `self` lies inside `outer`.
    fn within(&self, outer: &Bounds) -> bool {
        let lo = match (self.min, outer.min) {
            (_, None) => true,
            (Some(a), Some(b)) => a >= b,
            (None, Some(_)) => false,
        };
        let hi = match (self.max, outer.max) {
            (_, None) => true,
            (Some(a), Some(b)) => a <= b,
            (None, Some(_)) => false,
        };
        lo && hi
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.min.is_some_and(|v| !v.is_finite()) || self.max.is_some_and(|v| !v.is_finite()) {
            return Err("bounds must be finite numbers");
        }
        if let (Some(a), Some(b)) = (self.min, self.max)
            && a > b
        {
            return Err("min is greater than max");
        }
        Ok(())
    }
}

/// One check in a profile.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSpec {
    #[serde(default)]
    pub requirement: Requirement,
    /// Values inside are PASS. Required for checks that measure a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<Bounds>,
    /// Values inside but outside `pass` are WARN, outside FAIL. Without it, outside `pass` is FAIL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Bounds>,
}

/// A parsed, validated profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QcProfile {
    pub schema: String,
    pub id: String,
    pub version: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Whether the guided human review must be filled in before the QC can pass.
    #[serde(default)]
    pub human_review: Requirement,
    #[serde(default)]
    pub checks: BTreeMap<String, CheckSpec>,
    /// SHA-256 of the TOML source this profile was read from.
    #[serde(skip)]
    pub sha256: String,
}

/// What a report says about the profile it used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRef {
    pub id: String,
    pub version: u32,
    pub sha256: String,
}

fn check_id(s: &str) -> Result<(), Error> {
    let ok = !s.is_empty() && s.len() <= MAX_ID_LEN && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    if ok { Ok(()) } else { Err(Error::InvalidProfile(format!("invalid id `{}`", s.chars().take(MAX_ID_LEN).collect::<String>()))) }
}

impl QcProfile {
    /// Parse and validate a profile file.
    pub fn from_toml(text: &str) -> Result<QcProfile, Error> {
        if text.len() > MAX_PROFILE_BYTES {
            return Err(Error::InvalidProfile(format!("larger than {MAX_PROFILE_BYTES} bytes")));
        }
        let mut p: QcProfile = toml::from_str(text).map_err(|e| Error::InvalidProfile(e.to_string()))?;
        p.sha256 = hash::sha256_hex(text.as_bytes());
        p.validate()?;
        Ok(p)
    }

    /// A built-in profile by id.
    pub fn builtin(id: &str) -> Result<QcProfile, Error> {
        let (_, text) = BUILTIN.iter().find(|(i, _)| *i == id).ok_or_else(|| Error::UnknownProfile(id.chars().take(MAX_ID_LEN).collect()))?;
        QcProfile::from_toml(text)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != PROFILE_SCHEMA {
            return Err(Error::InvalidProfile(format!("schema must be `{PROFILE_SCHEMA}`")));
        }
        check_id(&self.id)?;
        if self.version == 0 {
            return Err(Error::InvalidProfile("version starts at 1".into()));
        }
        if self.checks.is_empty() {
            return Err(Error::InvalidProfile("no checks".into()));
        }
        for (id, spec) in &self.checks {
            let bad = |why: &str| Error::InvalidProfile(format!("check `{id}`: {why}"));
            let info = catalog::check(id).ok_or_else(|| bad("unknown check id"))?;
            for b in [&spec.pass, &spec.accept].into_iter().flatten() {
                b.validate().map_err(bad)?;
            }
            match info.measure {
                Measure::Value => {
                    let Some(pass) = &spec.pass else { return Err(bad("needs `pass` bounds")) };
                    if spec.accept.as_ref().is_some_and(|a| !pass.within(a)) {
                        return Err(bad("`pass` must lie inside `accept`"));
                    }
                }
                Measure::Findings => {
                    if spec.pass.is_some() || spec.accept.is_some() {
                        return Err(bad("this check reports findings, not a value: no bounds"));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn reference(&self) -> ProfileRef {
        ProfileRef { id: self.id.clone(), version: self.version, sha256: self.sha256.clone() }
    }

    /// The mandatory checks, in catalogue order.
    pub fn mandatory(&self) -> impl Iterator<Item = &str> {
        catalog::CHECKS.iter().map(|c| c.id).filter(|id| self.checks.get(*id).is_some_and(|s| s.requirement == Requirement::Mandatory))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
schema = "cfa4-qc-profile/1"
id = "t"
version = 1
title = "Test"
[checks."audio.loudness"]
pass = { min = -15.0, max = -13.0 }
accept = { min = -16.0, max = -12.0 }
[checks."integrity.decode"]
requirement = "optional"
"#;

    #[test]
    fn builtins_parse() {
        for (id, _) in BUILTIN {
            let p = QcProfile::builtin(id).unwrap();
            assert_eq!(&p.id, id);
            assert_eq!(p.sha256.len(), 64);
        }
    }

    #[test]
    fn minimal_parses_with_defaults() {
        let p = QcProfile::from_toml(MINIMAL).unwrap();
        assert_eq!(p.human_review, Requirement::Mandatory);
        assert_eq!(p.mandatory().collect::<Vec<_>>(), ["audio.loudness"]);
    }

    fn rejects(text: &str, what: &str) {
        match QcProfile::from_toml(text) {
            Err(Error::InvalidProfile(m)) => assert!(m.contains(what), "{m}"),
            other => panic!("expected an error with `{what}`, got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_profiles() {
        rejects(&MINIMAL.replace("audio.loudness", "audio.loudnes"), "unknown check id");
        rejects(&MINIMAL.replace("cfa4-qc-profile/1", "cfa4-qc-profile/9"), "schema");
        rejects(&MINIMAL.replace("version = 1", "version = 0"), "version");
        rejects(&MINIMAL.replace("id = \"t\"", "id = \"Bad Id\""), "invalid id");
        rejects(&MINIMAL.replace("accept = { min = -16.0, max = -12.0 }", "accept = { min = -14.0, max = -12.0 }"), "inside");
        rejects(&MINIMAL.replace("pass = { min = -15.0, max = -13.0 }", "pass = { min = -13.0, max = -15.0 }"), "greater");
        rejects(&MINIMAL.replace("pass = { min = -15.0, max = -13.0 }\n", ""), "needs `pass`");
        rejects(&format!("{MINIMAL}pass = {{ max = 1.0 }}\n"), "no bounds");
        rejects(&MINIMAL.replace("title", "titel"), "unknown field");
        rejects(&"#".repeat(MAX_PROFILE_BYTES + 1), "larger");
        rejects("not toml at all ===", "");
    }

    #[test]
    fn bounds() {
        let b = Bounds { min: Some(-1.0), max: None };
        assert!(b.contains(-1.0) && b.contains(1e9) && !b.contains(-1.5) && !b.contains(f64::NAN));
    }
}
