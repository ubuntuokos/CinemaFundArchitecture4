//! AMWA AS-02 bundle import (#1363): a bundle folder (or its `manifest.xml`) becomes one sequence
//! per version file.
//!
//! A version file (OP1b / OP2 / OP3) holds no essence; its material package says which parts of
//! which essence component file play in which order. The manifest says where each file is
//! (`media/…`); a component the manifest does not list is looked up by the network locator of its
//! file package, relative to the version file. Every path is resolved inside the bundle, never
//! outside it. The component files import once, in a bin named after the bundle.
//!
//! What cannot be placed is reported: a missing component, an unsafe path, a breach of the AS-02
//! rules in the version file (the version is still imported when it can be read).

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use cfa4_mxf::TrackKind;
use cfa4_mxf::as02::{self, Manifest};
use cfa4_mxf::as02_version::{VersionFile, read_version};
use cfa4_project::{BinId, ItemId};
use cfa4_time::{Tick, TimeRange};

use crate::dcp_import::{import_asset, resolve, units};
use crate::{EngineError, Result, Session};

const MANIFEST_NAME: &str = "manifest.xml";
/// Largest manifest or version file read (both are metadata only).
const MAX_XML_BYTES: u64 = 16 << 20;
const MAX_VERSION_BYTES: u64 = 64 << 20;
const MAX_VERSIONS: usize = 64;
const MAX_CLIPS_PER_TRACK: usize = 10_000;
const MAX_TRACKS: usize = 64;

/// The bundle root when `path` is a bundle folder or its manifest; `None` otherwise.
fn root_of(s: &Session, path: &str) -> Option<String> {
    let p = Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (root, manifest) = if name.eq_ignore_ascii_case(MANIFEST_NAME) && s.services.file_size(path).is_ok() {
        (p.parent()?.to_path_buf(), path.to_string())
    } else if s.services.file_size(path).is_err() {
        let m = p.join(MANIFEST_NAME).to_string_lossy().into_owned();
        s.services.file_size(&m).ok()?;
        (p.to_path_buf(), m)
    } else {
        return None;
    };
    let head = s.services.read_range(&manifest, 0, 2048).ok()?;
    String::from_utf8_lossy(&head).contains(as02::MANIFEST_NS).then(|| root.to_string_lossy().into_owned())
}

/// `None`: `path` is not an AS-02 bundle. `Some(Err)`: a bundle that cannot be read at all.
pub fn import_package(s: &mut Session, path: &str, bin: Option<BinId>) -> Option<Result<Value>> {
    let root = root_of(s, path)?;
    Some(import_root(s, &root, bin))
}

fn read_limited(s: &Session, path: &str, max: u64) -> std::result::Result<Vec<u8>, String> {
    let size = s.services.file_size(path).map_err(|e| format!("{path}: {e}"))?;
    if size > max {
        return Err(format!("{path}: larger than {max} bytes"));
    }
    s.services.read_file(path).map_err(|e| format!("{path}: {e}")).map(|b| b.to_vec())
}

fn import_root(s: &mut Session, root: &str, bin: Option<BinId>) -> Result<Value> {
    let mpath = Path::new(root).join(MANIFEST_NAME).to_string_lossy().into_owned();
    let bytes = read_limited(s, &mpath, MAX_XML_BYTES).map_err(EngineError::Other)?;
    let xml = String::from_utf8(bytes).map_err(|_| EngineError::Other(format!("{mpath}: not UTF-8")))?;
    let manifest = Manifest::parse(&xml).map_err(EngineError::Other)?;
    let mut warnings: Vec<String> = Vec::new();
    let mut versions: Vec<(String, String)> = Vec::new(); // (manifest path, file)
    for f in manifest.versions().take(MAX_VERSIONS) {
        match resolve(root, &f.path) {
            Ok(file) => versions.push((f.path.clone(), file)),
            Err(e) => warnings.push(format!("{}: {e}", f.path)),
        }
    }
    if manifest.versions().count() > MAX_VERSIONS {
        warnings.push(format!("more than {MAX_VERSIONS} version files; the rest are not imported"));
    }
    if versions.is_empty() {
        return Err(EngineError::Other(format!("{root}: the bundle has no version file")));
    }
    // component file id -> file
    let mut by_id: HashMap<String, String> = HashMap::new();
    for f in manifest.essence_components() {
        match resolve(root, &f.path) {
            Ok(file) => {
                by_id.insert(f.id.clone(), file);
            }
            Err(e) => warnings.push(format!("{}: {e}", f.path)),
        }
    }
    let bundle_name = if manifest.bundle_name.is_empty() {
        Path::new(root).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "AS-02".into())
    } else {
        manifest.bundle_name.clone()
    };
    s.edit_group(&format!("Import AS-02 {bundle_name}"), |s| {
        let bundle_bin = s.edit(&format!("New Bin {bundle_name}"), |p, _| Ok(p.add_bin(&bundle_name, bin)))?;
        let mut items: HashMap<String, ItemId> = HashMap::new();
        let mut sequences = Vec::new();
        for (mpath, file) in &versions {
            let data = match read_limited(s, file, MAX_VERSION_BYTES) {
                Ok(d) => d,
                Err(e) => {
                    warnings.push(e);
                    continue;
                }
            };
            let version = match read_version(&data) {
                Ok(v) => v,
                Err(e) => {
                    warnings.push(format!("{mpath}: {e}"));
                    continue;
                }
            };
            for p in &version.problems {
                warnings.push(format!("{mpath}: {p}"));
            }
            let dir = Path::new(file).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            match import_version(s, &version, mpath, &dir, root, &by_id, &mut items, bundle_bin, &mut warnings) {
                Ok(v) => sequences.push(v),
                Err(e) => warnings.push(format!("{mpath}: {e}")),
            }
        }
        if sequences.is_empty() {
            return Err(EngineError::Other(format!("{root}: nothing could be imported: {}", warnings.join("; "))));
        }
        Ok(json!({
            "format": "AS-02",
            "bundle": bundle_name,
            "sequences": sequences.iter().map(|c| c["sequence"].clone()).collect::<Vec<_>>(),
            "versions": sequences,
            "items": items.values().map(|i| i.0).collect::<Vec<_>>(),
            "warnings": warnings,
        }))
    })
}

