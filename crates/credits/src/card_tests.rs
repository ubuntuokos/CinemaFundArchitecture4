//! C15: a card spine's point (`Spine Position` `150 -50`, `0 50 Below`, `0 50 Above`).

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::FrameRate;

use crate::layout::{Frame, Measure};
use crate::sheet::{CardAnchor, CardPoint};
use crate::{Styling, layout, read};

/// 0.5 em per character, 1.2 em lines, centred.
struct M;
impl Measure for M {
    fn bounds(&self, e: &EffectInstance) -> Option<[f64; 4]> {
        let size = match e.params.get("size").map(|p| &p.value) {
            Some(ParamValue::Float(v)) => *v,
            _ => 40.0,
        };
        let n = match e.params.get("text").map(|p| &p.value) {
            Some(ParamValue::Text(t)) => t.chars().count(),
            _ => 0,
        };
        let w = n as f64 * size * 0.5;
        Some([-w / 2.0, -size * 0.9, w / 2.0, size * 0.3])
    }
}

const FRAME: Frame = Frame { width: 1920, height: 1080, rate: FrameRate::FPS_25 };

/// A block's top on the frame, by its first row.
fn top_of(p: &crate::PageOut, row: usize) -> f64 {
    p.content_top + p.block_tops.iter().find(|(r, _)| *r == row).map(|x| x.1).unwrap()
}

#[test]
fn spine_positions_are_read_with_a_card_point() {
    let st = Styling::default();
    let sheet = "Body,Spine Position\nA,-150\nB,150 -50\nC,0 50 Below\nD,0 -20 Above\nE,10px 20px alatta\nF,0 Above\nG,450 parallel\nH,\"1,5 -2,5\"\n";
    let c = read(sheet.as_bytes(), &st).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let pts: Vec<Option<CardPoint>> = c.pages[0].blocks.iter().map(|b| b.card_point).collect();
    let p = |y: f64, anchor| Some(CardPoint { y, anchor });
    assert_eq!(
        pts,
        vec![
            None,
            p(-50.0, CardAnchor::Middle),
            p(50.0, CardAnchor::Below),
            p(-20.0, CardAnchor::Above),
            p(20.0, CardAnchor::Below),
            p(0.0, CardAnchor::Above),
            None,
            p(-2.5, CardAnchor::Middle)
        ]
    );
    let xs: Vec<f64> = c.pages[0].blocks.iter().map(|b| b.spine).collect();
    assert_eq!(xs, vec![-150.0, 150.0, 0.0, 0.0, 10.0, 0.0, 450.0, 1.5]);
    assert!(c.pages[0].blocks[6].parallel);
    // not a spine position: each is a warning, and the spine stays as it was
    for bad in ["1 2 parallel", "1 2 3", "Below", "0 1e308", "0 x Below", "0 50 Sideways"] {
        let c = read(format!("Body,Spine Position\nA,\"{bad}\"\n").as_bytes(), &st).unwrap();
        assert_eq!(c.warnings.len(), 1, "{bad}");
        assert_eq!((c.pages[0].blocks[0].spine, c.pages[0].blocks[0].card_point), (0.0, None), "{bad}");
    }
}

