//! The integrity and consistency rules of DCP-05 (plan F02, F03, F04, F16, F17): pure functions
//! over the documents the native backend has read. Each returns the rule's findings and the message
//! it passes with.
//!
//! Sources: SMPTE ST 429-9 (asset map), ST 429-8 (packing list: `Hash` is the base64 SHA-1 of the
//! file, `Size` its length in bytes), ST 429-7 (composition: `EntryPoint` + `Duration` ≤
//! `IntrinsicDuration`, the assets of a reel play together, marker labels and offsets).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::Read as _;
use std::path::PathBuf;

use cfa4_dcp::sha1::{Sha1, base64};
use cfa4_dcp::{AssetKind, AssetMap, Composition, PackingList, Pos, Rational, ReelAsset, Uuid};

use crate::rules::*;
use crate::{Finding, Location, Status, ValidationControl};

/// Asset id → (chunk path as written, resolved file).
pub(crate) type Files = BTreeMap<Uuid, (String, PathBuf)>;

/// The ST 429-7 marker labels.
const MARKER_LABELS: &[&str] = &["FFOC", "LFOC", "FFTC", "LFTC", "FFOI", "LFOI", "FFEC", "LFEC", "FFOB", "LFOB", "FFMC", "LFMC"];

/// Bytes read per step while hashing; cancellation and progress are checked between steps.
const HASH_STEP: usize = 1 << 20;

pub(crate) fn at(file: &str, element: Option<String>, pos: Option<Pos>, asset: Option<Uuid>) -> Location {
    Location { file: Some(file.to_string()), element, line: pos.map(|p| p.line), column: pos.map(|p| p.column), asset }
}

fn fail(rule: &str, location: Location, message: String) -> Finding {
    Finding::new(rule, Status::Fail, message).at(location)
}

/// `DCP.AM.003`: no asset id appears twice in the asset map.
pub(crate) fn unique_ids(name: &str, am: &AssetMap) -> (Vec<Finding>, String) {
    let mut seen: HashMap<Uuid, usize> = HashMap::new();
    let mut out = Vec::new();
    for (k, a) in am.assets.iter().enumerate() {
        let Some(id) = a.id else { continue };
        if let Some(first) = seen.insert(id, k) {
            out.push(fail(
                AM_UNIQUE_IDS,
                at(name, Some(format!("AssetList/Asset[{}]", k + 1)), Some(a.pos), Some(id)),
                format!("asset id {id} is also asset {}", first + 1),
            ));
        }
    }
    if am.id.is_some_and(|id| seen.contains_key(&id)) {
        out.push(fail(AM_UNIQUE_IDS, at(name, Some("Id".into()), None, am.id), "the asset map's own id is also an asset id".into()));
    }
    (out, format!("{} asset ids, all distinct", seen.len()))
}

/// `DCP.AM.006`: the asset map declares at least one packing list.
pub(crate) fn has_pkl(name: &str, am: &AssetMap) -> (Vec<Finding>, String) {
    let n = am.assets.iter().filter(|a| a.packing_list).count();
    if n == 0 {
        return (vec![fail(AM_HAS_PKL, Location::file(name), "no asset is marked <PackingList>".into())], String::new());
    }
    (Vec::new(), format!("{n} packing lists declared"))
}

/// What the packing lists declare about one asset (the first declaration; later different ones are
/// reported).
struct Declared<'a> {
    pkl: &'a str,
    size: Option<u64>,
    sha1: Option<[u8; 20]>,
    pos: Pos,
}

