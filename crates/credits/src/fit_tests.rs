use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick};

use crate::fit::{Fit, Music, beat_period, fit_layout, suggestions, total_length};
use crate::layout::{Frame, Measure};
use crate::{Behaviour, Styling, layout, read};

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
const SHEET: &str = "Body,Page Style,Vertical Gap\nTITLE,Card,\nA,Scroll,\nB,,4\nC,,4\nEND,Card,\n";

fn secs(s: f64) -> i64 {
    (s * TICKS_PER_SECOND as f64) as i64
}

#[test]
fn fits_by_stretching_the_rolls_and_keeps_the_cards() {
    let c = read(SHEET.as_bytes(), &Styling::default()).unwrap();
    let natural = layout(&c, &Styling::default(), FRAME, &M).unwrap();
    let cards: Vec<Tick> = natural.pages.iter().filter(|p| p.behaviour == Behaviour::Card).map(|p| p.duration).collect();
    // natural: two 4.8 s cards and a roll of about 20 s (29.4 s); longer and shorter
    for target in [32.0, 27.0] {
        let (_, l, r) = fit_layout(&c, &Styling::default(), FRAME, &M, &Fit { total: secs(target), ..Default::default() }).unwrap();
        // exact to the frame
        assert_eq!(total_length(&l).0, secs(target), "{target}");
        assert_eq!(r.result, secs(target));
        assert_eq!(r.natural, total_length(&natural).0);
        let now: Vec<Tick> = l.pages.iter().filter(|p| p.behaviour == Behaviour::Card).map(|p| p.duration).collect();
        assert_eq!(now, cards, "cards keep their length");
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    }
}

#[test]
fn too_short_or_nothing_to_stretch_is_a_warning_not_a_failure() {
    let c = read(SHEET.as_bytes(), &Styling::default()).unwrap();
    let (_, l, r) = fit_layout(&c, &Styling::default(), FRAME, &M, &Fit { total: secs(3.0), ..Default::default() }).unwrap();
    assert!(r.result > secs(3.0) && !l.warnings.is_empty());
    let cards_only = read(b"Body,Page Style\nA,Card\n", &Styling::default()).unwrap();
    let (_, l, _) = fit_layout(&cards_only, &Styling::default(), FRAME, &M, &Fit { total: secs(30.0), ..Default::default() }).unwrap();
    assert_eq!(l.warnings.len(), 1);
}

#[test]
fn cards_on_the_beat() {
    let c = read(SHEET.as_bytes(), &Styling::default()).unwrap();
    // 120 bpm: half a second per beat; cards are 120 frames = 4.8 s → 10 beats = 5 s
    let beat = secs(0.5);
    let (_, l, _) = fit_layout(&c, &Styling::default(), FRAME, &M, &Fit { total: secs(30.0), card_beat: beat, ..Default::default() }).unwrap();
    for p in l.pages.iter().filter(|p| p.behaviour == Behaviour::Card) {
        assert_eq!(p.duration.0 % beat, 0, "{:?}", p.duration);
        assert_eq!(p.duration.0, secs(5.0));
    }
    assert_eq!(total_length(&l).0, secs(30.0));
}

#[test]
fn suggestions_and_beat_period() {
    let s = |x: f64| Tick(secs(x));
    let beats: Vec<Tick> = (0..100).map(|i| s(0.2 + i as f64 * 0.5)).collect();
    let m = Music { duration: s(60.0), sound_end: Some(s(52.3)), beats: beats.clone() };
    let v = suggestions(&m);
    let ids: Vec<&str> = v.iter().map(|x| x.id).collect();
    assert_eq!(ids, vec!["musicLength", "soundEnd", "lastBar"]);
    assert_eq!(v[1].total, secs(52.3));
    // bars start every fourth beat: 0.2, 2.2, …; the 100 beats end at 49.7, so 48.2 is the last
    assert_eq!(v[2].total, secs(48.2));
    assert_eq!(beat_period(&beats), Some(s(0.5)));
    assert_eq!(beat_period(&[]), None);
    // music that rings to its end and has no beat: only its length
    let v = suggestions(&Music { duration: s(30.0), sound_end: Some(s(29.9)), beats: vec![] });
    assert_eq!(v.len(), 1);
}

#[test]
fn hostile_fit_values_never_panic() {
    let c = read(SHEET.as_bytes(), &Styling::default()).unwrap();
    for total in [i64::MIN, -1, 0, 1, i64::MAX] {
        for card_beat in [i64::MIN, -5, 0, 1, i64::MAX] {
            let _ = fit_layout(&c, &Styling::default(), FRAME, &M, &Fit { total, card_beat, ..Default::default() });
        }
    }
}
