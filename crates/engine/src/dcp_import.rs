//! DCP package import (#101): a DCP folder (or its `ASSETMAP`, or one of its composition
//! playlists) becomes one sequence per composition playlist (CPL, SMPTE ST 429-7 or Interop).
//!
//! The asset map (ST 429-9) says which file holds each asset. Each reel of a CPL follows the
//! previous one on the timeline; its picture (`MainPicture`, or `MainStereoscopicPicture` as the
//! file holds it) goes on V1 and its sound (`MainSound`) on the audio tracks, each trimmed to the
//! asset's `EntryPoint` and `Duration` at its `EditRate`. The track files import as media items
//! in a bin named after the CPL; the sequence takes the first picture's frame size and rate.
//!
//! What is not placed is reported, never silent: an asset missing from the package (a version
//! file that names the original version's assets), an encrypted track file whose content key is
//! not loaded (`dcp.loadKdm` first), an unsafe path in the asset map, a reel asset of a kind this
//! import does not place yet (subtitles, captions, auxiliary data).

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use cfa4_dcp::{AssetKind, Composition, Document, Limits, Rational, ReelAsset, Uuid};
use cfa4_project::{BinId, ItemId};
use cfa4_time::{Tick, TimeRange};

use crate::{EngineError, Result, Session};

/// Asset map file names: SMPTE first, then Interop.
const ASSET_MAP_NAMES: [&str; 2] = ["ASSETMAP.xml", "ASSETMAP"];
/// Compositions imported from one package at most.
const MAX_COMPOSITIONS: usize = 64;
/// Reels per composition at most (a feature has a few dozen).
const MAX_REELS: usize = 1_000;

/// The package root and, when `path` names one CPL, that CPL's path. `None`: `path` is not a DCP
/// folder, asset map or CPL with an asset map beside it.
fn package_of(s: &Session, path: &str) -> Option<(String, Option<String>)> {
    let p = Path::new(path);
    let has_map = |dir: &Path| ASSET_MAP_NAMES.iter().any(|n| s.services.file_size(&dir.join(n).to_string_lossy()).is_ok());
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
    if ASSET_MAP_NAMES.iter().any(|n| n.eq_ignore_ascii_case(&name)) && s.services.file_size(path).is_ok() {
        return Some((parent.to_string_lossy().into_owned(), None));
    }
    // a folder: the file-size probe fails for it, and the asset map is inside
    if s.services.file_size(path).is_err() {
        return has_map(p).then(|| (path.to_string(), None));
    }
    let is_xml = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"));
    if !is_xml || !has_map(&parent) {
        return None;
    }
    let size = s.services.file_size(path).ok()?;
    if size > Limits::default().max_bytes {
        return None;
    }
    let head = s.services.read_range(path, 0, 4096).ok()?;
    if !String::from_utf8_lossy(&head).contains("CompositionPlaylist") {
        return None;
    }
    Some((parent.to_string_lossy().into_owned(), Some(path.to_string())))
}

/// A chunk path joined to the package root, refusing absolute paths, `..` and backslashes (the
/// asset map is data from the package, so it must not lead outside it).
fn resolve(root: &str, chunk: &str) -> std::result::Result<String, String> {
    let chunk = chunk.trim();
    if chunk.is_empty() {
        return Err("the path is empty".into());
    }
    if chunk.contains('\\') || chunk.contains('\0') {
        return Err(format!("unsafe path \"{chunk}\""));
    }
    for c in Path::new(chunk).components() {
        match c {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => return Err(format!("unsafe path \"{chunk}\"")),
        }
    }
    Ok(Path::new(root).join(chunk).to_string_lossy().into_owned())
}

fn read_document(s: &Session, path: &str) -> std::result::Result<Document, String> {
    let limits = Limits::default();
    let size = s.services.file_size(path).map_err(|e| format!("{path}: {e}"))?;
    if size > limits.max_bytes {
        return Err(format!("{path}: larger than {} bytes", limits.max_bytes));
    }
    let bytes = s.services.read_file(path).map_err(|e| format!("{path}: {e}"))?;
    cfa4_dcp::parse_bytes(&bytes, limits).map_err(|e| format!("{path}: {e}"))
}

/// `Some(Err)` for a DCP that cannot be read at all (no readable asset map, no composition);
/// `None` when `path` is not a DCP.
pub fn import_package(s: &mut Session, path: &str, bin: Option<BinId>) -> Option<Result<Value>> {
    let (root, only_cpl) = package_of(s, path)?;
    Some(import_root(s, &root, only_cpl.as_deref(), bin))
}

