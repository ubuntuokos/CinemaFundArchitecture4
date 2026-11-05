//! Composition playlist (SMPTE ST 429-7, Interop `PROTO-ASDCP-CPL-20040511`), with the reel assets of
//! ST 429-10 / 437-Y (stereoscopic picture), ST 429-16 (composition metadata) and any extension asset.

use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};

use crate::xml::{Reader, qualified};
use crate::{IssueKind, Pos, Rational, Standard, Uuid};

/// The composition playlist: the reels and, in each, the track files played together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub standard: Standard,
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    pub issue_date: Option<String>,
    pub issuer: Option<String>,
    pub creator: Option<String>,
    pub content_title: Option<String>,
    /// `ContentKind` as written (`feature`, `trailer`, …; a `scope` attribute is not interpreted).
    pub content_kind: Option<String>,
    pub content_version: Option<ContentVersion>,
    pub reels: Vec<Reel>,
    /// A `Signer` element is present.
    pub has_signer: bool,
    /// A `ds:Signature` element is present (not verified here).
    pub has_signature: bool,
    /// Elements of the root the reader does not know, as `{namespace}name`.
    pub extensions: Vec<String>,
    pub issues: Vec<crate::Issue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentVersion {
    /// `Id`: an URI, often a `urn:uuid:` (kept as text).
    pub id: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reel {
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    /// Every element child of the reel's `AssetList`, in document order.
    pub assets: Vec<ReelAsset>,
    pub pos: Pos,
}

/// What a reel asset is, from its element's local name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    MainPicture,
    /// ST 429-10 / Interop 437-Y.
    MainStereoscopicPicture,
    MainSound,
    MainSubtitle,
    MainCaption,
    MainClosedCaption,
    MainMarkers,
    /// An auxiliary data track (immersive audio, D-BOX, …), any namespace.
    AuxData,
    /// ST 429-16 `CompositionMetadataAsset`.
    CompositionMetadata,
    /// Any other element: an extension asset this reader does not model.
    Other,
}

impl AssetKind {
    fn from_name(name: &str) -> AssetKind {
        match name {
            "MainPicture" => AssetKind::MainPicture,
            "MainStereoscopicPicture" => AssetKind::MainStereoscopicPicture,
            "MainSound" => AssetKind::MainSound,
            "MainSubtitle" => AssetKind::MainSubtitle,
            "MainCaption" => AssetKind::MainCaption,
            "MainClosedCaption" | "ClosedCaption" => AssetKind::MainClosedCaption,
            "MainMarkers" => AssetKind::MainMarkers,
            "AuxData" => AssetKind::AuxData,
            "CompositionMetadataAsset" => AssetKind::CompositionMetadata,
            _ => AssetKind::Other,
        }
    }

    /// Track-file assets whose child elements are fully known to this reader.
    fn is_core(self) -> bool {
        !matches!(self, AssetKind::AuxData | AssetKind::CompositionMetadata | AssetKind::Other)
    }
}

/// A screen aspect ratio: SMPTE writes two integers (`1998 1080`), Interop a decimal (`1.85`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScreenAspectRatio {
    Ratio(Rational),
    Decimal(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    /// `Label` text (`FFOC`, `LFOC`, …).
    pub label: Option<String>,
    /// `Offset` in edit units from the reel's entry point.
    pub offset: Option<u64>,
    pub pos: Pos,
}

/// One asset of a reel. Fields that an asset kind does not have stay `None`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReelAsset {
    pub kind: AssetKind,
    /// The element as `{namespace}name`.
    pub element: String,
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    pub edit_rate: Option<Rational>,
    pub intrinsic_duration: Option<u64>,
    pub entry_point: Option<u64>,
    pub duration: Option<u64>,
    /// The `Hash` text as written.
    pub hash: Option<String>,
    pub sha1: Option<[u8; 20]>,
    pub key_id: Option<Uuid>,
    pub frame_rate: Option<Rational>,
    pub screen_aspect_ratio: Option<ScreenAspectRatio>,
    pub language: Option<String>,
    /// `DataType` of an `AuxData` asset (a `urn:smpte:ul:` label).
    pub data_type: Option<String>,
    pub markers: Vec<Marker>,
    /// Child elements not read (for `AuxData`, `CompositionMetadataAsset` and other extension
    /// assets these are listed without an issue).
    pub extensions: Vec<String>,
    pub pos: Pos,
}

const ROOT: &[&str] = &[
    "Id",
    "AnnotationText",
    "IconId",
    "IssueDate",
    "Issuer",
    "Creator",
    "ContentTitleText",
    "ContentKind",
    "ContentVersion",
    "RatingList",
    "ReelList",
    "Signer",
    "Signature",
];
const REEL: &[&str] = &["Id", "AnnotationText", "AssetList"];
const ASSET: &[&str] = &[
    "Id",
    "AnnotationText",
    "EditRate",
    "IntrinsicDuration",
    "EntryPoint",
    "Duration",
    "Hash",
    "KeyId",
    "FrameRate",
    "ScreenAspectRatio",
    "Language",
    "DataType",
    "MarkerList",
];

