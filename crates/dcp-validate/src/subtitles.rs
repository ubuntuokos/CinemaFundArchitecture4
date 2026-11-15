//! DCP-08 (plan F12): `DCP.SUB.001`, the subtitle and caption documents of the reels and the
//! fonts and images they load. The documents are read with `cfa4_dcp::subtitle`: SMPTE
//! `SubtitleReel` from ST 429-5 timed text track files (`cfa4-mxf`), Interop `DCSubtitle` XML
//! files with their fonts and PNG files next to them.
//!
//! `FAIL`:
//! - the file is neither a subtitle document nor a timed text track file holding one, or the
//!   document cannot be read, or the reader found a missing, invalid or repeated element;
//! - the document is of the other standard than the composition;
//! - its id is not the one the package gives it (Interop `SubtitleID` = the reel asset's `Id`;
//!   SMPTE `Id` = the track file's timed text resource id, ST 429-5);
//! - SMPTE: its `EditRate` is not the reel asset's;
//! - a font or image it loads is missing (SMPTE: no ancillary resource with that id; Interop: no
//!   such file inside the package), or an image is not PNG;
//! - a `Font` names an id no `LoadFont` declares;
//! - a subtitle does not end after it starts, starts before `StartTime` or ends after the reel.
//!
//! `WARNING`: fades longer than the subtitle, subtitles out of time order or spot-number order,
//! a subtitle with neither text nor image.
//!
//! Not checked: the XML schemas themselves beyond what the reader enforces, the glyphs of the
//! fonts (whether every character has one) and how the subtitles render.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cfa4_dcp::subtitle::{self, SubtitleDocument};
use cfa4_dcp::{AssetKind, Composition, IssueKind, ReelAsset, Standard, Uuid};

use crate::integrity::{Files, asset_element, at};
use cfa4_mxf::ByteSource;

use crate::media::FileSource;
use crate::package::{self, Resolved};
use crate::rules::SUB_DOCUMENTS;
use crate::{Finding, Location, Status, ValidationControl};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Bytes read to tell an MXF file from an XML document.
const HEAD: u64 = 65_536;

/// The findings of one document, the same problem once with the first place and a count.
#[derive(Default)]
struct Tally {
    order: Vec<(Status, String)>,
    seen: BTreeMap<(Status, String), (Location, usize)>,
}

impl Tally {
    fn add(&mut self, status: Status, message: String, at: Location) {
        let key = (status, message);
        if let Some(e) = self.seen.get_mut(&key) {
            e.1 += 1;
        } else {
            self.order.push(key.clone());
            self.seen.insert(key, (at, 1));
        }
    }

    fn findings(mut self) -> Vec<Finding> {
        self.order
            .into_iter()
            .filter_map(|k| {
                let (at, n) = self.seen.remove(&k)?;
                let more = if n > 1 { format!(" ({n} subtitles)") } else { String::new() };
                Some(Finding::new(SUB_DOCUMENTS, k.0, format!("{}{more}", k.1)).at(at))
            })
            .collect()
    }
}

fn is_subtitle(kind: AssetKind) -> bool {
    matches!(kind, AssetKind::MainSubtitle | AssetKind::MainCaption | AssetKind::MainClosedCaption)
}

/// What was read for one reel asset.
enum Read {
    Document {
        doc: SubtitleDocument,
        resources: Resources,
    },
    /// Not checkable, with the reason (`UNCHECKED`).
    Unchecked(String),
    /// Broken, with the reason (`FAIL`).
    Broken(String),
}

/// How the fonts and images a document names are found.
enum Resources {
    /// SMPTE: the ancillary resources of the track file, id → is PNG.
    Mxf(BTreeMap<String, bool>),
    /// Interop: files beside the subtitle file (its directory inside the package).
    Files(String),
}