/// The file of a component: the manifest's, else the file package's locator inside the bundle.
fn component_file(version: &VersionFile, umid: &[u8; 32], dir: &str, root: &str, by_id: &HashMap<String, String>) -> std::result::Result<String, String> {
    let id = as02::file_id_of_umid(umid);
    if let Some(f) = by_id.get(&id) {
        return Ok(f.clone());
    }
    let loc = version.component(umid).and_then(|c| c.locator.as_deref()).ok_or_else(|| format!("component {id} is not in the manifest and has no locator"))?;
    let rel = loc.strip_prefix("file:///").or_else(|| loc.strip_prefix("file://")).unwrap_or(loc);
    let file = resolve(dir, rel).map_err(|e| format!("component {id}: {e}"))?;
    if !Path::new(&file).starts_with(root) {
        return Err(format!("component {id}: \"{loc}\" is outside the bundle"));
    }
    Ok(file)
}

#[allow(clippy::too_many_arguments)]
fn import_version(
    s: &mut Session,
    version: &VersionFile,
    mpath: &str,
    dir: &str,
    root: &str,
    by_id: &HashMap<String, String>,
    items: &mut HashMap<String, ItemId>,
    bin: BinId,
    warnings: &mut Vec<String>,
) -> Result<Value> {
    let name = version.name.clone().or_else(|| Path::new(mpath).file_stem().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "Version".into());
    // each component imports once
    let mut comp_items: HashMap<[u8; 32], ItemId> = HashMap::new();
    for t in version.tracks.iter().take(MAX_TRACKS) {
        for c in &t.clips {
            let Some(pkg) = c.package else { continue };
            if comp_items.contains_key(&pkg) {
                continue;
            }
            match component_file(version, &pkg, dir, root, by_id) {
                Ok(file) => match import_asset(s, &file, Some(bin), items) {
                    Ok(i) => {
                        comp_items.insert(pkg, i);
                    }
                    Err(e) => warnings.push(format!("{mpath}: {file}: {e}")),
                },
                Err(e) => warnings.push(format!("{mpath}: {e}")),
            }
        }
    }
    if comp_items.is_empty() {
        return Err(EngineError::Other("none of its essence components could be imported".into()));
    }
    let pictures: Vec<_> = version.tracks.iter().filter(|t| t.kind == TrackKind::Picture).take(MAX_TRACKS).collect();
    let sounds: Vec<_> = version.tracks.iter().filter(|t| t.kind == TrackKind::Sound).take(MAX_TRACKS).collect();
    let first_picture = pictures.iter().flat_map(|t| t.clips.iter()).find_map(|c| c.package.and_then(|p| comp_items.get(&p).copied()));
    let settings =
        first_picture.and_then(|i| s.project.item(i).and_then(|it| it.as_media().map(crate::commands::default_seq_settings_for))).unwrap_or_default();
    let rate = settings.frame_rate;
    let seq = s.edit(&format!("New Sequence {name}"), |p, _| Ok(p.new_sequence(&name, settings, pictures.len().max(1), sounds.len(), Some(bin))))?;
    s.state.active_sequence = Some(seq);
    s.state.open_sequences.push(seq);
    let (vts, ats): (Vec<_>, Vec<_>) = s
        .project
        .sequence(seq)
        .map(|q| (q.video_tracks.iter().map(|t| t.id).collect::<Vec<_>>(), q.audio_tracks.iter().map(|t| t.id).collect::<Vec<_>>()))
        .unwrap_or_default();
    let mut placed = 0usize;
    for (kind_tracks, dests) in [(&pictures, &vts), (&sounds, &ats)] {
        for (k, t) in kind_tracks.iter().enumerate() {
            let dest = dests.get(k).copied();
            let mut at = Tick::ZERO;
            for c in t.clips.iter().take(MAX_CLIPS_PER_TRACK) {
                let er = cfa4_dcp::Rational { num: i64::from(t.edit_rate.num), den: i64::from(t.edit_rate.den) };
                let (Some(start), Some(dur)) = (units(u64::try_from(c.start).unwrap_or(0), er), units(u64::try_from(c.duration).unwrap_or(0), er)) else {
                    warnings.push(format!("{name}: track {} has no usable edit rate", t.track_id));
                    break;
                };
                let next = Tick(at.0.saturating_add(dur.0));
                if let Some(item) = c.package.and_then(|pkg| comp_items.get(&pkg)) {
                    {
                        let range = TimeRange::new(start, rate.snap_nearest(dur).max(rate.frame_duration()));
                        let (v, a) = if t.kind == TrackKind::Picture { (dest, None) } else { (None, dest) };
                        match crate::commands::place_item(s, *item, range, rate.snap_nearest(at), v, a, false, "Place AS-02 clip", None) {
                            Ok(_) => placed += 1,
                            Err(e) => warnings.push(format!("{name}: track {}: {e}", t.track_id)),
                        }
                    }
                }
                at = next;
            }
        }
    }
    Ok(json!({"sequence": seq.0, "name": name, "id": version.file_id(), "pattern": version.operational_pattern, "placed": placed}))
}
