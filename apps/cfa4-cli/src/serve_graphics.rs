//! `graphics.inspect`: what the Qt GUI's Essential Graphics ▸ Edit tab shows, as egui's
//! `crates/ui-egui/src/panels/graphics.rs` (`properties`) reads it from the project: the selected
//! graphic clip, its layers (front last, as stored), the selected layer's properties at the
//! playhead with their animation state, its Responsive Design – Position pin, the clip's
//! Responsive Design – Time and roll, and a template graphic's editable properties. Every change
//! goes through engine commands (`graphics.set`, `graphics.setText`, `graphics.selectLayer`,
//! `graphics.pin`, `graphics.setRoll`, `graphics.setResponsiveTime`, `graphics.template.set`,
//! `effects.toggleAnimation`, …).

use cfa4_engine::Session;
use cfa4_project::graphic::{self, layer_display_name, layer_indices};
use cfa4_project::{ClipId, ItemKind, ParamValue, PinTarget, RollMode};
use cfa4_time::Tick;
use serde_json::{Value, json};

fn num(x: f64) -> Value {
    if x.is_finite() { json!(x) } else { Value::Null }
}

fn hex(c: [f32; 4]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

/// A layer property as the GUI edits it (what `graphics.set` takes back).
fn value_json(v: &ParamValue) -> Value {
    match v {
        ParamValue::Float(x) => num(*x),
        ParamValue::Vec2(p) => json!([num(p.x), num(p.y)]),
        ParamValue::Color(c) => json!(hex(*c)),
        ParamValue::Bool(b) => json!(b),
        ParamValue::Choice(c) => json!(c),
        ParamValue::Text(s) => json!(s),
        _ => Value::Null,
    }
}

fn roll_name(m: RollMode) -> &'static str {
    match m {
        RollMode::Off => "off",
        RollMode::Roll => "roll",
        RollMode::CrawlLeft => "crawlLeft",
        RollMode::CrawlRight => "crawlRight",
    }
}