fn read(path: &str, file: &Path, limits: cfa4_dcp::Limits) -> Read {
    let src = match FileSource::open(file) {
        Ok(s) => s,
        Err(e) => return Read::Broken(format!("'{path}' cannot be opened: {e}")),
    };
    // enough for the MXF partition pack and header metadata labels, or the XML root element
    let mut head = vec![0; usize::try_from(src.len().min(HEAD)).unwrap_or(0)];
    if let Err(e) = src.read_at(0, &mut head) {
        return Read::Broken(format!("'{path}' cannot be read: {e}"));
    }
    if cfa4_mxf::sniff(&head) {
        if !cfa4_mxf::is_timed_text(&head) {
            return Read::Broken(format!("'{path}' is an MXF file, but not timed text (ST 429-5)"));
        }
        let t = match cfa4_mxf::open_timed_text(&src) {
            Ok(t) => t,
            Err(e) => return Read::Broken(format!("'{path}' is not a readable timed text track file: {e}")),
        };
        if t.encryption.is_some() {
            return Read::Unchecked(format!("'{path}' is encrypted; its subtitles need the content key"));
        }
        let bytes = match t.read_document(&src) {
            Ok(b) => b,
            Err(e) => return Read::Broken(format!("'{path}': the subtitle document cannot be read: {e}")),
        };
        let mut res = BTreeMap::new();
        for (i, r) in t.resources.iter().enumerate() {
            let png = t.read_resource(&src, i).is_ok_and(|b| b.starts_with(PNG));
            res.insert(Uuid(r.id).to_string(), png);
        }
        let doc = match subtitle::parse_bytes(&bytes, limits) {
            Ok(mut d) => {
                // ST 429-5: the document's Id is the timed text resource id
                if let (Some(rid), Some(id)) = (t.resource_id, d.id)
                    && rid != id.0
                {
                    d.issues.push(cfa4_dcp::Issue {
                        kind: IssueKind::Invalid,
                        path: "SubtitleReel/Id".into(),
                        pos: cfa4_dcp::Pos::default(),
                        message: format!("Id {id} is not the track file's timed text resource id {}", Uuid(rid)),
                    });
                }
                d
            }
            Err(e) => return Read::Broken(format!("'{path}': the subtitle document cannot be read: {e}")),
        };
        return Read::Document { doc, resources: Resources::Mxf(res) };
    }
    let text = String::from_utf8_lossy(&head);
    if !subtitle::sniff(&text) {
        return Read::Broken(format!("'{path}' is neither a subtitle document nor a timed text track file"));
    }
    match std::fs::read(file).map_err(|e| e.to_string()).and_then(|b| subtitle::parse_bytes(&b, limits).map_err(|e| e.to_string())) {
        Ok(doc) => {
            let dir = path.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
            Read::Document { doc, resources: Resources::Files(dir) }
        }
        Err(e) => Read::Broken(format!("'{path}' cannot be read: {e}")),
    }
}

/// Whether the resource `name` exists, and whether it is PNG.
fn resource(root: &Path, res: &Resources, name: &str) -> Option<bool> {
    match res {
        Resources::Mxf(m) => {
            let id = Uuid::parse(name).or_else(|| Uuid::parse_lenient(name))?.to_string();
            m.get(&id).copied()
        }
        Resources::Files(dir) => match package::resolve(root, &format!("{dir}{name}")) {
            Resolved::File(f) => {
                let mut head = [0u8; 8];
                let png = std::fs::File::open(f).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok() && head == PNG;
                Some(png)
            }
            _ => None,
        },
    }
}

/// `DCP.SUB.001`; `None` when cancelled.
pub(crate) fn check(
    root: &Path,
    cpls: &[(String, Composition)],
    files: &Files,
    limits: cfa4_dcp::Limits,
    control: &ValidationControl,
) -> Option<(Vec<Finding>, String)> {
    let mut out = Vec::new();
    let mut cache: BTreeMap<Uuid, Read> = BTreeMap::new();
    let mut documents = BTreeSet::new();
    let mut subtitles = 0usize;
    for (cpl, c) in cpls {
        for (r, reel) in c.reels.iter().enumerate() {
            for a in reel.assets.iter().filter(|a| is_subtitle(a.kind)) {
                if control.is_cancelled() {
                    return None;
                }
                let Some(id) = a.id else { continue };
                // a reel asset outside the package (a version file) is DCP.CPL.003's
                let Some((path, file)) = files.get(&id) else { continue };
                let loc = at(cpl, asset_element(r, a), Some(a.pos), a.id);
                let here = Location { file: Some(path.clone()), asset: Some(id), ..Location::default() };
                if a.key_id.is_some() {
                    out.push(Finding::new(SUB_DOCUMENTS, Status::Unchecked, format!("'{path}' is encrypted; its subtitles need the content key")).at(loc));
                    continue;
                }
                let read = cache.entry(id).or_insert_with(|| read(path, file, limits));
                let (doc, resources) = match read {
                    Read::Document { doc, resources } => (doc, resources),
                    Read::Unchecked(m) => {
                        out.push(Finding::new(SUB_DOCUMENTS, Status::Unchecked, m.clone()).at(here));
                        continue;
                    }
                    Read::Broken(m) => {
                        out.push(Finding::new(SUB_DOCUMENTS, Status::Fail, m.clone()).at(here));
                        continue;
                    }
                };
                let mut t = Tally::default();
                let first = documents.insert(id);
                if first {
                    subtitles += doc.subtitles.len();
                    document(root, path, id, doc, resources, &mut t);
                }
                reel_checks(path, c.standard, a, doc, &loc, &mut t);
                out.extend(t.findings());
            }
        }
    }
    Some((out, format!("{} subtitle documents with {subtitles} subtitles", documents.len())))
}

