//! F10-C16 (Q18): voices in the credits – the `Voice` column, its column in the layout, the
//! spelling check across cast and voices, and voices in lower thirds.

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use super::*;
use crate::layout::{Frame, Measure};
use crate::validate::{FontCheck, FontInfo, validate};

/// 0.5 em per character, 1.2 em lines, left-aligned origin.
struct M;
impl Measure for M {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = f(e, "size");
        let t = text(e);
        let w = t.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * size * 0.5;
        let x0 = match e.params.get("align").map(|p| &p.value) {
            Some(ParamValue::Choice(1)) => -w / 2.0,
            Some(ParamValue::Choice(2)) => -w,
            _ => 0.0,
        };
        Some([x0, -size * 0.9, x0 + w, -size * 0.9 + t.lines().count().max(1) as f64 * size * 1.2])
    }
}

struct AllFonts;
impl FontInfo for AllFonts {
    fn check(&self, _family: &str, _style: &str, _text: &str) -> FontCheck {
        FontCheck::default()
    }
}

fn f(e: &EffectInstance, k: &str) -> f64 {
    match e.params.get(k).map(|p| &p.value) {
        Some(ParamValue::Float(v)) => *v,
        _ => 0.0,
    }
}
fn text(e: &EffectInstance) -> String {
    match e.params.get("text").map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}
fn pos(e: &EffectInstance) -> (f64, f64) {
    match e.params.get("position").map(|p| &p.value) {
        Some(ParamValue::Vec2(v)) => (v.x, v.y),
        _ => (f64::NAN, f64::NAN),
    }
}
fn layer<'a>(p: &'a PageOut, t: &str) -> &'a EffectInstance {
    p.layers.iter().find(|e| text(e) == t).unwrap_or_else(|| panic!("no layer `{t}`: {:?}", p.layers.iter().map(text).collect::<Vec<_>>()))
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

#[test]
fn the_voice_column_is_read_in_any_language() {
    for header in ["Voice", "Hang", "Szinkronhang", "Stimme", "Voix", "Голос", "声優", "더빙"] {
        let csv = format!("Head,Body,{header}\nThe Detective,First Actor,First Voice\n,Second Actor,Second Voice\n,,Extra Voice\n");
        let c = read(csv.as_bytes(), &Styling::default()).unwrap();
        let b = &c.pages[0].blocks[0];
        assert_eq!(b.body, vec!["First Actor", "Second Actor"], "{header}");
        // a voice stands beside the body cell of its row; a voice alone joins the last one
        assert_eq!(b.voices, vec![(0, "First Voice".to_string()), (1, "Second Voice".to_string()), (1, "Extra Voice".to_string())], "{header}");
        assert!(c.warnings.is_empty(), "{header}: {:?}", c.warnings);
    }
}

#[test]
fn voices_stand_in_a_column_of_their_own_with_their_label() {
    let mut styling = Styling::default();
    let mut cs = styling.content_style("Voices").unwrap();
    cs.voice_label = "hangja: ".into();
    styling.content_styles.push(cs);
    let csv = "Head,Body,Hang,Content Style\nThe Detective,First Actor,First Voice,Voices\nThe Friend,Second Actor,Second Voice,\n";
    let c = read(csv.as_bytes(), &styling).unwrap();
    let laid = layout(&c, &styling, FRAME, &M).unwrap();
    let p = &laid.pages[0];
    let actor = layer(p, "First Actor");
    let voice = layer(p, "hangja: First Voice");
    // right of the actor, on the actor's baseline, in the voice letter style
    assert!(pos(voice).0 > pos(actor).0 + 11.0 * 40.0 * 0.5, "{:?} {:?}", pos(voice), pos(actor));
    assert!((pos(voice).1 - pos(actor).1).abs() < 1e-9);
    assert!(matches!(voice.params.get("font_style").map(|p| &p.value), Some(ParamValue::Text(s)) if s == "Italic"));
    // the second row's voice beside the second actor
    let actor2 = layer(p, "Second Actor");
    let voice2 = layer(p, "hangja: Second Voice");
    assert!((pos(voice2).1 - pos(actor2).1).abs() < 1e-9);
    // the voice column of both rows starts at the same x (each block measures its own)
    assert!(pos(voice2).0 > pos(actor2).0);
    assert!(laid.warnings.is_empty(), "{:?}", laid.warnings);
}

