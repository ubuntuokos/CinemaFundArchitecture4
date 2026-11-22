//! Tracked objects (Object Mask plan, OM-A5): `objects.*` commands.
//!
//! An object is an [`ObjectTrack`] of a media item (`Project.objects`): a label and boxes at media
//! times, in source pixels. `objects.createFromBox` starts one from a box drawn on a frame;
//! `objects.list` reads them; `objects.rename` and `objects.remove` change them. Every change is
//! one undo step. The item is `item`, or the item of `clip` (default: the selected clip). Times are
//! media times in ticks (`mediaTime`); for a clip the default is the playhead mapped into the media.

use cfa4_geom::Rect;
use cfa4_project::object::{ObjectId, ObjectSample, ObjectSource, ObjectTrack};
use cfa4_project::{ClipId, ItemId};
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, clip_p, has_seq, item_p, str_p};
use crate::{EngineError, Result, Session};

/// Tracks one item may hold, so a hostile script cannot grow the project without bound.
pub const MAX_OBJECTS_PER_ITEM: usize = 4096;
/// Box coordinates beyond this are not pixels of any picture.
const MAX_COORD: f64 = 1.0e7;

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(id: &'static str, label: &'static str, params: &'static str, enabled: crate::commands::Enabled, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec(
            "objects.createFromBox",
            "Create Object from Box",
            r#"{"item":id?,"clip":id?,"box":[x,y,w,h]|{"x","y","w","h"},"mediaTime":ticks?,"label":str?}"#,
            has_seq,
            create_from_box,
            true,
        ),
        spec("objects.list", "List Objects", r#"{"item":id?,"clip":id?}"#, always, list, false),
        spec("objects.rename", "Rename Object", r#"{"id":n,"label":str}"#, always, rename, true),
        spec("objects.remove", "Delete Object", r#"{"id":n}"#, always, remove, true),
    ]
}

fn rect_from_json(v: &Value) -> Option<Rect> {
    let n = |x: &Value| x.as_f64().filter(|f| f.is_finite());
    let (x, y, w, h) = if let Some(a) = v.as_array() {
        if a.len() != 4 {
            return None;
        }
        (n(&a[0])?, n(&a[1])?, n(&a[2])?, n(&a[3])?)
    } else {
        (n(v.get("x")?)?, n(v.get("y")?)?, n(v.get("w")?)?, n(v.get("h")?)?)
    };
    let ok = [x, y, w, h].iter().all(|c| c.abs() <= MAX_COORD) && w > 0.0 && h > 0.0;
    ok.then(|| Rect::new(x, y, w, h))
}

/// The item a command addresses: `item`, else the item of `clip` or of the selected clip. Returns
/// the clip too when one was used (to map the playhead).
fn target(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, Option<ClipId>)> {
    if let Some(i) = item_p(p, "item") {
        s.project.item(i).ok_or_else(|| bad(cmd, format!("no item {}", i.0)))?;
        return Ok((i, None));
    }
    let clip = clip_p(p, "clip").or_else(|| s.state.selection.first().copied()).ok_or_else(|| bad(cmd, "need `item` or `clip`"))?;
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = seq.find_item(clip).ok_or_else(|| bad(cmd, "no such clip"))?;
    Ok((it.item, Some(clip)))
}

fn find_mut(pr: &mut cfa4_project::Project, id: ObjectId) -> Option<(ItemId, usize)> {
    pr.objects.iter().find_map(|(item, v)| v.iter().position(|t| t.id == id).map(|i| (*item, i)))
}

/// At most `MAX_TEXT` bytes, cut on a character boundary.
fn clamp_text(s: &str) -> String {
    let mut end = s.len().min(cfa4_project::object::MAX_TEXT);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn object_id(p: &Value, cmd: &str) -> Result<ObjectId> {
    let v = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "need `id`"))?;
    Ok(ObjectId(v))
}

fn create_from_box(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "objects.createFromBox";
    let bbox = p.get("box").and_then(rect_from_json).ok_or_else(|| bad(C, "`box` is [x, y, w, h] in source pixels, finite, w and h > 0"))?;
    let (item, clip) = target(s, p, C)?;
    let time = match p.get("mediaTime") {
        Some(v) => Tick(v.as_i64().ok_or_else(|| bad(C, "`mediaTime` is ticks"))?),
        None => match clip {
            Some(c) => {
                let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
                let it = seq.find_item(c).map(|(_, i)| i).ok_or_else(|| bad(C, "no such clip"))?;
                it.source_time_at(s.playhead().clamp(it.start, it.end() - Tick(1)))
            }
            None => Tick::ZERO,
        },
    };
    let mut label = clamp_text(str_p(p, "label").unwrap_or_default().trim());
    let sample = ObjectSample { time, bbox, score: 1.0, mask: None };
    sample.validate().map_err(|e| bad(C, e.to_string()))?;
    let id = s.edit("Create Object", move |pr, _| {
        if pr.objects.get(&item).is_some_and(|v| v.len() >= MAX_OBJECTS_PER_ITEM) {
            return Err(bad(C, format!("an item holds at most {MAX_OBJECTS_PER_ITEM} objects")));
        }
        let id = ObjectId(pr.alloc_id());
        if label.trim().is_empty() {
            let n = pr.objects.get(&item).map_or(0, Vec::len) + 1;
            label = format!("Object {n}");
        }
        let mut t = ObjectTrack::new(id, label, ObjectSource::Manual);
        t.insert_sample(sample).map_err(|e| bad(C, e.to_string()))?;
        t.validate().map_err(|e| bad(C, e.to_string()))?;
        pr.objects.entry(item).or_default().push(t);
        Ok(id)
    })?;
    Ok(json!({"id": id.0, "item": item.0}))
}

fn track_json(item: ItemId, t: &ObjectTrack) -> Value {
    let range = t.time_range();
    json!({
        "id": t.id.0,
        "item": item.0,
        "label": t.label,
        "class": t.class,
        "source": t.source,
        "samples": t.samples().len(),
        "start": range.map(|r| r.0.0),
        "end": range.map(|r| r.1.0),
        "hasMasks": t.samples().iter().any(|x| x.mask.is_some()),
    })
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let only = if p.get("item").is_some() || p.get("clip").is_some() { Some(target(s, p, "objects.list")?.0) } else { None };
    let rows: Vec<Value> =
        s.project.objects.iter().filter(|(i, _)| only.is_none_or(|o| o == **i)).flat_map(|(i, v)| v.iter().map(move |t| track_json(*i, t))).collect();
    Ok(json!({"objects": rows}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "objects.rename";
    let id = object_id(p, C)?;
    let label = clamp_text(str_p(p, "label").ok_or_else(|| bad(C, "need `label`"))?.trim());
    if label.is_empty() {
        return Err(bad(C, "`label` is empty"));
    }
    s.edit("Rename Object", move |pr, _| {
        let (item, i) = find_mut(pr, id).ok_or_else(|| bad(C, format!("no object {}", id.0)))?;
        let t = pr.objects.get_mut(&item).and_then(|v| v.get_mut(i)).ok_or_else(|| bad(C, "no such object"))?;
        t.label = label;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "objects.remove";
    let id = object_id(p, C)?;
    s.edit("Delete Object", move |pr, _| {
        let (item, i) = find_mut(pr, id).ok_or_else(|| bad(C, format!("no object {}", id.0)))?;
        if let Some(v) = pr.objects.get_mut(&item) {
            v.remove(i);
            if v.is_empty() {
                pr.objects.remove(&item);
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}
