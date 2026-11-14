//! A harmonised body width (`harmonizeBody`): grid columns of one style share a width per page.

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::layout::{Frame, Measure};
use crate::{Styling, layout, read};

/// 0.5 em per character, 1.2 em lines; point text aligned by `align`.
struct M;
impl Measure for M {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = match e.params.get("size").map(|p| &p.value) {
            Some(ParamValue::Float(v)) => *v,
            _ => 40.0,
        };
        let t = match e.params.get("text").map(|p| &p.value) {
            Some(ParamValue::Text(t)) => t.clone(),
            _ => String::new(),
        };
        let w = t.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * size * 0.5;
        let x0 = match e.params.get("align").map(|p| &p.value) {
            Some(ParamValue::Choice(0)) => 0.0,
            Some(ParamValue::Choice(2)) => -w,
            _ => -w / 2.0,
        };
        Some([x0, -size * 0.9, x0 + w, -size * 0.9 + t.lines().count().max(1) as f64 * size * 1.2])
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

fn param_text(e: &EffectInstance, id: &str) -> String {
    match e.params.get(id).map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

fn x_of(layers: &[EffectInstance], text: &str) -> f64 {
    let e = layers.iter().find(|e| param_text(e, "text") == text).unwrap_or_else(|| panic!("no layer `{text}`"));
    match e.params.get("position").map(|p| &p.value) {
        Some(ParamValue::Vec2(v)) => v.x,
        _ => f64::NAN,
    }
}

fn styling(harmonise: bool) -> Styling {
    Styling::from_json(&format!(
        r#"{{"contentStyles":[
            {{"name":"Roles","orientation":"horizontal","headGap":40,"tailGap":30,"harmonizeBody":{harmonise}}},
            {{"name":"Grid","orientation":"vertical","columns":2,"columnGap":20,"harmonizeBody":{harmonise}}}
        ]}}"#
    ))
    .unwrap()
}

#[test]
fn harmonised_bodies_line_up_their_tails() {
    let sheet = "Head,Body,Tail,Content Style\nDirector,Anna,Tail A,Roles\n,,,\nCamera,Bartholomew Longname,Tail B,\n";
    let st = styling(true);
    let c = read(sheet.as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    let layers = &l.pages[0].layers;
    // the tails stand after the widest body: 20 characters × 20 px + the gaps
    assert!((x_of(layers, "Tail A") - x_of(layers, "Tail B")).abs() < 1e-6);
    assert!((x_of(layers, "Tail B") - (960.0 + 20.0 + 20.0 * 20.0 + 30.0)).abs() < 1e-6, "{}", x_of(layers, "Tail B"));
    // the bodies themselves stay where they were
    assert!((x_of(layers, "Anna") - x_of(layers, "Bartholomew Longname")).abs() < 1e-6);
    // without harmonisation each tail follows its own body
    let st = styling(false);
    let l = layout(&read(sheet.as_bytes(), &st).unwrap(), &st, FRAME, &M).unwrap();
    let layers = &l.pages[0].layers;
    assert!(x_of(layers, "Tail A") < x_of(layers, "Tail B"));
}

#[test]
fn harmonised_grids_share_their_column_width_per_page() {
    let sheet = "Head,Body,Content Style,Page Style\nCrew,A,Grid,Scroll\n,B,,\n,,,\nMore,Long name one,,\n,Long name two,,\nLast,C,,Scroll\n,D,,\n";
    let st = styling(true);
    let l = layout(&read(sheet.as_bytes(), &st).unwrap(), &st, FRAME, &M).unwrap();
    let layers = &l.pages[0].layers;
    let cols = |a: &str, b: &str| (x_of(layers, a), x_of(layers, b));
    // the short grid's columns stand where the long one's do
    let ((a1, a2), (l1, l2)) = (cols("A", "B"), cols("Long name one", "Long name two"));
    assert!((a1 - l1).abs() < 1e-6 && (a2 - l2).abs() < 1e-6, "{a1} {a2} / {l1} {l2}");
    // a column is 13 characters (260 px) wide plus the 20 px gap
    assert!((a2 - a1 - 280.0).abs() < 1e-6, "{}", a2 - a1);
    // the next page is harmonised by itself
    let p2 = &l.pages[1].layers;
    assert!(x_of(p2, "D") - x_of(p2, "C") < 280.0 - 1.0);
    // a style that does not ask for it keeps its own widths
    let st = styling(false);
    let l = layout(&read(sheet.as_bytes(), &st).unwrap(), &st, FRAME, &M).unwrap();
    let layers = &l.pages[0].layers;
    assert!(x_of(layers, "B") - x_of(layers, "A") < 280.0 - 1.0);
}

#[test]
fn hostile_harmonised_grids_do_not_crash() {
    for extra in [r#","columns":100000,"columnGap":1e308"#, r#","columns":0,"columnGap":-1e308"#, r#","layout":"flow""#, r#","layout":"paragraphs""#] {
        let st = Styling::from_json(&format!(
            r#"{{"contentStyles":[{{"name":"Roles","orientation":"horizontal","harmonizeBody":true}},{{"name":"Grid","harmonizeBody":true{extra}}}]}}"#
        ))
        .unwrap();
        let sheet = "Body,Content Style\nA,Grid\nBB,\n{{Pic x.png}},\nCCC,Roles\n";
        let c = read(sheet.as_bytes(), &st).unwrap();
        let l = layout(&c, &st, FRAME, &M).unwrap();
        for e in &l.pages[0].layers {
            if let Some(ParamValue::Vec2(v)) = e.params.get("position").map(|p| &p.value) {
                assert!(v.x.is_finite() && v.y.is_finite(), "{extra}");
            }
        }
    }
}
