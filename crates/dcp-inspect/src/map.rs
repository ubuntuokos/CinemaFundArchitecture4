//! The engine's `--dump-result` JSON (at [`crate::ENGINE_COMMIT`]) and its mapping onto the rule
//! catalogue.
//!
//! The engine attaches checks (`kind`, `status`, `message`) to the asset maps, packing lists,
//! compositions and assets it read. A check whose kind matches a catalogue rule becomes a finding of
//! that rule; every other check, and every error and hint the engine printed, is kept under
//! [`REF_ENGINE`], so nothing the engine said is lost. A profile rule the engine reported nothing on
//! is `Unchecked`.

use std::collections::BTreeSet;
use std::path::Path;

use cfa4_dcp::{Standard, Uuid};
use cfa4_dcp_validate::rules::*;
use cfa4_dcp_validate::{DocumentIds, Finding, Location, Profile, Status};
use serde::Deserialize;

/// The rules the engine reports on.
pub(crate) const REPORTED: &[&str] = &[
    AM_PRESENT,
    AM_DOCUMENT,
    AM_FILE_EXISTS,
    PKL_DOCUMENT,
    PKL_HASH,
    CPL_DOCUMENT,
    J2K_CODESTREAM,
    AUD_PCM,
    AUD_LOUDNESS,
    IAB_TRACK,
    SUB_DOCUMENTS,
    NAME_ISDCF,
    SIG_SIGNATURE,
    SIG_CERTIFICATES,
    MARK_MARKERS,
    REF_ENGINE,
];

/// The `--dump-result` document. Only the fields the mapping uses are read; a result without them
/// is not of the pinned engine.
#[derive(Debug, Deserialize)]
pub struct EngineResult {
    pub errors: Vec<String>,
    pub hints: Vec<String>,
    pub am_files: Vec<String>,
    pub pkls: Vec<String>,
    pub cpls: Vec<String>,
    pub inspection_run: Option<InspectionRun>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InspectionRun {
    #[serde(default)]
    pub assetmaps: Vec<Subject>,
    #[serde(default)]
    pub packing_lists: Vec<Subject>,
    #[serde(default)]
    pub compositions: Vec<Subject>,
    #[serde(default)]
    pub assets: Vec<Subject>,
}

/// An asset map, packing list, composition or asset of the engine's model.
#[derive(Debug, Deserialize)]
pub struct Subject {
    pub id: Option<String>,
    /// The package-relative path (`assetmap_path` on an asset).
    #[serde(default, alias = "assetmap_path")]
    pub path: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub present: Option<bool>,
    #[serde(default)]
    pub checks: Vec<Check>,
}

#[derive(Debug, Deserialize)]
pub struct Check {
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Of {
    AssetMap,
    PackingList,
    Composition,
    Asset,
}

impl EngineResult {
    pub fn parse(text: &str) -> Result<EngineResult, serde_json::Error> {
        serde_json::from_str(text)
    }

    fn run(&self) -> Option<&InspectionRun> {
        self.inspection_run.as_ref()
    }

    /// SMPTE or Interop from the document namespaces; `None` when they disagree or are unknown.
    pub fn standard(&self) -> Option<Standard> {
        let run = self.run()?;
        let mut found: BTreeSet<u8> = BTreeSet::new();
        for s in run.assetmaps.iter().chain(&run.packing_lists).chain(&run.compositions) {
            match s.namespace.as_deref() {
                Some(ns) if ns.contains("smpte-ra.org") => found.insert(0),
                Some(ns) if ns.contains("digicine.com") => found.insert(1),
                _ => false,
            };
        }
        match found.into_iter().collect::<Vec<_>>()[..] {
            [0] => Some(Standard::Smpte),
            [1] => Some(Standard::Interop),
            _ => None,
        }
    }

    pub fn documents(&self) -> DocumentIds {
        let ids = |v: &[String]| v.iter().filter_map(|s| uuid(s)).collect::<Vec<_>>();
        let asset_map = self.run().and_then(|r| r.assetmaps.first()).and_then(|a| a.id.as_deref()).and_then(uuid);
        DocumentIds { asset_map, packing_lists: ids(&self.pkls), compositions: ids(&self.cpls) }
    }

