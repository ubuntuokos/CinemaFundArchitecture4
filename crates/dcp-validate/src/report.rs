//! The report every backend produces (`cfa4-dcp-report/1`), its verdict and the JSON and Markdown
//! forms. HTML (DCP-15) renders from the same model.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use cfa4_dcp::{Standard, Uuid};
use serde::{Deserialize, Serialize};

use crate::{Finding, Profile, Status};

pub const SCHEMA: &str = "cfa4-dcp-report/1";

/// The package verdict, a separate field from the findings (plan §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    /// Every required rule of the profile is `Pass` or `NotApplicable`.
    Pass,
    /// A required rule failed.
    Fail,
    /// No required rule failed, but one was `Skipped` or `Unchecked`.
    Incomplete,
    /// The validation was cancelled.
    Interrupted,
}

impl Verdict {
    /// The verdict of `findings` under `profile`. A required rule with no finding at all counts as
    /// `Unchecked`. Optional rules never change the verdict.
    pub fn of(profile: &Profile, findings: &[Finding]) -> Verdict {
        if findings.iter().any(|f| f.status == Status::Interrupted) {
            return Verdict::Interrupted;
        }
        let mut fail = false;
        let mut incomplete = false;
        for pr in &profile.rules {
            let worst = findings.iter().filter(|f| f.rule == pr.rule).map(|f| f.status).max();
            if !pr.required {
                continue;
            }
            match worst {
                Some(Status::Fail) => fail = true,
                Some(Status::Pass | Status::NotApplicable | Status::Warning) => {}
                Some(Status::Skipped | Status::Unchecked | Status::Interrupted) | None => incomplete = true,
            }
        }
        if fail {
            Verdict::Fail
        } else if incomplete {
            Verdict::Incomplete
        } else {
            Verdict::Pass
        }
    }
}

/// Ids of the documents that were read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIds {
    pub asset_map: Option<Uuid>,
    pub packing_lists: Vec<Uuid>,
    pub compositions: Vec<Uuid>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub pass: usize,
    pub fail: usize,
    pub warning: usize,
    pub skipped: usize,
    pub unchecked: usize,
    pub not_applicable: usize,
    pub interrupted: usize,
}

impl Counts {
    pub fn of(findings: &[Finding]) -> Counts {
        let mut c = Counts::default();
        for f in findings {
            *match f.status {
                Status::Pass => &mut c.pass,
                Status::Fail => &mut c.fail,
                Status::Warning => &mut c.warning,
                Status::Skipped => &mut c.skipped,
                Status::Unchecked => &mut c.unchecked,
                Status::NotApplicable => &mut c.not_applicable,
                Status::Interrupted => &mut c.interrupted,
            } += 1;
        }
        c
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub id: String,
    pub version: u32,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub schema: String,
    pub verdict: Verdict,
    pub profile: ProfileInfo,
    pub backend: BackendInfo,
    /// The package root as given.
    pub root: String,
    /// The standard of the asset map, when one was read.
    pub standard: Option<Standard>,
    pub documents: DocumentIds,
    /// Files the backend looked at, relative to the root.
    pub assets_checked: usize,
    pub counts: Counts,
    pub findings: Vec<Finding>,
    /// RFC 3339, UTC.
    pub started_at: String,
    pub finished_at: String,
}

impl ValidationReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<ValidationReport> {
        let r: ValidationReport = serde_json::from_str(text).ok()?;
        (r.schema == SCHEMA).then_some(r)
    }

    /// Markdown for developer and audit documentation: the summary, then one table row per finding,
    /// failures first.
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# DCP validation: {:?}\n", self.verdict);
        let _ = writeln!(s, "| | |\n|---|---|");
        let _ = writeln!(s, "| Profile | {} (`{}` v{}) |", self.profile.title, self.profile.id, self.profile.version);
        let _ = writeln!(s, "| Backend | `{}` {} |", self.backend.id, self.backend.version);
        let _ = writeln!(s, "| Package | `{}` |", md(&self.root));
        if let Some(st) = self.standard {
            let _ = writeln!(s, "| Standard | {st:?} |");
        }
        if let Some(id) = self.documents.asset_map {
            let _ = writeln!(s, "| Asset map | `{id}` |");
        }
        for id in &self.documents.packing_lists {
            let _ = writeln!(s, "| Packing list | `{id}` |");
        }
        for id in &self.documents.compositions {
            let _ = writeln!(s, "| Composition | `{id}` |");
        }
        let c = &self.counts;
        let _ = writeln!(
            s,
            "| Results | {} pass, {} fail, {} warning, {} unchecked, {} skipped, {} not applicable, {} interrupted |",
            c.pass, c.fail, c.warning, c.unchecked, c.skipped, c.not_applicable, c.interrupted
        );
        let _ = writeln!(s, "| Time | {} to {} |\n", self.started_at, self.finished_at);
        let _ = writeln!(s, "| Status | Rule | Where | Message |\n|---|---|---|---|");
        let mut sorted: Vec<&Finding> = self.findings.iter().collect();
        sorted.sort_by(|a, b| b.status.cmp(&a.status).then_with(|| a.rule.cmp(&b.rule)));
        for f in sorted {
            let mut at = f.location.file.clone().unwrap_or_default();
            if let Some(l) = f.location.line {
                let _ = write!(at, ":{l}");
            }
            if let Some(e) = &f.location.element {
                let _ = write!(at, " {e}");
            }
            let mut msg = md(&f.message);
            if let Some(e) = &f.evidence {
                let _ = write!(msg, " ({})", md(e));
            }
            let _ = writeln!(s, "| {:?} | `{}` | {} | {} |", f.status, f.rule, md(&at), msg);
        }
        s
    }

    /// Findings grouped by rule, for the GUI and the QC mapping.
    pub fn by_rule(&self) -> BTreeMap<&str, Vec<&Finding>> {
        let mut m: BTreeMap<&str, Vec<&Finding>> = BTreeMap::new();
        for f in &self.findings {
            m.entry(f.rule.as_str()).or_default().push(f);
        }
        m
    }
}

/// Table-cell text: no pipes, no line breaks.
fn md(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Now, as RFC 3339 UTC with seconds.
pub(crate) fn now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    rfc3339(secs)
}

/// Seconds since 1970 as `YYYY-MM-DDTHH:MM:SSZ` (calendar math by `chrono`).
pub(crate) fn rfc3339(secs: u64) -> String {
    i64::try_from(secs)
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map_or_else(|| "1970-01-01T00:00:00Z".to_string(), |t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}
