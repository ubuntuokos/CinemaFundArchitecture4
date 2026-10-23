//! Credits sequences from a spreadsheet (`graphics.credits.*`), on [`filmcraft_credits`].
//!
//! - `graphics.credits.import` reads a credits spreadsheet (CSV / TSV, a file or inline text),
//!   lays it out with the real text engine and places one graphic clip per page from the
//!   playhead (or `time`), one after another with the page gaps, as one undo step. Scroll pages
//!   roll (Roll, start and end off screen); card pages fade in and out. Every page stays an
//!   ordinary, editable graphic.
//! - `graphics.credits.preview` does the same without changing the project and reports pages,
//!   durations and warnings.
//! - `graphics.credits.styles` lists the style names and returns a starter styling (JSON).

use filmcraft_credits::layout::{Frame, Measure};
use filmcraft_credits::{Laid, Styling};
use filmcraft_project::EffectInstance;
use filmcraft_project::graphic::eval_layer;
use filmcraft_render::graphic_clip::layer_local_bounds;
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, has_seq, str_p, u64_p};
use crate::graphics::place_video_clip;
use crate::{EngineError, Result, Session};

/// Largest spreadsheet or styling file read from disk.
const MAX_FILE: u64 = 16 << 20;

/// Measures text layers with the text engine the renderer uses.
struct EngineMeasure {
    canvas: (u32, u32),
}

impl Measure for EngineMeasure {
    fn bounds(&self, layer: &EffectInstance) -> Option<[f64; 4]> {
        let spec = eval_layer(layer, Tick::ZERO, self.canvas)?;
        let b = layer_local_bounds(&spec);
        Some(b.map(f64::from))
    }
}

fn read_file(cmd: &str, path: &str) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    if meta.len() > MAX_FILE {
        return Err(bad(cmd, format!("{path} is larger than {} MiB", MAX_FILE >> 20)));
    }
    std::fs::read(path).map_err(|e| bad(cmd, format!("{path}: {e}")))
}

fn styling_p(cmd: &str, p: &Value) -> Result<Styling> {
    match p.get("styling") {
        None | Some(Value::Null) => Ok(Styling::default()),
        Some(Value::Object(_)) => serde_json::from_value(p.get("styling").cloned().unwrap_or(Value::Null)).map_err(|e| bad(cmd, format!("styling: {e}"))),
        Some(Value::String(s)) => {
            let text = if s.trim_start().starts_with('{') { s.clone() } else { String::from_utf8_lossy(&read_file(cmd, s)?).into_owned() };
            Styling::from_json(&text).map_err(|e| bad(cmd, e.to_string()))
        }
        Some(_) => Err(bad(cmd, "`styling` is an object, JSON text or the path of a .json file")),
    }
}

/// Read and lay out the spreadsheet a command names, for the active sequence.
fn laid(s: &Session, cmd: &str, p: &Value) -> Result<Laid> {
    let styling = styling_p(cmd, p)?;
    let bytes = match (str_p(p, "csv"), str_p(p, "path")) {
        (Some(text), _) => text.as_bytes().to_vec(),
        (None, Some(path)) => {
            let lower = path.to_lowercase();
            if [".xlsx", ".xls", ".ods", ".numbers"].iter().any(|e| lower.ends_with(e)) {
                return Err(bad(cmd, "save the sheet as CSV (comma, semicolon or tab separated) first; office spreadsheet files are not read yet"));
            }
            read_file(cmd, path)?
        }
        (None, None) => return Err(bad(cmd, "need `path` (a .csv / .tsv file) or `csv` (the table as text)")),
    };
    let credits = filmcraft_credits::read(&bytes, &styling).map_err(|e| bad(cmd, e.to_string()))?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = Frame { width: q.settings.width, height: q.settings.height, rate: q.settings.frame_rate };
    filmcraft_credits::layout(&credits, &styling, frame, &EngineMeasure { canvas: (frame.width, frame.height) }).map_err(|e| bad(cmd, e.to_string()))
}

fn warnings_json(l: &Laid) -> Value {
    Value::Array(l.warnings.iter().map(|w| json!({"row": w.row, "message": w.message})).collect())
}

