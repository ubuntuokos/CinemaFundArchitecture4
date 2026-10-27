//! C4: backgrounds, transitions between pages, crawls.

use cfa4_project::graphic_design::RollMode;
use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::layout::{Frame, Measure};
use crate::validate::{FontInfo, colours_behind, contrast, validate};
use crate::{Background, BackgroundKind, Behaviour, Styling, layout, read};

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

struct F;
impl FontInfo for F {}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

fn styling(json: &str) -> Styling {
    Styling::from_json(json).unwrap()
}

fn x_of(e: &EffectInstance) -> (f64, f64) {
    match e.params.get("position").map(|p| &p.value) {
        Some(ParamValue::Vec2(v)) => (v.x, v.y),
        _ => (f64::NAN, f64::NAN),
    }
}

#[test]
fn crawl_puts_the_blocks_on_one_line_and_runs_left() {
    let st = Styling::default();
    let c = read("Head,Body,Page Style\nDirector,Kiss Anna,Crawl\nCamera,Nagy Béla,\n,Szabó Cili,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let p = &l.pages[0];
    assert_eq!(p.behaviour, Behaviour::Crawl);
    assert_eq!(p.meta.roll.mode, RollMode::CrawlLeft);
    assert!(p.meta.roll.start_off_screen && p.meta.roll.end_off_screen);
    // the blocks follow each other from left to right, on the line near the bottom
    let heads: Vec<(f64, f64)> =
        p.layers.iter().filter(|e| matches!(e.params.get("name").map(|x| &x.value), Some(ParamValue::Text(n)) if n == "Head")).map(x_of).collect();
    assert_eq!(heads.len(), 2);
    assert!(heads[1].0 > heads[0].0 + 200.0, "{heads:?}");
    for e in &p.layers {
        let (_, y) = x_of(e);
        assert!(y > 1080.0 * 0.7 && y < 1080.0, "{y}");
    }
    // 4 px per frame across the frame and the line
    assert!(p.content_width > 400.0);
    let frames = p.duration.0 / FRAME.rate.frame_duration().0;
    assert_eq!(frames, ((1920.0 + p.content_width) / 4.0).ceil() as i64);
    assert!(p.spines.is_empty());
    // a crawl with a runtime keeps it (its speed changes)
    let c = read("Body,Page Style,Page Runtime\nA,Crawl,10s\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert_eq!(l.pages[0].duration.0, 10 * cfa4_time::TICKS_PER_SECOND);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
}

#[test]
fn crawl_checks_its_width_and_its_line() {
    let st = styling(r#"{"pageStyles":[{"name":"Fast","behaviour":"crawl","scrollPxPerFrame":20},{"name":"Low","behaviour":"crawl","crawlLine":0.99}]}"#);
    let c = read("Body,Page Style\nA long name here,Fast\nB,Low\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let issues = validate(&c, &l, FRAME, &M, &F);
    let codes: Vec<(&str, Option<usize>)> = issues.iter().map(|i| (i.code, i.page)).collect();
    // 1920 px at 20 px per frame: 3.8 s
    assert!(codes.contains(&("fast-scroll", Some(0))), "{codes:?}");
    // a crawl goes off the sides by design, but not off the bottom
    assert!(!codes.contains(&("outside-title-safe", Some(0))), "{codes:?}");
    assert!(codes.contains(&("outside-title-safe", Some(1))), "{codes:?}");
}

#[test]
fn backgrounds_go_with_the_page_and_set_the_contrast() {
    let st = styling(
        r##"{"pageStyles":[
            {"name":"Light","behaviour":"card","background":{"kind":"color","color":"#f0f0f0"}},
            {"name":"Faint","behaviour":"card","background":{"kind":"color","color":"#f0f0f0","opacity":10}},
            {"name":"Photo","behaviour":"card","background":{"kind":"image","file":"sky.jpg","fit":"fit"}},
            {"name":"Empty","behaviour":"card","background":{"kind":"video"}},
            {"name":"Grad","background":{"kind":"gradient","color":"#000000","color2":"#zzz"}}
        ]}"##,
    );
    let c = read("Body,Page Style\nWhite on light,Light\nWhite on faint,Faint\nOn a photo,Photo\nX,Empty\nY,Grad\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let bg = |i: usize| l.pages[i].background.clone();
    assert_eq!(bg(0).map(|b| b.kind), Some(BackgroundKind::Color));
    assert_eq!(bg(2).map(|b| (b.file, b.fit)), Some(("sky.jpg".to_string(), crate::BackgroundFit::Fit)));
    // a video without a file, a bad colour: warnings, not failures
    assert!(l.warnings.iter().any(|w| w.message.contains("names no file")), "{:?}", l.warnings);
    assert!(l.warnings.iter().any(|w| w.message.contains("#zzz")), "{:?}", l.warnings);
    let issues = validate(&c, &l, FRAME, &M, &F);
    let low: Vec<Option<usize>> = issues.iter().filter(|i| i.code == "low-contrast").map(|i| i.page).collect();
    // white on light grey is unreadable; at 10 % the grey is nearly black
    assert_eq!(low, vec![Some(0)], "{issues:?}");
    let first = issues.iter().find(|i| i.code == "low-contrast").unwrap();
    assert!(first.values.iter().any(|(k, v)| *k == "against" && v == "background"), "{first:?}");
    let unchecked: Vec<Option<usize>> = issues.iter().filter(|i| i.code == "contrast-unchecked").map(|i| i.page).collect();
    assert_eq!(unchecked, vec![Some(2), Some(3)]);
}

#[test]
fn contrast_maths() {
    let white = [1.0, 1.0, 1.0, 1.0];
    let black = [0.0, 0.0, 0.0, 1.0];
    assert!((contrast(white, black) - 21.0).abs() < 1e-6);
    assert!((contrast(black, white) - 21.0).abs() < 1e-6);
    assert!((contrast(white, white) - 1.0).abs() < 1e-9);
    assert_eq!(colours_behind(None), Some(vec![black]));
    // opacity mixes with black in linear light, as the renderer does: white at 20 % over black
    // is light enough that white text on it reads only 4.2:1
    let fifth = Background { kind: BackgroundKind::Color, color: "#ffffff".into(), opacity: 20.0, ..Default::default() };
    let behind = colours_behind(Some(&fifth)).unwrap();
    let ratio = contrast(white, behind[0]);
    assert!((ratio - 4.2).abs() < 0.05, "{ratio}");
    let nan = Background { kind: BackgroundKind::Gradient, opacity: f64::NAN, ..Default::default() };
    assert_eq!(colours_behind(Some(&nan)).map(|v| v.len()), Some(2));
}

#[test]
fn a_transition_replaces_the_fades_at_its_cut() {
    let st = styling(r#"{"pageStyles":[{"name":"Mix","behaviour":"card","transition":"dissolve","transitionFrames":20}]}"#);
    let c = read("Body,Page Style,Page Gap\nA,Mix,\nB,Mix,\nC,Card,2s\nD,Card,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let fd = FRAME.rate.frame_duration().0;
    let t: Vec<Option<(String, i64)>> = l.pages.iter().map(|p| p.transition.clone().map(|(id, d)| (id, d.0 / fd))).collect();
    assert_eq!(t, vec![Some(("cross_dissolve".into(), 20)), Some(("cross_dissolve".into(), 20)), None, None]);
    let fades: Vec<(i64, i64)> = l.pages.iter().map(|p| (p.fade_in.0 / fd, p.fade_out.0 / fd)).collect();
    // A fades in, dissolves into B, which dissolves into C; C fades out before its gap
    assert_eq!(fades, vec![(12, 0), (0, 0), (0, 12), (12, 12)]);
    // the last page has nothing to go into
    let c = read("Body,Page Style\nA,Mix\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert_eq!(l.pages[0].transition, None);
    assert!(l.warnings.is_empty());
    // with a gap between the pages there is no cut to put it on: a warning
    let c = read("Body,Page Style,Page Gap\nA,Mix,1s\nB,Card,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert_eq!(l.pages[0].transition, None);
    assert!(l.warnings.iter().any(|w| w.message.contains("touch")), "{:?}", l.warnings);
}

#[test]
fn transition_names() {
    use crate::style::transition_id;
    assert_eq!(transition_id(""), None);
    assert_eq!(transition_id("Cut"), None);
    assert_eq!(transition_id("Dissolve").as_deref(), Some("cross_dissolve"));
    assert_eq!(transition_id("áttűnés").as_deref(), Some("cross_dissolve"));
    assert_eq!(transition_id("push").as_deref(), Some("push"));
    assert_eq!(transition_id(" iris_round ").as_deref(), Some("iris_round"));
}

#[test]
fn spines_and_content_top_for_guides() {
    let st = Styling::default();
    let c = read("Body,Spine Position,Page Style\nA,,Card\nB,-300,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let p = &l.pages[0];
    assert_eq!(p.spines, vec![660.0, 960.0]);
    assert!((p.content_top - (1080.0 - p.content_height) / 2.0).abs() < 1e-6);
}

#[test]
fn hostile_page_styles_do_not_crash() {
    for json in [
        r#"{"pageStyles":[{"name":"X","behaviour":"crawl","scrollPxPerFrame":1e308,"crawlLine":-5,"crawlGap":1e300}]}"#,
        r#"{"pageStyles":[{"name":"X","behaviour":"crawl","scrollPxPerFrame":0,"crawlLine":1e9,"crawlGap":-1}]}"#,
        r#"{"pageStyles":[{"name":"X","transition":"no such thing","transitionFrames":4294967295}]}"#,
        r#"{"pageStyles":[{"name":"X","background":{"kind":"gradient","angle":1e308,"opacity":-1e9,"blur":1e308}}]}"#,
    ] {
        let st = styling(json);
        let c = read("Body,Page Style\nA,X\nB,X\n,\nC,\n".as_bytes(), &st).unwrap();
        let l = layout(&c, &st, FRAME, &M).unwrap();
        let _ = validate(&c, &l, FRAME, &M, &F);
        for p in &l.pages {
            assert!(p.duration.0 > 0);
        }
    }
}
