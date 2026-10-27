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

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "effects.eyedropper",
        label: "Eyedropper",
        menu: &[],
        shortcut: None,
        params: r#"{"clip":id,"effect":index,"param":id,"mask":n?,"u":0..1,"v":0..1 (where in the Program picture, across and down)}"#,
        enabled: has_seq,
        run: pick,
        journal: false,
    }]
}

fn pick(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effects.eyedropper";
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
