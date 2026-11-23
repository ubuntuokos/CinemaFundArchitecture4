//! Repair proposals (DCP-14, plan DCP-F21): every finding that needs attention is put into one of
//! five classes with a suggested action. A plan only *proposes*: it never changes the package, and
//! nothing here applies a repair. An automatic change may only touch CFA4's own project data or a
//! working copy the user approved (plan §9), so a proposal says what to change and where, and the
//! change is made by the person or by a later, approved step.

use serde::{Deserialize, Serialize};

use crate::report::ValidationReport;
use crate::rules::*;
use crate::{Location, Status};

pub const SCHEMA: &str = "cfa4-dcp-repair/1";

/// What kind of repair a finding calls for (plan DCP-F21).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepairClass {
    /// Project metadata CFA4 owns (title, language, container, markers): fix it in the project or
    /// the export settings, then export again.
    ProjectMetadata,
    /// The package content is wrong: export it again (or ask the supplier of an imported package).
    Reexport,
    /// A deviation only the person can judge (loudness, subtitle timing).
    UserDecision,
    /// Needs a tool or a certificate outside CFA4.
    ExternalTool,
    /// The check did not run: there is nothing to repair, the result is unknown.
    Unsupported,
}

impl RepairClass {
    pub const ALL: [RepairClass; 5] =
        [RepairClass::ProjectMetadata, RepairClass::Reexport, RepairClass::UserDecision, RepairClass::ExternalTool, RepairClass::Unsupported];
}

/// The class of a rule's failures and what to do about it.
fn classify(rule: &str) -> (RepairClass, &'static str) {
    match rule {
        NAME_ISDCF => {
            (RepairClass::ProjectMetadata, "correct the title, language, container or resolution of the export (it names the package), then export again")
        }
        MARK_MARKERS => (RepairClass::ProjectMetadata, "fix the sequence markers in the project, then export again"),
        AUD_LOUDNESS => (RepairClass::UserDecision, "decide whether the measured loudness is acceptable for the venue; mix the sequence again if it is not"),
        SUB_DOCUMENTS => (RepairClass::UserDecision, "check the subtitle timing and text in the caption track, then export again"),
        SIG_SIGNATURE | SIG_CERTIFICATES => {
            (RepairClass::ExternalTool, "the signer chain or certificate comes from outside CFA4: re-sign with a valid certificate chain")
        }
        REF_ENGINE => (RepairClass::ExternalTool, "install or fix the reference engine (docs/development/dcp-validation.md), then validate again"),
        _ => (RepairClass::Reexport, "export the package again; for an imported package, ask its supplier for a corrected one"),
    }
}

/// One proposed repair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairProposal {
    pub rule: String,
    pub class: RepairClass,
    /// `FAIL` or `WARNING` for a finding to repair, `UNCHECKED` / `SKIPPED` for an unknown result.
    pub status: Status,
    pub location: Location,
    pub finding: String,
    /// What to do: the finding's own remedy when it has one, else the rule's class action.
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairPlan {
    pub schema: String,
    pub root: String,
    /// The proposals never change the package: always `false`.
    pub modifies_package: bool,
    pub proposals: Vec<RepairProposal>,
}

impl RepairPlan {
    pub fn count(&self, class: RepairClass) -> usize {
        self.proposals.iter().filter(|p| p.class == class).count()
    }
}

/// The repair plan of a report: failures and warnings by class, results that could not be known
/// as `Unsupported`. Passed and not-applicable rules need nothing. Failures come first.
pub fn plan(report: &ValidationReport) -> RepairPlan {
    let mut proposals: Vec<RepairProposal> = report
        .findings
        .iter()
        .filter(|f| matches!(f.status, Status::Fail | Status::Warning | Status::Unchecked | Status::Skipped | Status::Interrupted))
        .map(|f| {
            let (class, action) = match f.status {
                Status::Fail | Status::Warning => classify(&f.rule),
                _ => (RepairClass::Unsupported, "nothing to repair: this rule did not run, so the result is unknown; run a profile that checks it"),
            };
            RepairProposal {
                rule: f.rule.clone(),
                class,
                status: f.status,
                location: f.location.clone(),
                finding: f.message.clone(),
                action: f.remedy.clone().filter(|_| matches!(f.status, Status::Fail | Status::Warning)).unwrap_or_else(|| action.to_string()),
            }
        })
        .collect();
    let rank = |s: Status| match s {
        Status::Fail => 0,
        Status::Warning => 1,
        _ => 2,
    };
    proposals.sort_by_key(|p| rank(p.status));
    RepairPlan { schema: SCHEMA.to_string(), root: report.root.clone(), modifies_package: false, proposals }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Finding;
    use crate::report::{BackendInfo, Counts, DocumentIds, ProfileInfo, Verdict};

    fn report(findings: Vec<Finding>) -> ValidationReport {
        ValidationReport {
            schema: crate::report::SCHEMA.into(),
            verdict: Verdict::Fail,
            profile: ProfileInfo { id: "t".into(), version: 1, title: "T".into() },
            backend: BackendInfo { id: "test".into(), version: "1".into() },
            root: "/pkg".into(),
            standard: None,
            documents: DocumentIds::default(),
            assets_checked: 0,
            counts: Counts::of(&findings),
            findings,
            started_at: String::new(),
            finished_at: String::new(),
        }
    }

    #[test]
    fn findings_fall_into_the_five_classes_and_never_modify() {
        let mut with_remedy = Finding::new(PKL_HASH, Status::Fail, "hash differs");
        with_remedy.remedy = Some("recompute the hash".into());
        let r = report(vec![
            Finding::new(AM_PRESENT, Status::Pass, "ok"),
            Finding::new(AUD_LOUDNESS, Status::Warning, "loud"),
            Finding::new(NAME_ISDCF, Status::Warning, "name"),
            with_remedy,
            Finding::new(SIG_SIGNATURE, Status::Fail, "bad signature"),
            Finding::new(IAB_TRACK, Status::Unchecked, "no reader"),
            Finding::new(AM_HAS_PKL, Status::NotApplicable, "n/a"),
            Finding::new(SUB_DOCUMENTS, Status::Fail, "timing"),
        ]);
        let p = plan(&r);
        assert_eq!(p.schema, SCHEMA);
        assert!(!p.modifies_package);
        assert_eq!(p.proposals.len(), 6, "pass and not-applicable need nothing");
        let by = |class| p.count(class);
        assert_eq!(
            [
                by(RepairClass::ProjectMetadata),
                by(RepairClass::Reexport),
                by(RepairClass::UserDecision),
                by(RepairClass::ExternalTool),
                by(RepairClass::Unsupported)
            ],
            [1, 1, 2, 1, 1]
        );
        assert_eq!(p.proposals[0].status, Status::Fail, "failures first");
        assert_eq!(p.proposals.last().map(|x| x.status), Some(Status::Unchecked));
        let h = p.proposals.iter().find(|x| x.rule == PKL_HASH).map(|x| x.action.as_str());
        assert_eq!(h, Some("recompute the hash"), "a finding's own remedy wins");
        let u = p.proposals.iter().find(|x| x.status == Status::Unchecked).map(|x| x.action.as_str());
        assert!(u.is_some_and(|a| a.contains("did not run")));
    }

    #[test]
    fn a_clean_report_has_nothing_to_repair() {
        let p = plan(&report(vec![Finding::new(AM_PRESENT, Status::Pass, "ok")]));
        assert!(p.proposals.is_empty());
        assert_eq!(RepairClass::ALL.iter().map(|c| p.count(*c)).sum::<usize>(), 0);
    }
}
