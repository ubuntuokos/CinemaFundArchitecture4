//! The native backend: reads the package with `cfa4-dcp` and checks the rules it implements.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;

use cfa4_dcp::{AssetMap, Composition, Document, Issue, IssueKind, PackingList, Standard, Uuid};

use crate::integrity::{self, Files};
use crate::package::{self, Resolved};
use crate::report::{self, BackendInfo, Counts, DocumentIds, ProfileInfo, SCHEMA, ValidationReport, Verdict};
use crate::rules::{self, *};
use crate::{DcpValidator, Finding, Location, Status, ValidationControl, ValidationError, ValidationRequest, ValidatorCapabilities};

/// Rules the native backend checks today. The others of a profile are reported `Unchecked`.
const IMPLEMENTED: &[&str] = &[
    AM_PRESENT,
    AM_DOCUMENT,
    AM_UNIQUE_IDS,
    AM_PATH_SAFE,
    AM_FILE_EXISTS,
    AM_HAS_PKL,
    PKL_DOCUMENT,
    PKL_IN_ASSETMAP,
    PKL_SIZE,
    PKL_HASH,
    CPL_DOCUMENT,
    CPL_IN_PKL,
    CPL_ASSETS_IN_PKL,
    CPL_DURATIONS,
    CPL_REEL_SYNC,
    STD_CONSISTENT,
    MXF_TRACK_FILES,
    J2K_CODESTREAM,
    PIC_FORMAT,
    AUD_PCM,
    AUD_LOUDNESS,
    IAB_TRACK,
    MARK_MARKERS,
    SUB_DOCUMENTS,
];

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