fn declared<'a>(pkls: &'a [(String, PackingList)]) -> (BTreeMap<Uuid, Declared<'a>>, Vec<Finding>, Vec<Finding>) {
    let mut map: BTreeMap<Uuid, Declared<'a>> = BTreeMap::new();
    let (mut size_conflicts, mut hash_conflicts) = (Vec::new(), Vec::new());
    for (path, p) in pkls {
        for (k, a) in p.assets.iter().enumerate() {
            let Some(id) = a.id else { continue };
            let element = || Some(format!("AssetList/Asset[{}]", k + 1));
            match map.get(&id) {
                None => {
                    map.insert(id, Declared { pkl: path, size: a.size, sha1: a.sha1, pos: a.pos });
                }
                Some(d) => {
                    // an asset in two packing lists (F03, F17) must be declared alike
                    if a.size.is_some() && d.size.is_some() && a.size != d.size {
                        size_conflicts.push(fail(
                            PKL_SIZE,
                            at(path, element(), Some(a.pos), Some(id)),
                            format!("declared with size {} here and {} in {}", a.size.unwrap_or(0), d.size.unwrap_or(0), d.pkl),
                        ));
                    }
                    if a.sha1.is_some() && d.sha1.is_some() && a.sha1 != d.sha1 {
                        hash_conflicts.push(fail(PKL_HASH, at(path, element(), Some(a.pos), Some(id)), format!("declared with another hash in {}", d.pkl)));
                    }
                }
            }
        }
    }
    (map, size_conflicts, hash_conflicts)
}

/// `DCP.PKL.002`: every packing list asset is in the asset map.
pub(crate) fn pkl_in_assetmap(am: &AssetMap, pkls: &[(String, PackingList)]) -> (Vec<Finding>, String) {
    let in_am: BTreeSet<Uuid> = am.assets.iter().filter_map(|a| a.id).collect();
    let mut out = Vec::new();
    let mut n = 0usize;
    for (path, p) in pkls {
        for (k, a) in p.assets.iter().enumerate() {
            let Some(id) = a.id else { continue };
            n += 1;
            if !in_am.contains(&id) {
                out.push(fail(
                    PKL_IN_ASSETMAP,
                    at(path, Some(format!("AssetList/Asset[{}]", k + 1)), Some(a.pos), Some(id)),
                    format!("asset {id} is not in the asset map"),
                ));
            }
        }
    }
    (out, format!("{n} packing list assets, all in the asset map"))
}

/// `DCP.PKL.003`: every file has the size its packing list declares.
pub(crate) fn sizes(pkls: &[(String, PackingList)], files: &Files) -> (Vec<Finding>, String) {
    let (decl, mut out, _) = declared(pkls);
    let mut checked = 0usize;
    for (id, d) in &decl {
        let Some(size) = d.size else { continue };
        let Some((path, file)) = files.get(id) else {
            out.push(Finding::new(PKL_SIZE, Status::Unchecked, format!("asset {id} has no readable file (see DCP.AM.005)")).at(at(
                d.pkl,
                None,
                Some(d.pos),
                Some(*id),
            )));
            continue;
        };
        match std::fs::metadata(file) {
            Ok(m) if m.len() == size => checked += 1,
            Ok(m) => out.push(fail(PKL_SIZE, at(path, None, None, Some(*id)), format!("'{path}' is {} bytes; {} declares {size}", m.len(), d.pkl))),
            Err(e) => out.push(fail(PKL_SIZE, at(path, None, None, Some(*id)), format!("'{path}' cannot be read: {e}"))),
        }
    }
    (out, format!("{checked} files have their declared size"))
}

