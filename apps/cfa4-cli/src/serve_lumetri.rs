//! `lumetri.inspect`: what the Qt GUI's Lumetri Color panel shows, as egui's
//! `crates/ui-egui/src/panels/lumetri.rs` reads it from the project: the selected video clip, its
//! Lumetri effect's values at the playhead with the slider ranges, each curve's drawn line
//! (`cfa4_render::effects::curve_lut`, the hue curves wrapped as egui draws them), the LUT menus
//! and whether the sequence grades in HDR. Every change goes through engine commands
//! (`effects.setParam`, `lumetri.setSection`, `lumetri.setInputLut`, `lumetri.setLook`,
//! `lumetri.applyMatch`, `effects.apply`, `effects.toggleEnabled`).

use cfa4_engine::Session;
use cfa4_project::{ClipId, ParamKind, ParamValue, TrackKind};
use cfa4_time::Tick;
use serde_json::{Value, json};

/// Points of a drawn curve line (egui's curve editor draws 128).
const LINE_POINTS: usize = 128;

fn num(x: f64) -> Value {
    if x.is_finite() { json!(x) } else { Value::Null }
}

/// The clip egui's panel edits: `clip` when it is a video clip, else the first selected video clip.
fn target(s: &Session, p: &Value) -> Option<ClipId> {
    let seq = s.active_sequence()?;
    let video = |c: ClipId| seq.find_item(c).is_some_and(|(t, _)| seq.track(t).is_some_and(|t| t.kind == TrackKind::Video));
    match p.get("clip").and_then(Value::as_u64) {
        Some(c) => Some(ClipId(c)).filter(|c| video(*c)),
        None => s.state.selection.iter().copied().find(|c| video(*c)),
    }
}

/// The drawn line of a curve: y at `LINE_POINTS` x positions over 0…1. A hue curve wraps around
/// (its points repeat one period to each side) and is flat at 0.5 without points.
fn curve_line(points: &[[f32; 2]], hue: bool) -> Vec<Value> {
    let lut = if hue && !points.is_empty() {
        let ext: Vec<[f32; 2]> = [-1.0f32, 0.0, 1.0].iter().flat_map(|off| points.iter().map(move |q| [(q[0] + off + 1.0) / 3.0, q[1]])).collect();
        let l = cfa4_render::effects::curve_lut(&ext, 3 * LINE_POINTS);
        l.get(LINE_POINTS..2 * LINE_POINTS).map(<[f32]>::to_vec).unwrap_or_default()
    } else if hue {
        vec![0.5; LINE_POINTS]
    } else {
        cfa4_render::effects::curve_lut(points, LINE_POINTS)
    };
    lut.into_iter().map(|y| num(f64::from(y.clamp(0.0, 1.0)))).collect()
}

fn lut_entry(s: &Session, r: &str) -> Value {
    json!({"ref": r, "name": cfa4_render::luts::label(Some(&s.project), r)})
}

