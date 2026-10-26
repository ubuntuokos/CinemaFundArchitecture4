use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::layout::{Frame, Measure};
use crate::validate::{FontCheck, FontInfo, Severity, contrast_on_black, name_key, validate};
use crate::{Styling, layout, read};

/// 0.5 em per character, 1.2 em lines (as in the layout tests).
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
        let align = match e.params.get("align").map(|p| &p.value) {
            Some(ParamValue::Choice(c)) => *c,
            _ => 0,
        };
        let x0 = match align {
            0 => 0.0,
            1 => -w / 2.0,
            _ => -w,
        };
        Some([x0, -size * 0.9, x0 + w, -size * 0.9 + t.lines().count().max(1) as f64 * size * 1.2])
    }
    fn picture_size(&self, file: &str) -> Option<(f64, f64)> {
        (file == "tiny.png").then_some((40.0, 20.0))
    }
}

/// "Inter" has no ő/ű; "Nincs" is not installed.
struct F;
impl FontInfo for F {
    fn check(&self, family: &str, _style: &str, text: &str) -> FontCheck {
        FontCheck {
            missing: family == "Nincs", uncovered: if family == "Inter" { text.chars().filter(|c| matches!(c, 'ő' | 'ű')).collect() } else { vec![] }
        }
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

fn codes(csv: &str, styling: &Styling) -> Vec<(&'static str, Severity, Option<usize>)> {
    let c = read(csv.as_bytes(), styling).unwrap();
    let l = layout(&c, styling, FRAME, &M).unwrap();
    validate(&c, &l, FRAME, &M, &F).into_iter().map(|i| (i.code, i.severity, i.row)).collect()
}

fn has(v: &[(&'static str, Severity, Option<usize>)], code: &str) -> bool {
    v.iter().any(|(c, _, _)| *c == code)
}

#[test]
fn clean_credits_have_no_issues() {
    let v = codes("Head,Body,Style\nDirected by,Ann Smith,Centered\n,,\nCast,Bob Jones,Gutter\n", &Styling::default());
    assert!(v.is_empty(), "{v:?}");
}

#[test]
fn names_twice_and_spelled_two_ways() {
    let v = codes("Head,Body\nCast,Kiss Anna\n,Kiss Anna\n,,\nMusic,Kiss Ánna\n", &Styling::default());
    assert!(has(&v, "duplicate-name"), "{v:?}");
    let spelled = v.iter().find(|(c, _, _)| *c == "name-spelling").unwrap();
    assert_eq!(spelled.2, Some(5), "the row of the second spelling");
    let dup = v.iter().find(|(c, _, _)| *c == "duplicate-name").unwrap();
    assert_eq!(dup.2, Some(3), "the row of the repeated name");
    assert_eq!(name_key("  Kiss-Ánna  Mária "), "kiss anna maria");
}

#[test]
fn fonts_missing_or_without_some_letters() {
    let st = Styling::from_json(r#"{"contentStyles":[{"name":"Gutter","body":{"font":"Nincs"}}]}"#).unwrap();
    let v = codes("Head,Body\nZene,Hegedűs Hanna\n", &st);
    let missing = v.iter().find(|(c, _, _)| *c == "missing-font").unwrap();
    assert_eq!(missing.1, Severity::Error);
    // the heads stay in Inter, which here lacks ő and ű
    let v = codes("Head,Body\nZeneszerző,Anna\n", &Styling::default());
    assert!(has(&v, "missing-glyphs"), "{v:?}");
    // errors sort first
    let v = codes("Head,Body\nZeneszerző,Hegedűs\n", &st);
    assert_eq!(v[0].1, Severity::Error);
}

#[test]
fn contrast_size_safe_area() {
    assert!((contrast_on_black([1.0, 1.0, 1.0, 1.0]) - 21.0).abs() < 1e-9);
    assert!(contrast_on_black([0.2, 0.2, 0.2, 1.0]) < 4.5);
    let st = Styling::from_json(
        r##"{"contentStyles":[{"name":"Dark","body":{"color":"#333333"}},{"name":"Tiny","body":{"size":16}},{"name":"Huge","body":{"size":200}}]}"##,
    )
    .unwrap();
    assert!(has(&codes("Body,Style\nA,Dark\n", &st), "low-contrast"));
    assert!(has(&codes("Body,Style\nA,Tiny\n", &st), "small-text"));
    // 200 px × 20 characters = 2000 px wide: beyond the sides
    assert!(has(&codes("Body,Style\nAAAAAAAAAAAAAAAAAAAA,Huge\n", &st), "outside-title-safe"));
}

#[test]
fn scroll_speed_and_card_length() {
    let st = Styling::from_json(
        r#"{"pageStyles":[{"name":"Fast","scrollPxPerFrame":9},{"name":"Odd","scrollPxPerFrame":2.5},{"name":"Blink","behaviour":"card","cardFrames":40}]}"#,
    )
    .unwrap();
    // 1080 / (9 × 25) = 4.8 s
    assert!(has(&codes("Body,Page Style\nA,Fast\n", &st), "fast-scroll"));
    assert!(has(&codes("Body,Page Style\nA,Odd\n", &st), "uneven-scroll"));
    // 40 frames − 2 × 12 fade frames = 0.64 s
    assert!(has(&codes("Body,Page Style\nA,Blink\n", &st), "short-card"));
    assert!(has(&codes("Body,Page Style,Page Gap\nA,Blink,-10f\nB,Blink,\n", &st), "overlap"));
}

#[test]
fn upscaled_pictures() {
    let v = codes("Body\n{{Pic tiny.png 200}}\n", &Styling::default());
    assert!(has(&v, "upscaled-picture"), "{v:?}");
}

#[test]
fn many_issues_of_one_kind_are_capped() {
    let st = Styling::from_json(r##"{"contentStyles":[{"name":"Gutter","body":{"color":"#111111"}}]}"##).unwrap();
    let mut csv = String::from("Body\n");
    for i in 0..120 {
        csv.push_str(&format!("N{i}\n,\n"));
    }
    let c = read(csv.as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let v = validate(&c, &l, FRAME, &M, &F);
    assert_eq!(v.iter().filter(|i| i.code == "low-contrast").count(), 50);
    assert!(v.iter().any(|i| i.code == "more-issues" && i.values.get("more").is_some_and(|m| m == "70")));
}

#[test]
fn layer_issues_point_at_their_block_row() {
    let st = Styling::from_json(r##"{"contentStyles":[{"name":"Dark","body":{"color":"#222222"}}]}"##).unwrap();
    let v = codes("Body,Style\nFine,Centered\n,\nToo dark,Dark\n", &st);
    let dark = v.iter().find(|(c, _, _)| *c == "low-contrast").unwrap();
    assert_eq!(dark.2, Some(4));
}