#[test]
fn each_card_spine_stands_at_its_own_point() {
    let st = Styling::default();
    // the guide's example: a lefty centred on the middle line, a top-righty 50 px higher, and two
    // blocks starting 50 px below the middle
    let sheet = "Head,Body,Spine Position,Page Style\n\
                 I'm a lefty!,A,-150,Card\n\
                 I'm a top-righty!,B,150 -50,\n\
                 I'm below y'all!,C,0 50 Below,\n\
                 Me too!,D,,\n";
    let c = read(sheet.as_bytes(), &st).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    let p = &l.pages[0];
    assert_eq!(p.spine_points, vec![(810.0, 540.0), (1110.0, 490.0), (960.0, 590.0)]);
    // lefty and top-righty are one block each, the same height: centred 50 px apart
    let (lefty, righty, below) = (top_of(p, 2), top_of(p, 3), top_of(p, 4));
    assert!((lefty - righty - 50.0).abs() < 1e-6, "{lefty} {righty}");
    assert!(lefty < 540.0 && 540.0 - lefty < 60.0);
    // the third spine starts at its point and runs down
    assert!((below - 590.0).abs() < 1e-6, "{below}");
    assert!(top_of(p, 5) > below);
    // the content reaches from the highest to the lowest spine
    let bottom = p.content_top + p.content_height;
    assert!((p.content_top - righty).abs() < 1e-6 && bottom > top_of(p, 5));
    assert_eq!(p.spines, vec![810.0, 960.0, 1110.0]);

    // `Above`: the spine ends at its point
    let c = read("Body,Spine Position,Page Style\nA,0 -100 Above,Card\nB,,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let p = &l.pages[0];
    assert!((p.content_top + p.content_height - 440.0).abs() < 1e-6);
    assert_eq!(p.spine_points, vec![(960.0, 440.0)]);
}

#[test]
fn a_card_spine_keeps_its_parallel_columns_and_scroll_pages_ignore_the_point() {
    let st = Styling::default();
    // a parallel column belongs to the spine before it: both start at the point
    let sheet = "Head,Body,Spine Position,Page Style\nCamera,Anna,-300 100 Below,Card\n,Béla,,\nSound,Cili,300 parallel,\n";
    let c = read(sheet.as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let p = &l.pages[0];
    assert_eq!(p.spine_points, vec![(660.0, 640.0)]);
    assert!((top_of(p, 2) - 640.0).abs() < 1e-6 && (top_of(p, 4) - 640.0).abs() < 1e-6);

    // a scroll page has no card points: a warning, laid out as without the vertical part
    let with = read("Body,Spine Position\nA,-150 40 Below\nB,\n".as_bytes(), &st).unwrap();
    let without = read("Body,Spine Position\nA,-150\nB,\n".as_bytes(), &st).unwrap();
    let (lw, lo) = (layout(&with, &st, FRAME, &M).unwrap(), layout(&without, &st, FRAME, &M).unwrap());
    assert_eq!(lw.warnings.len(), 1, "{:?}", lw.warnings);
    assert_eq!(lw.warnings[0].message, "page 1: a spine's vertical position works on card pages only; it is ignored");
    assert_eq!(lw.warnings[0].row, 2);
    assert_eq!(lw.pages[0].block_tops, lo.pages[0].block_tops);
    assert!(lw.pages[0].spine_points.is_empty());
}

#[test]
fn a_card_without_points_is_laid_as_before_and_notes_come_once() {
    let st = Styling::default();
    // one spine: centred, as before
    let c = read("Body,Page Style\nA,Card\nB,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    let p = &l.pages[0];
    assert!((p.content_top - (1080.0 - p.content_height) / 2.0).abs() < 1e-6);
    assert_eq!(p.spine_points, vec![(960.0, 540.0)]);
    // a missing content style on two spines is one note
    let c = read("Body,Spine Position,Content Style,Page Style\nA,-300,Nope,Card\nB,300,,\n".as_bytes(), &st).unwrap();
    let l = layout(&c, &st, FRAME, &M).unwrap();
    assert_eq!(l.warnings.iter().filter(|w| w.message.contains("Nope")).count(), 1, "{:?}", l.warnings);
}

#[test]
fn far_card_points_do_not_crash() {
    let st = Styling::default();
    for (cell, other) in [("0 99999 Below", "0 -99999"), ("0 -99999 Above", "0 99999"), ("-99999 99999", "0 -99999 Above"), ("99999 0 Below", "0 99999")] {
        let c = read(format!("Body,Spine Position,Page Style\nA,{cell},Card\nB,{other},\n").as_bytes(), &st).unwrap();
        let l = layout(&c, &st, FRAME, &M).unwrap();
        let p = &l.pages[0];
        assert!(p.content_top.is_finite() && p.content_height.is_finite() && p.content_height >= 0.0);
        // far apart spines make the card taller than the frame: the usual note
        assert!(l.warnings.iter().any(|w| w.message.contains("taller than the frame")), "{cell}");
    }
}