/// `DCP.PKL.004`: every file has the SHA-1 its packing list declares. `None` when cancelled.
pub(crate) fn hashes(pkls: &[(String, PackingList)], files: &Files, control: &ValidationControl) -> Option<(Vec<Finding>, String)> {
    let (decl, _, mut out) = declared(pkls);
    let total: u64 = decl.keys().filter_map(|id| files.get(id)).filter_map(|(_, f)| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    let mut done = 0u64;
    let mut checked = 0usize;
    let mut buf = vec![0u8; HASH_STEP];
    for (id, d) in &decl {
        let Some(want) = d.sha1 else {
            out.push(Finding::new(PKL_HASH, Status::Unchecked, format!("asset {id} declares no valid hash (see DCP.PKL.001)")).at(at(
                d.pkl,
                None,
                Some(d.pos),
                Some(*id),
            )));
            continue;
        };
        let Some((path, file)) = files.get(id) else {
            out.push(Finding::new(PKL_HASH, Status::Unchecked, format!("asset {id} has no readable file (see DCP.AM.005)")).at(at(
                d.pkl,
                None,
                Some(d.pos),
                Some(*id),
            )));
            continue;
        };
        let mut f = match File::open(file) {
            Ok(f) => f,
            Err(e) => {
                out.push(fail(PKL_HASH, at(path, None, None, Some(*id)), format!("'{path}' cannot be opened: {e}")));
                continue;
            }
        };
        let mut sha = Sha1::new();
        loop {
            control.report("hash", done, total);
            if control.is_cancelled() {
                return None;
            }
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    sha.update(&buf[..n]);
                    done = done.saturating_add(n as u64);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    out.push(fail(PKL_HASH, at(path, None, None, Some(*id)), format!("'{path}' cannot be read: {e}")));
                    break;
                }
            }
        }
        let got = sha.finish();
        if got == want {
            checked += 1;
        } else {
            out.push(fail(PKL_HASH, at(path, None, None, Some(*id)), format!("'{path}' hashes to {}; {} declares {}", base64(&got), d.pkl, base64(&want))));
        }
    }
    control.report("hash", total, total);
    Some((out, format!("{checked} files match their declared SHA-1")))
}

/// A reel asset that stands for a file of the package (markers and composition metadata do not).
fn is_track_file(a: &ReelAsset) -> bool {
    match a.kind {
        AssetKind::MainMarkers | AssetKind::CompositionMetadata => false,
        AssetKind::Other => a.hash.is_some(),
        _ => true,
    }
}

pub(crate) fn asset_element(r: usize, a: &ReelAsset) -> Option<String> {
    Some(format!("ReelList/Reel[{}]/AssetList/{:?}", r + 1, a.kind))
}

/// `DCP.CPL.002`: no composition is outside the packing lists. `cpls_outside` are the compositions
/// found among the asset map assets that no packing list names.
pub(crate) fn cpl_in_pkl(read: usize, cpls_outside: &[(String, Option<Uuid>)]) -> (Vec<Finding>, String) {
    let out = cpls_outside
        .iter()
        .map(|(path, id)| fail(CPL_IN_PKL, at(path, None, None, *id), format!("'{path}' is a composition playlist that no packing list names")))
        .collect();
    (out, format!("{read} composition playlists, all in a packing list"))
}

/// `DCP.CPL.003`: every reel asset that is a file is in a packing list. One that is not in this
/// package at all is a `WARNING` (a version file plays with its original version); one that the
/// asset map has but no packing list names is a `FAIL`.
pub(crate) fn cpl_assets_in_pkl(cpls: &[(String, Composition)], pkls: &[(String, PackingList)], am: &AssetMap) -> (Vec<Finding>, String) {
    let in_pkl: BTreeSet<Uuid> = pkls.iter().flat_map(|(_, p)| p.assets.iter().filter_map(|a| a.id)).collect();
    let in_am: BTreeSet<Uuid> = am.assets.iter().filter_map(|a| a.id).collect();
    let mut out = Vec::new();
    let mut n = 0usize;
    for (path, c) in cpls {
        for (r, reel) in c.reels.iter().enumerate() {
            for a in reel.assets.iter().filter(|a| is_track_file(a)) {
                let Some(id) = a.id else { continue };
                n += 1;
                let loc = at(path, asset_element(r, a), Some(a.pos), Some(id));
                if in_pkl.contains(&id) {
                    continue;
                }
                if in_am.contains(&id) {
                    out.push(fail(CPL_ASSETS_IN_PKL, loc, format!("{:?} {id} is in the asset map but in no packing list", a.kind)));
                } else {
                    out.push(
                        Finding::new(
                            CPL_ASSETS_IN_PKL,
                            Status::Warning,
                            format!("{:?} {id} is not in this package; as a version file it needs its original version", a.kind),
                        )
                        .at(loc),
                    );
                }
            }
        }
    }
    (out, format!("{n} reel assets, all in a packing list"))
}

