//! The verdict rules of the plan (§3, §6 gates): a skipped or failed mandatory check never
//! passes, cancelled runs are incomplete, render and QC results stay apart, the same inputs give
//! the same report, and a report whose verdict does not match its results is refused.

use cfa4_qc::catalog::{self, Measure};
use cfa4_qc::report::{Answer, AnswerValue, Event, Finding, Parts, Position, RenderOutcome, RenderStatus, ReviewState, RunInfo, Severity, Status, Verdict};
use cfa4_qc::{CheckResult, FileRef, HumanReview, QcProfile, QcReport, Requirement};

fn profile() -> QcProfile {
    QcProfile::builtin("delivery-web").unwrap()
}

fn output() -> FileRef {
    FileRef { path: "/tmp/export árvíz.mp4".into(), sha256: "0".repeat(64), bytes: 1234 }
}

/// A passing value for each value check of the profile.
fn good_value(id: &str) -> f64 {
    match id {
        "audio.loudness" => -14.0,
        "audio.truePeak" => -1.5,
        "audio.loudnessRange" => 6.0,
        _ => 0.0,
    }
}

/// Every check of the profile passes.
fn all_pass(p: &QcProfile) -> Vec<CheckResult> {
    p.checks
        .iter()
        .map(|(id, spec)| match catalog::check(id).unwrap().measure {
            Measure::Value => CheckResult::measured(id, spec, good_value(id)),
            Measure::Findings => CheckResult::findings(id, spec, Status::Fail, vec![]),
        })
        .collect()
}

fn review_done() -> HumanReview {
    HumanReview {
        state: ReviewState::Done,
        reviewer: Some("QC".into()),
        answers: vec![Answer { question: "picture.defect".into(), answer: AnswerValue::No, note: String::new() }],
        findings: vec![],
    }
}

fn parts(checks: Vec<CheckResult>) -> Parts {
    Parts { checks, human: Some(review_done()), render: Some(RenderOutcome { status: RenderStatus::Done, error: None }), ..Parts::default() }
}

fn replace(checks: &mut [CheckResult], r: CheckResult) {
    let slot = checks.iter_mut().find(|c| c.id == r.id).unwrap();
    *slot = r;
}

#[test]
fn everything_passing_is_pass() {
    let p = profile();
    let r = QcReport::assemble(&p, output(), parts(all_pass(&p))).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Pass, "{:?}", r.summary.reasons);
    assert!(r.quality_checked());
    assert_eq!(r.profile.id, "delivery-web");
    assert_eq!(r.profile.version, 1);
    assert_eq!(r.profile.sha256.len(), 64);
    let loud = r.checks.iter().find(|c| c.id == "audio.loudness").unwrap();
    assert_eq!((loud.unit.as_str(), loud.pass.unwrap().min), ("LUFS", Some(-15.0)), "every measurement shows its bounds");
}

#[test]
fn mandatory_fail_is_fail() {
    let p = profile();
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::measured("audio.loudness", &p.checks["audio.loudness"], -23.0));
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Fail);
    assert!(!r.quality_checked());
    assert!(r.summary.reasons.iter().any(|s| s.starts_with("audio.loudness: FAIL")));
}

#[test]
fn warn_band() {
    let p = profile();
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::measured("audio.loudness", &p.checks["audio.loudness"], -15.5));
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Warn);
    assert!(r.quality_checked());
}

#[test]
fn skipped_or_missing_mandatory_is_never_pass() {
    let p = profile();
    // explicitly NOT_RUN (AV1 export with no usable decoder, for example)
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::not_run("integrity.decode", &p.checks["integrity.decode"], "no decoder for this AV1 profile"));
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
    assert!(!r.quality_checked());
    let s = r.summary.skipped.iter().find(|s| s.id == "integrity.decode").unwrap();
    assert_eq!((s.requirement, s.reason.as_str()), (Requirement::Mandatory, "no decoder for this AV1 profile"));

    // simply missing
    let mut checks = all_pass(&p);
    checks.retain(|c| c.id != "audio.truePeak");
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
    assert_eq!(r.checks.iter().find(|c| c.id == "audio.truePeak").unwrap().status, Status::NotRun);

    // a non-finite measurement
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::measured("audio.loudness", &p.checks["audio.loudness"], f64::NAN));
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
}

#[test]
fn optional_not_run_is_listed_but_does_not_block() {
    let p = profile();
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::not_run("video.black", &p.checks["video.black"], "no video stream decoded yet"));
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Pass);
    assert!(r.summary.skipped.iter().any(|s| s.id == "video.black" && s.requirement == Requirement::Optional));
}

#[test]
fn optional_fail_warns() {
    let p = profile();
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::measured("video.clipWhite", &p.checks["video.clipWhite"], 30.0));
    let r = QcReport::assemble(&p, output(), parts(checks)).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Warn);
}