/// The checks of the document itself, once per file.
fn document(root: &Path, path: &str, id: Uuid, doc: &SubtitleDocument, res: &Resources, t: &mut Tally) {
    let at = |element: &str, pos: Option<cfa4_dcp::Pos>| Location {
        file: Some(path.to_string()),
        element: Some(element.to_string()),
        line: pos.map(|p| p.line),
        column: pos.map(|p| p.column),
        asset: Some(id),
    };
    for i in &doc.issues {
        let status = if i.kind == IssueKind::Unknown { Status::Warning } else { Status::Fail };
        t.add(status, format!("'{path}': {}", i.message), at(&i.path, Some(i.pos)));
    }
    // Interop: the SubtitleID is the reel asset's id
    if doc.standard == Standard::Interop
        && let Some(sid) = doc.id
        && sid != id
    {
        t.add(Status::Fail, format!("'{path}': SubtitleID {sid} is not the reel asset's Id {id}"), at("DCSubtitle/SubtitleID", None));
    }
    let loaded: BTreeSet<&str> = doc.fonts.iter().map(|f| f.id.as_str()).collect();
    for f in &doc.fonts {
        if resource(root, res, &f.resource).is_none() {
            t.add(Status::Fail, format!("'{path}': the font '{}' ({}) is not in the package", f.id, f.resource), at("LoadFont", Some(f.pos)));
        }
    }
    let mut last_in = None;
    let mut last_spot = None;
    for s in &doc.subtitles {
        let here = at(&s.path, Some(s.pos));
        for f in s.fonts.iter().filter(|f| !loaded.contains(f.as_str())) {
            t.add(Status::Fail, format!("'{path}': a Font names '{f}', which no LoadFont declares"), here.clone());
        }
        for img in &s.images {
            match resource(root, res, img) {
                None => t.add(Status::Fail, format!("'{path}': the image '{img}' is not in the package"), here.clone()),
                Some(false) => t.add(Status::Fail, format!("'{path}': the image '{img}' is not PNG"), here.clone()),
                Some(true) => {}
            }
        }
        if s.texts == 0 && s.images.is_empty() {
            t.add(Status::Warning, format!("'{path}': a subtitle has neither text nor image"), here.clone());
        }
        if let (Some(i), Some(o)) = (s.time_in, s.time_out) {
            if o <= i {
                t.add(Status::Fail, format!("'{path}': a subtitle does not end after it starts (TimeOut ≤ TimeIn)"), here.clone());
            } else if s.fade_up.unwrap_or(0) + s.fade_down.unwrap_or(0) > o - i {
                t.add(Status::Warning, format!("'{path}': a subtitle's fades are longer than the subtitle"), here.clone());
            }
            if i < doc.start_time {
                t.add(Status::Fail, format!("'{path}': a subtitle starts before the StartTime"), here.clone());
            }
            if last_in.is_some_and(|l| i < l) {
                t.add(Status::Warning, format!("'{path}': subtitles are not in time order"), here.clone());
            }
            last_in = Some(i);
        }
        if let Some(n) = s.spot_number {
            if last_spot.is_some_and(|l| n <= l) {
                t.add(Status::Warning, format!("'{path}': spot numbers do not increase"), here.clone());
            }
            last_spot = Some(n);
        }
    }
}

/// The checks against the reel asset that plays the document.
fn reel_checks(path: &str, standard: Standard, a: &ReelAsset, doc: &SubtitleDocument, loc: &Location, t: &mut Tally) {
    if doc.standard != standard {
        let name = |s: Standard| if s == Standard::Interop { "an Interop" } else { "a SMPTE" };
        t.add(Status::Fail, format!("'{path}' is {} subtitle document in {} composition", name(doc.standard), name(standard)), loc.clone());
    }
    if let (Some(dr), Some(ar)) = (doc.edit_rate, a.edit_rate)
        && !dr.same_value(ar)
    {
        t.add(Status::Fail, format!("'{path}' has the EditRate {dr}, the reel asset {ar}"), loc.clone());
    }
    // the reel's length in time units: IntrinsicDuration edit units at the reel asset's rate
    let (Some(d), Some(rate)) = (a.intrinsic_duration, a.edit_rate) else { return };
    if rate.num <= 0 || rate.den <= 0 {
        return;
    }
    let tcr = u128::from(doc.time_code_rate);
    // `units` time units after StartTime are past the reel when units·num > d·den·tcr
    let past = |units: u64| u128::from(units) * rate.num as u128 > u128::from(d) * rate.den as u128 * tcr;
    for s in &doc.subtitles {
        if let Some(o) = s.time_out
            && past(o.saturating_sub(doc.start_time))
        {
            let here =
                Location { element: Some(s.path.clone()), line: Some(s.pos.line), column: Some(s.pos.column), file: Some(path.to_string()), asset: a.id };
            t.add(Status::Fail, format!("'{path}': a subtitle ends after the reel ({d} edit units at {rate})"), here);
        }
    }
}
