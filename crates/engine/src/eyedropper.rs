//! `effects.eyedropper`: fill a colour parameter with the colour under a point of the Program
//! picture (Effect Controls' eyedropper, egui's `monitor.rs` `eyedropper`). The frame at the
//! playhead is rendered with the armed effect switched off, so an Ultra Key can pick the green it
//! is about to remove; the 5×5 pixels around the point are averaged (a noisy green screen gives
//! its true green) and the parameter gets that colour, opaque, through `effects.setParam` (one
//! undo step; that is what the journal records, so a replay does not depend on the media).

use cfa4_project::{ClipId, ParamKind};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, clip_p, f64_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// Output scale of the sampled frame (half size is plenty and quick to render).
const SCALE: f32 = 0.5;

/// Lumetri's white balance gains in linear light: red `1 + K_TEMP·t`, green `1 − K_TINT·n`, blue
/// `1 − K_TEMP·t` for Temperature `100·t` and Tint `100·n` (`crates/render/src/effects.rs`, `lumetri`).
const K_TEMP: f32 = 0.35;
const K_TINT: f32 = 0.3;
/// Below this linear level (or channel sum) a point has no usable colour (black, or a clipped hole).
const MIN_LEVEL: f32 = 0.002;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "effects.eyedropper",
            label: "Eyedropper",
            menu: &[],
            shortcut: None,
            params: r#"{"clip":id,"effect":index,"param":id,"mask":n?,"u":0..1,"v":0..1 (where in the Program picture, across and down),"mode":"whiteBalance"? (the Lumetri White Balance Selector: same as effects.pickWhiteBalance, `param` not needed)}"#,
            enabled: has_seq,
            run: pick,
            journal: false,
        },
        CommandSpec {
            id: "effects.pickWhiteBalance",
            label: "White Balance Selector",
            menu: &[],
            shortcut: None,
            params: r#"{"clip":id,"effect":index (a Lumetri Color effect),"u":0..1,"v":0..1 (the neutral point in the Program picture, across and down)}; sets Temperature and Tint so that point is neutral (one undo step) and returns {"temperature","tint","clamped"}"#,
            enabled: has_seq,
            run: pick_white_balance,
            journal: false,
        },
    ]
}

/// Temperature and Tint (sliders, −100…100) that make the linear colour `rgb` neutral, and whether a
/// slider hit its end. `None` for a colour that is too dark or not finite (nothing to balance on).
pub(crate) fn solve_white_balance(rgb: [f32; 3]) -> Option<(f32, f32, bool)> {
    let [r, g, b] = rgb;
    if !rgb.iter().all(|c| c.is_finite()) || rgb.iter().any(|c| *c < 0.0) || r + g + b < 3.0 * MIN_LEVEL || g < MIN_LEVEL || r + b < MIN_LEVEL {
        return None;
    }
    let t = (b - r) / (K_TEMP * (r + b));
    let tc = t.clamp(-1.0, 1.0);
    // the level red and blue meet at; green is brought to it with Tint
    let level = (r * (1.0 + K_TEMP * tc) + b * (1.0 - K_TEMP * tc)) * 0.5;
    let n = (1.0 - level / g) / K_TINT;
    let nc = n.clamp(-1.0, 1.0);
    let clamped = (t - tc).abs() > 1e-6 || (n - nc).abs() > 1e-6;
    Some((tc * 100.0, nc * 100.0, clamped))
}

