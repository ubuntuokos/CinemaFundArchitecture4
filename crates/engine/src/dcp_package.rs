//! `dcp.package.info` (DCP-22): a read-only model of a DCP folder for the Qt6 package views (plan
//! F19): the asset map with the state of every file, the packing lists, the compositions with
//! their reels, assets and markers, and what is signed or encrypted. Only the XML documents are
//! read (each under the `cfa4-dcp` size and node limits); no track file is opened, nothing is
//! written and no program is run. Only `KeyId` references are shown, never key material (plan F15).

use std::path::Path;

use serde_json::{Value, json};

use cfa4_dcp::{AssetKind, Document, Limits};
use cfa4_dcp_validate::package::{self, Resolved};

use crate::commands::{CommandSpec, always, bad, str_p};
use crate::{Result, Session};

/// Schema id of the answer.
pub const SCHEMA: &str = "cfa4-dcp-package/1";

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn read_doc(path: &Path) -> std::result::Result<Document, String> {
    cfa4_dcp::read(path, Limits::default()).map_err(|e| e.to_string())
}

/// State of a chunk path inside the package, and the size of the file when it is one.
fn file_state(root: &Path, path: &str) -> (&'static str, Option<u64>, Option<std::path::PathBuf>) {
    match package::resolve(root, path) {
        Resolved::File(p) => {
            let size = std::fs::metadata(&p).ok().map(|m| m.len());
            ("file", size, Some(p))
        }
        Resolved::Missing => ("missing", None, None),
        Resolved::NotAFile => ("notAFile", None, None),
        Resolved::Unsafe(_) => ("unsafe", None, None),
    }
}

fn info(_: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dcp.package.info";
    let root_arg = str_p(p, "root").ok_or_else(|| bad(cmd, "need `root` (the DCP folder)"))?;
    let root = Path::new(root_arg).canonicalize().map_err(|e| bad(cmd, format!("{root_arg}: {e}")))?;
    if !root.is_dir() {
        return Err(bad(cmd, format!("{root_arg}: not a folder")));
    }
    let maps = package::asset_maps(&root);
    let Some(map_name) = maps.first().copied() else {
        return Err(bad(cmd, format!("{root_arg}: no ASSETMAP.xml or ASSETMAP (not a DCP folder)")));
    };
    let mut problems: Vec<Value> = Vec::new();
    let map = match read_doc(&root.join(map_name)) {
        Ok(Document::AssetMap(m)) => m,
        Ok(_) => return Err(bad(cmd, format!("{map_name} is not an asset map"))),
        Err(e) => return Err(bad(cmd, format!("{map_name}: {e}"))),
    };

    let mut files = Vec::new();
    let mut packing_lists = Vec::new();
    let mut compositions = Vec::new();
    let mut signed = Vec::new();
    let mut encrypted = Vec::new();
    // asset id → path of the file, to name the files of the reel assets
    let mut paths: std::collections::HashMap<cfa4_dcp::Uuid, String> = Default::default();

    for a in &map.assets {
        let chunk = a.chunks.first().and_then(|c| c.path.clone());
        if let (Some(id), Some(path)) = (a.id, &chunk) {
            paths.insert(id, path.clone());
        }
        let (state, size, real) = chunk.as_deref().map_or(("noPath", None, None), |c| file_state(&root, c));
        files.push(json!({
            "id": a.id, "path": chunk, "state": state, "size": size, "packingList": a.packing_list,
            "annotation": a.annotation, "chunks": a.chunks.len(),
        }));
        if a.packing_list
            && let (Some(path), Some(real)) = (chunk.as_deref(), real)
        {
            match read_doc(&real) {
                Ok(Document::PackingList(pkl)) => {
                    if pkl.has_signer || pkl.has_signature {
                        signed.push(json!({"document": path, "kind": "packingList", "signer": pkl.has_signer, "signature": pkl.has_signature}));
                    }
                    let assets: Vec<Value> = pkl
                        .assets
                        .iter()
                        .map(|x| {
                            json!({
                                "id": x.id, "mime": x.mime_type, "size": x.size, "hash": x.hash,
                                "originalFileName": x.original_file_name, "annotation": x.annotation,
                                "inAssetMap": x.id.is_some_and(|i| map.assets.iter().any(|m| m.id == Some(i))),
                            })
                        })
                        .collect();
                    packing_lists.push(json!({"file": path, "document": to_value(&pkl), "assets": assets}));
                }
                Ok(_) => problems.push(json!({"file": path, "message": "listed as a packing list but is another kind of document"})),
                Err(e) => problems.push(json!({"file": path, "message": e})),
            }
        }
    }
    // Compositions: the XML assets the packing lists name (and the asset map holds) that read as a CPL.
    for pl in &packing_lists {
        let Some(list) = pl["assets"].as_array() else { continue };
        for x in list {
            if !x["mime"].as_str().is_some_and(|m| m.starts_with("text/xml")) {
                continue;
            }
            let Some(id) = x["id"].as_str().and_then(cfa4_dcp::Uuid::parse) else { continue };
            let Some(path) = paths.get(&id) else { continue };
            let Resolved::File(real) = package::resolve(&root, path) else { continue };
            if let Ok(Document::Composition(cpl)) = read_doc(&real) {
                if cpl.has_signer || cpl.has_signature {
                    signed.push(json!({"document": path, "kind": "composition", "signer": cpl.has_signer, "signature": cpl.has_signature}));
                }
                for (ri, reel) in cpl.reels.iter().enumerate() {
                    for ra in &reel.assets {
                        if let Some(key) = ra.key_id {
                            encrypted.push(json!({"composition": path, "reel": ri + 1, "kind": to_value(&ra.kind), "asset": ra.id, "keyId": key}));
                        }
                    }
                }
                let file_of = |id: Option<cfa4_dcp::Uuid>| id.and_then(|i| paths.get(&i).cloned());
                let reels: Vec<Value> = cpl
                    .reels
                    .iter()
                    .map(|r| {
                        let assets: Vec<Value> = r
                            .assets
                            .iter()
                            .map(|a| {
                                let mut v = to_value(a);
                                v["file"] = json!(file_of(a.id));
                                v["encrypted"] = json!(a.key_id.is_some());
                                v["track"] = json!(!matches!(a.kind, AssetKind::MainMarkers | AssetKind::CompositionMetadata | AssetKind::Other));
                                v
                            })
                            .collect();
                        json!({"id": r.id, "annotation": r.annotation, "assets": assets})
                    })
                    .collect();
                let mut doc = to_value(&cpl);
                doc["reels"] = Value::Array(reels);
                compositions.push(json!({"file": path, "document": doc}));
            }
        }
    }
    let unlisted: Vec<&str> = maps.iter().skip(1).copied().collect();
    Ok(json!({
        "schema": SCHEMA,
        "root": root.to_string_lossy(),
        "standard": map.standard,
        "assetMap": {"file": map_name, "document": to_value(&map), "otherAssetMaps": unlisted},
        "files": files,
        "packingLists": packing_lists,
        "compositions": compositions,
        "security": {"signedDocuments": signed, "encryptedAssets": encrypted},
        "problems": problems,
    }))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "dcp.package.info",
        label: "Read DCP Package Structure",
        menu: &[],
        shortcut: None,
        params: r#"{"root":str}"#,
        enabled: always,
        run: info,
        journal: false,
    }]
}

#[cfg(test)]
mod tests;
