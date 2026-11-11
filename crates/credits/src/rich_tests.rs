//! C5: inline letter styles, leaders, harmonised head columns.

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::layout::{Frame, Measure};
use crate::markup::{plain, style_tag, styled};
use crate::validate::{FontInfo, validate};
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

struct F;
impl FontInfo for F {}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

fn text_of(e: &EffectInstance) -> String {
    match e.params.get("text").map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

fn name_of(e: &EffectInstance) -> String {
    match e.params.get("name").map(|p| &p.value) {
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

#[test]
fn style_tags_become_runs() {
    assert_eq!(style_tag("Style Bold"), Some("Bold"));
    assert_eq!(style_tag(" stílus  Dőlt "), Some("Dőlt"));
    assert_eq!(style_tag("Style"), Some(""));
    assert_eq!(style_tag("Pic a.png"), None);
    let bold = cfa4_project::graphic_design::CharStyle { font_style: Some("Bold".into()), ..Default::default() };
    let look = |n: &str| (n == "Bold").then(|| bold.clone());
    let s = styled("Anna {{Style Bold}}Kiss{{Style}} és Béla", look);
    assert_eq!(s.text, "Anna Kiss és Béla");
    assert_eq!(s.runs.len(), 1);
    assert_eq!((s.runs[0].start, s.runs[0].end), (5, 9));
    // a style ends with its line; characters, not bytes, are counted
    let s = styled("ő{{Style Bold}}ű\nx", look);
    assert_eq!(s.text, "őű\nx");
    assert_eq!((s.runs[0].start, s.runs[0].end), (1, 2));
    // unknown names are reported once; other tags stay; broken tags stay as text
    let s = styled("{{Style Nope}}a{{Style Nope}}b {{Pic x.png}} {{Style", look);
    assert_eq!(s.unknown, vec!["Nope".to_string()]);
    assert_eq!(s.text, "ab {{Pic x.png}} {{Style");
    assert_eq!(plain("A {{Stílus Félkövér}}B"), "A B");
}

#[test]
fn inline_styles_reach_the_text_layers() {
    let st = Styling::from_json(r##"{"inlineStyles":[{"name":"Gold","color":"#d4af37","size":60},{"name":"Bad","color":"nope"}]}"##).unwrap();
    let c = read("Head,Body\nZene,{{Style Gold}}Hegedűs{{Style}} Hanna\n,{{Stílus Dőlt}}vendég\n,{{Style Bad}}x {{Style Missing}}y\n".as_bytes(), &st).unwrap();
    // no "tags removed" warning for letter styles
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let body = l.pages[0].layers.iter().find(|e| text_of(e).starts_with("Hegedűs")).unwrap();
    assert_eq!(text_of(body), "Hegedűs Hanna\nvendég\nx y");
    let runs = &body.layer.as_ref().unwrap().runs;
    let gold = runs.iter().find(|r| r.start == 0).unwrap();
    assert_eq!(gold.end, 7);
    assert_eq!(gold.style.size, Some(60.0));
    assert!(gold.style.fill.is_some());
    assert!(runs.iter().any(|r| r.style.font_style.as_deref() == Some("Italic")));
    assert!(l.warnings.iter().any(|w| w.message.contains("`nope`")), "{:?}", l.warnings);
    assert!(l.warnings.iter().any(|w| w.message.contains("`Missing`")), "{:?}", l.warnings);
    // the names check reads the names without their tags
    let c = read("Body\n{{Style Bold}}Kiss Anna\nKiss Anna\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let issues = validate(&c, &l, FRAME, &M, &F);
    assert!(issues.iter().any(|i| i.code == "duplicate-name"), "{issues:?}");
    // the page is named without tags
    assert!(!l.pages[0].name.contains("{{"), "{}", l.pages[0].name);
}

#[test]
fn leaders_run_from_the_head_to_the_body_and_heads_share_a_column() {
    let st = Styling::default();
    let c = read("Head,Body,Content Style\nDirector,Anna,Leaders\nDirector of photography,Béla,\nMusic,Cili,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    let layers = &l.pages[0].layers;
    let heads: Vec<&EffectInstance> = layers.iter().filter(|e| name_of(e) == "Head").collect();
    let leaders: Vec<&EffectInstance> = layers.iter().filter(|e| name_of(e) == "Leader").collect();
    let bodies: Vec<&EffectInstance> = layers.iter().filter(|e| name_of(e) == "Body").collect();
    assert_eq!((heads.len(), bodies.len()), (3, 3));
    // the heads start at one left edge, the widest one included
    let lefts: Vec<f64> = heads.iter().map(|e| pos(e).0).collect();
    assert!(lefts.iter().all(|x| (x - lefts[0]).abs() < 1e-6), "{lefts:?}");
    // the longest head leaves room for few or no dots; the short ones get more
    let short = leaders.iter().map(|e| text_of(e).len()).max().unwrap();
    assert!(short > 10, "{short}");
    // every leader ends just before the body, on its head's baseline
    let body_x = pos(bodies[0]).0;
    for le in &leaders {
        let (x, y) = pos(le);
        assert!((x - (body_x - 16.0)).abs() < 1e-6, "{x} vs {body_x}");
        assert!(heads.iter().any(|h| (pos(h).1 - y).abs() < 1e-6));
        assert!(text_of(le).chars().all(|c| c == ' ' || c == '.'));
    }
    // a hostile leader and gap do not crash and do not run away
    let st = Styling::from_json(
        r#"{"contentStyles":[{"name":"X","orientation":"horizontal","leader":"·","leaderGap":-1e308,"headGap":1e308,"harmonizeHeads":true}]}"#,
    )
    .unwrap();
    let c = read("Head,Body,Content Style\nA,B,X\nCC,D,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    for e in l.pages[0].layers.iter().filter(|e| name_of(e) == "Leader") {
        assert!(text_of(e).chars().count() <= 2_000);
    }
}

#[test]
fn hostile_markup_never_panics() {
    let st = Styling::default();
    for cell in ["{{", "}}", "{{Style}}", "{{Style ő}}ű{{", "a{{Style Bold}}\n\n{{Style}}", "{{{{Style Bold}}}}", &"{{Style Bold}}x".repeat(20_000)] {
        let sheet = format!("Head,Body\n\"{}\",\"{}\"\n", cell.replace('"', "\"\""), cell.replace('"', "\"\""));
        if let Ok(c) = read(sheet.as_bytes(), &st) {
            let _ = layout(&c, &st, FRAME, &M);
        }
        let _ = styled(cell, |_| Some(Default::default()));
    }
}

#[test]
fn parallel_columns_start_at_the_same_top() {
    let st = Styling::default();
    let sheet = "Head,Body,Spine Position,Content Style\n\
                 Camera,Anna,-450,Centered\n,Béla,,\n,,,\n\
                 Sound,Cili,450 parallel,\n,,,\n,Dóra,,\nEdit,Feri,,\n\
                 Thanks,Everyone,0,\n";
    let c = read(sheet.as_bytes(), &st).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert!(c.pages[0].blocks[1].parallel && !c.pages[0].blocks[2].parallel);
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let layers = &l.pages[0].layers;
    let y_of = |t: &str| layers.iter().find(|e| text_of(e).contains(t)).map(|e| pos(e).1).unwrap();
    let x_of = |t: &str| layers.iter().find(|e| text_of(e).contains(t)).map(|e| pos(e).0).unwrap();
    // the two columns start at the same height, side by side
    assert!((y_of("Camera") - y_of("Sound")).abs() < 1e-6);
    assert!(x_of("Camera") < 960.0 && x_of("Sound") > 960.0);
    // the last block goes below the longer (right) column
    assert!(y_of("Thanks") > y_of("Feri"));
    assert!(y_of("Feri") > y_of("Béla"));
    // the content is as tall as the longer column
    let bottom = layers.iter().map(|e| pos(e).1).fold(f64::MIN, f64::max);
    assert!(l.pages[0].content_height >= bottom - layers.iter().map(|e| pos(e).1).fold(f64::MAX, f64::min));
    // `párhuzamos` too; a bad spine is a warning
    let c = read("Body,Spine Position\nA,-300\nB,300 párhuzamos\nC,left\n".as_bytes(), &st).unwrap();
    assert!(c.pages[0].blocks[1].parallel);
    assert_eq!(c.warnings.len(), 1);
}

/// Every picture is 300 × 400 (portrait); a video 1920 × 1080.
struct P;
impl Measure for P {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        M.bounds(e)
    }
    fn picture_size(&self, file: &str) -> Option<(f64, f64)> {
        if file == "missing.jpg" {
            None
        } else if file.ends_with(".mp4") {
            Some((1920.0, 1080.0))
        } else {
            Some((300.0, 400.0))
        }
    }
}

#[test]
fn portraits_stand_above_or_beside_their_names() {
    let st = Styling::default();
    let sheet = "Body,Content Style\n{{Pic a.jpg}} Kiss Anna,Portraits\n{{Kép b.jpg}} Nagy Béla,\nTóth Cili,\n{{Pic missing.jpg}} Szabó Dóra,\n";
    let c = read(sheet.as_bytes(), &st).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let b = &c.pages[0].blocks[0];
    assert_eq!(b.body, vec!["Kiss Anna", "Nagy Béla", "Tóth Cili", "Szabó Dóra"]);
    assert_eq!(
        b.portraits.iter().map(|p| p.as_ref().map(|p| p.file.as_str())).collect::<Vec<_>>(),
        vec![Some("a.jpg"), Some("b.jpg"), None, Some("missing.jpg")]
    );
    let l = layout(&c, &st, FRAME, &P).unwrap();
    let pg = &l.pages[0];
    // two readable portraits, 220 high (300 × 400 → 165 × 220), each above its name
    assert_eq!(pg.pictures.len(), 2, "a.jpg and b.jpg: {:?}", pg.pictures);
    for pic in &pg.pictures {
        assert!((pic.size.1 - 220.0).abs() < 1e-6 && (pic.size.0 - 165.0).abs() < 1e-6);
    }
    let name = |t: &str| pg.layers.iter().find(|e| text_of(e) == t).map(pos).unwrap();
    let anna = name("Kiss Anna");
    let a = &pg.pictures[0];
    assert!((a.center.0 - anna.0).abs() < 1e-6, "centred over the name");
    assert!(a.center.1 + a.size.1 / 2.0 < anna.1, "above the name");
    // four columns: the names stand side by side on one line
    assert!((name("Tóth Cili").1 - anna.1).abs() < 1e-6);
    assert!(l.warnings.iter().any(|w| w.message.contains("missing.jpg")));
    // beside the name
    let st = Styling::from_json(r#"{"contentStyles":[{"name":"Side","columns":1,"portraitSide":"left","portraitHeight":100}]}"#).unwrap();
    let c = read("Body,Content Style\n{{Pic a.jpg}} Kiss Anna,Side\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &P).unwrap();
    let pg = &l.pages[0];
    let pic = &pg.pictures[0];
    let anna = pg.layers.iter().find(|e| text_of(e) == "Kiss Anna").map(pos).unwrap();
    assert!(pic.center.0 + pic.size.0 / 2.0 <= anna.0 + 1e-6, "left of the name");
}

#[test]
fn videos_are_placed_like_pictures() {
    let st = Styling::default();
    let c = read("Body\n{{Video intro.mp4 270}}\n{{Videó szalag.mp4}}\n".as_bytes(), &st).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let pics = &c.pages[0].blocks[0].pictures;
    assert!(pics.iter().all(|p| p.video));
    let l = layout(&c, &st, FRAME, &P).unwrap();
    let v: Vec<&crate::PicturePlace> = l.pages[0].pictures.iter().collect();
    assert_eq!(v.len(), 2);
    assert!(v.iter().all(|p| p.video));
    assert!((v[0].size.1 - 270.0).abs() < 1e-6 && (v[0].size.0 - 480.0).abs() < 1e-6);
}

#[test]
fn cards_slide_in_and_stagger_their_blocks() {
    let st = Styling::from_json(
        r#"{"pageStyles":[{"name":"In","behaviour":"card","entrance":"slideUp","entranceFrames":10,"entranceDistance":100,"staggerFrames":5}]}"#,
    )
    .unwrap();
    let c = read("Head,Body,Page Style\nA,One,In\n,,\nB,Two,\n,,\nC,{{Pic a.jpg}},\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &P).unwrap();
    let pg = &l.pages[0];
    let fd = FRAME.rate.frame_duration().0;
    let (dx, dy, len) = pg.entrance.unwrap();
    assert_eq!((dx, dy, len.0), (0.0, 100.0, 10 * fd));
    let one = pg.layers.iter().find(|e| text_of(e) == "One").unwrap();
    let two = pg.layers.iter().find(|e| text_of(e) == "Two").unwrap();
    let start = |e: &EffectInstance| e.params["position"].keyframes.iter().map(|k| k.time.0).collect::<Vec<_>>();
    // the first block moves from the start, the second five frames later
    assert_eq!(start(one), vec![0, 10 * fd]);
    assert_eq!(start(two), vec![0, 5 * fd, 15 * fd]);
    // it is hidden until then
    let op = &two.params["opacity"];
    assert_eq!(op.value_at(cfa4_time::Tick(4 * fd)), ParamValue::Float(0.0));
    assert_eq!(op.value_at(cfa4_time::Tick(30 * fd)), ParamValue::Float(100.0));
    // the picture block comes third
    assert_eq!(pg.pictures[0].delay.0, 10 * fd);
    // scroll pages ignore an entrance
    let st = Styling::from_json(r#"{"pageStyles":[{"name":"R","entrance":"slideLeft","staggerFrames":5}]}"#).unwrap();
    let c = read("Body,Page Style\nA,R\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &P).unwrap();
    assert!(l.pages[0].entrance.is_none());
    assert!(!l.pages[0].layers[0].params["position"].is_animated());
}