/// `graphics.inspect {time?}`; `time` stands in for the playhead while the GUI plays.
pub fn inspect(s: &mut Session, p: &Value) -> Result<Value, String> {
    let Some(seq) = s.active_sequence() else { return Ok(json!({"clip": null})) };
    let graphic = |c: &ClipId| seq.find_item(*c).and_then(|(_, it)| s.project.item(it.item)).is_some_and(|i| matches!(i.kind, ItemKind::Graphic { .. }));
    // egui's selected_graphic: the first selected graphic clip
    let Some(clip) = s.state.selection.iter().copied().find(graphic) else { return Ok(json!({"clip": null})) };
    let (_, it) = seq.find_item(clip).ok_or("no such clip")?;
    let it = it.clone();
    let rate = s.sequence_rate();
    let frames = |x: Tick| rate.frame_at(rate.snap_nearest(x));
    let ph = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let mt = it.source_time_at(ph.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
    let idx = layer_indices(&it.effects);
    let layers: Vec<Value> = idx
        .iter()
        .enumerate()
        .map(|(li, &ei)| {
            let e = &it.effects[ei];
            json!({"layer": li, "effectIndex": ei, "kind": if e.effect == graphic::TEXT_LAYER { "text" } else { "shape" }, "name": layer_display_name(e, li), "enabled": e.enabled})
        })
        .collect();
    let selected = s.state.graphic_layers.first().copied().filter(|l| *l < idx.len());
    let label = s.prefs.labels.rgb(it.label);
    let m = it.graphic.as_deref().cloned().unwrap_or_default();
    let mut out = json!({
        "clip": clip.0,
        "name": it.name,
        "labelColor": format!("#{:02x}{:02x}{:02x}", label[0], label[1], label[2]),
        "layers": layers,
        "layer": selected,
        "time": {"intro": frames(m.intro), "outro": frames(m.outro)},
        "roll": {
            "mode": roll_name(m.roll.mode),
            "startOffScreen": m.roll.start_off_screen,
            "endOffScreen": m.roll.end_off_screen,
            "preroll": frames(m.roll.preroll),
            "easeIn": frames(m.roll.ease_in),
            "easeOut": frames(m.roll.ease_out),
            "postroll": frames(m.roll.postroll),
        },
        "rollModes": RollMode::ALL.iter().map(|r| json!([roll_name(*r), r.label()])).collect::<Vec<_>>(),
        "shapes": graphic::SHAPE_OPTS,
        "fillKinds": graphic::FILL_KIND_OPTS,
        "strokes": graphic::STROKE_OPTS,
        "template": null,
        "params": null,
        "pin": null,
    });
    if it.graphic.as_ref().is_some_and(|g| g.template.is_some()) {
        out["template"] = s.execute("graphics.template.controls", json!({"clip": clip.0})).map_err(|e| e.to_string())?;
    }
    let Some(l) = selected else { return Ok(out) };
    let e = &it.effects[idx[l]];
    let def = e.def().ok_or("unknown layer")?;
    let mut params = serde_json::Map::new();
    for pd in &def.params {
        let prm = e.params.get(pd.id);
        let v = prm.map(|x| x.value_at(mt)).unwrap_or_else(|| pd.default.clone());
        params.insert(pd.id.to_string(), json!({"value": value_json(&v), "animatable": pd.animatable, "animated": prm.is_some_and(|x| x.is_animated())}));
    }
    out["params"] = Value::Object(params);
    out["effectIndex"] = json!(idx[l]);
    out["kind"] = json!(if e.effect == graphic::TEXT_LAYER { "text" } else { "shape" });
    if let Some(pin) = e.layer.as_ref().and_then(|x| x.pin.clone()) {
        let to = match pin.to {
            PinTarget::Frame => json!("frame"),
            PinTarget::Layer(uid) => json!((0..idx.len()).find(|&i| cfa4_project::gtemplate::layer_uid(&it.effects[idx[i]]) == uid)),
        };
        out["pin"] = json!({"to": to, "left": pin.left, "top": pin.top, "right": pin.right, "bottom": pin.bottom});
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_the_selected_graphic() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let none = inspect(&mut s, &json!({})).expect("inspect");
        assert_eq!(none["clip"], Value::Null);
        s.execute("graphics.newText", json!({"text": "Hello Qt"})).expect("text");
        let clip = s.state.selection.first().copied().expect("the new graphic is selected");
        s.execute("graphics.newShape", json!({"clip": clip.0, "shape": "ellipse"})).expect("shape");
        s.execute("graphics.selectLayer", json!({"clip": clip.0, "layers": [0]})).expect("select layer");
        let r = inspect(&mut s, &json!({})).expect("inspect");
        assert_eq!(r["clip"], json!(clip.0));
        let kinds: Vec<&str> = r["layers"].as_array().expect("layers").iter().filter_map(|l| l["kind"].as_str()).collect();
        assert_eq!(kinds, vec!["text", "shape"], "{r}");
        assert_eq!(r["layer"], json!(0));
        assert_eq!(r["kind"], json!("text"));
        assert_eq!(r["params"]["text"]["value"], json!("Hello Qt"), "{r}");
        assert!(r["params"]["position"]["value"].is_array());
        assert!(r["params"]["fill_color"]["value"].as_str().is_some_and(|c| c.starts_with('#')));
        assert_eq!(r["params"]["opacity"]["animatable"], json!(true));
        assert_eq!(r["roll"]["mode"], json!("off"));
        assert_eq!(r["pin"], Value::Null);
        s.execute("graphics.pin", json!({"clip": clip.0, "layer": 0, "to": "frame", "edges": ["left", "top"]})).expect("pin");
        let r = inspect(&mut s, &json!({})).expect("inspect");
        assert_eq!(r["pin"], json!({"to": "frame", "left": true, "top": true, "right": false, "bottom": false}), "{r}");
    }
}
