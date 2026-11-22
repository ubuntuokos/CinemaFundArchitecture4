//! DCP export integration (DCP-13, plan DCP-F20): a preflight before a DCP export is queued and a
//! postflight validation of the finished package.
//!
//! * `dcp.preflight` (and every `export.queue.add` of a DCP) checks the settings, the target
//!   folder and the sequence before anything is written. A blocking finding refuses the export;
//!   the rest are warnings. A folder that already holds files is blocking: the exporter would
//!   mix the new track files into the earlier package, so an earlier valid export is never
//!   touched.
//! * `export.queue.add` with `dcpValidate` (default for a DCP: `delivery-preflight`; `false`
//!   switches it off) validates the package when it has been written. The result is kept apart
//!   from the render state, as the QC result is ([`ItemDcp`]); a failed package keeps its files
//!   and gets its report next to it (`<package>.cfa4dcp.json`, never inside it).
//!
//! The validation says nothing about whether CFA4 writes complete DCPs: the preflight names what
//! the exporter does not do (unsigned, unencrypted, no KDM) as an `info` finding.

use serde::Serialize;
use serde_json::{Value, json};

use cfa4_dcp_validate::{Status, ValidationReport, Verdict};
use cfa4_export::{DcpAspect, DcpResolution, ExportSettings, Format};

use crate::commands::{CommandSpec, bad, has_seq};
use crate::export_tools::QueueStatus;
use crate::{Result, Session};

/// Suffix of the report kept next to an exported package.
pub const REPORT_SUFFIX: &str = ".cfa4dcp.json";
/// Profile used for the postflight of a DCP export when none is named.
pub const DEFAULT_PROFILE: &str = "delivery-preflight";

/// The validation state of a queued DCP export, kept apart from its render state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DcpStatus {
    #[default]
    NotChecked,
    Running,
    Passed,
    Failed,
    Incomplete,
}

/// Validation of a queued DCP export: what was asked and where it stands.
#[derive(Debug, Clone, Default)]
pub struct ItemDcp {
    pub status: DcpStatus,
    /// Profile (id or `.json` file) to validate with when the export finishes.
    pub profile: Option<String>,
    /// Where the report was written.
    pub report: Option<String>,
    /// Why it failed or could not be checked.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The export is refused.
    Error,
    Warning,
    Info,
}

/// One preflight finding.
#[derive(Debug, Clone, Serialize)]
pub struct Preflight {
    pub id: &'static str,
    pub severity: Severity,
    pub message: String,
    pub fix: Option<String>,
}

fn finding(id: &'static str, severity: Severity, message: impl Into<String>, fix: Option<&str>) -> Preflight {
    Preflight { id, severity, message: message.into(), fix: fix.map(str::to_owned) }
}

