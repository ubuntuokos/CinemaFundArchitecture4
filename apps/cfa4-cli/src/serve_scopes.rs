//! `scopes.image`: the Lumetri Scopes' trace images for the Qt GUI. The counting is
//! `cfa4-scopes` (as for the egui editor's scopes panel); the trace colours, the parade's side by
//! side layout and the histogram image follow `crates/ui-egui/src/panels/scopes.rs`, so both
//! editors show the same traces. The GUI draws the graticules and labels around them.

use cfa4_engine::Session;
use cfa4_scopes::{self as sc, Brightness, ColorSpace, ParadeType, Params, Scale, ScopeKind, Targets, WaveformType};
use serde_json::{Value, json};

const RED: [f32; 3] = [1.0, 0.16, 0.16];
const GREEN: [f32; 3] = [0.16, 1.0, 0.16];
const BLUE: [f32; 3] = [0.3, 0.38, 1.0];
const WHITE: [f32; 3] = [0.85, 0.85, 0.85];
const LUMA: [f32; 3] = [0.55, 1.0, 0.7];

fn trace_color(name: &str, wt: Option<WaveformType>) -> [f32; 3] {
    match (name, wt) {
        ("R", _) => RED,
        ("G", _) => GREEN,
        ("B", _) | ("Cb", _) | ("C", _) => BLUE,
        ("Cr", _) => [1.0, 0.45, 0.45],
        ("Y", Some(WaveformType::Luma)) => WHITE,
        ("Y", Some(_)) => LUMA,
        _ => WHITE,
    }
}

fn parade_image(w: &sc::Waveform, gain: f32) -> (usize, usize, Vec<u8>) {
    let n = w.traces.len();
    let Some(first) = w.traces.first() else { return (0, 0, Vec::new()) };
    let (cw, rows) = (first.cols, first.rows);
    let gap = (cw / 24).max(2);
    let width = cw * n + gap * (n - 1);
    let mut out = vec![0u8; width * rows * 4];
    for (i, g) in w.traces.iter().enumerate() {
        let (_, _, px) = sc::paint::grids(&[(g, trace_color(g.name, None))], sc::paint::waveform_reference(w.per_column), gain);
        let x0 = i * (cw + gap);
        for y in 0..rows {
            let src = &px[y * cw * 4..(y + 1) * cw * 4];
            out[(y * width + x0) * 4..(y * width + x0 + cw) * 4].copy_from_slice(src);
        }
    }
    (width, rows, out)
}

fn histogram_image(h: &sc::Histogram, gain: f32) -> (usize, usize, Vec<u8>) {
    let (w, rows) = (256usize, sc::BINS);
    let max = h.r.iter().chain(&h.g).chain(&h.b).copied().max().unwrap_or(0).max(1) as f32;
    let len = |c: u32| ((c as f32 / max).sqrt() * w as f32).round() as usize;
    let mut out = vec![0u8; w * rows * 4];
    for bin in 0..rows {
        let y = rows - 1 - bin;
        let (lr, lg, lb) = (len(h.r[bin]), len(h.g[bin]), len(h.b[bin]));
        for x in 0..w {
            let c = [if x < lr { RED } else { [0.0; 3] }, if x < lg { GREEN } else { [0.0; 3] }, if x < lb { BLUE } else { [0.0; 3] }];
            let s = [c[0][0] + c[1][0] + c[2][0], c[0][1] + c[1][1] + c[2][1], c[0][2] + c[1][2] + c[2][2]].map(|v| (v * 0.75 * gain).min(1.0));
            let a = s[0].max(s[1]).max(s[2]);
            if a > 0.0 {
                let o = &mut out[(y * w + x) * 4..(y * w + x) * 4 + 4];
                o.copy_from_slice(&[(s[0] * 255.0) as u8, (s[1] * 255.0) as u8, (s[2] * 255.0) as u8, (a * 255.0) as u8]);
            }
        }
    }
    (w, rows, out)
}

fn pick<T: Copy>(p: &Value, key: &str, from: impl Fn(&str) -> Option<T>, default: T) -> Result<T, String> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_str().and_then(&from).ok_or_else(|| format!("bad `{key}`")),
    }
}