fn pick_white_balance(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effects.pickWhiteBalance";
    let clip = clip_p(p, "clip").ok_or_else(|| bad(CMD, "need `clip`"))?;
    let index = p.get("effect").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(CMD, "need `effect` (an index)"))?;
    let uv = |k: &str| f64_p(p, k).filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0)).ok_or_else(|| bad(CMD, format!("need `{k}` in 0..1")));
    let (u, v) = (uv("u")?, uv("v")?);
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(CMD, "no such clip in the active sequence"))?;
    let e = it.effects.get(index).ok_or_else(|| bad(CMD, "no such effect on the clip"))?;
    let has = |id: &str| e.def().is_some_and(|d| d.params.iter().any(|x| x.id == id && matches!(x.kind, ParamKind::Float { .. })));
    if e.effect != "lumetri" || !has("temperature") || !has("tint") {
        return Err(bad(CMD, "that effect is not a Lumetri Color effect"));
    }
    // the frame without this Lumetri, so a white balance already set does not skew the pick
    let rgb = sample(s, seq_id, clip, index, u, v).ok_or_else(|| bad(CMD, "no picture to sample"))?;
    let (temp, tint, clamped) = solve_white_balance(rgb).ok_or_else(|| bad(CMD, "that point is too dark to balance on; pick a neutral grey or white area"))?;
    let set = |param: &str, value: f32, begin: bool| {
        json!({"clip": clip.0, "effect": index, "param": param, "value": value, "merge": "whiteBalance", "begin": begin})
    };
    // one undo step for both sliders: the second setParam merges into the first
    s.execute("effects.setParam", set("temperature", temp, true))?;
    s.execute("effects.setParam", set("tint", tint, false))?;
    Ok(json!({"temperature": temp, "tint": tint, "clamped": clamped}))
}

fn pick(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effects.eyedropper";
    if str_p(p, "mode") == Some("whiteBalance") {
        return pick_white_balance(s, p);
    }
    let clip = clip_p(p, "clip").ok_or_else(|| bad(CMD, "need `clip`"))?;
    let index = p.get("effect").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(CMD, "need `effect` (an index)"))?;
    let pid = str_p(p, "param").ok_or_else(|| bad(CMD, "need `param`"))?;
    let mask = match p.get("mask") {
        None | Some(Value::Null) => None,
        Some(m) => Some(m.as_u64().and_then(|m| usize::try_from(m).ok()).ok_or_else(|| bad(CMD, "`mask` is an index"))?),
    };
    let uv = |k: &str| f64_p(p, k).filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0)).ok_or_else(|| bad(CMD, format!("need `{k}` in 0..1")));
    let (u, v) = (uv("u")?, uv("v")?);
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = q.find_item(clip).ok_or_else(|| bad(CMD, "no such clip in the active sequence"))?;
    let e = it.effects.get(index).ok_or_else(|| bad(CMD, "no such effect on the clip"))?;
    let defs = match mask {
        Some(_) => cfa4_project::mask::mask_param_defs(),
        None => e.def().map(|d| d.params.as_slice()).unwrap_or_default(),
    };
    if !defs.iter().any(|d| d.id == pid && matches!(d.kind, ParamKind::Color)) {
        return Err(bad(CMD, format!("`{pid}` is not a colour parameter of that effect")));
    }
    let rgb = sample(s, seq_id, clip, index, u, v).ok_or_else(|| bad(CMD, "no picture to sample"))?;
    let mut set = json!({"clip": clip.0, "effect": index, "param": pid, "value": [rgb[0], rgb[1], rgb[2], 1.0]});
    if let Some(m) = mask {
        set["mask"] = json!(m);
    }
    s.execute("effects.setParam", set)?;
    Ok(json!({"color": rgb}))
}

/// The linear colour at (`u`, `v`) of the frame at the playhead with effect `index` of `clip` off.
fn sample(s: &Session, seq_id: cfa4_project::ItemId, clip: ClipId, index: usize, u: f64, v: f64) -> Option<[f32; 3]> {
    s.renderable_sequence(SCALE).ok()?;
    let mut project = (*s.project).clone();
    let (_, it) = project.sequence_mut(seq_id)?.find_item_mut(clip)?;
    it.effects.get_mut(index)?.enabled = false;
    let project = std::sync::Arc::new(project);
    let provider = s.media.provider(project.clone(), s.services.clone());
    let t = s.sequence_rate().snap(s.playhead().max(cfa4_time::Tick::ZERO));
    let opts = cfa4_render::RenderOptions { scale: SCALE, captions: true, ..Default::default() };
    let img = cfa4_render::render_sequence(&project, seq_id, t, opts, &provider);
    average(&img, u, v)
}

