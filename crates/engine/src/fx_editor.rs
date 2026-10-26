//! `audio.fxEditor`: what the Clip / Track Fx Editor windows draw (egui's
//! `panels/audio_fx_editor.rs`): the edited effect's parameters and the curves the real DSP gives
//! for them (`render::audio_fx::configured` → `response_db` / `transfer_db`), so a front end
//! draws exactly what is heard. `overrides` previews uncommitted values (a live drag) without
//! touching the project; edits go through `effects.setParam` / `mixer.setInsert` as usual.

use cfa4_project::{EffectInstance, ParamKind, ParamValue, TrackId};
use cfa4_time::Tick;
use serde_json::{Map, Value, json};

use crate::commands::{CommandSpec, always, bad, clip_p, u64_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "audio.fxEditor";
const F_MIN: f64 = 20.0;
const F_MAX: f64 = 20000.0;
/// Points on the frequency response (log-spaced 20 Hz … 20 kHz) and on a transfer curve.
const RESPONSE_POINTS: usize = 240;
const TRANSFER_POINTS: usize = 120;
/// Transfer curves run over −60 … 0 dB in.
const TRANSFER_LO: f64 = -60.0;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Fx Editor Curves",
        menu: &[],
        shortcut: None,
        params: r#"{"clip":id,"effect":index | "strip":id|"A1"|"Mix","slot":index, "overrides":{param:number}? (uncommitted values to preview)}"#,
        enabled: always,
        run: inspect,
        journal: false,
    }]
}

/// Effects that have a graphical editor and which one.
pub fn editor_of(effect: &str) -> Option<&'static str> {
    Some(match effect {
        "parametric_eq" => "parametric",
        "graphic_eq" | "graphic_eq_20" | "graphic_eq_30" => "graphic",
        "multiband_compressor" => "multiband",
        "dynamics_rack" => "dynamics",
        _ => return None,
    })
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    // the rate the programme is rendered at: filter designs (and so the curves) depend on it
    let rate = q.settings.sample_rate.clamp(8_000, 384_000);
    let ph = s.playhead();
    let (inst, mt, kind, track, clip_name) = if p.get("clip").is_some() {
        let clip = clip_p(p, "clip").ok_or_else(|| bad(CMD, "`clip` is an id"))?;
        let index = u64_p(p, "effect").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(CMD, "need `effect` (an index)"))?;
        let (tid, it) = q.find_item(clip).ok_or_else(|| bad(CMD, "no such clip in the active sequence"))?;
        let e = it.effects.get(index).ok_or_else(|| bad(CMD, "no such effect on the clip"))?.clone();
        let track = q.track(tid).map(|t| t.name.clone()).unwrap_or_default();
        // as the audio render evaluates clip effects
        let last = Tick(it.end().0.saturating_sub(1)).max(it.start);
        (e, it.source_time_at(ph.clamp(it.start, last)), "clip", track, Some(it.name.clone()))
    } else {
        let v = p.get("strip").ok_or_else(|| bad(CMD, "need `clip`+`effect` or `strip`+`slot`"))?;
        let id: TrackId = crate::mixer::strip_ref(q, v).ok_or_else(|| bad(CMD, format!("no strip {v}")))?;
        let slot = u64_p(p, "slot").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(CMD, "need `slot` (0-based)"))?;
        let tr = q.strip(id).ok_or_else(|| bad(CMD, "no such strip"))?;
        let e = tr.effects.get(slot).ok_or_else(|| bad(CMD, "no effect in that slot"))?.clone();
        (e, ph, "insert", tr.name.clone(), None)
    };
    let editor = editor_of(&inst.effect).ok_or_else(|| bad(CMD, format!("`{}` has no graphical editor", inst.effect)))?;
    let def = inst.def().ok_or_else(|| bad(CMD, "unknown effect"))?;
    let preview = with_overrides(&inst, p.get("overrides"));
    let mut params = Map::new();
    for d in &def.params {
        let (min, max, unit) = match &d.kind {
            ParamKind::Float { min, max, unit, .. } => (*min, *max, *unit),
            _ => (0.0, 1.0, ""),
        };
        let v = preview.f64_at(d.id, mt);
        let v = if v.is_finite() { v } else { 0.0 };
        params.insert(d.id.to_string(), json!({"value": v, "min": min, "max": max, "unit": unit, "bool": matches!(d.kind, ParamKind::Bool), "label": d.label}));
    }
    let dsp = cfa4_render::audio_fx::configured(&preview, mt, rate);
    let response: Vec<Value> = dsp
        .as_ref()
        .map(|d| {
            (0..=RESPONSE_POINTS)
                .filter_map(|i| {
                    let f = F_MIN * (F_MAX / F_MIN).powf(i as f64 / RESPONSE_POINTS as f64);
                    d.response_db(f).filter(|db| db.is_finite()).map(|db| json!([f, db]))
                })
                .collect()
        })
        .unwrap_or_default();
    let bands = match editor {
        "multiband" => 4,
        "dynamics" => 1,
        _ => 0,
    };
    let transfer: Vec<Value> = (0..bands)
        .map(|band| {
            let pts: Vec<Value> = dsp
                .as_ref()
                .map(|d| {
                    (0..=TRANSFER_POINTS)
                        .filter_map(|i| {
                            let x = TRANSFER_LO - TRANSFER_LO * i as f64 / TRANSFER_POINTS as f64;
                            d.transfer_db(band, x as f32).filter(|y| y.is_finite()).map(|y| json!([x, y]))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Value::Array(pts)
        })
        .collect();
    let labels: &[&str] = match inst.effect.as_str() {
        "graphic_eq" => &cfa4_project::effect::GEQ10_LABELS,
        "graphic_eq_20" => &cfa4_project::effect::GEQ20_LABELS,
        "graphic_eq_30" => &cfa4_project::effect::GEQ30_LABELS,
        _ => &[],
    };
    Ok(json!({
        "sampleRate": rate,
        "effect": inst.effect,
        "name": def.name,
        "editor": editor,
        "kind": kind,
        "track": track,
        "clip": clip_name,
        "params": params,
        "response": response,
        "transfer": transfer,
        "labels": labels,
    }))
}

/// `inst` with the numbers in `overrides` (by parameter id) as static values; unknown ids and
/// non-finite numbers are ignored, values are kept inside the parameter's range.
fn with_overrides(inst: &EffectInstance, overrides: Option<&Value>) -> EffectInstance {
    let mut e = inst.clone();
    let Some(obj) = overrides.and_then(Value::as_object) else { return e };
    for (k, v) in obj {
        let Some(d) = inst.def().and_then(|d| d.param(k)) else { continue };
        let Some(x) = v.as_f64().or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 })).filter(|x| x.is_finite()) else { continue };
        let x = match &d.kind {
            ParamKind::Float { min, max, .. } if min <= max => x.clamp(*min, *max),
            _ => x,
        };
        let p = e.params.entry(k.clone()).or_insert_with(|| cfa4_project::Param::new(d.default.clone()));
        p.keyframes.clear();
        p.value = match p.value {
            ParamValue::Bool(_) => ParamValue::Bool(x >= 0.5),
            ParamValue::Choice(_) => ParamValue::Choice(x.clamp(0.0, f64::from(u32::MAX)) as u32),
            _ => ParamValue::Float(x),
        };
    }
    e
}