/// Check a DCP export of `seq` with `settings` (target path already set) before it is queued.
/// Reads the file system only to see the target; nothing is written.
pub fn preflight(s: &Session, seq: cfa4_project::ItemId, settings: &ExportSettings) -> Vec<Preflight> {
    if settings.format != Format::Dcp {
        return vec![finding(
            "DCP.PRE.000",
            Severity::Info,
            format!("the format is {}, not a DCP: no DCP preflight or validation applies", settings.format.id()),
            None,
        )];
    }
    let mut out = Vec::new();
    if let Err(e) = settings.validate() {
        out.push(finding("DCP.PRE.001", Severity::Error, e.to_string(), Some("change the DCP settings")));
    }
    let on_disk = !s.services.export_in_memory();
    let target = std::path::Path::new(&settings.path);
    if on_disk && target.is_file() {
        out.push(finding(
            "DCP.PRE.002",
            Severity::Error,
            format!("{}: a file is in the way of the package folder", settings.path),
            Some("pick another folder name"),
        ));
    } else if on_disk && target.is_dir() && std::fs::read_dir(target).is_ok_and(|mut d| d.next().is_some()) {
        out.push(finding(
            "DCP.PRE.002",
            Severity::Error,
            format!("{}: the folder is not empty; exporting into it would mix the new files into the package that is there", settings.path),
            Some("export to a new folder, or move the earlier package away"),
        ));
    }
    let Some(q) = s.project.sequence(seq) else {
        out.push(finding("DCP.PRE.003", Severity::Error, "the sequence does not exist", None));
        return out;
    };
    match cfa4_export::export_range(&s.project, seq, settings) {
        Ok(r) if r.duration > cfa4_time::Tick::ZERO => {}
        Ok(_) => out.push(finding(
            "DCP.PRE.003",
            Severity::Error,
            "the exported range is empty",
            Some("set an In/Out range with content, or export the entire sequence"),
        )),
        Err(e) => out.push(finding("DCP.PRE.003", Severity::Error, e.to_string(), None)),
    }
    let res = DcpResolution::of(settings);
    let want = settings.frame_rate.unwrap_or(q.settings.frame_rate);
    let got = cfa4_export::dcp_rate(res, want);
    if (want.sane().as_f64() - got.as_f64()).abs() > 1e-6 {
        out.push(finding(
            "DCP.PRE.004",
            Severity::Warning,
            format!("the frame rate {} is not a {} DCP rate; the package is written at {}", want.label(), res.id(), got.label()),
            Some("set the sequence to a DCI frame rate to avoid the conversion"),
        ));
    }
    let (cw, ch) = DcpAspect::of(settings).size(res);
    let (w, h) = (f64::from(q.settings.width.max(1)), f64::from(q.settings.height.max(1)));
    if ((w / h) / (f64::from(cw) / f64::from(ch)) - 1.0).abs() > 0.02 {
        out.push(finding(
            "DCP.PRE.005",
            Severity::Info,
            format!("the {}x{} picture is scaled to fit the {cw}x{ch} container with black bars", q.settings.width, q.settings.height),
            Some("choose the container (flat, scope, full) that matches the picture"),
        ));
    }
    if settings.dcp_language.trim().is_empty() {
        out.push(finding(
            "DCP.PRE.006",
            Severity::Warning,
            "no spoken language is set; the package name says XX",
            Some("set the DCP language (for example en-US)"),
        ));
    }
    if !settings.has_audio() && settings.dcp_immersive == "off" {
        out.push(finding("DCP.PRE.007", Severity::Warning, "audio is switched off: the package has no sound track", Some("include the audio in the export")));
    }
    let title = s.project.item(seq).map(|i| i.name.clone()).unwrap_or_default();
    let letters = title.split(|c: char| !c.is_alphanumeric()).map(|w| w.chars().filter(char::is_ascii_alphanumeric).count()).sum::<usize>();
    if letters > 14 {
        out.push(finding("DCP.PRE.008", Severity::Info, format!("the title \"{title}\" is shortened to 14 characters in the ISDCF name"), None));
    }
    out.push(finding("DCP.PRE.009", Severity::Info, "the package is written unencrypted and unsigned (no KDM, no XML signature)", None));
    out
}

/// The blocking findings as one line.
pub fn blocking(findings: &[Preflight]) -> Option<String> {
    let errors: Vec<String> = findings.iter().filter(|f| f.severity == Severity::Error).map(|f| format!("{}: {}", f.id, f.message)).collect();
    (!errors.is_empty()).then(|| errors.join("; "))
}

/// `Ok`/`Warnings`/`Blocked` of a finding list, as a word.
pub fn verdict_word(findings: &[Preflight]) -> &'static str {
    if findings.iter().any(|f| f.severity == Severity::Error) {
        "blocked"
    } else if findings.iter().any(|f| f.severity == Severity::Warning) {
        "warnings"
    } else {
        "ok"
    }
}

/// `dcp.preflight`: what a DCP export with these settings would run into, before it is queued.
/// Takes the parameters of `export.resolve` (the format defaults to `dcp`).
fn preflight_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.preflight";
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.entry("format").or_insert_with(|| json!("dcp"));
    }
    let seq = crate::export_tools::sequence_param(s, &p, cmd)?;
    let (_, mut settings) = crate::export_tools::settings_from_params(s, &p, cmd)?;
    let project = s.project.clone();
    let (range, _) = crate::export_tools::range_with_warnings(s, &project, seq, p.get("range"), &p, cmd)?;
    settings.range = range;
    settings.path = crate::export_tools::output_path(s, &p, seq, &settings, None);
    let findings = preflight(s, seq, &settings);
    Ok(json!({"path": settings.path, "verdict": verdict_word(&findings), "findings": findings}))
}

/// The profile a queued DCP export is validated with: `dcpValidate` (a profile id, a `.json`
/// profile file, `true` for the default, `false` for none); a DCP defaults to [`DEFAULT_PROFILE`].
pub(crate) fn profile_param(p: &Value, format: Format, cmd: &str) -> Result<Option<String>> {
    if format != Format::Dcp {
        return Ok(None);
    }
    let profile = match p.get("dcpValidate") {
        None | Some(Value::Null) | Some(Value::Bool(true)) => DEFAULT_PROFILE.to_string(),
        Some(Value::Bool(false)) => return Ok(None),
        Some(Value::String(id)) => id.clone(),
        Some(_) => return Err(bad(cmd, "`dcpValidate` must be a profile id, a profile file or a boolean")),
    };
    crate::dcp_validation::load_profile(cmd, Some(&profile))?;
    Ok(Some(profile))
}

