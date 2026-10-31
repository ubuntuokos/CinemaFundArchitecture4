//! The QC report (`cfa4-qc-report/1`): what was checked, with which profile and tools, what was
//! measured against which bounds, what a person noted, and the verdict.
//!
//! The verdict is computed here and nowhere else ([`QcReport::assemble`]), from rules that never
//! let a missing, skipped or failed mandatory result pass:
//!
//! 1. a mandatory check FAIL, or a critical human finding → [`Verdict::Fail`];
//! 2. otherwise a mandatory check NOT_RUN or missing, a required human review not done, a render
//!    that did not finish, or a cancelled run → [`Verdict::Incomplete`];
//! 3. otherwise any WARN, an optional check FAIL, or a major/minor human finding → [`Verdict::Warn`];
//! 4. otherwise [`Verdict::Pass`].
//!
//! The render's own outcome is kept in [`QcReport::render`], apart from the QC result: a render
//! that succeeded says nothing about quality, and a QC failure does not rewrite the render status.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::catalog::{self, Measure};
use crate::profile::{Bounds, CheckSpec, ProfileRef, QcProfile, Requirement};

/// Schema id of every report.
pub const REPORT_SCHEMA: &str = "cfa4-qc-report/1";
/// Most events kept per check (the rest are counted in [`CheckResult::events_dropped`]).
pub const MAX_EVENTS: usize = 1000;
/// Most human findings and answers kept in one review.
pub const MAX_REVIEW_ITEMS: usize = 1000;
/// Longest free text kept (descriptions, notes, reasons), in characters.
pub const MAX_TEXT: usize = 4000;
/// Reason prefix of a result that does not apply ([`CheckResult::not_applicable`]).
pub const NOT_APPLICABLE: &str = "not applicable: ";

fn clip(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

/// The result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    /// The check did not run or could not measure; never counts as a pass.
    NotRun,
}

/// The overall QC result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    /// A mandatory result is missing: the QC cannot say the file is good.
    Incomplete,
}