fn import_root(s: &mut Session, root: &str, only_cpl: Option<&str>, bin: Option<BinId>) -> Result<Value> {
    let map = ASSET_MAP_NAMES
        .iter()
        .map(|n| Path::new(root).join(n).to_string_lossy().into_owned())
        .find(|p| s.services.file_size(p).is_ok())
        .ok_or_else(|| EngineError::Other(format!("{root}: no ASSETMAP")))?;
    let Document::AssetMap(map) = read_document(s, &map).map_err(EngineError::Other)? else {
        return Err(EngineError::Other(format!("{map}: not an asset map")));
    };
    let mut warnings: Vec<String> = Vec::new();
    // asset id -> file
    let mut files: HashMap<Uuid, String> = HashMap::new();
    for a in &map.assets {
        let (Some(id), Some(chunk)) = (a.id, a.chunks.first().and_then(|c| c.path.as_deref())) else { continue };
        match resolve(root, chunk) {
            Ok(f) => {
                files.insert(id, f);
            }
            Err(e) => warnings.push(format!("asset {id}: {e}")),
        }
    }
    // the compositions: the one asked for, else every CPL the asset map lists
    let mut compositions: Vec<(String, Composition)> = Vec::new();
    let candidates: Vec<String> = match only_cpl {
        Some(p) => vec![p.to_string()],
        None => {
            let pkls: Vec<Uuid> = map.assets.iter().filter(|a| a.packing_list).filter_map(|a| a.id).collect();
            let mut v: Vec<String> = files
                .iter()
                .filter(|(id, f)| !pkls.contains(id) && Path::new(f.as_str()).extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")))
                .map(|(_, f)| f.clone())
                .collect();
            v.sort();
            v
        }
    };
    for f in candidates {
        // a small read first: track files and other XML are not parsed whole
        let head = s.services.read_range(&f, 0, 4096).unwrap_or_default();
        if !String::from_utf8_lossy(&head).contains("CompositionPlaylist") {
            continue;
        }
        match read_document(s, &f) {
            Ok(Document::Composition(c)) => compositions.push((f, c)),
            Ok(_) => {}
            Err(e) => warnings.push(e),
        }
        if compositions.len() >= MAX_COMPOSITIONS {
            warnings.push(format!("more than {MAX_COMPOSITIONS} compositions; the rest are not imported"));
            break;
        }
    }
    if compositions.is_empty() {
        return Err(EngineError::Other(format!("{root}: the package has no composition playlist")));
    }
    let root_name = Path::new(root).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "DCP".into());
    s.edit_group(&format!("Import DCP {root_name}"), |s| {
        let mut sequences = Vec::new();
        let mut items: HashMap<String, ItemId> = HashMap::new();
        for (cpl_path, cpl) in &compositions {
            let name = cpl.content_title.clone().or_else(|| cpl.annotation.clone()).unwrap_or_else(|| root_name.clone());
            match import_composition(s, &name, cpl, &files, &mut items, bin, &mut warnings) {
                Ok(v) => sequences.push(v),
                Err(e) => warnings.push(format!("{cpl_path}: {e}")),
            }
        }
        if sequences.is_empty() {
            return Err(EngineError::Other(format!("{root}: nothing could be imported: {}", warnings.join("; "))));
        }
        Ok(json!({
            "format": "DCP",
            "standard": match compositions.first().map(|c| c.1.standard) {
                Some(cfa4_dcp::Standard::Interop) => "Interop",
                _ => "SMPTE",
            },
            "sequences": sequences.iter().map(|c| c["sequence"].clone()).collect::<Vec<_>>(),
            "compositions": sequences,
            "items": items.values().map(|i| i.0).collect::<Vec<_>>(),
            "warnings": warnings,
        }))
    })
}

/// An asset's time in ticks: `units` edit units at `rate`.
fn units(units: u64, rate: Rational) -> Option<Tick> {
    if rate.num <= 0 || rate.den <= 0 {
        return None;
    }
    Some(Tick::from_rational(i64::try_from(units).ok()?, rate.den, rate.num))
}

/// The asset's range in its track file: from `EntryPoint`, `Duration` long (default: the rest of
/// the intrinsic duration).
fn asset_range(a: &ReelAsset, fallback_rate: Option<Rational>) -> Option<(Tick, Tick)> {
    let rate = a.edit_rate.or(fallback_rate)?;
    let entry = a.entry_point.unwrap_or(0);
    let dur = a.duration.or_else(|| a.intrinsic_duration.map(|d| d.saturating_sub(entry)))?;
    Some((units(entry, rate)?, units(dur, rate)?))
}

fn import_asset(s: &mut Session, file: &str, bin: Option<BinId>, items: &mut HashMap<String, ItemId>) -> Result<ItemId> {
    if let Some(id) = items.get(file) {
        return Ok(*id);
    }
    let id = if s.services.reader(file).is_some() {
        crate::commands::import_streamed(s, file, bin)?
    } else {
        let b = s.services.read_file(file).map_err(|e| EngineError::Other(format!("{file}: {e}")))?;
        crate::commands::import_bytes(s, file, b.into(), bin)?
    };
    items.insert(file.to_string(), id);
    Ok(id)
}

