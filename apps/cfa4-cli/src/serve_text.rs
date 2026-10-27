//! `text.inspect`: what the Qt GUI's Text panel needs beyond the engine's `captions.list` and
//! `transcript.inspect`, as egui's `crates/ui-egui/src/panels/text.rs` reads it: the Graphics
//! tab's text layers (every text layer of every graphic clip in the active sequence, with its text
//! at the playhead), the caption formats a new track can have and whether speech-to-text is there.

use cfa4_engine::Session;
use cfa4_project::graphic::{TEXT_LAYER, layer_indices};
use cfa4_project::{CaptionFormat, ParamValue};
use cfa4_time::{Tick, TimeDisplay, format_time};
use serde_json::{Value, json};

/// `text.inspect {time?}`; `time` stands in for the playhead while the GUI plays.
pub fn inspect(s: &Session, p: &Value) -> Result<Value, String> {
    let speech = s.transcriber.is_some() || cfa4_engine::transcript::speech_available();
    let formats: Vec<&str> = CaptionFormat::ALL.iter().map(|f| f.label()).collect();
    let Some(seq) = s.active_sequence() else { return Ok(json!({"sequence": false, "graphics": [], "captionFormats": formats, "speech": speech})) };
    let at = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let (rate, df) = (seq.settings.frame_rate, seq.settings.drop_frame);
    let mut rows: Vec<(usize, Tick, usize, Value)> = Vec::new();
    for (ti, track) in seq.video_tracks.iter().enumerate() {
        for it in &track.items {
            let t = if it.start <= at && at < it.end() { at } else { it.start };
            let mt = it.effect_time_at(t);
            for (layer, ei) in layer_indices(&it.effects).into_iter().enumerate() {
                let Some(e) = it.effects.get(ei).filter(|e| e.effect == TEXT_LAYER) else { continue };
                let text = match e.param("text").map(|p| p.value_at(mt)) {
                    Some(ParamValue::Text(s)) => s,
                    _ => String::new(),
                };
                let name = match e.param("name").map(|p| p.value.clone()) {
                    Some(ParamValue::Text(n)) if !n.trim().is_empty() => Some(n),
                    _ => None,
                };
                let row = json!({
                    "clip": it.id.0,
                    "layer": layer,
                    "start": it.start.0,
                    "timecode": format_time(it.start, rate, df, TimeDisplay::Timecode, 48_000),
                    "track": format!("V{}", ti + 1),
                    "name": name,
                    "text": text,
                    "at": t.0,
                });
                rows.push((ti, it.start, layer, row));
            }
        }
    }
    // timeline order, then lower tracks first, then paint order (egui's `graphic_texts`)
    rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)).then(a.2.cmp(&b.2)));
    Ok(json!({
        "sequence": true,
        "graphics": rows.into_iter().map(|r| r.3).collect::<Vec<_>>(),
        "graphicSelection": {"clips": s.state.selection.iter().map(|c| c.0).collect::<Vec<_>>(), "layers": s.state.graphic_layers},
        "captionFormats": formats,
        "speech": speech,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_sequence_text_layers() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let r = inspect(&s, &json!({})).expect("inspect");
        assert_eq!(r["sequence"], json!(true));
        assert_eq!(r["captionFormats"].as_array().map(Vec::len), Some(4));
        let before = r["graphics"].as_array().map_or(0, Vec::len);
        s.execute("graphics.newText", json!({"text": "Hello Qt"})).expect("text");
        let r = inspect(&s, &json!({})).expect("inspect");
        let rows = r["graphics"].as_array().cloned().expect("graphics");
        assert_eq!(rows.len(), before + 1, "{r}");
        let row = rows.iter().find(|g| g["text"] == json!("Hello Qt")).expect("the new text layer");
        assert!(row["track"].as_str().is_some_and(|t| t.starts_with('V')), "{r}");
        assert_eq!(row["layer"], json!(0));
    }
}
