//! F10-C16: title, opening, chapter and lower-third pages fill graphics templates.

use cfa4_project::gtemplate::layer_uid;
use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::{FrameRate, Tick};

use super::*;
use crate::layout::{Frame, Measure};

/// Monospace stand-in for the text engine (only the template pages are measured here, and they
/// are not measured at all).
struct Fake;

impl Measure for Fake {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = match e.params.get("size").map(|p| &p.value) {
            Some(ParamValue::Float(v)) => *v,
            _ => 40.0,
        };
        Some([0.0, -size * 0.9, size * 4.0, size * 0.3])
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

fn lay(csv: &str, styling: &Styling) -> Laid {
    let c = read(csv.as_bytes(), styling).unwrap();
    layout(&c, styling, FRAME, &Fake).unwrap()
}

/// The text of the layer named `name`, and whether it is shown.
fn text_of(p: &PageOut, name: &str) -> Option<(String, bool)> {
    p.layers.iter().find(|e| matches!(e.params.get("name").map(|x| &x.value), Some(ParamValue::Text(n)) if n == name)).map(|e| {
        let t = match e.params.get("text").map(|x| &x.value) {
            Some(ParamValue::Text(t)) => t.clone(),
            _ => String::new(),
        };
        (t, e.enabled)
    })
}

#[test]
fn title_page_fills_the_centred_title_template() {
    let laid = lay("Head,Body,Page Style\nan original story,THE LONG WAY HOME,Title\n", &Styling::default());
    assert_eq!(laid.pages.len(), 1);
    let p = &laid.pages[0];
    assert_eq!(p.behaviour, Behaviour::Title);
    assert_eq!(text_of(p, "Title"), Some(("THE LONG WAY HOME".into(), true)));
    assert_eq!(text_of(p, "Subtitle"), Some(("an original story".into(), true)));
    // linked to its template, so Essential Graphics shows the controls
    assert_eq!(p.meta.template.as_ref().map(|t| t.id.as_str()), Some("builtin:title-centered"));
    // a card of the built-in Title style: 150 frames, fading 18 in and out
    assert_eq!(p.duration, Tick(FRAME.rate.frame_duration().0 * 150));
    assert_eq!(p.fade_in, Tick(FRAME.rate.frame_duration().0 * 18));
    for e in &p.layers {
        let op = e.params.get("opacity").expect("faded");
        assert_eq!(op.keyframes.first().map(|k| k.value.clone()), Some(ParamValue::Float(0.0)));
        assert_eq!(op.keyframes.last().map(|k| k.time), Some(p.duration));
    }
    assert!(laid.warnings.is_empty(), "{:?}", laid.warnings);
}

#[test]
fn empty_parts_hide_their_layer() {
    let laid = lay("Body,Page Style\nTHE END,Title\n", &Styling::default());
    let p = &laid.pages[0];
    assert_eq!(text_of(p, "Title"), Some(("THE END".into(), true)));
    assert_eq!(text_of(p, "Subtitle"), Some((String::new(), false)));
}

#[test]
fn lower_thirds_are_one_graphic_per_block() {
    let csv = "Head,Body,Page Style\n\
               Historian,Anna Kovács,Alsó sáv\n\
               Archivist,Ben Ode,\n\
               ,Chris Day,\n\
               Directed by,Name Surname,Scroll\n";
    let c = read(csv.as_bytes(), &Styling::default()).unwrap();
    // the three people are three pages; the roll stays one
    assert_eq!(c.pages.len(), 4);
    assert!(c.pages[..3].iter().all(|p| p.blocks.len() == 1 && p.style == "Alsó sáv"));
    assert_eq!(c.pages[1].row, 3);
    let laid = layout(&c, &Styling::default(), FRAME, &Fake).unwrap();
    let kinds: Vec<Behaviour> = laid.pages.iter().map(|p| p.behaviour).collect();
    assert_eq!(kinds, vec![Behaviour::LowerThird, Behaviour::LowerThird, Behaviour::LowerThird, Behaviour::Scroll]);
    assert_eq!(text_of(&laid.pages[0], "Name"), Some(("Anna Kovács".into(), true)));
    assert_eq!(text_of(&laid.pages[0], "Role"), Some(("Historian".into(), true)));
    assert_eq!(text_of(&laid.pages[2], "Role"), Some((String::new(), false)));
    // a second apart (25 frames)
    assert_eq!(laid.pages[0].gap_after, Tick(FRAME.rate.frame_duration().0 * 25));
    // the slab's own 92 % opacity is kept under the fade
    let slab = laid.pages[0].layers.iter().find(|e| layer_uid(e) == 1).unwrap();
    let top = slab.params["opacity"].keyframes.iter().filter_map(|k| if let ParamValue::Float(v) = k.value { Some(v) } else { None }).fold(0.0, f64::max);
    assert!((top - 92.0).abs() < 1e-9, "{top}");
    assert_eq!(laid.pages[0].name, "Lower Third – Slab 1: Anna Kovács");
}

#[test]
fn fields_choose_the_controls_and_bad_ones_warn() {
    let styling = Styling::from_json(
        r#"{"pageStyles":[{"name":"Names","behaviour":"lowerThird","template":"Lower Third – Rule",
            "fields":{"name":"head","role":"body","colour":"tail"}}]}"#,
    )
    .unwrap();
    let laid = lay("Head,Body,Page Style\nAnna,Editor,Names\n", &styling);
    let p = &laid.pages[0];
    assert_eq!(p.meta.template.as_ref().map(|t| t.id.as_str()), Some("builtin:lower-third-rule"));
    assert_eq!(text_of(p, "Name"), Some(("Anna".into(), true)));
    assert_eq!(text_of(p, "Role"), Some(("Editor".into(), true)));
    assert_eq!(laid.warnings.len(), 1, "{:?}", laid.warnings);
    assert!(laid.warnings[0].message.contains("template field `colour` → `tail` is not used"), "{:?}", laid.warnings);
}

