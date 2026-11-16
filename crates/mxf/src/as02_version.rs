//! Reading an AMWA AS-02 version file: an OP1b / OP2 / OP3 file whose header metadata holds one
//! material package and a copy of the file package of every essence component file, with no
//! essence of its own (AS-02 sections 4.3.5, 5.2, 5.4). The essence lives in the component files
//! of the bundle's `media/` folder; the version says which parts of which component play, in
//! which order, and each file package's network locator says where its file is.

use crate::file::{TrackKind, components, track_kind_of, umid_of, walk};
use crate::meta::{Set, set_type as st, tag};
use crate::source::ByteSource;
use crate::{Rational, as02};

/// One piece of a version's track: a range of a component file's track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionClip {
    /// UMID of the file package (the essence component), `None` for a filler (blank).
    pub package: Option<[u8; 32]>,
    /// Track id inside that package.
    pub source_track: u32,
    /// First edit unit used, in the version track's edit rate.
    pub start: i64,
    /// Edit units played.
    pub duration: i64,
}

/// A track of the version's material package.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionTrack {
    pub track_id: u32,
    pub track_number: u32,
    pub kind: TrackKind,
    pub name: Option<String>,
    pub edit_rate: Rational,
    pub clips: Vec<VersionClip>,
}

/// The copy of an essence component's file package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRef {
    pub umid: [u8; 32],
    /// The network locator: the component file's path relative to the version file.
    pub locator: Option<String>,
}

/// What a version file says.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionFile {
    pub material_umid: [u8; 32],
    pub name: Option<String>,
    /// `OP1b`, `OP2a`, …
    pub operational_pattern: String,
    pub tracks: Vec<VersionTrack>,
    pub components: Vec<ComponentRef>,
    /// Breaches of the AS-02 rules found while reading (empty for a conforming file).
    pub problems: Vec<String>,
}

impl VersionFile {
    /// The essence component a package UMID names.
    pub fn component(&self, umid: &[u8; 32]) -> Option<&ComponentRef> {
        self.components.iter().find(|c| &c.umid == umid)
    }

    /// The manifest identifier of the version (the material package's UMID, 9.2.1).
    pub fn file_id(&self) -> String {
        as02::file_id_of_umid(&self.material_umid)
    }
}

fn locator_of(md: &crate::meta::Metadata, pkg: &Set) -> (Option<String>, usize) {
    let Some(desc) = pkg.reference(tag::SOURCE_PACKAGE_DESCRIPTOR).and_then(|r| md.get(&r)) else { return (None, 0) };
    // a multiple descriptor keeps the locators of the file in itself or in its sub-descriptors
    let mut sets: Vec<&Set> = vec![desc];
    sets.extend(desc.references(tag::SUB_DESCRIPTORS).iter().filter_map(|r| md.get(r)));
    let urls: Vec<String> = sets
        .iter()
        .flat_map(|d| d.references(tag::LOCATORS))
        .filter_map(|r| md.get(&r))
        .filter(|l| l.kind() == Some(st::NETWORK_LOCATOR))
        .filter_map(|l| l.string(tag::URL_STRING))
        .collect();
    (urls.first().cloned(), urls.len())
}