    /// Assets the engine found in the package.
    pub fn assets_checked(&self) -> usize {
        self.run().map_or(0, |r| r.assets.iter().filter(|a| a.present == Some(true)).count())
    }
}

fn uuid(s: &str) -> Option<Uuid> {
    Uuid::parse(s).or_else(|| Uuid::parse_lenient(s))
}

/// The rule a check of `kind` on `of` belongs to.
fn rule_of(of: Of, kind: &str) -> &'static str {
    match (of, kind) {
        (Of::AssetMap, "schema") => AM_DOCUMENT,
        (Of::PackingList, "schema") | (Of::Asset, "type") => PKL_DOCUMENT,
        (Of::Composition, "schema") => CPL_DOCUMENT,
        (Of::Asset, "presence") => AM_FILE_EXISTS,
        (Of::Asset, "hash") => PKL_HASH,
        (Of::PackingList | Of::Composition, "signature") => SIG_SIGNATURE,
        (_, "dci_content_authenticator") => SIG_CERTIFICATES,
        (_, "jpeg2000") => J2K_CODESTREAM,
        (_, "iab") => IAB_TRACK,
        (_, "audio_channels") => AUD_PCM,
        (_, "composition_audio") => AUD_LOUDNESS,
        (_, "subtitles") => SUB_DOCUMENTS,
        (_, "naming") => NAME_ISDCF,
        (_, "markers") => MARK_MARKERS,
        _ => REF_ENGINE,
    }
}

fn status_of(kind: &str, status: &str) -> Status {
    match status {
        "ok" => Status::Pass,
        "error" => Status::Fail,
        "hint" | "warning" => Status::Warning,
        "skipped" => Status::Skipped,
        // an unsigned document: the signature rules do not apply
        "info" if matches!(kind, "signature" | "dci_content_authenticator") => Status::NotApplicable,
        "info" => Status::Pass,
        _ => Status::Unchecked,
    }
}

/// The text with the package root removed, so reports carry package-relative paths.
fn relative(text: &str, root: &str) -> String {
    if root.is_empty() {
        return text.to_string();
    }
    text.replace(&format!("{root}/"), "").replace(root, ".")
}

/// The findings of `result` under `profile`.
pub(crate) fn findings(result: &EngineResult, profile: &Profile, root: &Path) -> Vec<Finding> {
    let root = root.to_string_lossy();
    let mut all: Vec<Finding> = Vec::new();
    all.push(if result.am_files.is_empty() {
        Finding::new(AM_PRESENT, Status::Fail, "dcp_inspect found no asset map")
    } else {
        Finding::new(AM_PRESENT, Status::Pass, format!("dcp_inspect read {}", result.am_files.join(", ")))
    });
    let run = result.inspection_run.as_ref();
    let empty = InspectionRun::default();
    let run = run.unwrap_or(&empty);
    let groups = [(Of::AssetMap, &run.assetmaps), (Of::PackingList, &run.packing_lists), (Of::Composition, &run.compositions), (Of::Asset, &run.assets)];
    for (of, subjects) in groups {
        for s in subjects {
            let location = Location { file: s.path.as_deref().map(|p| relative(p, &root)), asset: s.id.as_deref().and_then(uuid), ..Location::default() };
            for c in &s.checks {
                let rule = rule_of(of, &c.kind);
                let mut message = relative(c.message.as_deref().unwrap_or(""), &root);
                if message.is_empty() {
                    message = format!("{}: {}", c.kind, c.status);
                }
                if rule == REF_ENGINE {
                    message = format!("{}: {message}", c.kind);
                }
                all.push(Finding::new(rule, status_of(&c.kind, &c.status), message).at(location.clone()).evidence(format!("dcp_inspect check {}", c.kind)));
            }
        }
    }
    for e in &result.errors {
        all.push(Finding::new(REF_ENGINE, Status::Fail, relative(e, &root)).evidence("dcp_inspect error"));
    }
    for h in &result.hints {
        all.push(Finding::new(REF_ENGINE, Status::Warning, relative(h, &root)).evidence("dcp_inspect hint"));
    }

    let mut out: Vec<Finding> = Vec::new();
    for pr in &profile.rules {
        let mine: Vec<Finding> = all.iter().filter(|f| f.rule == pr.rule).cloned().collect();
        if mine.is_empty() {
            out.push(Finding::new(&pr.rule, Status::Unchecked, "dcp_inspect reported nothing on this rule"));
        } else {
            out.extend(mine);
        }
    }
    if !profile.contains(REF_ENGINE) {
        out.extend(all.into_iter().filter(|f| f.rule == REF_ENGINE));
    }
    out
}
