//! C6: credits templates.

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::layout::{Frame, Measure};
use crate::template::{BUILTIN_LICENSE, Category, CreditsTemplate, builtin_templates, slug};
use crate::{layout, read};

struct M;
impl Measure for M {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = match e.params.get("size").map(|p| &p.value) {
            Some(ParamValue::Float(v)) => *v,
            _ => 40.0,
        };
        Some([-100.0, -size * 0.9, 100.0, size * 0.3])
    }
    fn picture_size(&self, _file: &str) -> Option<(f64, f64)> {
        Some((300.0, 400.0))
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

#[test]
fn builtin_templates_are_original_licensed_and_lay_out_their_samples() {
    let all = builtin_templates();
    assert!(all.len() >= 4);
    let mut ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all.len(), "ids are unique");
    for t in &all {
        assert!(t.id.starts_with("cfa4-") && t.category == Category::Builtin);
        assert_eq!(t.license, BUILTIN_LICENSE);
        assert!(!t.description.is_empty());
        // the sample uses the template's own styles, and lays out cleanly
        let csv = t.sample_csv();
        let c = read(csv.as_bytes(), &t.styling).unwrap();
        let l = layout(&c, &t.styling, FRAME, &M).unwrap();
        assert!(c.warnings.is_empty() && l.warnings.is_empty(), "{}: {:?} {:?}", t.id, c.warnings, l.warnings);
        // and survives a round trip
        let back = CreditsTemplate::from_bytes(t.to_json().as_bytes()).unwrap();
        assert_eq!(&back, t);
    }
}

#[test]
fn templates_refuse_what_they_cannot_read() {
    assert!(CreditsTemplate::from_bytes(b"{}").is_err());
    assert!(CreditsTemplate::from_bytes(br#"{"format":"cfa4.credits-template","version":99,"name":"x"}"#).is_err());
    assert!(CreditsTemplate::from_bytes(br#"{"format":"cfa4.credits-template","version":1,"name":"  "}"#).is_err());
    assert!(CreditsTemplate::from_bytes(br#"{"format":"cfa4.credits","name":"x"}"#).is_err());
    assert!(CreditsTemplate::from_bytes(&vec![b' '; 9 << 20]).is_err());
    let ok = CreditsTemplate::from_bytes(br#"{"format":"cfa4.credits-template","version":1,"name":"Mine","future":true}"#).unwrap();
    assert_eq!(ok.category, Category::User);
}

#[test]
fn slugs_and_sample_quoting() {
    assert_eq!(slug("Saját Stáblista – 2026!"), "saját-stáblista-2026");
    assert_eq!(slug("   "), "template");
    assert!(slug(&"x".repeat(500)).len() <= 66);
    let t = CreditsTemplate { sample: vec![vec!["Head".into(), "Body".into()], vec!["a,b".into(), "say \"hi\"\nthere".into()]], ..Default::default() };
    let csv = t.sample_csv();
    let rows = crate::table::read_table(csv.as_bytes(), crate::table::Limits::default()).unwrap();
    assert_eq!(rows[1], vec!["a,b".to_string(), "say \"hi\"\nthere".to_string()]);
}