/// A checked file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Where the checked file came from. Every field is optional: a file checked from the command
/// line has no project.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Links {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_job: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// How the render that produced the file ended (kept apart from the QC result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderStatus {
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutcome {
    pub status: RenderStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Versions of the tools that produced the measurements. `None` = not used in this run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tools {
    pub cfa4: String,
    pub ffmpeg: Option<String>,
    pub ffprobe: Option<String>,
}

impl Default for Tools {
    fn default() -> Self {
        Tools { cfa4: env!("CARGO_PKG_VERSION").to_string(), ffmpeg: None, ffprobe: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComputeMode {
    #[default]
    Cpu,
    Gpu,
}

/// When, where and how the QC ran. Everything else in a report is reproducible from the same
/// input and profile; this section is not.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub started_unix_ms: u64,
    pub duration_ms: u64,
    /// The executable that ran the QC.
    pub tool_path: String,
    pub compute: ComputeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_device: Option<String>,
    /// The run was stopped before every check finished.
    #[serde(default)]
    pub cancelled: bool,
}

/// A place in the checked file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u64>,
    /// `cfa4_time::Tick` from the start of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecode: Option<String>,
}

/// Something a check found, with where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub at: Position,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Position>,
    pub message: String,
}

/// One check's result, with the bounds it was judged by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub id: String,
    pub requirement: Requirement,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<Bounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Bounds>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub events_dropped: u64,
    /// Why the check is NOT_RUN, or what made it WARN/FAIL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Measured facts the check is based on (stream parameters, per-channel values…).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub detail: BTreeMap<String, serde_json::Value>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// PASS / WARN / FAIL of `value` against a check's bounds.
pub fn evaluate(spec: &CheckSpec, value: f64) -> Status {
    if !value.is_finite() {
        return Status::NotRun;
    }
    let Some(pass) = &spec.pass else { return Status::NotRun };
    if pass.contains(value) {
        Status::Pass
    } else if spec.accept.as_ref().is_some_and(|a| a.contains(value)) {
        Status::Warn
    } else {
        Status::Fail
    }
}

impl CheckResult {
    fn base(id: &str, spec: &CheckSpec, status: Status) -> CheckResult {
        let unit = catalog::check(id).map(|c| c.unit.to_string()).unwrap_or_default();
        CheckResult {
            id: id.to_string(),
            requirement: spec.requirement,
            status,
            value: None,
            unit,
            pass: spec.pass,
            accept: spec.accept,
            events: Vec::new(),
            events_dropped: 0,
            reason: None,
            detail: BTreeMap::new(),
        }
    }

    /// A measured value, judged by the profile's bounds. A non-finite value is NOT_RUN.
    pub fn measured(id: &str, spec: &CheckSpec, value: f64) -> CheckResult {
        let status = evaluate(spec, value);
        let mut r = CheckResult::base(id, spec, status);
        if value.is_finite() {
            r.value = Some(value);
        } else {
            r.reason = Some("the measurement gave no finite value".into());
        }
        r
    }

    /// A findings check: PASS without events; the caller says how bad the events are.
    pub fn findings(id: &str, spec: &CheckSpec, status_if_any: Status, events: Vec<Event>) -> CheckResult {
        let status = if events.is_empty() { Status::Pass } else { status_if_any };
        let mut r = CheckResult::base(id, spec, status);
        for e in events {
            r.push_event(e);
        }
        r
    }

    /// The check does not apply: the render plan rules out what it measures (no audio in a
    /// video-only export) and the file agrees. PASS without a value, with the reason.
    pub fn not_applicable(id: &str, spec: &CheckSpec, reason: &str) -> CheckResult {
        let mut r = CheckResult::base(id, spec, Status::Pass);
        r.reason = Some(clip(&format!("{NOT_APPLICABLE}{reason}")));
        r
    }

    /// A [`CheckResult::not_applicable`] result.
    pub fn is_not_applicable(&self) -> bool {
        self.status == Status::Pass && self.value.is_none() && self.reason.as_deref().is_some_and(|r| r.starts_with(NOT_APPLICABLE))
    }

    /// The check could not run, and why.
    pub fn not_run(id: &str, spec: &CheckSpec, reason: &str) -> CheckResult {
        let mut r = CheckResult::base(id, spec, Status::NotRun);
        r.reason = Some(clip(reason));
        r
    }

    pub fn with_reason(mut self, reason: &str) -> CheckResult {
        self.reason = Some(clip(reason));
        self
    }

    pub fn push_event(&mut self, mut e: Event) {
        if self.events.len() < MAX_EVENTS {
            e.message = clip(&e.message);
            self.events.push(e);
        } else {
            self.events_dropped = self.events_dropped.saturating_add(1);
        }
    }
}

/// How serious a human finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Minor,
    Major,
    Critical,
}

/// Something a person saw or heard, at a frame / time code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub description: String,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Position>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnswerValue {
    Yes,
    No,
    Unsure,
}

/// An answer to one of the guided questions ([`QUESTIONS`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub question: String,
    pub answer: AnswerValue,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The guided questions of the human review, as (id, question). Picture quality is judged by a
/// person: these answers are never shown as automatic measurements.
pub const QUESTIONS: &[(&str, &str)] = &[
    ("picture.defect", "Is there a visible picture defect?"),
    ("picture.colorContrast", "Are colour and contrast acceptable?"),
    ("audio.distortion", "Is there audible distortion?"),
    ("audio.noise", "Is there unwanted noise?"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    #[default]
    NotStarted,
    InProgress,
    Done,
}

/// The person's review, saved next to the automatic results.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanReview {
    pub state: ReviewState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Answer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
}

impl HumanReview {
    /// Check sizes and question ids; clip free text.
    pub fn sanitized(mut self) -> Result<HumanReview, Error> {
        if self.answers.len() > MAX_REVIEW_ITEMS || self.findings.len() > MAX_REVIEW_ITEMS {
            return Err(Error::InvalidReport(format!("more than {MAX_REVIEW_ITEMS} review items")));
        }
        for a in &mut self.answers {
            if !QUESTIONS.iter().any(|(id, _)| *id == a.question) {
                return Err(Error::InvalidReport(format!("unknown review question `{}`", a.question.chars().take(64).collect::<String>())));
            }
            a.note = clip(&a.note);
        }
        for f in &mut self.findings {
            f.description = clip(&f.description);
            f.note = clip(&f.note);
        }
        self.reviewer = self.reviewer.as_deref().map(clip);
        Ok(self)
    }
}