pub(crate) fn read(doc: &Document, standard: Standard) -> Composition {
    let mut r = Reader::new(doc);
    let root = doc.root_element();
    let p = "CompositionPlaylist";
    let id = r.uuid(root, p, "Id", true);
    let annotation = r.text(root, p, "AnnotationText");
    let issue_date = r.required_text(root, p, "IssueDate");
    let issuer = r.text(root, p, "Issuer");
    let creator = r.text(root, p, "Creator");
    let content_title = r.required_text(root, p, "ContentTitleText");
    let content_kind = r.required_text(root, p, "ContentKind");
    let content_version = r.child(root, p, "ContentVersion").map(|cv| {
        let cp = format!("{p}/ContentVersion");
        let id = r.text(cv, &cp, "Id");
        let label = r.text(cv, &cp, "LabelText");
        r.unknown(cv, &cp, &["Id", "LabelText"]);
        ContentVersion { id, label }
    });
    let has_signer = r.child(root, p, "Signer").is_some();
    let has_signature = Reader::children(root, "Signature").any(|n| n.tag_name().namespace() == Some(crate::ns::XMLDSIG));
    let extensions = r.unknown(root, p, ROOT);
    let mut reels = Vec::new();
    match r.child(root, p, "ReelList") {
        None => r.issue(IssueKind::Missing, "CompositionPlaylist/ReelList", root, "ReelList is missing"),
        Some(list) => {
            r.unknown(list, "CompositionPlaylist/ReelList", &["Reel"]);
            for (i, reel) in Reader::children(list, "Reel").enumerate() {
                let rp = format!("CompositionPlaylist/ReelList/Reel[{}]", i + 1);
                reels.push(read_reel(&mut r, reel, &rp));
            }
            if reels.is_empty() {
                r.issue(IssueKind::Missing, "CompositionPlaylist/ReelList/Reel", list, "ReelList has no Reel");
            }
        }
    }
    Composition {
        standard,
        id,
        annotation,
        issue_date,
        issuer,
        creator,
        content_title,
        content_kind,
        content_version,
        reels,
        has_signer,
        has_signature,
        extensions,
        issues: r.issues,
    }
}

fn read_reel<'a, 'i>(r: &mut Reader<'a, 'i>, reel: Node<'a, 'i>, rp: &str) -> Reel {
    let id = r.uuid(reel, rp, "Id", true);
    let annotation = r.text(reel, rp, "AnnotationText");
    r.unknown(reel, rp, REEL);
    let mut assets = Vec::new();
    match r.child(reel, rp, "AssetList") {
        None => r.issue(IssueKind::Missing, &format!("{rp}/AssetList"), reel, "AssetList is missing"),
        Some(list) => {
            let mut seen: Vec<(String, usize)> = Vec::new();
            for a in list.children().filter(|c| c.is_element()) {
                let name = a.tag_name().name().to_string();
                let n = match seen.iter_mut().find(|(s, _)| *s == name) {
                    Some((_, n)) => {
                        *n += 1;
                        *n
                    }
                    None => {
                        seen.push((name.clone(), 1));
                        1
                    }
                };
                let ap = format!("{rp}/AssetList/{name}[{n}]");
                assets.push(read_asset(r, a, &ap));
            }
            if assets.is_empty() {
                r.issue(IssueKind::Missing, &format!("{rp}/AssetList"), list, "AssetList has no asset");
            }
        }
    }
    let pos = r.pos(reel);
    Reel { id, annotation, assets, pos }
}

fn read_asset<'a, 'i>(r: &mut Reader<'a, 'i>, a: Node<'a, 'i>, ap: &str) -> ReelAsset {
    let kind = AssetKind::from_name(a.tag_name().name());
    let element = qualified(a);
    let id = r.uuid(a, ap, "Id", true);
    let annotation = r.text(a, ap, "AnnotationText");
    let edit_rate = r.rational(a, ap, "EditRate", kind.is_core());
    let intrinsic_duration = r.u64(a, ap, "IntrinsicDuration", kind.is_core());
    let entry_point = r.u64(a, ap, "EntryPoint", false);
    let duration = r.u64(a, ap, "Duration", false);
    let (hash, sha1) = r.hash(a, ap, "Hash", false);
    let key_id = r.uuid(a, ap, "KeyId", false);
    let frame_rate = r.rational(a, ap, "FrameRate", false);
    let screen_aspect_ratio = r.value(a, ap, "ScreenAspectRatio", false).and_then(|(t, n)| {
        if let Some(q) = Rational::parse(&t) {
            return Some(ScreenAspectRatio::Ratio(q));
        }
        if is_decimal(&t) {
            return Some(ScreenAspectRatio::Decimal(t));
        }
        r.issue(IssueKind::Invalid, &format!("{ap}/ScreenAspectRatio"), n, format!("ScreenAspectRatio '{t}' is neither two integers nor a decimal"));
        None
    });
    let language = r.text(a, ap, "Language");
    let data_type = r.text(a, ap, "DataType");
    let mut markers = Vec::new();
    if let Some(ml) = r.child(a, ap, "MarkerList") {
        let mp = format!("{ap}/MarkerList");
        r.unknown(ml, &mp, &["Marker"]);
        for (k, m) in Reader::children(ml, "Marker").enumerate() {
            let kp = format!("{mp}/Marker[{}]", k + 1);
            let label = r.required_text(m, &kp, "Label");
            let offset = r.u64(m, &kp, "Offset", true);
            r.unknown(m, &kp, &["Label", "Offset", "AnnotationText"]);
            let pos = r.pos(m);
            markers.push(Marker { label, offset, pos });
        }
    }
    let extensions = if kind.is_core() {
        r.unknown(a, ap, ASSET)
    } else {
        a.children().filter(|c| c.is_element() && !ASSET.contains(&c.tag_name().name())).map(qualified).collect()
    };
    let pos = r.pos(a);
    ReelAsset {
        kind,
        element,
        id,
        annotation,
        edit_rate,
        intrinsic_duration,
        entry_point,
        duration,
        hash,
        sha1,
        key_id,
        frame_rate,
        screen_aspect_ratio,
        language,
        data_type,
        markers,
        extensions,
        pos,
    }
}

/// `1.85`, `2.39`, `1`: digits with at most one decimal point.
fn is_decimal(t: &str) -> bool {
    let mut parts = t.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next();
    !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()) && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}