/// The played duration of a reel asset: `Duration`, or `IntrinsicDuration` − `EntryPoint`.
fn played(a: &ReelAsset) -> Option<u64> {
    a.duration.or_else(|| a.intrinsic_duration.map(|d| d.saturating_sub(a.entry_point.unwrap_or(0))))
}

/// `DCP.CPL.004`: `EntryPoint` + `Duration` fit in `IntrinsicDuration`, and something is played.
pub(crate) fn durations(cpls: &[(String, Composition)]) -> (Vec<Finding>, String) {
    let mut out = Vec::new();
    let mut n = 0usize;
    for (path, c) in cpls {
        for (r, reel) in c.reels.iter().enumerate() {
            for a in &reel.assets {
                let Some(intrinsic) = a.intrinsic_duration else { continue };
                n += 1;
                let entry = a.entry_point.unwrap_or(0);
                let loc = || at(path, asset_element(r, a), Some(a.pos), a.id);
                let Some(dur) = played(a) else { continue };
                if entry.checked_add(dur).is_none_or(|end| end > intrinsic) {
                    out.push(fail(CPL_DURATIONS, loc(), format!("EntryPoint {entry} + Duration {dur} exceeds IntrinsicDuration {intrinsic}")));
                } else if dur == 0 {
                    out.push(fail(CPL_DURATIONS, loc(), "the asset plays no edit unit (Duration 0)".into()));
                }
            }
        }
    }
    (out, format!("{n} reel assets within their intrinsic duration"))
}

/// `a` edit units at `ra` equal `b` edit units at `rb` (exactly).
fn same_time(a: u64, ra: Rational, b: u64, rb: Rational) -> Option<bool> {
    let pos = |v: i64| u128::try_from(v).ok().filter(|v| *v > 0);
    let (an, ad, bn, bd) = (pos(ra.num)?, pos(ra.den)?, pos(rb.num)?, pos(rb.den)?);
    Some(u128::from(a) * ad * bn == u128::from(b) * bd * an)
}

fn same_rate(a: Rational, b: Rational) -> bool {
    i128::from(a.num) * i128::from(b.den) == i128::from(b.num) * i128::from(a.den)
}

/// `DCP.CPL.005`: the assets of a reel have the same edit rate and play for the same time.
pub(crate) fn reel_sync(cpls: &[(String, Composition)]) -> (Vec<Finding>, String) {
    let mut out = Vec::new();
    let mut n = 0usize;
    for (path, c) in cpls {
        for (r, reel) in c.reels.iter().enumerate() {
            n += 1;
            let timed: Vec<(&ReelAsset, Rational, u64)> = reel.assets.iter().filter_map(|a| Some((a, a.edit_rate?, played(a)?))).collect();
            let Some(&(first, rate, dur)) = timed.first() else { continue };
            let reel_at = at(path, Some(format!("ReelList/Reel[{}]", r + 1)), Some(reel.pos), reel.id);
            for &(a, ra, da) in &timed[1..] {
                if !same_rate(rate, ra) {
                    out.push(fail(
                        CPL_REEL_SYNC,
                        reel_at.clone(),
                        format!("{:?} has EditRate {} {}, {:?} has {} {}", a.kind, ra.num, ra.den, first.kind, rate.num, rate.den),
                    ));
                } else if same_time(da, ra, dur, rate) == Some(false) {
                    out.push(fail(CPL_REEL_SYNC, reel_at.clone(), format!("{:?} plays {da} edit units, {:?} plays {dur}", a.kind, first.kind)));
                }
            }
        }
    }
    (out, format!("{n} reels in sync"))
}

