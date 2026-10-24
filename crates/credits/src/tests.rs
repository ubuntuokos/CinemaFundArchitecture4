use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick};

use super::*;
use crate::layout::{Frame, Measure};

/// A deterministic stand-in for the text engine: monospace, 1.2 line height.
struct Fake;

fn f(e: &EffectInstance, k: &str) -> f64 {
    match e.params.get(k).map(|p| &p.value) {
        Some(ParamValue::Float(v)) => *v,
        _ => 0.0,
    }
}
fn txt(e: &EffectInstance) -> String {
    match e.params.get("text").map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}
fn choice(e: &EffectInstance, k: &str) -> u32 {
    match e.params.get(k).map(|p| &p.value) {
        Some(ParamValue::Choice(c)) => *c,
        _ => 0,
    }
}
fn pos(e: &EffectInstance) -> (f64, f64) {
    match e.params.get("position").map(|p| &p.value) {
        Some(ParamValue::Vec2(v)) => (v.x, v.y),
        _ => (f64::NAN, f64::NAN),
    }
}

impl Measure for Fake {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = f(e, "size");
        let lh = size * 1.2;
        let t = txt(e);
        let bw = f(e, "box_width");
        if bw > 0.0 {
            let per_line = ((bw / (size * 0.5)).floor() as usize).max(1);
            let lines: usize = t.split('\n').map(|l| l.chars().count().div_ceil(per_line).max(1)).sum();
            return Some([0.0, 0.0, bw, lines as f64 * lh]);
        }
        let lines = t.split('\n').count() as f64;
        let w = t.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f64 * size * 0.5;
        let x0 = match choice(e, "align") {
            0 => 0.0,
            1 => -w / 2.0,
            _ => -w,
        };
        Some([x0, -size * 0.9, x0 + w, -size * 0.9 + lines * lh])
    }
    fn picture_size(&self, file: &str) -> Option<(f64, f64)> {
        match file {
            "logo.png" | "Alap logó.png" => Some((400.0, 200.0)),
            "wide.png" => Some((10_000.0, 100.0)),
            _ => None,
        }
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

fn lay(csv: &str, styling: &Styling) -> Laid {
    let c = read(csv.as_bytes(), styling).unwrap();
    layout(&c, styling, FRAME, &Fake).unwrap()
}

#[test]
fn csv_quotes_delimiters_and_breaks() {
    let rows = table::read_table("\u{feff}a;\"b;c\";\"d\"\"e\"\r\n\"multi\nline\";x\r\nlast".as_bytes(), table::Limits::default()).unwrap();
    assert_eq!(rows, vec![vec!["a", "b;c", "d\"e"], vec!["multi\nline", "x"], vec!["last"]]);
    assert_eq!(table::detect_delimiter("Head\tBody\n"), '\t');
    assert_eq!(table::detect_delimiter("Head,Body\n"), ',');
}

#[test]
fn csv_limits_are_errors_not_crashes() {
    let lim = table::Limits { max_bytes: 100, max_rows: 2, max_cols: 3, max_cell_chars: 5 };
    assert!(table::read_table(&[b'a'; 101], lim).is_err());
    assert!(table::parse("a\nb\nc\n", ',', lim).is_err());
    assert!(table::parse("a,b,c,d", ',', lim).is_err());
    assert!(table::parse("abcdef", ',', lim).is_err());
    assert!(table::parse("\"unterminated", ',', table::Limits::default()).is_ok());
}

#[test]
fn blocks_gaps_styles_and_pages() {
    let csv = "My film credits\n\
               @Head,@Body,@Tail,@Vertical Gap,@Content Style,@Page Style,@Page Runtime\n\
               ,THE FILM,,,Heading,Card,5s\n\
               Directed by,Jordan Lee,,,Centered,Scroll,\n\
               ,,,,,,\n\
               Cast,Ann,,,Gutter,,\n\
               ,Bob,,,,,\n\
               ,{{Blank}},,,,,\n\
               ,Cyd,as herself,,,,\n\
               ,Dee,,3,,,\n";
    let c = read(csv.as_bytes(), &Styling::default()).unwrap();
    assert_eq!(c.pages.len(), 2);
    assert_eq!(c.pages[0].style, "Card");
    assert_eq!(c.pages[0].runtime, Some(TimeSpec::Ticks(Tick(5 * TICKS_PER_SECOND))));
    let p = &c.pages[1];
    assert_eq!(p.blocks.len(), 3);
    assert_eq!(p.blocks[0].head.as_deref(), Some("Directed by"));
    assert_eq!(p.blocks[1].gap_before, Gap::Units(1.0));
    assert_eq!(p.blocks[1].style, "Gutter");
    assert_eq!(p.blocks[1].body, vec!["Ann", "Bob", "", "Cyd"]);
    assert_eq!(p.blocks[1].tails, vec![(3, "as herself".to_string())]);
    assert_eq!(p.blocks[2].gap_before, Gap::Units(3.0));
    assert_eq!(p.blocks[2].body, vec!["Dee"]);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
}

#[test]
fn hungarian_headers_and_unknown_markup_warn() {
    let csv = "Fej;Törzs;Stílus\nRendező;{{Pic logo}}Kiss Anna;\n";
    let c = read(csv.as_bytes(), &Styling::default()).unwrap();
    assert_eq!(c.pages[0].blocks[0].body, vec!["Kiss Anna"]);
    assert_eq!(c.warnings.len(), 1);
}

#[test]
fn no_header_and_empty_are_errors() {
    assert_eq!(read(b"a,b\nc,d\n", &Styling::default()), Err(CreditsError::NoHeader));
    assert_eq!(read(b"Body\n\n\n", &Styling::default()), Err(CreditsError::Empty));
}

#[test]
fn scroll_page_rolls_and_lasts_travel_over_speed() {
    let laid = lay("Body\nAnn\nBob\n", &Styling::default());
    let p = &laid.pages[0];
    assert_eq!(p.behaviour, Behaviour::Scroll);
    assert_eq!(p.meta.roll.mode, cfa4_project::RollMode::Roll);
    assert!(p.meta.roll.start_off_screen && p.meta.roll.end_off_screen);
    // one layer for a one-column body; two lines of 40 px * 1.2
    assert_eq!(p.layers.len(), 1);
    assert!((p.content_height - 96.0).abs() < 1e-9);
    let frames = ((1080.0 + 96.0) / 3.0f64).ceil() as i64;
    assert_eq!(p.duration, Tick(FRAME.rate.frame_duration().0 * frames));
    // the first baseline sits 0.9 em below the top of the content
    assert!((pos(&p.layers[0]).1 - 36.0).abs() < 1e-9);
}

#[test]
fn card_is_centred_and_fades() {
    let laid = lay("Body,Page Style\nTHE END,Card\n", &Styling::default());
    let p = &laid.pages[0];
    assert_eq!(p.behaviour, Behaviour::Card);
    assert_eq!(p.meta.roll.mode, cfa4_project::RollMode::Off);
    assert_eq!(p.duration, Tick(FRAME.rate.frame_duration().0 * 120));
    let (_, y) = pos(&p.layers[0]);
    // 48 px line box centred in 1080: top 516, baseline 516 + 36
    assert!((y - 552.0).abs() < 1e-9, "{y}");
    let op = p.layers[0].params.get("opacity").unwrap();
    assert_eq!(op.keyframes.len(), 4);
    assert_eq!(op.keyframes.last().unwrap().time, p.duration);
}

#[test]
fn gutter_puts_head_left_and_body_right_of_the_spine() {
    let laid = lay("Head,Body,Style\nMusic,Hana,Gutter\n", &Styling::default());
    let l = &laid.pages[0].layers;
    assert_eq!(l.len(), 2);
    assert_eq!(choice(&l[0], "align"), 2);
    assert!(pos(&l[0]).0 < 960.0 && pos(&l[1]).0 > 960.0);
    assert_eq!(choice(&l[1], "align"), 0);
}

#[test]
fn grid_columns_stay_in_step() {
    let laid = lay("Body,Style\nA,Grid\n\"B\nb2\",\nC,\nD,\n", &Styling::default());
    let l = &laid.pages[0].layers;
    assert_eq!(l.len(), 3);
    // row 1: A | B\nb2 | C → A and C padded to two lines; row 2: D
    assert_eq!(txt(&l[0]), "A\n\nD");
    assert_eq!(txt(&l[1]), "B\nb2\n");
    let xs: Vec<f64> = l.iter().map(|e| pos(e).0).collect();
    assert!(xs[0] < xs[1] && xs[1] < xs[2]);
}

#[test]
fn flow_and_paragraphs_wrap() {
    let laid = lay("Body,Style\nA,Bullets\nB,\n,\nLong text,Paragraphs\nMore,\n", &Styling::default());
    let l = &laid.pages[0].layers;
    assert_eq!(txt(&l[0]), "A  •  B");
    assert!(f(&l[0], "box_width") > 0.0);
    assert_eq!(l.len(), 3);
}

#[test]
fn runtime_overrides_and_gaps_resolve_against_the_rate() {
    let laid = lay("Body,Page Runtime,Page Gap\nA,00:00:10:00,-12f\n", &Styling::default());
    let p = &laid.pages[0];
    assert_eq!(p.duration, Tick(10 * TICKS_PER_SECOND));
    assert_eq!(p.gap_after, Tick(-12 * FRAME.rate.frame_duration().0));
}

#[test]
fn unknown_styles_fall_back_with_a_warning() {
    let laid = lay("Body,Style,Page Style\nA,Nope,Nada\n", &Styling::default());
    assert_eq!(laid.warnings.len(), 2, "{:?}", laid.warnings);
}

#[test]
fn custom_styling_json() {
    let s = Styling::from_json(r##"{"defaultContentStyle":"Big","contentStyles":[{"name":"Big","body":{"size":80,"color":"#ff0000"}}],"pageStyles":[{"name":"Slow","scrollPxPerFrame":1}]}"##).unwrap();
    let laid = lay("Body,Page Style\nA,Slow\n", &s);
    let p = &laid.pages[0];
    assert!((f(&p.layers[0], "size") - 80.0).abs() < 1e-9);
    assert_eq!(p.duration, Tick(FRAME.rate.frame_duration().0 * (1080 + 96)));
    assert!(Styling::from_json("{not json").is_err());
}

#[test]
fn scales_with_the_frame_height() {
    let c = read(b"Body\nA\n", &Styling::default()).unwrap();
    let uhd = Frame { width: 3840, height: 2160, ..FRAME };
    let laid = layout(&c, &Styling::default(), uhd, &Fake).unwrap();
    assert!((f(&laid.pages[0].layers[0], "size") - 80.0).abs() < 1e-9);
    assert!(layout(&c, &Styling::default(), Frame { width: 0, ..FRAME }, &Fake).is_err());
}

/// Hostile input: random bytes, truncations and absurd numbers never panic.
#[test]
fn hostile_input_never_panics() {
    let base = "Head,Body,Tail,Vertical Gap,Content Style,Spine Position,Page Style,Page Runtime,Page Gap\n\
                A,B,C,1e309,Grid,NaN,Card,-99999999999999999999f,99:99:99:99\n\
                ,\"x\ny\",,-5,Paragraphs,1e9,Scroll,1e300s,inf\n\
                {{,{{Blank,}},px,,,,,\n";
    let styling = Styling::from_json(r#"{"unitPx":1e308,"contentStyles":[{"name":"Grid","columns":4000000000,"columnGap":-1e308,"body":{"size":-1,"leading":1e308}}],"pageStyles":[{"name":"Scroll","scrollPxPerFrame":0}]}"#).unwrap();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let bytes = base.as_bytes();
    for i in 0..3000 {
        let mut b = bytes[..(i * 7) % (bytes.len() + 1)].to_vec();
        if i % 2 == 0 {
            b = bytes.to_vec();
            for _ in 0..(rnd() % 8) {
                let n = b.len();
                let k = (rnd() as usize) % n;
                b[k] = rnd() as u8;
            }
        }
        if let Ok(c) = read(&b, &styling) {
            let _ = layout(&c, &styling, FRAME, &Fake);
            let _ = layout(&c, &Styling::default(), FRAME, &Fake);
        }
    }
}

#[test]
fn the_example_sheet_reads_cleanly() {
    let laid = lay(include_str!("../examples/credits.csv"), &Styling::default());
    assert!(laid.warnings.is_empty(), "{:?}", laid.warnings);
    let kinds: Vec<Behaviour> = laid.pages.iter().map(|p| p.behaviour).collect();
    assert_eq!(kinds, vec![Behaviour::Card, Behaviour::Scroll, Behaviour::Card]);
}

#[test]
fn tails_stand_beside_their_body_lines() {
    let laid = lay("Head,Body,Tail,Style\nCast,Ann,as Ann,Gutter\n,Bob,,\n,Cyd,as herself,\n", &Styling::default());
    let l = &laid.pages[0].layers;
    let tails: Vec<&EffectInstance> = l.iter().filter(|e| txt(e).starts_with("as ")).collect();
    assert_eq!(tails.len(), 2);
    let body = l.iter().find(|e| txt(e).starts_with("Ann")).unwrap();
    let head = l.iter().find(|e| txt(e) == "Cast").unwrap();
    // same baseline as the first body line; the second tail two lines (2 × 48 px) lower
    assert!((pos(head).1 - pos(body).1).abs() < 1e-9);
    assert!((pos(tails[0]).1 - pos(body).1).abs() < 1e-9);
    assert!((pos(tails[1]).1 - pos(body).1 - 96.0).abs() < 1e-9);
}

#[test]
fn picture_tags_parse() {
    let (rest, p) = sheet::take_pictures("{{Pic logo.png}} {{Kép Alap logó.png 120px}}{{Bold}}x");
    assert_eq!(rest, " {{Bold}}x");
    assert_eq!(p, vec![Picture { file: "logo.png".into(), height: None }, Picture { file: "Alap logó.png".into(), height: Some(120.0) }]);
    assert_eq!(sheet::take_pictures("{{Pic}} {{Pic a b}}").1, vec![Picture { file: "a b".into(), height: None }]);
}

#[test]
fn pictures_get_placeholders_and_placements() {
    let laid = lay(
        "Head,Body,Style\nSupported by,{{Pic logo.png}} {{Logo Alap logó.png 75}},Centered\n,Names,\n,{{Pic missing.png}},\n,{{Pic wide.png}},\n",
        &Styling::default(),
    );
    let p = &laid.pages[0];
    assert_eq!(laid.warnings.len(), 1, "{:?}", laid.warnings);
    assert_eq!(p.pictures.len(), 3);
    let (a, b) = (&p.pictures[0], &p.pictures[1]);
    assert_eq!(a.size, (300.0, 150.0));
    assert_eq!(b.size, (150.0, 75.0));
    // one row, centred on the spine, vertically centred on each other
    let left = a.center.0 - a.size.0 / 2.0;
    let right = b.center.0 + b.size.0 / 2.0;
    assert!(((left + right) / 2.0 - 960.0).abs() < 1e-9);
    assert!((a.center.1 - b.center.1).abs() < 1e-9);
    // never wider than the frame
    assert!(p.pictures[2].size.0 <= 1920.0 * 0.95 + 1e-9);
    // placeholders are transparent shape layers counted in the page height
    let shapes = p.layers.iter().filter(|e| e.effect == "graphic_shape").count();
    assert_eq!(shapes, 3);
    assert!(p.content_height > 150.0 + 48.0);
    // the names block follows the picture row, picture blocks take no more body cells
    assert!(p.layers.iter().any(|e| txt(e) == "Names"));
}

#[test]
fn card_pictures_report_the_fades() {
    let laid = lay("Body,Page Style\n{{Pic logo.png}},Card\n", &Styling::default());
    let p = &laid.pages[0];
    assert_eq!(p.fade_in, Tick(FRAME.rate.frame_duration().0 * 12));
    assert_eq!(p.fade_out, p.fade_in);
    // the picture sits in the middle of the frame
    assert!((p.pictures[0].center.1 - 540.0).abs() < 1e-9);
}