/// A check that did not give a result, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub id: String,
    pub requirement: Requirement,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub pass: u32,
    pub warn: u32,
    pub fail: u32,
    pub not_run: u32,
}

/// The verdict and what led to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub verdict: Verdict,
    pub counts: Counts,
    /// Why the verdict is not PASS, most serious first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<Skipped>,
}

/// A complete QC report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QcReport {
    pub schema: String,
    pub profile: ProfileRef,
    /// The source the render was made from, when the QC compares the two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<FileRef>,
    /// The checked file (the export artefact).
    pub output: FileRef,
    #[serde(default)]
    pub links: Links,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<RenderOutcome>,
    pub tools: Tools,
    pub run: RunInfo,
    pub human_review_requirement: Requirement,
    pub checks: Vec<CheckResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<HumanReview>,
    pub summary: Summary,
}

/// Everything a report is made from, before the verdict.
#[derive(Debug, Clone, Default)]
pub struct Parts {
    pub input: Option<FileRef>,
    pub links: Links,
    pub render: Option<RenderOutcome>,
    pub tools: Tools,
    pub run: RunInfo,
    pub checks: Vec<CheckResult>,
    pub human: Option<HumanReview>,
}

impl QcReport {
    /// Build a report: checks in catalogue order, a NOT_RUN result for every profile check
    /// without one, and the verdict. Results for checks the profile does not name, or two results
    /// for one check, are errors.
    pub fn assemble(profile: &QcProfile, output: FileRef, parts: Parts) -> Result<QcReport, Error> {
        let mut by_id: BTreeMap<String, CheckResult> = BTreeMap::new();
        for mut r in parts.checks {
            let Some(spec) = profile.checks.get(&r.id) else {
                return Err(Error::InvalidReport(format!("result for `{}`, which profile `{}` does not list", r.id, profile.id)));
            };
            // the profile decides what is mandatory and which bounds apply, not the caller
            r.requirement = spec.requirement;
            r.pass = spec.pass;
            r.accept = spec.accept;
            if let Some(v) = r.value
                && r.status != Status::NotRun
            {
                r.status = evaluate(spec, v);
            } else if catalog::check(&r.id).is_some_and(|c| c.measure == Measure::Value) && r.status != Status::NotRun && !r.is_not_applicable() {
                r.status = Status::NotRun;
                r.reason = Some("no measured value".into());
            }
            if r.status == Status::NotRun && r.reason.is_none() {
                r.reason = Some("no reason given".into());
            }
            let id = r.id.clone();
            if by_id.insert(id.clone(), r).is_some() {
                return Err(Error::InvalidReport(format!("two results for `{id}`")));
            }
        }
        let missing_reason = if parts.run.cancelled { "the run was cancelled before this check" } else { "no result" };
        let mut checks = Vec::with_capacity(profile.checks.len());
        for info in catalog::CHECKS {
            let Some(spec) = profile.checks.get(info.id) else { continue };
            checks.push(by_id.remove(info.id).unwrap_or_else(|| CheckResult::not_run(info.id, spec, missing_reason)));
        }
        let human = parts.human.map(HumanReview::sanitized).transpose()?;
        let mut report = QcReport {
            schema: REPORT_SCHEMA.to_string(),
            profile: profile.reference(),
            input: parts.input,
            output,
            links: parts.links,
            render: parts.render,
            tools: parts.tools,
            run: parts.run,
            human_review_requirement: profile.human_review,
            checks,
            human,
            summary: Summary { verdict: Verdict::Incomplete, counts: Counts::default(), reasons: Vec::new(), skipped: Vec::new() },
        };
        report.summary = report.summarize();
        Ok(report)
    }