#[test]
fn vertical_blocks_put_the_voices_under_the_names() {
    let csv = "Head,Body,Voice,Content Style\nThe Narrator,Third Actor,Third Voice,Centered\n";
    let laid = layout(&read(csv.as_bytes(), &Styling::default()).unwrap(), &Styling::default(), FRAME, &M).unwrap();
    let p = &laid.pages[0];
    let (actor, voice) = (layer(p, "Third Actor"), layer(p, "Third Voice"));
    assert!((pos(voice).0 - pos(actor).0).abs() < 1e-9);
    assert!(pos(voice).1 > pos(actor).1 + 40.0, "{:?} {:?}", pos(voice), pos(actor));
}

#[test]
fn a_voice_changes_the_block_key() {
    let a = read(b"Head,Body,Voice\nX,Ann,Bea\n", &Styling::default()).unwrap();
    let b = read(b"Head,Body,Voice\nX,Ann,Bia\n", &Styling::default()).unwrap();
    assert_ne!(crate::layout::block_key(&a.pages[0].blocks[0]), crate::layout::block_key(&b.pages[0].blocks[0]));
}

#[test]
fn one_person_in_cast_and_voices_written_two_ways_is_noticed() {
    let csv = "Head,Body,Voice\nThe Detective,Kovács Anna,Szabó Béla\nThe Friend,Szabo Bela,\n";
    let styling = Styling::default();
    let c = read(csv.as_bytes(), &styling).unwrap();
    let laid = layout(&c, &styling, FRAME, &M).unwrap();
    let issues = validate(&c, &laid, FRAME, &M, &AllFonts);
    let spelling: Vec<_> = issues.iter().filter(|i| i.code == "name-spelling").collect();
    assert_eq!(spelling.len(), 1, "{issues:?}");
    assert_eq!(spelling[0].row, Some(3));
    // the same voice in two blocks is not a duplicate
    let csv = "Head,Body,Voice\nA,Ann,Szabó Béla\nB,Bob,Szabó Béla\n";
    let c = read(csv.as_bytes(), &styling).unwrap();
    let laid = layout(&c, &styling, FRAME, &M).unwrap();
    assert!(validate(&c, &laid, FRAME, &M, &AllFonts).iter().all(|i| i.code != "name-spelling" && i.code != "duplicate-name"));
}

#[test]
fn a_lower_third_names_the_voice_with_the_role() {
    let mut styling = Styling::default();
    let mut cs = styling.content_style("Gutter").unwrap();
    cs.voice_label = "hangja: ".into();
    styling.content_styles.push(cs);
    let csv = "Head,Body,Voice,Page Style\nDetective,Anna Kovács,Béla Szabó,Alsó sáv\n";
    let laid = layout(&read(csv.as_bytes(), &styling).unwrap(), &styling, FRAME, &M).unwrap();
    let p = &laid.pages[0];
    assert_eq!(p.behaviour, Behaviour::LowerThird);
    layer(p, "Anna Kovács");
    layer(p, "Detective, hangja: Béla Szabó");
    assert!(laid.warnings.is_empty(), "{:?}", laid.warnings);
    // a field for the voice takes it instead
    let styling = Styling::from_json(r#"{"pageStyles":[{"name":"V","behaviour":"lowerThird","fields":{"name":"body","role":"voice"}}]}"#).unwrap();
    let laid = layout(&read("Head,Body,Voice,Page Style\nDetective,Anna,Béla,V\n".as_bytes(), &styling).unwrap(), &styling, FRAME, &M).unwrap();
    layer(&laid.pages[0], "Béla");
    // the head has no field now
    assert_eq!(laid.warnings.len(), 1, "{:?}", laid.warnings);
}
