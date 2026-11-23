//! `export.queue.addFromMarkers` (Kdenlive plan K37): turn the sequence markers into export ranges
//! and queue them with `export.queue.add`.
//!
//! A range marker (it has a duration) is one range. Point markers are zone boundaries: each one
//! starts a range that ends at the next point marker, the last one at the end of the sequence.
//! `markers` keeps only the markers whose colour name or name matches (case-insensitive).

use cfa4_project::Marker;
use cfa4_time::Tick;
use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, bad, has_seq};
use crate::{EngineError, Result, Session};

const CMD: &str = "export.queue.addFromMarkers";

fn matches(m: &Marker, filter: Option<&str>) -> bool {
    filter.is_none_or(|f| m.color.name().eq_ignore_ascii_case(f) || m.name.eq_ignore_ascii_case(f))
}

/// The (start, end) zones the matching markers describe, in time order.
fn zones(markers: &[Marker], filter: Option<&str>, end: Tick) -> Vec<(Tick, Tick)> {
    let mut out: Vec<(Tick, Tick)> = Vec::new();
    let mut points: Vec<Tick> = Vec::new();
    for m in markers.iter().filter(|m| matches(m, filter)) {
        if m.duration > Tick::ZERO {
            out.push((m.start, m.start + m.duration));
        } else {
            points.push(m.start);
        }
    }
    points.sort_unstable();
    points.dedup();
    for (i, &a) in points.iter().enumerate() {
        let b = points.get(i + 1).copied().unwrap_or(end);
        out.push((a, b));
    }
    out.retain(|(a, b)| b > a);
    out.sort_unstable();
    out
}

fn add_from_markers(s: &mut Session, p: &Value) -> Result<Value> {
    let filter = match p.get("markers") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) if !f.trim().is_empty() => Some(f.trim().to_string()),
        Some(_) => return Err(bad(CMD, "`markers` must be a colour or marker name")),
    };
    if p.get("ranges").is_some() || p.get("range").is_some() {
        return Err(bad(CMD, "the ranges come from the markers; do not pass `ranges` or `range`"));
    }
    let seq = crate::export_tools::sequence_param(s, p, CMD)?;
    let q = s.project.sequence(seq).ok_or(EngineError::NoSequence)?;
    let zones = zones(&q.markers, filter.as_deref(), q.duration());
    if zones.is_empty() {
        return Err(bad(CMD, "no markers match: nothing to export"));
    }
    let mut params: Map<String, Value> = p.as_object().cloned().unwrap_or_default();
    params.remove("markers");
    params.insert("sequence".into(), json!(seq.0));
    params.insert("ranges".into(), Value::Array(zones.iter().map(|(a, b)| json!({"mode": "custom", "startTime": a.0, "endTime": b.0})).collect()));
    crate::export_tools::queue_add(s, &Value::Object(params))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Send Marker Ranges to Export Queue",
        menu: &[],
        shortcut: None,
        params: r#"{"markers":"colour|name"?,"sequence":id?,…export.queue.add params}"#,
        enabled: has_seq,
        run: add_from_markers,
        journal: true,
    }]
}