#[test]
fn cancelled_run_is_incomplete() {
    let p = profile();
    let mut checks = all_pass(&p);
    checks.truncate(3);
    let mut pp = parts(checks);
    pp.run = RunInfo { cancelled: true, ..RunInfo::default() };
    let r = QcReport::assemble(&p, output(), pp).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
    assert!(r.checks.iter().any(|c| c.reason.as_deref() == Some("the run was cancelled before this check")));
}

#[test]
fn render_failure_is_kept_apart() {
    let p = profile();
    let mut pp = parts(all_pass(&p));
    pp.render = Some(RenderOutcome { status: RenderStatus::Failed, error: Some("disk full".into()) });
    let r = QcReport::assemble(&p, output(), pp).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
    assert_eq!(r.render.as_ref().unwrap().error.as_deref(), Some("disk full"));
    assert!(r.checks.iter().all(|c| c.status == Status::Pass), "the render error does not rewrite check results");
}

#[test]
fn human_review_rules() {
    let p = profile();
    // required but missing
    let mut pp = parts(all_pass(&p));
    pp.human = None;
    assert_eq!(QcReport::assemble(&p, output(), pp).unwrap().summary.verdict, Verdict::Incomplete);

    // a critical finding at a time code fails; a minor one warns
    let base = QcReport::assemble(&p, output(), parts(all_pass(&p))).unwrap();
    let finding = |severity| Finding {
        description: "dropout".into(),
        severity,
        at: Some(Position { frame: Some(240), tick: None, timecode: Some("00:00:10:00".into()) }),
        note: String::new(),
    };
    let mut review = review_done();
    review.findings.push(finding(Severity::Critical));
    let r = base.with_review(review).unwrap();
    assert_eq!(r.summary.verdict, Verdict::Fail);
    assert_eq!(r.human.as_ref().unwrap().findings[0].at.as_ref().unwrap().frame, Some(240));
    let mut review = review_done();
    review.findings.push(finding(Severity::Minor));
    assert_eq!(base.with_review(review).unwrap().summary.verdict, Verdict::Warn);

    // unknown questions are refused
    let mut review = review_done();
    review.answers[0].question = "made.up".into();
    assert!(base.with_review(review).is_err());
}

#[test]
fn bad_results_are_refused() {
    let p = profile();
    let mut checks = all_pass(&p);
    checks.push(checks[0].clone());
    assert!(QcReport::assemble(&p, output(), parts(checks)).is_err(), "two results for one check");
    let other = QcProfile::from_toml("schema = \"cfa4-qc-profile/1\"\nid = \"x\"\nversion = 1\ntitle = \"x\"\n[checks.\"probe.open\"]\n").unwrap();
    assert!(QcReport::assemble(&other, output(), parts(all_pass(&p))).is_err(), "results the profile does not list");
}

#[test]
fn reproducible_and_round_trips() {
    let p = profile();
    let mut checks = all_pass(&p);
    let mut gap = CheckResult::findings(
        "audio.silence",
        &p.checks["audio.silence"],
        Status::Warn,
        vec![Event { at: Position { frame: None, tick: Some(0), timecode: Some("00:00:00:00".into()) }, end: None, message: "2.0 s silence".into() }],
    );
    gap.detail.insert("thresholdDb".into(), serde_json::json!(-60.0));
    replace(&mut checks, gap);
    let a = QcReport::assemble(&p, output(), parts(checks.clone())).unwrap().to_json().unwrap();
    checks.reverse();
    let b = QcReport::assemble(&p, output(), parts(checks)).unwrap().to_json().unwrap();
    assert_eq!(a, b, "same input and profile give the same report");
    let back = QcReport::from_json(&a).unwrap();
    assert_eq!(back.to_json().unwrap(), a);
    assert!(a.contains("\"NOT_RUN\"") || a.contains("\"PASS\""));
}

#[test]
fn tampered_reports_are_refused() {
    let p = profile();
    let mut checks = all_pass(&p);
    replace(&mut checks, CheckResult::measured("audio.loudness", &p.checks["audio.loudness"], -23.0));
    let json = QcReport::assemble(&p, output(), parts(checks)).unwrap().to_json().unwrap();
    // flipping the verdict
    assert!(QcReport::from_json(&json.replacen("\"verdict\": \"FAIL\"", "\"verdict\": \"PASS\"", 1)).is_err());
    // flipping a check status without changing its value
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    for c in v["checks"].as_array_mut().unwrap() {
        if c["id"] == "audio.loudness" {
            c["status"] = "PASS".into();
        }
    }
    assert!(QcReport::from_json(&v.to_string()).is_err());
    // truncated
    assert!(QcReport::from_json(&json[..json.len() / 2]).is_err());
    // wrong schema
    assert!(QcReport::from_json(&json.replace("cfa4-qc-report/1", "cfa4-qc-report/0")).is_err());
}