/// `scopes.image {kind, time?, waveformType?, paradeType?, colorSpace?, brightness?, scale?,
/// clamp?, rows?}`: one scope of the active sequence's frame at `time` (the playhead) as a
/// premultiplied RGBA8 trace image (`w × h`), with what the GUI needs for the graticule: the
/// plotted level range (`lo`, `hi`), whether levels are cd/m² (`nits`), the colour space and,
/// for the vectorscopes, the extent, the skin-tone angle and the colour targets.
pub fn image(s: &Session, p: &Value, time: cfa4_time::Tick) -> Result<(Value, Vec<u8>), String> {
    let kind = p
        .get("kind")
        .and_then(Value::as_str)
        .and_then(ScopeKind::from_name)
        .ok_or("`kind` must be a scope (waveform, parade, histogram, vectorscopeYuv, vectorscopeHls)")?;
    let wt = pick(p, "waveformType", WaveformType::from_name, WaveformType::Rgb)?;
    let pt = pick(p, "paradeType", ParadeType::from_name, ParadeType::Rgb)?;
    let space = pick(p, "colorSpace", ColorSpace::from_name, ColorSpace::Auto)?;
    let brightness = pick(
        p,
        "brightness",
        |n| Brightness::ALL.into_iter().find(|b| serde_json::to_value(b).ok().and_then(|v| v.as_str().map(|x| x == n)).unwrap_or(false)),
        Brightness::Normal,
    )?;
    let scale = pick(p, "scale", Scale::from_name, Scale::Bits8)?;
    let clamp = p.get("clamp").and_then(Value::as_bool).unwrap_or(true);
    let rows = p.get("rows").and_then(Value::as_u64).map_or(256, |r| r.clamp(64, 256) as usize);
    let seq = s.active_sequence().ok_or("no active sequence")?;
    let hdr_seq = seq.settings.color.working.is_hdr();
    let space = space.resolve(hdr_seq);
    let pq = hdr_seq && space == ColorSpace::Rec2100;
    let signal = cfa4_engine::scopes::scope_signal(s, time, if pq { 0.125 } else { 0.25 }, pq).ok_or("no frame")?;
    let nits = pq || scale == Scale::Hdr;
    let plotted = if scale == Scale::Hdr && !pq { signal.map(|c| c.map(sc::sdr_to_pq)) } else { signal.clone() };
    let params = Params { matrix: space.matrix(), clamp, rows, ..Params::default() };
    let gain = brightness.gain();
    let (lo, hi) = match kind {
        ScopeKind::Histogram => (0.0, 1.0),
        _ => params.range(),
    };
    let (w, h, px) = match kind {
        ScopeKind::Waveform => {
            let wf = sc::waveform(&plotted, wt, &params);
            let layers: Vec<(&sc::Grid, [f32; 3])> = wf.traces.iter().map(|g| (g, trace_color(g.name, Some(wt)))).collect();
            sc::paint::grids(&layers, sc::paint::waveform_reference(wf.per_column), gain)
        }
        ScopeKind::Parade => parade_image(&sc::parade(&plotted, pt, &params), gain),
        ScopeKind::Histogram => histogram_image(&sc::histogram(&plotted, &params), gain),
        ScopeKind::VectorscopeYuv => sc::paint::vectorscope(&sc::vectorscope_yuv(&signal, &params), WHITE, gain, true),
        ScopeKind::VectorscopeHls => sc::paint::vectorscope(&sc::vectorscope_hls(&signal, &params), WHITE, gain, true),
    };
    let m = space.matrix();
    let mut meta = json!({
        "kind": kind.name(), "w": w, "h": h, "format": "rgba8premultiplied",
        "lo": lo, "hi": hi, "nits": nits, "pq203": sc::nits_to_pq(203.0),
        "colorSpace": space.label(), "parades": match pt { ParadeType::RgbWhite => 4, _ => 3 },
    });
    if nits {
        meta["nitsTicks"] = json!([0.0, 10.0, 100.0, 203.0, 1000.0, 4000.0, 10_000.0].map(|n: f32| json!([n, sc::nits_to_pq(n)])));
    }
    if matches!(kind, ScopeKind::VectorscopeYuv | ScopeKind::VectorscopeHls) {
        meta["extent"] = json!(sc::VECTOR_EXTENT);
        meta["skinToneDeg"] = json!(sc::SKIN_TONE_DEG);
        meta["targets"] = json!([Targets::Percent75, Targets::Percent100].map(|t| {
            let a = t.amplitude();
            json!({"amplitude": a, "points": sc::targets(m, a).iter().map(|tg| json!({"name": tg.name, "cb": tg.cb, "cr": tg.cr, "rgb": tg.rgb})).collect::<Vec<_>>()})
        }));
        meta["hls"] = json!(sc::hls_targets(m).iter().map(|(n, cb, cr)| json!({"name": n, "cb": cb, "cr": cr})).collect::<Vec<_>>());
    }
    Ok((meta, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scope_renders_a_trace_image() {
        let mut s = Session::default();
        let _ = s.execute("file.openDemoProject", json!({}));
        let t = cfa4_time::Tick::from_seconds_f64(10.0);
        for kind in ["waveform", "parade", "histogram", "vectorscopeYuv", "vectorscopeHls"] {
            let (meta, px) = image(&s, &json!({"kind": kind, "rows": 128}), t).unwrap_or_else(|e| panic!("{kind}: {e}"));
            let (w, h) = (meta["w"].as_u64().unwrap_or(0) as usize, meta["h"].as_u64().unwrap_or(0) as usize);
            assert!(w > 0 && h > 0, "{kind}: {meta}");
            assert_eq!(px.len(), w * h * 4, "{kind}");
            assert!(px.chunks(4).any(|c| c[3] > 0), "{kind}: a trace is drawn");
            if kind.starts_with("vectorscope") {
                assert_eq!(meta["targets"][0]["points"].as_array().map(Vec::len), Some(6), "{kind}");
            }
        }
        let (meta, _) = image(&s, &json!({"kind": "waveform", "clamp": false}), t).expect("waveform");
        assert!(meta["lo"].as_f64().is_some_and(|lo| lo < 0.0), "an unclamped waveform plots below 0: {meta}");
        assert!(image(&s, &json!({"kind": "nope"}), t).is_err());
    }
}