/// Start the postflight of a queued DCP export that finished rendering. Called by the queue.
pub(crate) fn after_export(s: &mut Session, item_id: u64, wait: bool) {
    let Some(it) = s.export_queue.items.iter().find(|i| i.id == item_id) else { return };
    let Some(profile) = it.dcp.profile.clone() else { return };
    if it.status != QueueStatus::Done || it.dcp.status != DcpStatus::NotChecked {
        return;
    }
    let root = it.settings.path.clone();
    let fail = |s: &mut Session, why: String| {
        if let Some(it) = s.export_queue.items.iter_mut().find(|i| i.id == item_id) {
            it.dcp.status = DcpStatus::Incomplete;
            it.dcp.error = Some(why.clone());
        }
        s.error_toast("dcp.validate", format!("DCP validation: {why}"));
    };
    if s.services.export_in_memory() {
        return fail(s, "the package was written in memory; validation needs a folder on disk".into());
    }
    let cmd = "dcp.validate";
    match crate::dcp_validation::load_profile(cmd, Some(&profile)) {
        Ok(pr) => {
            if let Err(e) = crate::dcp_validation::start(s, cmd, root, pr, wait, Some(item_id)) {
                fail(s, e.to_string());
            }
        }
        Err(e) => fail(s, e.to_string()),
    }
}

/// The validation of a queued export started.
pub(crate) fn running(s: &mut Session, item_id: u64) {
    if let Some(it) = s.export_queue.items.iter_mut().find(|i| i.id == item_id) {
        it.dcp.status = DcpStatus::Running;
        it.dcp.error = None;
    }
}

/// The validation of a queued export ended: its state, the error text, the report next to the
/// package. Never touches the package itself.
pub(crate) fn settled(s: &mut Session, item_id: u64, result: std::result::Result<&ValidationReport, &str>) {
    let Some(i) = s.export_queue.items.iter().position(|i| i.id == item_id) else { return };
    let root = s.export_queue.items[i].settings.path.clone();
    let (status, error, report) = match result {
        Ok(rep) => {
            let path = format!("{}{REPORT_SUFFIX}", root.trim_end_matches(['/', '\\']));
            let saved = s.services.write_file(&path, rep.to_json().as_bytes()).map(|()| path.clone());
            let status = match rep.verdict {
                Verdict::Pass => DcpStatus::Passed,
                Verdict::Fail => DcpStatus::Failed,
                Verdict::Incomplete | Verdict::Interrupted => DcpStatus::Incomplete,
            };
            let mut error = match status {
                DcpStatus::Failed => {
                    let fails: Vec<_> = rep.findings.iter().filter(|f| f.status == Status::Fail).collect();
                    let first = fails.first().map(|f| format!(" ({}: {})", f.rule, f.message)).unwrap_or_default();
                    Some(format!("{} rule(s) failed{first}", fails.len()))
                }
                DcpStatus::Incomplete => Some(format!("{} rule(s) could not be checked", rep.counts.skipped + rep.counts.unchecked + rep.counts.interrupted)),
                _ => None,
            };
            if let Err(e) = &saved {
                let note = format!("report not saved: {e}");
                error = Some(error.map_or(note.clone(), |x| format!("{x}; {note}")));
            }
            (status, error, saved.ok())
        }
        Err(e) => (DcpStatus::Incomplete, Some(e.to_string()), None),
    };
    let it = &mut s.export_queue.items[i];
    it.dcp.status = status;
    it.dcp.error = error;
    it.dcp.report = report;
}

/// A queued DCP export is rendered again: its earlier validation no longer describes the files
/// (the profile stays, so the new render is checked). A check still running is cancelled.
pub(crate) fn reset_for_retry(s: &mut Session, item_id: u64) {
    let jobs: Vec<u64> = s
        .dcp_validation
        .pending
        .iter_mut()
        .filter(|v| v.queue_item == Some(item_id))
        .map(|v| {
            v.queue_item = None;
            v.job
        })
        .collect();
    for j in s.jobs.iter().filter(|j| jobs.contains(&j.id)) {
        j.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(it) = s.export_queue.items.iter_mut().find(|i| i.id == item_id) {
        it.dcp = ItemDcp { profile: it.dcp.profile.take(), ..ItemDcp::default() };
    }
}

/// The DCP part of a queue item's JSON.
pub(crate) fn item_json(d: &ItemDcp) -> Value {
    json!({"status": d.status, "profile": d.profile, "report": d.report, "error": d.error})
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "dcp.preflight",
        label: "Check DCP Export Settings",
        menu: &[],
        shortcut: None,
        params: "{…export.resolve params, \"path\":str?}",
        enabled: has_seq,
        run: preflight_cmd,
        journal: false,
    }]
}

#[cfg(test)]
mod tests;
