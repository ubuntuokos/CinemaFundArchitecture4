//! Asset map and volume index (SMPTE ST 429-9, Interop `PROTO-ASDCP-AM-20040311`).

use roxmltree::Document;
use serde::{Deserialize, Serialize};

use crate::xml::Reader;
use crate::{Issue, IssueKind, Pos, Standard, Uuid};

/// `ASSETMAP` / `ASSETMAP.xml`: where every asset of the volume lies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetMap {
    pub standard: Standard,
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    pub creator: Option<String>,
    pub volume_count: Option<u64>,
    pub issue_date: Option<String>,
    pub issuer: Option<String>,
    pub assets: Vec<AssetMapAsset>,
    /// Elements of the root the reader does not know, as `{namespace}name`.
    pub extensions: Vec<String>,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetMapAsset {
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    /// The asset is a packing list (`<PackingList>true</PackingList>`, or an empty
    /// `<PackingList/>` in Interop).
    pub packing_list: bool,
    pub chunks: Vec<Chunk>,
    pub pos: Pos,
}

/// One piece of an asset on a volume. DCPs almost always have exactly one chunk per asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// Path relative to the volume root, as written (never resolved here).
    pub path: Option<String>,
    pub volume_index: Option<u64>,
    pub offset: Option<u64>,
    pub length: Option<u64>,
    pub pos: Pos,
}

/// `VOLINDEX` / `VOLINDEX.xml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VolumeIndex {
    pub standard: Standard,
    pub index: Option<u64>,
    pub issues: Vec<Issue>,
}

const ROOT: &[&str] = &["Id", "AnnotationText", "Creator", "VolumeCount", "IssueDate", "Issuer", "AssetList"];
const ASSET: &[&str] = &["Id", "AnnotationText", "PackingList", "ChunkList"];
const CHUNK: &[&str] = &["Path", "VolumeIndex", "Offset", "Length"];

pub(crate) fn read(doc: &Document, standard: Standard) -> AssetMap {
    let mut r = Reader::new(doc);
    let root = doc.root_element();
    let p = "AssetMap";
    let id = r.uuid(root, p, "Id", true);
    let annotation = r.text(root, p, "AnnotationText");
    let creator = r.required_text(root, p, "Creator");
    let volume_count = r.positive(root, p, "VolumeCount", true);
    let issue_date = r.date_time(root, p, "IssueDate", true);
    let issuer = r.required_text(root, p, "Issuer");
    let extensions = r.unknown(root, p, ROOT);
    let mut assets = Vec::new();
    match r.child(root, p, "AssetList") {
        None => r.issue(IssueKind::Missing, "AssetMap/AssetList", root, "AssetList is missing"),
        Some(list) => {
            r.unknown(list, "AssetMap/AssetList", &["Asset"]);
            for (i, a) in Reader::children(list, "Asset").enumerate() {
                let ap = format!("AssetMap/AssetList/Asset[{}]", i + 1);
                let id = r.uuid(a, &ap, "Id", true);
                let annotation = r.text(a, &ap, "AnnotationText");
                let packing_list = match r.child(a, &ap, "PackingList") {
                    None => false,
                    Some(n) => match n.text().unwrap_or("").trim() {
                        // Interop writes an empty element; SMPTE is an xs:boolean, where empty is invalid
                        "" if standard == Standard::Interop => true,
                        "true" | "1" => true,
                        "false" | "0" => false,
                        other => {
                            r.issue(IssueKind::Invalid, &format!("{ap}/PackingList"), n, format!("PackingList '{other}' is not a boolean"));
                            false
                        }
                    },
                };
                r.unknown(a, &ap, ASSET);
                let mut chunks = Vec::new();
                match r.child(a, &ap, "ChunkList") {
                    None => r.issue(IssueKind::Missing, &format!("{ap}/ChunkList"), a, "ChunkList is missing"),
                    Some(cl) => {
                        r.unknown(cl, &format!("{ap}/ChunkList"), &["Chunk"]);
                        for (k, c) in Reader::children(cl, "Chunk").enumerate() {
                            let cp = format!("{ap}/ChunkList/Chunk[{}]", k + 1);
                            let path = r.required_text(c, &cp, "Path");
                            let volume_index = r.positive(c, &cp, "VolumeIndex", false);
                            let offset = r.u64(c, &cp, "Offset", false);
                            let length = r.u64(c, &cp, "Length", false);
                            r.unknown(c, &cp, CHUNK);
                            let pos = r.pos(c);
                            chunks.push(Chunk { path, volume_index, offset, length, pos });
                        }
                        if chunks.is_empty() {
                            r.issue(IssueKind::Missing, &format!("{ap}/ChunkList/Chunk"), cl, "ChunkList has no Chunk");
                        }
                    }
                }
                let pos = r.pos(a);
                assets.push(AssetMapAsset { id, annotation, packing_list, chunks, pos });
            }
        }
    }
    AssetMap { standard, id, annotation, creator, volume_count, issue_date, issuer, assets, extensions, issues: r.issues }
}

pub(crate) fn read_volume_index(doc: &Document, standard: Standard) -> VolumeIndex {
    let mut r = Reader::new(doc);
    let root = doc.root_element();
    let index = r.positive(root, "VolumeIndex", "Index", true);
    r.unknown(root, "VolumeIndex", &["Index"]);
    VolumeIndex { standard, index, issues: r.issues }
}