/// Read a version file. The problems of a file that can be read but breaks the AS-02 rules are
/// listed in [`VersionFile::problems`]; only a file with no usable header metadata or no material
/// package is an error.
pub fn read_version(src: &(impl ByteSource + ?Sized)) -> Result<VersionFile, String> {
    let w = walk(src).map_err(|e| e.to_string())?;
    let (_, md) = crate::file::best_metadata(&w).map_err(|e| e.to_string())?;
    let mut problems: Vec<String> = Vec::new();
    let preface = md.of_kind(st::PREFACE).next();
    let op_label = preface.and_then(|p| p.ul(tag::OPERATIONAL_PATTERN)).unwrap_or(w.partitions.first().map(|p| p.operational_pattern).unwrap_or_default());
    let op = crate::file::OperationalPattern::from_ul(&op_label);
    let operational_pattern = op.name();
    if !matches!(op, crate::file::OperationalPattern::Generalized { item: 1..=3, package: 2 } | crate::file::OperationalPattern::Generalized { item: 2..=3, package: 1 }) {
        problems.push(format!("the operational pattern is {operational_pattern}, a version file is OP1b, OP2a, OP2b, OP3a or OP3b"));
    }
    if md.of_kind(st::ESSENCE_CONTAINER_DATA).next().is_some() {
        problems.push("a version file has no Essence Container Data (all essence is external)".into());
    }
    if !w.elements.is_empty() {
        problems.push("a version file holds no essence".into());
    }
    if !w.index.is_empty() {
        problems.push("a version file holds no essence index tables".into());
    }
    let materials: Vec<&Set> = md.of_kind(st::MATERIAL_PACKAGE).collect();
    if materials.len() != 1 {
        problems.push(format!("a version file has one material package, not {}", materials.len()));
    }
    let material = materials.first().copied().ok_or("no material package in the header metadata")?;
    if let Some(p) = preface
        && let Some(primary) = p.reference(0x3B08)
        && material.instance_uid != primary
    {
        problems.push("the primary package is not the material package".into());
    }
    let material_umid = umid_of(material, tag::PACKAGE_UID).ok_or("the material package has no UMID")?;

    let mut components_out = Vec::new();
    for pkg in md.of_kind(st::SOURCE_PACKAGE) {
        // top-level file packages carry a descriptor; the lower-level (tape) packages do not need a locator
        if pkg.reference(tag::SOURCE_PACKAGE_DESCRIPTOR).is_none() {
            continue;
        }
        let Some(umid) = umid_of(pkg, tag::PACKAGE_UID) else { continue };
        let (locator, n) = locator_of(&md, pkg);
        if n != 1 {
            problems.push(format!("the file package {} has {n} network locators, it needs one", as02::file_id_of_umid(&umid)));
        }
        components_out.push(ComponentRef { umid, locator });
    }

    let mut tracks = Vec::new();
    for r in material.references(tag::PACKAGE_TRACKS) {
        let Some(t) = md.get(&r) else { continue };
        let (seq, comps) = components(&md, t);
        let Some((false, kind)) = track_kind_of(seq.and_then(|s| s.ul(tag::DATA_DEFINITION))) else { continue };
        let mut clips = Vec::new();
        for c in comps {
            let duration = c.i(tag::DURATION).unwrap_or(0).max(0);
            match c.kind() {
                Some(st::FILLER) => clips.push(VersionClip { package: None, source_track: 0, start: 0, duration }),
                Some(st::SOURCE_CLIP) => {
                    let package = umid_of(c, tag::SOURCE_PACKAGE_ID).filter(|u| *u != [0; 32]);
                    if let Some(u) = package
                        && !components_out.iter().any(|x| x.umid == u)
                    {
                        problems.push(format!("a source clip names the package {}, which is not in the version file", as02::file_id_of_umid(&u)));
                    }
                    clips.push(VersionClip { package, source_track: c.u(tag::SOURCE_TRACK_ID).unwrap_or(0) as u32, start: c.i(tag::START_POSITION).unwrap_or(0), duration });
                }
                _ => {}
            }
        }
        tracks.push(VersionTrack {
            track_id: t.u(tag::TRACK_ID).unwrap_or(0) as u32,
            track_number: t.u(tag::TRACK_NUMBER).unwrap_or(0) as u32,
            kind,
            name: t.string(tag::TRACK_NAME),
            edit_rate: t.rational(tag::EDIT_RATE).unwrap_or_default(),
            clips,
        });
    }
    // track numbers: unique and non-zero (4.3.5, item 11)
    let mut numbers: Vec<u32> = tracks.iter().map(|t| t.track_number).collect();
    if numbers.contains(&0) {
        problems.push("a version file's tracks have non-zero track numbers".into());
    }
    numbers.sort_unstable();
    if numbers.windows(2).any(|p| p[0] == p[1]) {
        problems.push("a version file's track numbers are unique".into());
    }
    Ok(VersionFile { material_umid, name: material.string(tag::PACKAGE_NAME), operational_pattern, tracks, components: components_out, problems })
}