#[test]
fn a_missing_template_falls_back_and_extra_text_warns() {
    let styling = Styling::from_json(r#"{"pageStyles":[{"name":"Part","behaviour":"chapter","template":"No Such Template"}]}"#).unwrap();
    let laid = lay("Head,Body,Tail,Page Style\nPart One,The Beginning,1971,Part\n", &styling);
    let p = &laid.pages[0];
    assert_eq!(p.behaviour, Behaviour::Chapter);
    assert_eq!(p.meta.template.as_ref().map(|t| t.id.as_str()), Some("builtin:title-boxed"));
    // the boxed title has one text field: the body
    assert_eq!(text_of(p, "Title"), Some(("The Beginning".into(), true)));
    let msgs: Vec<&str> = laid.warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(msgs.len(), 2, "{msgs:?}");
    assert!(msgs[0].contains("graphics template `No Such Template` was not found; `Title – Boxed` is used"), "{msgs:?}");
    assert!(msgs[1].contains("has no field for all of the block's text"), "{msgs:?}");
}

#[test]
fn keys_stay_while_the_block_is_unchanged() {
    let a = lay("Head,Body,Page Style\nHistorian,Anna,LowerThird\nArchivist,Ben,\n", &Styling::default());
    let b = lay("Head,Body,Page Style\nHistorian,Anna,LowerThird\nCurator,Ben,\n", &Styling::default());
    assert_eq!(a.pages[0].keys, b.pages[0].keys);
    assert_ne!(a.pages[1].keys, b.pages[1].keys);
    // every layer has a key, unique on the page
    let mut k: Vec<&String> = a.pages[0].keys.iter().map(|(_, k)| k).collect();
    assert_eq!(k.len(), a.pages[0].layers.len());
    k.dedup();
    assert_eq!(k.len(), a.pages[0].layers.len());
}

#[test]
fn pictures_are_left_out_with_a_note() {
    let laid = lay("Body,Page Style\n{{Pic logo.png}},Opening\n", &Styling::default());
    assert_eq!(laid.pages[0].behaviour, Behaviour::Opening);
    assert!(laid.pages[0].pictures.is_empty());
    assert!(laid.warnings.iter().any(|w| w.message.contains("pictures are left out on title, opening, chapter and lower-third pages")), "{:?}", laid.warnings);
}

#[test]
fn behaviours_read_and_write_as_json() {
    for (b, s) in
        [(Behaviour::Title, "\"title\""), (Behaviour::Opening, "\"opening\""), (Behaviour::Chapter, "\"chapter\""), (Behaviour::LowerThird, "\"lowerThird\"")]
    {
        assert_eq!(serde_json::to_string(&b).unwrap(), s);
        assert_eq!(serde_json::from_str::<Behaviour>(s).unwrap(), b);
        assert!(b.uses_template() && b.is_card() && !b.moves());
    }
    assert_eq!(serde_json::from_str::<Behaviour>("\"lower_third\"").unwrap(), Behaviour::LowerThird);
    assert!(!Behaviour::Card.uses_template() && Behaviour::Card.is_card());
    // a document keeps them: the template pages come back split the same way
    let styling = Styling::default();
    let doc = crate::doc::CreditsDoc {
        table: vec![
            vec!["Head".into(), "Body".into(), "Page Style".into()],
            vec!["A".into(), "B".into(), "LowerThird".into()],
            vec!["C".into(), "D".into(), String::new()],
        ],
        styling,
        ..Default::default()
    };
    assert_eq!(doc.credits().unwrap().pages.len(), 2);
}