/// `DCP.MARK.001`: composition markers are known, in range, unique and in order.
pub(crate) fn markers(cpls: &[(String, Composition)]) -> (Vec<Finding>, String) {
    let mut out = Vec::new();
    let mut total = 0usize;
    for (path, c) in cpls {
        // label → (composition position, reel, marker pos)
        let mut seen: BTreeMap<String, (u64, usize, Pos)> = BTreeMap::new();
        let mut base = 0u64;
        let mut lengths_known = true;
        let mut any = false;
        for (r, reel) in c.reels.iter().enumerate() {
            // the reel's length: its first timed asset (all are equal when DCP.CPL.005 passes)
            let reel_len = reel.assets.iter().find(|a| a.kind != AssetKind::MainMarkers && a.edit_rate.is_some()).and_then(played);
            for a in reel.assets.iter().filter(|a| a.kind == AssetKind::MainMarkers) {
                any = true;
                let len = played(a).or(reel_len);
                for m in &a.markers {
                    total += 1;
                    let loc = || at(path, asset_element(r, a), Some(m.pos), c.id);
                    let Some(label) = m.label.as_deref() else { continue };
                    if !MARKER_LABELS.contains(&label) {
                        out.push(Finding::new(MARK_MARKERS, Status::Warning, format!("marker label '{label}' is not one of ST 429-7's")).at(loc()));
                    }
                    let Some(offset) = m.offset else { continue };
                    if len.is_some_and(|l| offset >= l) {
                        out.push(fail(MARK_MARKERS, loc(), format!("{label} at offset {offset} is outside the reel ({} edit units)", len.unwrap_or(0))));
                    }
                    let position = base.saturating_add(offset);
                    if let Some((_, first_reel, _)) = seen.insert(label.to_string(), (position, r, m.pos)) {
                        out.push(fail(MARK_MARKERS, loc(), format!("{label} appears again (first in reel {})", first_reel + 1)));
                    }
                }
            }
            lengths_known &= reel_len.is_some();
            base = base.saturating_add(reel_len.unwrap_or(0));
        }
        if !any && !c.reels.is_empty() {
            out.push(
                Finding::new(MARK_MARKERS, Status::Warning, "the composition has no markers (FFOC, LFOC)")
                    .at(Location::file(path.as_str()).with_asset_id(c.id)),
            );
            continue;
        }
        let pos_of = |l: &str| seen.get(l).map(|(p, _, _)| *p);
        // each first-frame marker comes before its last-frame marker
        for ff in MARKER_LABELS.iter().filter(|l| l.starts_with("FF")) {
            let lf = format!("LF{}", &ff[2..]);
            if let (Some(a), Some(b)) = (pos_of(ff), pos_of(&lf))
                && a > b
            {
                out.push(fail(MARK_MARKERS, Location::file(path.as_str()).with_asset_id(c.id), format!("{ff} ({a}) comes after {lf} ({b})")));
            }
        }
        // FFOC marks the composition's first frame and LFOC its last. Mastering tools differ on
        // whether the first frame is offset 0 or 1 (CFA4's export writes 1), so only a later FFOC
        // or an LFOC away from the last frame is reported, as a warning.
        let whole = || Location::file(path.as_str()).with_asset_id(c.id);
        if let Some(p) = pos_of("FFOC")
            && p > 1
        {
            out.push(Finding::new(MARK_MARKERS, Status::Warning, format!("FFOC is at {p}, not at the first frame")).at(whole()));
        }
        if let Some(p) = pos_of("LFOC")
            && lengths_known
            && base > 0
            && p != base - 1
        {
            out.push(Finding::new(MARK_MARKERS, Status::Warning, format!("LFOC is at {p}, not at the last frame ({})", base - 1)).at(whole()));
        }
        // nothing before the first or after the last frame of the composition
        if let (Some(first), Some(last)) = (pos_of("FFOC"), pos_of("LFOC")) {
            for (label, (p, _, _)) in &seen {
                if *p < first || *p > last {
                    out.push(fail(
                        MARK_MARKERS,
                        Location::file(path.as_str()).with_asset_id(c.id),
                        format!("{label} ({p}) is outside FFOC ({first}) … LFOC ({last})"),
                    ));
                }
            }
        }
    }
    (out, format!("{total} markers checked"))
}

impl Location {
    fn with_asset_id(mut self, id: Option<Uuid>) -> Location {
        self.asset = id;
        self
    }
}
