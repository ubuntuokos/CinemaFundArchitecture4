//! The native backend: reads the package with `cfa4-dcp` and checks the rules it implements.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use cfa4_dcp::{AssetMap, Document, Issue, IssueKind, Standard, Uuid};

use crate::package::{self, Resolved};
use crate::report::{self, BackendInfo, Counts, DocumentIds, ProfileInfo, SCHEMA, ValidationReport, Verdict};
use crate::rules::{self, *};
use crate::{DcpValidator, Finding, Location, Status, ValidationControl, ValidationError, ValidationRequest, ValidatorCapabilities};

/// Rules the native backend checks today. The others of a profile are reported `Unchecked`.
const IMPLEMENTED: &[&str] = &[AM_PRESENT, AM_DOCUMENT, AM_PATH_SAFE, AM_FILE_EXISTS, PKL_DOCUMENT, CPL_DOCUMENT, STD_CONSISTENT];

/// The native validator. Stateless; one value can validate many packages from many threads.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeValidator;

impl DcpValidator for NativeValidator {
    fn capabilities(&self) -> ValidatorCapabilities {
        ValidatorCapabilities { id: "native".into(), version: env!("CARGO_PKG_VERSION").into(), rules: IMPLEMENTED.iter().map(|r| (*r).to_string()).collect() }
    }

    fn validate(&self, request: &ValidationRequest, control: &ValidationControl) -> Result<ValidationReport, ValidationError> {
        let started_at = report::now();
        let root = request.root.canonicalize().map_err(|_| ValidationError::NotADirectory(request.root.clone()))?;
        if !root.is_dir() {
            return Err(ValidationError::NotADirectory(request.root.clone()));
        }
        let mut run = Run {
            root,
            request,
            control,
            findings: Vec::new(),
            done: BTreeSet::new(),
            passed: BTreeMap::new(),
            docs: DocumentIds::default(),
            standard: None,
            assets_checked: 0,
            standards: Vec::new(),
        };
        run.package();
        let interrupted = control.is_cancelled();
        let profile = &request.profile;
        let mut findings: Vec<Finding> = Vec::new();
        for pr in &profile.rules {
            let id = pr.rule.as_str();
            let mine: Vec<Finding> = run.findings.iter().filter(|f| f.rule == id).cloned().collect();
            if !IMPLEMENTED.contains(&id) {
                let task = rules::find(id).map_or("a later work package", |r| r.task);
                findings.push(Finding::new(id, Status::Unchecked, format!("not implemented by the native backend yet ({task})")));
            } else if !run.done.contains(id) {
                findings.extend(mine);
                let status = if interrupted { Status::Interrupted } else { Status::Unchecked };
                findings.push(Finding::new(id, status, if interrupted { "cancelled before this check finished" } else { "not reached" }));
            } else if mine.is_empty() {
                findings.push(Finding::new(id, Status::Pass, run.passed.remove(id).unwrap_or_else(|| "ok".into())));
            } else {
                findings.extend(mine);
            }
        }
        let verdict = Verdict::of(profile, &findings);
        Ok(ValidationReport {
            schema: SCHEMA.into(),
            verdict,
            profile: ProfileInfo { id: profile.id.clone(), version: profile.version, title: profile.title.clone() },
            backend: BackendInfo { id: "native".into(), version: env!("CARGO_PKG_VERSION").into() },
            root: request.root.to_string_lossy().into_owned(),
            standard: run.standard,
            documents: run.docs,
            assets_checked: run.assets_checked,
            counts: Counts::of(&findings),
            findings,
            started_at,
            finished_at: report::now(),
        })
    }
}

struct Run<'a> {
    root: PathBuf,
    request: &'a ValidationRequest,
    control: &'a ValidationControl,
    findings: Vec<Finding>,
    /// Rules whose check ran to the end.
    done: BTreeSet<&'static str>,
    /// The message of a rule that passed.
    passed: BTreeMap<&'static str, String>,
    docs: DocumentIds,
    standard: Option<Standard>,
    assets_checked: usize,
    /// (file, standard) of every document read.
    standards: Vec<(String, Standard)>,
}