fn behaviour_name(b: filmcraft_credits::Behaviour) -> &'static str {
    match b {
        filmcraft_credits::Behaviour::Card => "card",
        filmcraft_credits::Behaviour::Scroll => "scroll",
    }
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    let l = laid(s, "graphics.credits.preview", p)?;
    let total: i64 = l.pages.iter().fold(0i64, |a, pg| a.saturating_add(pg.duration.0).saturating_add(pg.gap_after.0));
    Ok(json!({
        "pages": l.pages.iter().map(|pg| json!({
            "name": pg.name,
            "behaviour": behaviour_name(pg.behaviour),
            "duration": pg.duration.0,
            "seconds": pg.duration.0 as f64 / filmcraft_time::TICKS_PER_SECOND as f64,
            "gapAfter": pg.gap_after.0,
            "layers": pg.layers.len(),
            "contentHeight": pg.content_height,
        })).collect::<Vec<_>>(),
        "duration": total.max(0),
        "warnings": warnings_json(&l),
    }))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "graphics.credits.import";
    let l = laid(s, CMD, p)?;
    let warnings = warnings_json(&l);
    let rate = s.sequence_rate();
    let mut t = crate::commands::time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let before = s.history.undo.len();
    let mut placed = Vec::new();
    let result = (|| -> Result<()> {
        for pg in l.pages {
            let mut pp = json!({"time": t.0});
            if let Some(tr) = u64_p(p, "track") {
                pp["track"] = json!(tr);
            }
            let dur = pg.duration;
            let meta = pg.meta;
            let clip = place_video_clip(s, CMD, &pg.name, &pp, "Import Credits", pg.layers, move |pr, (w, h, rate)| {
                (crate::graphics::graphic_source(pr, w, h, rate), dur)
            })?;
            s.edit_sequence("Import Credits", move |q, _, _| {
                let (_, it) = q.find_item_mut(clip).ok_or(filmcraft_edit::EditError::NoItem(clip))?;
                it.graphic = Some(Box::new(meta));
                Ok(())
            })?;
            placed.push(json!({"clip": clip.0, "name": pg.name, "start": t.0, "duration": dur.0, "behaviour": behaviour_name(pg.behaviour)}));
            t = rate.snap(Tick(t.0.saturating_add(dur.0).saturating_add(pg.gap_after.0).max(0)));
        }
        Ok(())
    })();
    // one undo step for the whole sequence of pages
    if s.history.undo.len() > before + 1 {
        s.history.undo.truncate(before + 1);
    }
    if let Some(e) = s.history.undo.get_mut(before) {
        e.0 = "Import Credits".to_string();
    }
    if let Err(e) = result {
        if s.history.undo.len() > before {
            let _ = s.execute("edit.undo", json!({}));
        }
        return Err(e);
    }
    s.state.graphic_layers.clear();
    Ok(json!({"clips": placed, "end": t.0, "warnings": warnings}))
}

fn styles(_: &mut Session, p: &Value) -> Result<Value> {
    let styling = styling_p("graphics.credits.styles", p)?;
    let (pages, contents) = styling.names();
    let starter = Styling {
        page_styles: filmcraft_credits::style::builtin_page_styles(),
        content_styles: filmcraft_credits::style::builtin_content_styles(),
        ..Styling::default()
    };
    Ok(json!({
        "pageStyles": pages,
        "contentStyles": contents,
        "starter": serde_json::to_value(&starter).unwrap_or(Value::Null),
        "columns": ["Head", "Body", "Tail", "Vertical Gap", "Content Style", "Spine Position", "Page Style", "Page Runtime", "Page Gap"],
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    const P: &str = r#"{"path":str (.csv / .tsv)?,"csv":str (the table as text)?,"styling":object|JSON text|path of a .json?,"time":ticks?,"track":index?}"#;
    vec![
        CommandSpec {
            id: "graphics.credits.import",
            label: "Import Credits Spreadsheet…",
            menu: &["Graphics and Titles"],
            shortcut: None,
            params: P,
            enabled: has_seq,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: "graphics.credits.preview",
            label: "Preview Credits",
            menu: &[],
            shortcut: None,
            params: P,
            enabled: has_seq,
            run: preview,
            journal: false,
        },
        CommandSpec {
            id: "graphics.credits.styles",
            label: "Credits Styles",
            menu: &[],
            shortcut: None,
            params: r#"{"styling":object|JSON text|path?}"#,
            enabled: always,
            run: styles,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "credits_tests.rs"]
mod tests;