/// Import one CPL as a sequence; returns `{sequence, name, reels, placed}`.
fn import_composition(
    s: &mut Session,
    name: &str,
    cpl: &Composition,
    files: &HashMap<Uuid, String>,
    items: &mut HashMap<String, ItemId>,
    bin: Option<BinId>,
    warnings: &mut Vec<String>,
) -> Result<Value> {
    let cpl_bin = s.edit(&format!("New Bin {name}"), |p, _| Ok(p.add_bin(name, bin)))?;
    // the track files of each reel asset that can be placed, imported once
    let mut placed_assets: Vec<(usize, AssetKind, ItemId, Tick, Tick)> = Vec::new();
    let mut reel_lengths: Vec<Tick> = Vec::new();
    for (ri, reel) in cpl.reels.iter().take(MAX_REELS).enumerate() {
        let picture_rate = reel.assets.iter().find(|a| matches!(a.kind, AssetKind::MainPicture | AssetKind::MainStereoscopicPicture)).and_then(|a| a.edit_rate);
        let mut length = Tick::ZERO;
        for a in &reel.assets {
            let what = format!("reel {} {}", ri + 1, a.element);
            match a.kind {
                AssetKind::MainPicture | AssetKind::MainStereoscopicPicture | AssetKind::MainSound => {}
                AssetKind::CompositionMetadata | AssetKind::MainMarkers => continue,
                _ => {
                    warnings.push(format!("{what}: not placed (this import places picture and sound)"));
                    continue;
                }
            }
            let Some((entry, dur)) = asset_range(a, picture_rate) else {
                warnings.push(format!("{what}: no usable EditRate / Duration"));
                continue;
            };
            if a.kind != AssetKind::MainSound || length == Tick::ZERO {
                length = length.max(dur);
            }
            let Some(id) = a.id else {
                warnings.push(format!("{what}: no Id"));
                continue;
            };
            let Some(file) = files.get(&id) else {
                warnings.push(format!("{what}: asset {id} is not in this package (a version file needs its original version)"));
                continue;
            };
            if let Some(key) = a.key_id
                && cfa4_mxf::crypt::key(&key.0).is_none()
            {
                warnings.push(format!("{what}: encrypted, and the content key {key} is not loaded (load its KDM with dcp.loadKdm first)"));
                continue;
            }
            match import_asset(s, file, Some(cpl_bin), items) {
                Ok(item) => placed_assets.push((ri, a.kind, item, entry, dur)),
                Err(e) => warnings.push(format!("{what}: {file}: {e}")),
            }
        }
        reel_lengths.push(length);
    }
    if cpl.reels.len() > MAX_REELS {
        warnings.push(format!("more than {MAX_REELS} reels; the rest are not imported"));
    }
    // the sequence: the first picture's size and rate
    let first_picture = placed_assets.iter().find(|p| p.1 != AssetKind::MainSound).map(|p| p.2);
    let settings =
        first_picture.and_then(|i| s.project.item(i).and_then(|it| it.as_media().map(crate::commands::default_seq_settings_for))).unwrap_or_default();
    let rate = settings.frame_rate;
    let seq = s.edit(&format!("New Sequence {name}"), |p, _| Ok(p.new_sequence(name, settings, 1, 1, Some(cpl_bin))))?;
    s.state.active_sequence = Some(seq);
    s.state.open_sequences.push(seq);
    let (vt, at) = s.project.sequence(seq).map(|q| (q.video_tracks.first().map(|t| t.id), q.audio_tracks.first().map(|t| t.id))).unwrap_or((None, None));
    let mut starts = Vec::with_capacity(reel_lengths.len());
    let mut t = Tick::ZERO;
    for l in &reel_lengths {
        starts.push(t);
        t = rate.snap_nearest(Tick(t.0.saturating_add(l.0)));
    }
    let mut placed = 0usize;
    for (ri, kind, item, entry, dur) in placed_assets {
        let at_time = starts.get(ri).copied().unwrap_or_default();
        let (v, a) = if kind == AssetKind::MainSound { (None, at) } else { (vt, None) };
        let range = TimeRange::new(entry, rate.snap_nearest(dur).max(rate.frame_duration()));
        match crate::commands::place_item(s, item, range, at_time, v, a, false, "Place DCP Reel", None) {
            Ok(_) => placed += 1,
            Err(e) => warnings.push(format!("reel {}: {e}", ri + 1)),
        }
    }
    Ok(json!({"sequence": seq.0, "name": name, "reels": reel_lengths.len(), "placed": placed}))
}
