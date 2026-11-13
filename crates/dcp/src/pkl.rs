//! Packing list (SMPTE ST 429-8, Interop `PROTO-ASDCP-PKL-20040311`).

use roxmltree::Document;
use serde::{Deserialize, Serialize};

use crate::xml::Reader;
use crate::{Issue, IssueKind, Pos, Standard, Uuid};

/// The packing list: every asset of a delivery with its size, type and SHA-1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackingList {
    pub standard: Standard,
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    pub issue_date: Option<String>,
    pub issuer: Option<String>,
    pub creator: Option<String>,
    pub group_id: Option<Uuid>,
    pub assets: Vec<PackingListAsset>,
    /// A `Signer` element is present.
    pub has_signer: bool,
    /// A `ds:Signature` element is present (not verified here).
    pub has_signature: bool,
    pub extensions: Vec<String>,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackingListAsset {
    pub id: Option<Uuid>,
    pub annotation: Option<String>,
    /// The `Hash` text as written (base64 of the SHA-1 digest).
    pub hash: Option<String>,
    /// The decoded digest, when `hash` is valid base64 of 20 bytes.
    pub sha1: Option<[u8; 20]>,
    pub size: Option<u64>,
    /// `Type`: a MIME type (`application/mxf`, `text/xml`; Interop adds `;asdcpKind=…`).
    pub mime_type: Option<String>,
    pub original_file_name: Option<String>,
    pub pos: Pos,
}

const ROOT: &[&str] = &["Id", "AnnotationText", "IconId", "IssueDate", "Issuer", "Creator", "GroupId", "AssetList", "Signer", "Signature"];
const ASSET: &[&str] = &["Id", "AnnotationText", "Hash", "Size", "Type", "OriginalFileName"];

pub(crate) fn read(doc: &Document<'_>, standard: Standard) -> PackingList {
    let mut r = Reader::new(doc);
    let root = doc.root_element();
    let p = "PackingList";
    let id = r.uuid(root, p, "Id", true);
    let annotation = r.text(root, p, "AnnotationText");
    let issue_date = r.date_time(root, p, "IssueDate", true);
    let issuer = r.required_text(root, p, "Issuer");
    let creator = r.required_text(root, p, "Creator");
    let group_id = r.uuid(root, p, "GroupId", false);
    let has_signer = r.child(root, p, "Signer").is_some();
    let has_signature = crate::xml::has_signature(root);
    let extensions = r.unknown(root, p, ROOT);
    let mut assets = Vec::new();
    match r.child(root, p, "AssetList") {
        None => r.issue(IssueKind::Missing, "PackingList/AssetList", root, "AssetList is missing"),
        Some(list) => {
            r.unknown(list, "PackingList/AssetList", &["Asset"]);
            for (i, a) in Reader::children(list, "Asset").enumerate() {
                let ap = format!("PackingList/AssetList/Asset[{}]", i + 1);
                let id = r.uuid(a, &ap, "Id", true);
                let annotation = r.text(a, &ap, "AnnotationText");
                let (hash, sha1) = r.hash(a, &ap, "Hash", true);
                let size = r.positive(a, &ap, "Size", true);
                let mime_type = r.required_text(a, &ap, "Type");
                let original_file_name = r.text(a, &ap, "OriginalFileName");
                r.unknown(a, &ap, ASSET);
                let pos = r.pos(a);
                assets.push(PackingListAsset { id, annotation, hash, sha1, size, mime_type, original_file_name, pos });
            }
            if assets.is_empty() {
                r.issue(IssueKind::Missing, "PackingList/AssetList/Asset", list, "AssetList has no Asset");
            }
        }
    }
    PackingList { standard, id, annotation, issue_date, issuer, creator, group_id, assets, has_signer, has_signature, extensions, issues: r.issues }
}
