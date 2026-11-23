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

    /// A self-contained HTML page (no scripts, no external files): the summary, then the findings,
    /// failures first, with the evidence and the remedy. Every value is escaped.
    pub fn to_html(&self) -> String {
        let mut s = String::from(
            "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>DCP validation report</title>\n<style>\
body{font:14px/1.45 system-ui,sans-serif;margin:2rem auto;max-width:72rem;padding:0 1rem;color:#1b1b1b;background:#fff}\
table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{border:1px solid #c8c8c8;padding:.3rem .5rem;text-align:left;vertical-align:top}\
th{background:#f0f0f0}code{font-family:ui-monospace,monospace}.FAIL{background:#fde2e2}.WARNING{background:#fff3cd}.PASS{background:#e3f4e1}\
.verdict{font-size:1.4rem;font-weight:700}@media (prefers-color-scheme:dark){body{color:#e8e8e8;background:#181818}th{background:#2a2a2a;color:#e8e8e8}td,th{border-color:#444}\
.FAIL{background:#5a2323}.WARNING{background:#5a4a1a}.PASS{background:#1f4a1f}}\
</style></head><body>\n",
        );
        let _ = writeln!(s, "<h1>DCP validation</h1>\n<p class=\"verdict {v}\">{v}</p>", v = h(&format!("{:?}", self.verdict).to_uppercase()));
        s.push_str("<table>\n");
        let mut row = |k: &str, v: String| {
            let _ = writeln!(s, "<tr><th>{}</th><td>{}</td></tr>", h(k), v);
        };
        row("Profile", format!("{} (<code>{}</code> v{})", h(&self.profile.title), h(&self.profile.id), self.profile.version));
        row("Backend", format!("<code>{}</code> {}", h(&self.backend.id), h(&self.backend.version)));
        row("Package", format!("<code>{}</code>", h(&self.root)));
        if let Some(st) = self.standard {
            row("Standard", h(&format!("{st:?}")));
        }
        if let Some(id) = self.documents.asset_map {
            row("Asset map", format!("<code>{id}</code>"));
        }
        for id in &self.documents.packing_lists {
            row("Packing list", format!("<code>{id}</code>"));
        }
        for id in &self.documents.compositions {
            row("Composition", format!("<code>{id}</code>"));
        }
        let c = &self.counts;
        row(
            "Results",
            format!(
                "{} pass, {} fail, {} warning, {} unchecked, {} skipped, {} not applicable, {} interrupted",
                c.pass, c.fail, c.warning, c.unchecked, c.skipped, c.not_applicable, c.interrupted
            ),
        );
        row("Time", format!("{} to {}", h(&self.started_at), h(&self.finished_at)));
        s.push_str("</table>\n<table>\n<tr><th>Status</th><th>Rule</th><th>Where</th><th>Message</th><th>How to fix</th></tr>\n");
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
            let mut msg = h(&f.message);
            if let Some(e) = &f.evidence {
                let _ = write!(msg, " <em>({})</em>", h(e));
            }
            let status = h(&format!("{:?}", f.status).to_uppercase());
            let class = if status == "FAIL" || status == "WARNING" || status == "PASS" { status.as_str() } else { "" };
            let _ = writeln!(
                s,
                "<tr class=\"{class}\"><td>{status}</td><td><code>{}</code></td><td>{}</td><td>{msg}</td><td>{}</td></tr>",
                h(&f.rule),
                h(&at),
                f.remedy.as_deref().map(h).unwrap_or_default()
            );
        }
        s.push_str("</table>\n</body></html>\n");
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

/// HTML text: the five special characters escaped.
fn h(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Table-cell text: no pipes, no line breaks.
fn md(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Now, as RFC 3339 UTC with seconds (the report's `started_at` and `finished_at`).
pub fn now() -> String {
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