/// The packing lists and compositions read, each with its chunk path.
type Read = (Vec<(String, PackingList)>, Vec<(String, Composition)>);

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

    /// Marks `rules` as done with `Unchecked`, because what they need could not be read. After a
    /// cancel they are left not done, so the report says they were interrupted.
    fn unreadable(&mut self, rules: &[&'static str], why: &str) {
        if self.control.is_cancelled() {
            return;
        }
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
        let downstream = [
            AM_DOCUMENT,
            AM_UNIQUE_IDS,
            AM_PATH_SAFE,
            AM_FILE_EXISTS,
            AM_HAS_PKL,
            PKL_DOCUMENT,
            PKL_IN_ASSETMAP,
            PKL_SIZE,
            PKL_HASH,
            CPL_DOCUMENT,
            CPL_IN_PKL,
            CPL_ASSETS_IN_PKL,
            CPL_DURATIONS,
            CPL_REEL_SYNC,
            STD_CONSISTENT,
            MXF_TRACK_FILES,
            J2K_CODESTREAM,
            PIC_FORMAT,
            AUD_PCM,
            AUD_LOUDNESS,
            IAB_TRACK,
            MARK_MARKERS,
            SUB_DOCUMENTS,
        ];
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
        let r = integrity::unique_ids(name, &am);
        self.rule(AM_UNIQUE_IDS, r);
        let r = integrity::has_pkl(name, &am);
        self.rule(AM_HAS_PKL, r);
        if self.control.is_cancelled() {
            return;
        }
        let files = self.chunks(name, &am);
        if self.control.is_cancelled() {
            return;
        }
        let Some((pkls, cpls)) = self.documents(&am, &files) else { return };
        self.integrity(&am, &files, &pkls, &cpls);
        if self.control.is_cancelled() {
            return;
        }
        self.media(&files, &cpls);
        if self.control.is_cancelled() {
            return;
        }
        self.audio(&files, &cpls);
        if self.control.is_cancelled() {
            return;
        }
        self.subtitles(&files, &cpls);
    }

    /// DCP-08: the subtitle and caption documents with their fonts and images.
    fn subtitles(&mut self, files: &Files, cpls: &[(String, Composition)]) {
        if !self.request.profile.contains(SUB_DOCUMENTS) {
            return;
        }
        if cpls.is_empty() {
            self.findings.push(Finding::new(SUB_DOCUMENTS, Status::Unchecked, "no composition playlist was read"));
            self.done.insert(SUB_DOCUMENTS);
            return;
        }
        let Some(r) = crate::subtitles::check(&self.root, cpls, files, self.request.limits.document, self.control) else { return };
        self.rule(SUB_DOCUMENTS, r);
    }

    /// DCP-07: sound, loudness and immersive audio. Measuring loudness reads every sample and
    /// parsing IAB every frame, so each runs only when the profile asks for it.
    fn audio(&mut self, files: &Files, cpls: &[(String, Composition)]) {
        let profile = &self.request.profile;
        if ![AUD_PCM, AUD_LOUDNESS, IAB_TRACK].iter().any(|r| profile.contains(r)) {
            return;
        }
        let Some(a) = crate::audio::check(cpls, files, profile.contains(AUD_LOUDNESS), profile.contains(IAB_TRACK), self.control) else { return };
        if cpls.is_empty() {
            for r in [AUD_PCM, AUD_LOUDNESS, IAB_TRACK] {
                self.findings.push(Finding::new(r, Status::Unchecked, "no composition playlist was read"));
            }
        }
        self.rule(AUD_PCM, a.pcm);
        if let Some(l) = a.loudness {
            self.rule(AUD_LOUDNESS, l);
        }
        if let Some(i) = a.iab {
            self.rule(IAB_TRACK, i);
        }
    }

    /// DCP-06: the track files and picture codestreams. Only a profile with one of its rules reads
    /// the track files, and only one with `DCP.J2K.001` reads every frame.
    fn media(&mut self, files: &Files, cpls: &[(String, Composition)]) {
        let profile = &self.request.profile;
        if ![MXF_TRACK_FILES, J2K_CODESTREAM, PIC_FORMAT].iter().any(|r| profile.contains(r)) {
            return;
        }
        let Some(m) = crate::media::check(cpls, files, profile.contains(J2K_CODESTREAM), self.control) else { return };
        if cpls.is_empty() {
            for r in [MXF_TRACK_FILES, J2K_CODESTREAM, PIC_FORMAT] {
                self.findings.push(Finding::new(r, Status::Unchecked, "no composition playlist was read"));
            }
        }
        self.rule(MXF_TRACK_FILES, m.mxf);
        if let Some(j) = m.j2k {
            self.rule(J2K_CODESTREAM, j);
        }
        self.rule(PIC_FORMAT, m.pic);
    }

    /// Records a pure rule's result: its findings, and its pass message when it has none.
    fn rule(&mut self, rule: &'static str, (findings, passed): (Vec<Finding>, String)) {
        self.findings.extend(findings);
        self.passed.insert(rule, passed);
        self.done.insert(rule);
    }

    /// DCP-05: the rules over the documents read.
    fn integrity(&mut self, am: &AssetMap, files: &Files, pkls: &[(String, PackingList)], cpls: &[(String, Composition)]) {
        let pkl_note = self.findings.iter().any(|f| f.rule == PKL_DOCUMENT && f.status == Status::Unchecked);
        self.rule(PKL_IN_ASSETMAP, integrity::pkl_in_assetmap(am, pkls));
        self.rule(PKL_SIZE, integrity::sizes(pkls, files));
        if self.request.profile.contains(PKL_HASH) {
            // hashing reads every byte, so only a profile that asks for it pays for it
            let Some(r) = integrity::hashes(pkls, files, self.control) else { return };
            self.rule(PKL_HASH, r);
        }
        if pkl_note {
            // packing lists that were not read leave the rules over them incomplete
            for r in [PKL_IN_ASSETMAP, PKL_SIZE, PKL_HASH, CPL_IN_PKL, CPL_ASSETS_IN_PKL] {
                self.findings.push(Finding::new(r, Status::Unchecked, "not every packing list was read (see DCP.PKL.001)"));
            }
        }
        if self.control.is_cancelled() {
            return;
        }
        // nothing read means nothing checked, not a pass
        let empty = |rules: &[&str], what: &str, out: &mut Vec<Finding>| {
            for r in rules {
                out.push(Finding::new(r, Status::Unchecked, format!("no {what} was read")));
            }
        };
        if pkls.is_empty() {
            empty(&[PKL_IN_ASSETMAP, PKL_SIZE, PKL_HASH], "packing list", &mut self.findings);
        }
        if cpls.is_empty() {
            empty(&[CPL_ASSETS_IN_PKL, CPL_DURATIONS, CPL_REEL_SYNC, MARK_MARKERS], "composition playlist", &mut self.findings);
        }
        let outside = self.cpls_outside_pkls(am, files, pkls);
        self.rule(CPL_IN_PKL, integrity::cpl_in_pkl(cpls.len(), &outside));
        self.rule(CPL_ASSETS_IN_PKL, integrity::cpl_assets_in_pkl(cpls, pkls, am));
        self.rule(CPL_DURATIONS, integrity::durations(cpls));
        self.rule(CPL_REEL_SYNC, integrity::reel_sync(cpls));
        self.rule(MARK_MARKERS, integrity::markers(cpls));
    }

    /// Compositions among the asset map's XML files that no packing list names.
    fn cpls_outside_pkls(&mut self, am: &AssetMap, files: &Files, pkls: &[(String, PackingList)]) -> Vec<(String, Option<Uuid>)> {
        let named: BTreeSet<Uuid> = pkls.iter().flat_map(|(_, p)| p.assets.iter().filter_map(|a| a.id)).collect();
        let mut out = Vec::new();
        let mut read = 0usize;
        for a in am.assets.iter().filter(|a| !a.packing_list) {
            let Some(id) = a.id else { continue };
            if named.contains(&id) {
                continue;
            }
            let Some((path, file)) = files.get(&id) else { continue };
            if !path.to_ascii_lowercase().ends_with(".xml") {
                continue;
            }
            read += 1;
            if read > self.request.limits.max_documents {
                self.findings.push(Finding::new(CPL_IN_PKL, Status::Unchecked, "too many unlisted XML files; the rest were not opened"));
                break;
            }
            if let Ok(Document::Composition(c)) = cfa4_dcp::read(file, self.request.limits.document) {
                out.push((path.clone(), c.id.or(Some(id))));
            }
        }
        out
    }

    /// Resolves every chunk; returns asset id → (path as written, file).
    fn chunks(&mut self, name: &str, am: &AssetMap) -> Files {
        let mut files = BTreeMap::new();
        let total = am.assets.len().min(self.request.limits.max_assets);
        let mut count = 0usize;
        // ST 429-9 Annex A: path values are unique
        let mut paths: HashSet<&str> = HashSet::new();
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
                if !paths.insert(path.as_str()) {
                    let at = Location {
                        file: Some(name.into()),
                        element: Some(format!("Chunk/Path '{path}'")),
                        line: Some(c.pos.line),
                        column: Some(c.pos.column),
                        asset: a.id,
                    };
                    self.fail(AM_DOCUMENT, at, format!("'{path}' is the path of more than one chunk"));
                }
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
                        // the optional chunk Length is the file's length (a single-chunk asset)
                        if let (Some(len), Ok(m)) = (c.length, std::fs::metadata(&f))
                            && c.offset.unwrap_or(0) == 0
                            && a.chunks.len() == 1
                            && m.len() != len
                        {
                            self.fail(AM_FILE_EXISTS, at.clone(), format!("'{path}' is {} bytes; the chunk Length is {len}", m.len()));
                        }
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

    /// Reads the packing lists and compositions; `None` when cancelled.
    fn documents(&mut self, am: &AssetMap, files: &Files) -> Option<Read> {
        let mut pkl_docs: Vec<(String, PackingList)> = Vec::new();
        let mut cpl_docs: Vec<(String, Composition)> = Vec::new();
        let limits = self.request.limits;
        let mut read = 0usize;
        let mut pkls_read = 0usize;
        // Only the ids of XML assets are kept from each packing list, not the parsed documents.
        let mut xml: BTreeSet<Uuid> = BTreeSet::new();
        let pkl_ids: Vec<Uuid> = am.assets.iter().filter(|a| a.packing_list).filter_map(|a| a.id).collect();
        let pkl_set: BTreeSet<Uuid> = pkl_ids.iter().copied().collect();
        // XML assets the packing lists name but the asset map cannot open are counted, not kept, so
        // the set stays bounded by the asset map.
        let mut xml_unopened = 0usize;
        let mut over_limit = false;
        for id in &pkl_ids {
            if self.control.is_cancelled() {
                return None;
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
                    for id in p.assets.iter().filter(|a| a.mime_type.as_deref().is_some_and(|t| t.to_ascii_lowercase().contains("xml"))).filter_map(|a| a.id) {
                        if pkl_set.contains(&id) {
                            continue;
                        }
                        if files.contains_key(&id) {
                            xml.insert(id);
                        } else {
                            xml_unopened = xml_unopened.saturating_add(1);
                        }
                    }
                    pkl_docs.push((path.clone(), p));
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
            return None;
        }

        if xml_unopened > 0 {
            let msg = format!("{xml_unopened} XML asset references of the packing lists are not among the asset map's readable files, so they were not opened");
            self.findings.push(Finding::new(CPL_DOCUMENT, Status::Unchecked, msg));
        }
        let xml: Vec<Uuid> = xml.into_iter().collect();
        let total = xml.len();
        let mut cpls = 0usize;
        for (k, id) in xml.iter().enumerate() {
            self.control.report("documents", k as u64, total as u64);
            if self.control.is_cancelled() {
                return None;
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
                    cpl_docs.push((path.clone(), c));
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
        Some((pkl_docs, cpl_docs))
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
