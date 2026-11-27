//! `edit.deleteShortClips` (Kdenlive plan KDN-37): find the clips shorter than a given number of
//! frames (typically leftovers of razor cuts or of `clip.removeSilence`) and delete them, closing
//! the gaps by default. Nothing is deleted unless `dryRun` is `false`; the default only reports.

use cfa4_project::{ClipId, TrackId};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq};
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.deleteShortClips";

fn find(s: &mut Session, p: &Value) -> Result<Value> {
    let min = match p.get("minFrames") {
        Some(v) => v.as_i64().filter(|n| *n >= 1).ok_or_else(|| bad(CMD, "`minFrames` must be an integer of at least 1"))?,
        None => return Err(bad(CMD, "need `minFrames`: clips shorter than this many frames are found")),
    };
    let ripple = bool_p(p, "ripple").unwrap_or(true);
    let dry = bool_p(p, "dryRun").unwrap_or(true);
    let only: Option<Vec<u64>> = match p.get("tracks") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_array()
                .ok_or_else(|| bad(CMD, "`tracks` must be an array of track ids"))?
                .iter()
                .map(|t| t.as_u64().ok_or_else(|| bad(CMD, "`tracks` must be an array of track ids")))
                .collect::<Result<_>>()?,
        ),
    };
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let rate = seq.frame_rate();
    if let Some(list) = &only
        && let Some(missing) = list.iter().find(|t| seq.track(TrackId(**t)).is_none())
    {
        return Err(bad(CMD, format!("no track {missing} in the active sequence")));
    }
    let limit = rate.tick_of(min);
    let mut found: Vec<(ClipId, Value)> = Vec::new();
    let mut skipped_locked = 0usize;
    for t in seq.all_tracks() {
        if only.as_ref().is_some_and(|l| !l.contains(&t.id.0)) {
            continue;
        }
        for i in t.items.iter().filter(|i| i.duration < limit) {
            if t.locked {
                skipped_locked += 1;
                continue;
            }
            found.push((i.id, json!({"clip": i.id.0, "track": t.id.0, "start": i.start.0, "frames": rate.frame_at(i.duration)})));
        }
    }
    let ids: Vec<u64> = found.iter().map(|(c, _)| c.0).collect();
    let deleted = !dry && !found.is_empty();
    if deleted {
        s.execute(if ripple { "edit.rippleDelete" } else { "edit.clear" }, json!({"clips": ids}))?;
    }
    Ok(json!({
        "minFrames": min,
        "dryRun": dry,
        "ripple": ripple,
        "found": found.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "deleted": deleted,
        "skippedLocked": skipped_locked,
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Delete Short Clips",
        menu: &["Edit"],
        shortcut: None,
        params: r#"{"minFrames":n,"tracks":[id]?,"ripple":bool=true,"dryRun":bool=true}"#,
        enabled: has_seq,
        run: find,
        journal: true,
    }]
}