impl Run<'_> {
    fn fail(&mut self, rule: &str, at: Location, message: impl Into<String>) {
        self.findings.push(Finding::new(rule, Status::Fail, message).at(at));
    }

    /// Marks `rules` as done with `Unchecked`, because what they need could not be read.
    fn unreadable(&mut self, rules: &[&'static str], why: &str) {
        for &r in rules {
            self.findings.push(Finding::new(r, Status::Unchecked, why.to_string()));
            self.done.insert(r);
        }
    }

    /// The reader's issues of one document as findings of `rule`.
    fn issues(&mut self, rule: &str, file: &str, issues: &[Issue]) {
        for i in issues {
            let status = if i.kind == IssueKind::Unknown { Status::Warning } else { Status::Fail };
            let at = Location { file: Some(file.into()), element: Some(i.path.clone()), line: Some(i.pos.line), column: Some(i.pos.column), asset: None };
            self.findings.push(Finding::new(rule, status, i.message.clone()).at(at));
        }
    }

    fn package(&mut self) {
        let downstream = [AM_DOCUMENT, AM_PATH_SAFE, AM_FILE_EXISTS, PKL_DOCUMENT, CPL_DOCUMENT, STD_CONSISTENT];
        self.control.report("discover", 0, 1);
        let maps = package::asset_maps(&self.root);
        let Some(&name) = maps.first() else {
            self.fail(AM_PRESENT, Location::default(), "no ASSETMAP.xml or ASSETMAP in the package root");
            self.done.insert(AM_PRESENT);
            self.unreadable(&downstream, "the asset map was not read");
            return;
        };
        if maps.len() > 1 {
            self.findings
                .push(Finding::new(AM_PRESENT, Status::Warning, "both ASSETMAP.xml and ASSETMAP are present; ASSETMAP.xml is used").at(Location::file(name)));
        }
        let am = match cfa4_dcp::read(&self.root.join(name), self.request.limits.document) {
            Ok(Document::AssetMap(am)) => am,
            Ok(other) => {
                self.fail(AM_PRESENT, Location::file(name), format!("{name} is not an asset map but a {}", kind(&other)));
                self.done.insert(AM_PRESENT);
                self.unreadable(&downstream, "the asset map was not read");
                return;
            }
            Err(e) => {
                self.fail(AM_PRESENT, Location::file(name), format!("{name} cannot be read: {e}"));
                self.done.insert(AM_PRESENT);
                self.unreadable(&downstream, "the asset map was not read");
                return;
            }
        };
        match (am.standard, name) {
            // ST 429-9:2014 names the file ASSETMAP.xml; the extensionless name is Interop's
            (Standard::Smpte, "ASSETMAP") => {
                self.fail(AM_PRESENT, Location::file(name), "a SMPTE asset map must be named ASSETMAP.xml (ST 429-9); ASSETMAP is the Interop name")
            }
            (Standard::Interop, "ASSETMAP.xml") => self.findings.push(
                Finding::new(AM_PRESENT, Status::Warning, "an Interop asset map is named ASSETMAP; some players do not look for ASSETMAP.xml")
                    .at(Location::file(name)),
            ),
            _ => {}
        }
        self.passed.insert(AM_PRESENT, format!("{name} read"));
        self.done.insert(AM_PRESENT);
        self.docs.asset_map = am.id;
        self.standard = Some(am.standard);
        self.standards.push((name.to_string(), am.standard));
        self.issues(AM_DOCUMENT, name, &am.issues);
        if am.assets.len() > self.request.limits.max_assets {
            self.fail(
                AM_DOCUMENT,
                Location::file(name),
                format!("{} assets, over the limit of {}; the rest is not read", am.assets.len(), self.request.limits.max_assets),
            );
        }
        self.passed.insert(AM_DOCUMENT, format!("{} assets", am.assets.len()));
        self.done.insert(AM_DOCUMENT);
        if self.control.is_cancelled() {
            return;
        }
        let files = self.chunks(name, &am);
        if self.control.is_cancelled() {
            return;
        }
        self.documents(&am, &files);
    }

    /// Resolves every chunk; returns asset id → (path as written, file).
    fn chunks(&mut self, name: &str, am: &AssetMap) -> BTreeMap<Uuid, (String, PathBuf)> {
        let mut files = BTreeMap::new();
        let total = am.assets.len().min(self.request.limits.max_assets);
        let mut count = 0usize;
        for (k, a) in am.assets.iter().take(total).enumerate() {
            if k % 64 == 0 {
                self.control.report("discover", k as u64, total as u64);
                if self.control.is_cancelled() {
                    return files;
                }
            }
            for c in &a.chunks {
                let Some(path) = &c.path else { continue };
                count += 1;
                let at = Location {
                    file: Some(name.into()),
                    element: Some(format!("Chunk/Path '{path}'")),
                    line: Some(c.pos.line),
                    column: Some(c.pos.column),
                    asset: a.id,
                };
                match package::resolve(&self.root, path) {
                    Resolved::File(f) => {
                        self.assets_checked += 1;
                        if let Some(id) = a.id {
                            files.entry(id).or_insert((path.clone(), f));
                        }
                    }
                    Resolved::Missing => self.fail(AM_FILE_EXISTS, at, format!("'{path}' does not exist")),
                    Resolved::NotAFile => self.fail(AM_FILE_EXISTS, at, format!("'{path}' is not a regular file")),
                    Resolved::Unsafe(why) => self.fail(AM_PATH_SAFE, at, format!("'{path}' is refused: {why}")),
                }
            }
        }
        if am.assets.len() > total {
            let msg = format!("only the first {total} of {} assets were checked (limit)", am.assets.len());
            self.findings.push(Finding::new(AM_PATH_SAFE, Status::Unchecked, msg.clone()));
            self.findings.push(Finding::new(AM_FILE_EXISTS, Status::Unchecked, msg));
        }
        self.passed.insert(AM_PATH_SAFE, format!("{count} chunk paths inside the package"));
        self.passed.insert(AM_FILE_EXISTS, format!("{} files present", self.assets_checked));
        self.done.insert(AM_PATH_SAFE);
        self.done.insert(AM_FILE_EXISTS);
        files
    }

    fn documents(&mut self, am: &AssetMap, files: &BTreeMap<Uuid, (String, PathBuf)>) {
        let limits = self.request.limits;
        let mut read = 0usize;
        let mut pkls_read = 0usize;
        // Only the ids of XML assets are kept from each packing list, not the parsed documents.
        let mut xml: BTreeSet<Uuid> = BTreeSet::new();
        let pkl_ids: Vec<Uuid> = am.assets.iter().filter(|a| a.packing_list).filter_map(|a| a.id).collect();
        let mut over_limit = false;
        for id in &pkl_ids {
            if self.control.is_cancelled() {
                return;
            }
            let Some((path, file)) = files.get(id) else {
                let msg = format!("packing list {id} could not be opened (see DCP.AM.004 / DCP.AM.005)");
                self.findings.push(Finding::new(PKL_DOCUMENT, Status::Unchecked, msg).at(Location { asset: Some(*id), ..Location::default() }));
                continue;
            };
            read += 1;
            if read > limits.max_documents {
                over_limit = true;
                break;
            }
            match cfa4_dcp::read(file, limits.document) {
                Ok(Document::PackingList(p)) => {
                    pkls_read += 1;
                    self.issues(PKL_DOCUMENT, path, &p.issues);
                    self.standards.push((path.clone(), p.standard));
                    if let Some(id) = p.id {
                        self.docs.packing_lists.push(id);
                    }
                    xml.extend(p.assets.iter().filter(|a| a.mime_type.as_deref().is_some_and(|t| t.contains("xml"))).filter_map(|a| a.id));
                }
                Ok(other) => self.fail(PKL_DOCUMENT, Location::file(path.as_str()), format!("declared as a packing list, but it is a {}", kind(&other))),
                Err(e) => self.fail(PKL_DOCUMENT, Location::file(path.as_str()), format!("cannot be read: {e}")),
            }
        }
        if over_limit {
            let msg = format!("more than {} documents; the remaining packing lists were not read", limits.max_documents);
            self.findings.push(Finding::new(PKL_DOCUMENT, Status::Unchecked, msg));
        }
        if pkl_ids.is_empty() {
            self.fail(PKL_DOCUMENT, Location::default(), "the asset map declares no packing list");
        }
        self.passed.insert(PKL_DOCUMENT, format!("{pkls_read} packing lists read"));
        self.done.insert(PKL_DOCUMENT);
        if self.control.is_cancelled() {
            return;
        }

        let xml: Vec<Uuid> = xml.into_iter().filter(|id| !pkl_ids.contains(id)).collect();
        let total = xml.len();
        let mut cpls = 0usize;
        for (k, id) in xml.iter().enumerate() {
            self.control.report("documents", k as u64, total as u64);
            if self.control.is_cancelled() {
                return;
            }
            let Some((path, file)) = files.get(id) else {
                let msg = format!("XML asset {id} of a packing list is not in the asset map's readable files, so it was not opened");
                self.findings.push(Finding::new(CPL_DOCUMENT, Status::Unchecked, msg).at(Location { asset: Some(*id), ..Location::default() }));
                continue;
            };
            read += 1;
            if read > limits.max_documents {
                let msg = format!("more than {} documents; the remaining XML assets were not read", limits.max_documents);
                self.findings.push(Finding::new(CPL_DOCUMENT, Status::Unchecked, msg));
                break;
            }
            match cfa4_dcp::read(file, limits.document) {
                Ok(Document::Composition(c)) => {
                    cpls += 1;
                    self.issues(CPL_DOCUMENT, path, &c.issues);
                    self.standards.push((path.clone(), c.standard));
                    if let Some(id) = c.id {
                        self.docs.compositions.push(id);
                    }
                }
                // subtitles, KDMs and other XML assets are other rules' business
                Ok(_) | Err(cfa4_dcp::Error::UnknownDocument(_)) => {}
                Err(e) => self.fail(CPL_DOCUMENT, Location::file(path.as_str()).with_asset(*id), format!("XML asset cannot be read: {e}")),
            }
        }
        if cpls == 0 && pkls_read > 0 && !self.findings.iter().any(|f| f.rule == CPL_DOCUMENT && f.status == Status::Unchecked) {
            self.fail(CPL_DOCUMENT, Location::default(), "no composition playlist among the packing list assets");
        }
        self.passed.insert(CPL_DOCUMENT, format!("{cpls} composition playlists read"));
        self.done.insert(CPL_DOCUMENT);
        self.standards();
    }

    fn standards(&mut self) {
        let expected = self.request.profile.standard.or(self.standard);
        let wrong: Vec<(String, Standard)> = self.standards.iter().filter(|(_, s)| Some(*s) != expected).cloned().collect();
        for (file, s) in wrong {
            let want = expected.map_or_else(String::new, |e| format!("{e:?}"));
            self.fail(STD_CONSISTENT, Location::file(file), format!("the document is {s:?}, expected {want}"));
        }
        self.passed.insert(STD_CONSISTENT, format!("{} documents, all {}", self.standards.len(), expected.map_or_else(String::new, |e| format!("{e:?}"))));
        self.done.insert(STD_CONSISTENT);
    }
}

impl Location {
    fn with_asset(mut self, id: Uuid) -> Location {
        self.asset = Some(id);
        self
    }
}

fn kind(d: &Document) -> &'static str {
    match d {
        Document::AssetMap(_) => "asset map",
        Document::VolumeIndex(_) => "volume index",
        Document::PackingList(_) => "packing list",
        Document::Composition(_) => "composition playlist",
    }
}
