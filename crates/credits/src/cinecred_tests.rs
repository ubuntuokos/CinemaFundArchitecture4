//! C7: Cinecred projects.

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::cinecred::{column, read_styling};
use crate::layout::{Frame, Measure};
use crate::sheet::Column;
use crate::{Behaviour, BodyLayout, Orientation, layout, read};

struct M;
impl Measure for M {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = match e.params.get("size").map(|p| &p.value) {
            Some(ParamValue::Float(v)) => *v,
            _ => 40.0,
        };
        Some([-100.0, -size * 0.9, 100.0, size * 0.3])
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

/// A small styling in Cinecred's format (written for this test).
const STYLING: &str = r#"
version = "1.9.1"

[global]
resolution = "3840x2160"
fps = "24"
unitVGapPx = 64.0
runtimeFrames = 2400

[[pageStyle]]
name = "Scroll"
behavior = "SCROLL"
scrollPxPerFrame = 6.0
subsequentGapFrames = 24

[[pageStyle]]
name = "Card"
behavior = "CARD"
cardRuntimeFrames = 96
cardFadeInFrames = 6
cardFadeOutFrames = 18

[[contentStyle]]
name = "Gutter"
blockOrientation = "HORIZONTAL"
bodyLetterStyleName = "Name"
headLetterStyleName = "Small"
headHJustify = "LEFT"
headHarmonizeWidth = "ACROSS_BLOCKS"
headGapPx = 64.0
headLeader = "."
gridCols = 1

[[contentStyle]]
name = "Flowing"
bodyLayout = "FLOW"
bodyLetterStyleName = "Name"
flowRowWidthPx = 2000.0
sort = "ASCENDING"

[[letterStyle]]
name = "Name"
font = "Inter Semi Bold"
heightPx = 64.0
leadingTopRh = 0.1
leadingBottomRh = 0.1
uppercase = true
  [[letterStyle.layers]]
  coloring = "PLAIN"
  plainColor = [0.9642, 1.0, 0.8251, 1.0]

[[letterStyle]]
name = "Small"
font = "Inter"
heightPx = 48.0
  [[letterStyle.layers]]
  coloring = "PLAIN"
  plainColor = [0.4361, 0.2225, 0.0139, 1.0]
  [[letterStyle.layers]]
  coloring = "PLAIN"
  plainColor = [0.0, 0.0, 0.0, 1.0]

[[transitionStyle]]
name = "Smooth"
"#;

#[test]
fn a_cinecred_styling_becomes_cfa4_styles() {
    let imp = read_styling(STYLING).unwrap();
    let st = &imp.styling;
    // a 2160-high project: everything halves to a 1080-high frame
    assert!((st.unit_px - 32.0).abs() < 1e-9);
    let scroll = st.page_style("Scroll").unwrap();
    assert_eq!(scroll.behaviour, Behaviour::Scroll);
    assert!((scroll.scroll_px_per_frame - 3.0).abs() < 1e-9);
    assert_eq!(scroll.gap_after_frames, 24);
    let card = st.page_style("Card").unwrap();
    assert_eq!((card.behaviour, card.card_frames, card.fade_in_frames, card.fade_out_frames), (Behaviour::Card, 96, 6, 18));
    let gutter = st.content_style("Gutter").unwrap();
    assert_eq!(gutter.orientation, Orientation::Horizontal);
    assert!(gutter.harmonize_heads && gutter.leader == "." && gutter.head_align == crate::Align::Left);
    assert!((gutter.head_gap - 32.0).abs() < 1e-9);
    // letter styles: family and weight split, D50 white → white, sRGB red primary → red
    assert_eq!((gutter.body.font.as_str(), gutter.body.font_style.as_str()), ("Inter", "SemiBold"));
    assert_eq!(gutter.body.color, "#ffffff");
    assert!(gutter.body.all_caps);
    assert_eq!(gutter.head.color, "#ff0000");
    assert_eq!(gutter.head.font_style, "Regular");
    // 64 px high with 20 % leading at 2160 lines: 25.6 px of font, about 21 px em
    assert!((gutter.body.size - 32.0 * 0.8 / 1.2).abs() < 1e-6, "{}", gutter.body.size);
    assert_eq!(st.content_style("Flowing").unwrap().layout, BodyLayout::Flow);
    // letter styles are inline styles too
    assert!(st.inline_styles.iter().any(|s| s.name == "Small"));
    // what could not be carried over is said
    let notes = imp.notes.join("\n");
    for word in ["runtime", "only the first coloured layer", "sorting", "transition curves"] {
        assert!(notes.contains(word), "{word}: {notes}");
    }
    assert_eq!(st.default_page_style, "Scroll");
}

#[test]
fn cinecred_sheets_in_other_languages_read() {
    assert_eq!(column("@Kopf"), Some(Column::Head));
    assert_eq!(column("@Senkrechte Lücke"), Some(Column::Gap));
    assert_eq!(column("@Style de page"), Some(Column::Page));
    assert_eq!(column("@页面时长"), Some(Column::Runtime));
    assert_eq!(column("nope"), None);
    let imp = read_styling(STYLING).unwrap();
    let sheet = "@Kopf,@Körper,@Fuß,@Senkrechte Lücke,@Inhaltsstil,@Breche Harmonisierung,@Rückgratposition,@Seitenstil,@Seitenlaufzeit,@Seitenlücke\n\
                 Regie,{{Stil Small}}Anna{{Stil}} Kiss,,,Gutter,,,Card,,\n\
                 ,{{Leer}},,,,,,,,\n\
                 Kamera,Béla,,,,,-400,Scroll,,\n\
                 Ton,Cili,,,,,400 Parallel,,,\n\
                 ,{{Bild logo.png}},,,,,,,,\n";
    let c = read(sheet.as_bytes(), &imp.styling).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert_eq!(c.pages.len(), 2);
    let b = &c.pages[0].blocks[0];
    assert_eq!(b.head.as_deref(), Some("Regie"));
    assert_eq!(b.body, vec!["{{Stil Small}}Anna{{Stil}} Kiss".to_string(), String::new()]);
    assert!(c.pages[1].blocks[1].parallel);
    assert_eq!(c.pages[1].blocks[2].pictures.len(), 1);
    let l = layout(&c, &imp.styling, FRAME, &M).unwrap();
    let text = l.pages[0].layers.iter().find_map(|e| match e.params.get("text").map(|p| &p.value) {
        Some(ParamValue::Text(t)) if t.starts_with("Anna") => Some(e.clone()),
        _ => None,
    });
    let text = text.unwrap();
    assert!(!text.layer.as_ref().unwrap().runs.is_empty(), "the inline style became a run");
}

#[test]
fn broken_or_hostile_stylings_do_not_crash() {
    assert!(read_styling("this is not toml = = =").is_err());
    assert!(read_styling(&"#".repeat(5 << 20)).is_err());
    for t in [
        "",
        "[global]\nresolution = \"0x0\"",
        "[global]\nresolution = \"1x-5\"\nunitVGapPx = 1e308",
        "[[pageStyle]]\nname = \"\"\n[[pageStyle]]\nname = \"X\"\nscrollPxPerFrame = -1e308\ncardRuntimeFrames = -5",
        "[[letterStyle]]\nname = \"A\"\nheightPx = 1e308\nleadingTopRh = 5.0\n[[letterStyle.layers]]\nplainColor = [\"x\"]",
        "[[contentStyle]]\nname = \"C\"\ngridCols = 99999\nbodyLetterStyleName = \"missing\"",
        "pageStyle = 5\ncontentStyle = \"x\"",
    ] {
        if let Ok(imp) = read_styling(t) {
            let c = read(b"Body\nA\n", &imp.styling).unwrap();
            let _ = layout(&c, &imp.styling, FRAME, &M);
        }
    }
}