/// The average of the 5×5 pixels around (`u`, `v`), in sRGB steps as a display shows them
/// (egui's), back in linear light. `None` for an empty frame.
fn average(img: &cfa4_render::Image, u: f64, v: f64) -> Option<[f32; 3]> {
    if img.w == 0 || img.h == 0 || img.px.len() < img.w.checked_mul(img.h)?.checked_mul(4)? {
        return None;
    }
    let cx = ((u * img.w as f64) as usize).min(img.w - 1);
    let cy = ((v * img.h as f64) as usize).min(img.h - 1);
    let (mut sum, mut n) = ([0u32; 3], 0u32);
    for y in cy.saturating_sub(2)..=(cy + 2).min(img.h - 1) {
        for x in cx.saturating_sub(2)..=(cx + 2).min(img.w - 1) {
            let i = (y * img.w + x) * 4;
            for (c, total) in sum.iter_mut().enumerate() {
                *total += u32::from(cfa4_color::linear_to_srgb_u8(*img.px.get(i + c)?));
            }
            n += 1;
        }
    }
    let table = cfa4_color::srgb_u8_to_linear_table();
    let avg = |c: usize| -> Option<f32> { table.get(usize::try_from(sum[c].checked_div(n)?).ok()?).copied() };
    Some([avg(0)?, avg(1)?, avg(2)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(rgb: [f32; 3], temp: f32, tint: f32) -> [f32; 3] {
        let (t, n) = (temp / 100.0, tint / 100.0);
        [rgb[0] * (1.0 + K_TEMP * t), rgb[1] * (1.0 - K_TINT * n), rgb[2] * (1.0 - K_TEMP * t)]
    }

    #[test]
    fn a_neutral_grey_needs_no_correction() {
        for g in [0.05_f32, 0.18, 0.5, 1.0] {
            let (t, n, clamped) = solve_white_balance([g, g, g]).unwrap();
            assert!(t.abs() < 1e-3 && n.abs() < 1e-3 && !clamped, "{g}: {t} {n}");
        }
    }

    #[test]
    fn a_cast_is_corrected_to_neutral_within_tolerance() {
        for cast in [[0.30_f32, 0.25, 0.20], [0.20, 0.26, 0.30], [0.22, 0.30, 0.22], [0.27, 0.22, 0.27]] {
            let (t, n, clamped) = solve_white_balance(cast).unwrap();
            assert!(!clamped, "{cast:?}");
            let out = apply(cast, t, n);
            let m = (out[0] + out[1] + out[2]) / 3.0;
            assert!(out.iter().all(|c| (c - m).abs() < 1e-4), "{cast:?} -> {out:?}");
        }
        // warm cast (red > blue) is cooled: negative Temperature
        assert!(solve_white_balance([0.4, 0.3, 0.2]).is_some_and(|(t, _, _)| t < 0.0));
        assert!(solve_white_balance([0.2, 0.3, 0.4]).is_some_and(|(t, _, _)| t > 0.0));
    }

    #[test]
    fn extreme_and_hostile_colours_are_clamped_or_refused() {
        let (t, _, clamped) = solve_white_balance([0.9, 0.3, 0.003]).unwrap();
        assert!(clamped && (-100.0..=100.0).contains(&t));
        for bad in [[0.0; 3], [f32::NAN, 0.5, 0.5], [f32::INFINITY, 0.5, 0.5], [-0.1, 0.5, 0.5], [0.5, 0.0, 0.5], [0.0, 0.5, 0.0]] {
            assert!(solve_white_balance(bad).is_none(), "{bad:?}");
        }
    }
}
