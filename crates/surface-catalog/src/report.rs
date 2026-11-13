//! Hardware test reports (§4): versioned YAML files under `docs/audit/hw/`, the only source of
//! VERIFIED.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::{Error, Problem};

/// The `format` value every report carries.
pub const REPORT_FORMAT: &str = "cfa4-hw-report";
/// The report schema version this crate reads.
pub const REPORT_VERSION: u32 = 1;

/// Where a report comes from (§4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// A wizard-guided test on a real device on the target platform: may verify any feature.
    Cfa4Hil,
    /// An automatic test with recorded USB traffic or a simulated device: protocol-level only.
    Cfa4Ci,
    /// Another project's claim: never VERIFIED, at most EXPERIMENTAL.
    External,
}

/// One feature's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Pass,
    Fail,
    Skipped,
}

/// A hardware test report (§4.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub format: String,
    pub version: u32,
    pub category: Category,
    pub device: ReportDevice,
    pub environment: Environment,
    pub results: Vec<FeatureResult>,
    pub tested_by: String,
    /// ISO date (`2026-11-14`).
    pub date: String,
}

/// The tested device. A raw serial number never appears: the repository is public (§4.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportDevice {
    /// Catalogue model id (`ajazz.akp153e`).
    pub model: String,
    #[serde(default)]
    pub variant: Option<String>,
    pub vid: u16,
    pub pid: u16,
    /// `sha256:` and 64 lower-case hex digits.
    #[serde(default)]
    pub serial_hash: Option<String>,
    #[serde(default)]
    pub firmware: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub os: String,
    pub kernel: String,
    pub session: String,
    pub cfa4_version: String,
    pub adapter: AdapterRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterRef {
    pub id: String,
    /// Semantic version; the major part decides validity (§4.3).
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureResult {
    pub feature: String,
    pub result: Outcome,
    #[serde(default)]
    pub note: Option<String>,
}

impl AdapterRef {
    /// The major version, or `None` when the version is not `<major>[.…]`.
    pub fn major(&self) -> Option<u64> {
        self.version.split('.').next()?.parse().ok()
    }
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

fn valid_serial_hash(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
}

impl Report {
    /// Parses one report. `origin` names it in errors.
    pub fn parse(origin: &str, text: &str) -> Result<Report, Error> {
        let report: Report = serde_saphyr::from_str(text).map_err(|e| Error::Parse { origin: origin.to_owned(), message: e.to_string() })?;
        if report.format != REPORT_FORMAT {
            return Err(Error::Parse { origin: origin.to_owned(), message: format!("format is `{}`, expected `{REPORT_FORMAT}`", report.format) });
        }
        if report.version > REPORT_VERSION {
            return Err(Error::Parse { origin: origin.to_owned(), message: format!("schema version {} is newer than {REPORT_VERSION}", report.version) });
        }
        Ok(report)
    }

    /// Checks the report against the catalogue: known model and USB ids, known features, the
    /// mandatory fields of §4.2 and the hashed serial number.
    pub fn check(&self, catalog: &Catalog) -> Vec<String> {
        let mut problems = Vec::new();
        let Some(model) = catalog.model(&self.device.model) else {
            return vec![format!("unknown model `{}`", self.device.model)];
        };
        if !model.usb.is_empty() && !model.usb.iter().any(|u| u.vid == self.device.vid && u.pid == self.device.pid) {
            problems.push(format!("{:04x}:{:04x} is not a USB id of `{}` in the catalogue", self.device.vid, self.device.pid, model.id));
        }
        if model.adapter.as_deref() != Some(self.environment.adapter.id.as_str()) {
            problems.push(format!("adapter `{}` does not drive `{}`", self.environment.adapter.id, model.id));
        }
        if self.environment.adapter.major().is_none() {
            problems.push(format!("adapter version `{}` is not a version", self.environment.adapter.version));
        }
        if let Some(h) = &self.device.serial_hash
            && !valid_serial_hash(h)
        {
            problems.push("serial_hash must be `sha256:` followed by 64 lower-case hex digits".to_owned());
        }
        if !valid_date(&self.date) {
            problems.push(format!("date `{}` is not YYYY-MM-DD", self.date));
        }
        for (field, value) in [
            ("tested_by", &self.tested_by),
            ("environment.os", &self.environment.os),
            ("environment.kernel", &self.environment.kernel),
            ("environment.session", &self.environment.session),
            ("environment.cfa4_version", &self.environment.cfa4_version),
        ] {
            if value.trim().is_empty() {
                problems.push(format!("{field} is empty"));
            }
        }
        let features = model.declared_features();
        for r in &self.results {
            if !features.contains_key(&r.feature) {
                problems.push(format!("`{}` is not a feature of `{}`", r.feature, model.id));
            }
        }
        if self.results.is_empty() {
            problems.push("no results".to_owned());
        }
        problems
    }
}

/// Reads every `*.yaml` report of a directory (not recursive) and checks it against the catalogue.
pub fn load_reports(dir: &Path, catalog: &Catalog) -> Result<Vec<(String, Report)>, Error> {
    let mut paths = Vec::new();
    if dir.exists() {
        let entries = std::fs::read_dir(dir).map_err(|e| Error::Io { path: dir.display().to_string(), message: e.to_string() })?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::Io { path: dir.display().to_string(), message: e.to_string() })?;
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "yaml" || x == "yml") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for path in paths {
        let origin = path.display().to_string();
        let text = std::fs::read_to_string(&path).map_err(|e| Error::Io { path: origin.clone(), message: e.to_string() })?;
        match Report::parse(&origin, &text) {
            Ok(r) => {
                problems.extend(r.check(catalog).into_iter().map(|p| Problem::new(&origin, p)));
                out.push((origin, r));
            }
            Err(e) => problems.push(Problem::new(&origin, e.to_string())),
        }
    }
    if !problems.is_empty() {
        return Err(Error::Invalid(problems));
    }
    Ok(out)
}
