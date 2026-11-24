//! `captions.importAsTitles` (Kdenlive plan KDN-37): read a caption file (SRT, WebVTT, …) and place
//! every cue as a timed `graphics.newText` clip instead of a caption. `captions.import` stays the
//! way to get real captions. All the clips are one undo step.

use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, bad, has_seq, str_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "captions.importAsTitles";
/// A file with more cues than this is refused (each cue becomes a clip).
const MAX_CUES: usize = 10_000;
/// Parameters handed on to every `graphics.newText` call.
const PASS: [&str; 6] = ["position", "box", "size", "font", "fontStyle", "track"];

/// The cue text without inline markup (`<i>`, `<c.cls>`, `<00:01.000>` …); line breaks stay.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn import_as_titles(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad(CMD, "need `path`"))?.to_string();
    let bytes = s.services.read_file(&path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let format = cfa4_captions::detect(&bytes, ext.as_deref()).ok_or_else(|| EngineError::Other(format!("{path}: not a caption file")))?;
    let doc = cfa4_captions::parse(&bytes, format).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if doc.cues.len() > MAX_CUES {
        return Err(bad(CMD, format!("{} cues: more than {MAX_CUES}", doc.cues.len())));
    }
    let offset = crate::commands::time_p(s, p, "offset").unwrap_or_default();
    let mut base = Map::new();
    for k in PASS {
        if let Some(v) = p.get(k) {
            base.insert(k.into(), v.clone());
        }
    }
    let cues: Vec<_> = doc
        .cues
        .iter()
        .map(|c| (plain(&c.text), (c.start + offset).max(cfa4_time::Tick::ZERO), c.end - c.start))
        .filter(|(t, _, d)| !t.is_empty() && d.0 > 0)
        .collect();
    if cues.is_empty() {
        return Err(bad(CMD, format!("{path}: no cue with text and a duration")));
    }
    let skipped = doc.cues.len() - cues.len();
    let clips = s.edit_group("Import Captions as Titles", |s| {
        let mut ids = Vec::with_capacity(cues.len());
        for (text, start, dur) in &cues {
            let mut q = base.clone();
            q.insert("text".into(), json!(text));
            q.insert("time".into(), json!(start.0));
            q.insert("seconds".into(), json!(dur.seconds()));
            q.insert("newClip".into(), json!(true));
            let r = s.execute("graphics.newText", Value::Object(q))?;
            let r = r.get("result").unwrap_or(&r);
            ids.push(r["clip"].as_u64().unwrap_or(0));
        }
        Ok(ids)
    })?;
    Ok(json!({"clips": clips, "cues": cues.len(), "skipped": skipped, "warnings": doc.warnings}))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Import Captions as Titles…",
        menu: &[],
        shortcut: None,
        params: r#"{"path":str,"offsetTime":ticks?|"offsetSeconds"|"offsetFrame"|"offsetTimecode","track":index?,"size":px?,"font":str?,"fontStyle":str?,"position":[x,y]?,"box":[w,h]?}"#,
        enabled: has_seq,
        run: import_as_titles,
        journal: true,
    }]
}