/// `lumetri.inspect {clip?, time?}`; `time` stands in for the playhead while the GUI plays.
pub fn inspect(s: &Session, p: &Value) -> Result<Value, String> {
    let Some(clip) = target(s, p) else { return Ok(json!({"clip": null})) };
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let (_, it) = seq.find_item(clip).ok_or("no such clip")?;
    let hdr = seq.settings.color.working.is_hdr();
    let ph = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let at = it.effect_time_at(ph.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
    let mut out = json!({"clip": clip.0, "name": it.name, "hdr": hdr, "effect": null});
    let Some((idx, e)) = it.effects.iter().enumerate().find(|(_, e)| e.effect == "lumetri") else { return Ok(out) };
    let def = e.def().ok_or("unknown effect")?;
    let mut params = serde_json::Map::new();
    for pd in &def.params {
        let prm = e.params.get(pd.id).cloned().unwrap_or_else(|| cfa4_project::Param::new(pd.default.clone()));
        let value = prm.value_at(at);
        let mut j = json!({"label": pd.label, "animated": prm.is_animated()});
        match (&pd.kind, &value) {
            (ParamKind::Float { min, max, soft_min, soft_max, decimals, unit }, v) => {
                j["type"] = json!("float");
                j["value"] = num(v.as_f64().unwrap_or(0.0));
                j["default"] = num(pd.default.as_f64().unwrap_or(0.0));
                j["min"] = num(*min);
                j["max"] = num(*max);
                j["softMin"] = num(*soft_min);
                j["softMax"] = num(*soft_max);
                j["decimals"] = json!(decimals);
                j["unit"] = json!(unit);
            }
            (ParamKind::Choice(opts), v) => {
                j["type"] = json!("choice");
                j["options"] = json!(opts);
                j["value"] = json!(if let ParamValue::Choice(c) = v { *c } else { 0 });
            }
            (ParamKind::Bool, v) => {
                j["type"] = json!("bool");
                j["value"] = json!(v.as_bool().unwrap_or(false));
            }
            (ParamKind::Text, v) => {
                let t = if let ParamValue::Text(t) = v { t.clone() } else { String::new() };
                j["type"] = json!("text");
                j["value"] = json!(t);
                j["name"] = json!(cfa4_render::luts::label(Some(&s.project), &t));
            }
            (ParamKind::Curve { hue }, v) => {
                let pts: Vec<[f32; 2]> = v.as_curve().map(<[[f32; 2]]>::to_vec).unwrap_or_default();
                j["type"] = json!("curve");
                j["hue"] = json!(hue);
                j["value"] = json!(pts.iter().map(|q| json!([num(f64::from(q[0])), num(f64::from(q[1]))])).collect::<Vec<_>>());
                j["line"] = Value::Array(curve_line(&pts, *hue));
            }
            (ParamKind::Wheel, v) => {
                let w = if let ParamValue::Vec2(w) = v { *w } else { Default::default() };
                j["type"] = json!("wheel");
                j["value"] = json!([num(w.x), num(w.y)]);
            }
            (ParamKind::Color, ParamValue::Color(c)) => {
                j["type"] = json!("color");
                j["value"] = json!(c);
            }
            _ => continue,
        }
        params.insert(pd.id.to_string(), j);
    }
    let lib: Vec<Value> = s.project.luts.iter().map(|l| json!({"ref": format!("lib:{}", l.id), "name": l.name})).collect();
    let input: Vec<Value> = std::iter::once(lut_entry(s, ""))
        .chain(cfa4_render::luts::input_builtins().map(|b| json!({"ref": format!("builtin:{}", b.id), "name": b.label})))
        .chain(lib.iter().cloned())
        .collect();
    let look: Vec<Value> = std::iter::once(lut_entry(s, ""))
        .chain(cfa4_render::luts::builtins().iter().filter(|b| b.id.starts_with("look-")).map(|b| json!({"ref": format!("builtin:{}", b.id), "name": b.label})))
        .chain(lib)
        .collect();
    out["effect"] = json!(idx);
    out["enabled"] = json!(e.enabled);
    out["mediaTime"] = json!(at.0);
    out["start"] = json!(it.start.0);
    out["end"] = json!(it.end().0);
    out["params"] = Value::Object(params);
    out["luts"] = json!({"input": input, "look": look});
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_video_clip(s: &Session) -> u64 {
        let seq = s.active_sequence().expect("demo sequence");
        seq.video_tracks.iter().flat_map(|t| t.items.iter()).map(|it| it.id.0).next().expect("a video clip")
    }

    #[test]
    fn inspects_the_lumetri_color_panel() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        assert_eq!(inspect(&s, &json!({})).expect("inspect")["clip"], json!(null), "nothing selected");
        let clip = first_video_clip(&s);
        let r = inspect(&s, &json!({"clip": clip})).expect("inspect");
        assert_eq!(r["clip"], json!(clip));
        assert_eq!(r["effect"], json!(null), "no Lumetri yet: {r}");
        assert_eq!(r["hdr"], json!(false));
        s.execute("effects.apply", json!({"clips": [clip], "effect": "lumetri"})).expect("apply");
        let r = inspect(&s, &json!({"clip": clip})).expect("inspect");
        let idx = r["effect"].as_u64().expect("the Lumetri effect");
        let p = &r["params"];
        assert_eq!(p["saturation"]["value"], json!(100.0));
        assert_eq!(p["exposure"]["softMin"], json!(-5.0));
        assert_eq!(p["hdr_white"]["decimals"], json!(0));
        assert_eq!(p["look"]["options"][1], json!("Teal & Orange"));
        assert_eq!(p["hsl_on"]["value"], json!(false));
        assert_eq!(p["input_lut"]["name"], json!("None"));
        assert_eq!(p["wheel_midtones"]["value"], json!([0.0, 0.0]));
        let luma = p["curve_luma"]["line"].as_array().expect("luma line");
        assert_eq!(luma.len(), LINE_POINTS);
        assert!(luma.iter().all(|y| y.as_f64().is_some_and(|y| (0.0..=1.0).contains(&y))), "{r}");
        assert_eq!(p["hue_vs_sat"]["line"][17], json!(0.5), "a hue curve without points is flat");
        assert!(r["luts"]["input"].as_array().is_some_and(|l| l.len() > 1 && l[0]["ref"] == json!("")), "{r}");
        assert!(r["luts"]["look"].as_array().is_some_and(|l| l.iter().skip(1).all(|e| e["ref"].as_str().is_some_and(|r| r.starts_with("builtin:look-")))));
        // edits land where the panel reads them
        s.execute("effects.setParam", json!({"clip": clip, "effect": idx, "param": "curve_luma", "value": [[0.0, 0.0], [0.5, 0.7], [1.0, 1.0]]}))
            .expect("curve");
        s.execute("lumetri.setSection", json!({"clip": clip, "section": "hsl"})).expect("section");
        let r = inspect(&s, &json!({"clip": clip})).expect("inspect");
        assert_eq!(r["params"]["curve_luma"]["value"].as_array().map(Vec::len), Some(3));
        assert!(r["params"]["curve_luma"]["line"][64].as_f64().is_some_and(|y| y > 0.6), "{r}");
        assert_eq!(r["params"]["hsl_on"]["value"], json!(true));
        // selection: the first selected video clip
        s.execute("timeline.select", json!({"clips": [clip]})).expect("select");
        assert_eq!(inspect(&s, &json!({})).expect("inspect")["clip"], json!(clip));
    }
}