    /// The verdict from the results (rules in the module docs).
    pub fn summarize(&self) -> Summary {
        let mut counts = Counts::default();
        let mut fail = Vec::new();
        let mut incomplete = Vec::new();
        let mut warn = Vec::new();
        let mut skipped = Vec::new();
        for c in &self.checks {
            let mandatory = c.requirement == Requirement::Mandatory;
            match c.status {
                Status::Pass => counts.pass = counts.pass.saturating_add(1),
                Status::Warn => {
                    counts.warn = counts.warn.saturating_add(1);
                    warn.push(format!("{}: WARN", c.id));
                }
                Status::Fail => {
                    counts.fail = counts.fail.saturating_add(1);
                    if mandatory { &mut fail } else { &mut warn }.push(format!("{}: FAIL{}", c.id, if mandatory { "" } else { " (optional)" }));
                }
                Status::NotRun => {
                    counts.not_run = counts.not_run.saturating_add(1);
                    let reason = c.reason.clone().unwrap_or_else(|| "no reason given".into());
                    if mandatory {
                        incomplete.push(format!("{}: NOT_RUN ({reason})", c.id));
                    }
                    skipped.push(Skipped { id: c.id.clone(), requirement: c.requirement, reason });
                }
            }
        }
        let findings = self.human.as_ref().map(|h| h.findings.as_slice()).unwrap_or_default();
        for f in findings {
            let text = format!("human review ({:?}): {}", f.severity, f.description.chars().take(200).collect::<String>());
            match f.severity {
                Severity::Critical => fail.push(text),
                Severity::Major | Severity::Minor => warn.push(text),
                Severity::Info => {}
            }
        }
        if self.human_review_requirement == Requirement::Mandatory && self.human.as_ref().is_none_or(|h| h.state != ReviewState::Done) {
            incomplete.push("human review: not done".into());
        }
        if let Some(r) = &self.render
            && r.status != RenderStatus::Done
        {
            incomplete.push(format!("render: {:?}", r.status).to_lowercase());
        }
        if self.run.cancelled {
            incomplete.push("run: cancelled".into());
        }
        let verdict = if !fail.is_empty() {
            Verdict::Fail
        } else if !incomplete.is_empty() {
            Verdict::Incomplete
        } else if !warn.is_empty() {
            Verdict::Warn
        } else {
            Verdict::Pass
        };
        let reasons = fail.into_iter().chain(incomplete).chain(warn).collect();
        Summary { verdict, counts, reasons, skipped }
    }

    /// The file may be marked quality-checked (PASS or WARN; never FAIL or INCOMPLETE).
    pub fn quality_checked(&self) -> bool {
        matches!(self.summary.verdict, Verdict::Pass | Verdict::Warn)
    }

    /// A new report with the person's review replacing the previous one, verdict recomputed.
    pub fn with_review(&self, review: HumanReview) -> Result<QcReport, Error> {
        let mut r = self.clone();
        r.human = Some(review.sanitized()?);
        r.summary = r.summarize();
        Ok(r)
    }

    /// Pretty JSON. Same inputs give the same bytes (fields in a fixed order, checks in catalogue
    /// order).
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|e| Error::InvalidReport(e.to_string()))
    }

    /// Read a report and check it: known schema, and a verdict that matches its results (a
    /// hand-edited or truncated report is refused instead of trusted).
    pub fn from_json(text: &str) -> Result<QcReport, Error> {
        let r: QcReport = serde_json::from_str(text).map_err(|e| Error::InvalidReport(e.to_string()))?;
        if r.schema != REPORT_SCHEMA {
            return Err(Error::InvalidReport(format!("schema must be `{REPORT_SCHEMA}`")));
        }
        for c in &r.checks {
            if let (Some(v), Some(pass)) = (c.value, c.pass)
                && c.status != Status::NotRun
                && evaluate(&CheckSpec { requirement: c.requirement, pass: Some(pass), accept: c.accept }, v) != c.status
            {
                return Err(Error::InvalidReport(format!("`{}`: status does not match its value and bounds", c.id)));
            }
            if catalog::check(&c.id).is_some_and(|i| i.measure == Measure::Value)
                && c.status != Status::NotRun
                && !c.is_not_applicable()
                && (c.value.is_none() || c.pass.is_none())
            {
                return Err(Error::InvalidReport(format!("`{}`: a result without a value or bounds", c.id)));
            }
        }
        if r.summarize() != r.summary {
            return Err(Error::InvalidReport("the summary does not match the results".into()));
        }
        Ok(r)
    }
}
