//! Contract tests: they pin the shapes the spec (C0 v0.1) names. A change that breaks one of them
//! is a contract change: update the spec and [`crate::CONTRACT_VERSION`] together.

use cfa4_color::ColorInfo;
use cfa4_time::Tick;

use crate::*;

#[test]
fn video_frame_field_list_is_pinned() {
    // Exhaustive destructuring: adding, removing or renaming a field of `VideoFrame` stops compiling here.
    fn fields(f: VideoFrame) {
        let VideoFrame { width: _, height: _, data: _, color: _, par: _, pts: _, layout: _ } = f;
    }
    let _ = fields;
    let _: Option<ColorInfo> = None;
    let side = FrameSideData::default();
    let FrameSideData { source: _, rotation: _, hdr: _, residency: _ } = side;
}

#[test]
fn queue_states_map_onto_the_plans_job_states() {
    use RenderState as R;
    use State as S;
    let table = [
        (S::Scheduled, R::Queued),
        (S::Queued, R::Queued),
        (S::Waiting, R::Queued),
        (S::Blocked, R::Queued),
        (S::Running, R::Running),
        (S::Paused, R::Paused),
        (S::Cancelling, R::Cancelling),
        (S::Succeeded, R::Completed),
        (S::Failed, R::Failed),
        (S::Cancelled, R::Cancelled),
    ];
    for (s, r) in table {
        assert_eq!(RenderState::of(s), r, "{s:?}");
    }
    for r in [R::Completed, R::Failed, R::Cancelled] {
        assert!(r.is_terminal());
    }
    assert!(!R::Cancelling.is_terminal());
}

#[test]
fn the_conversion_matrix_is_pinned_for_all_25_pairs() {
    use PixelFormat::*;
    for from in PixelFormat::ALL {
        for to in PixelFormat::ALL {
            let got = plan(from, to);
            if from == to {
                let c = got.unwrap_or_else(|e| panic!("{from:?}->{to:?}: {e:?}"));
                assert_eq!((c.cost, c.lossy), (Cost::Share, false), "{from:?}->{to:?}");
            } else if to == RgbaF32 {
                let c = got.unwrap_or_else(|e| panic!("{from:?}->{to:?}: {e:?}"));
                assert_eq!((c.cost, c.lossy), (Cost::Convert, false), "{from:?}->{to:?}");
            } else if to == Rgba8 {
                let c = got.unwrap_or_else(|e| panic!("{from:?}->{to:?}: {e:?}"));
                assert_eq!((c.cost, c.lossy), (Cost::Convert, true), "{from:?}->{to:?}");
            } else {
                assert_eq!(got, Err(Unsupported { from, to }), "{from:?}->{to:?} must be an explicit error");
            }
        }
    }
}

#[test]
fn render_events_round_trip_and_stay_consistent() {
    let events = [RenderEvent::progress("job-1", 10, 48, Some(1500), "rendering"), RenderEvent::fallback("job-1", "gpu", "cpu", "no VRAM")];
    for e in events {
        assert!(e.is_consistent());
        let s = serde_json::to_string(&e).unwrap_or_default();
        assert_eq!(serde_json::from_str::<RenderEvent>(&s).ok(), Some(e));
    }
    let bad = RenderEvent { kind: RenderEventKind::Fallback, ..RenderEvent::progress("j", 1, 2, None, "") };
    assert!(!bad.is_consistent());
}

#[test]
fn version_and_capability_defaults_are_pinned() {
    assert_eq!(CONTRACT_VERSION, ContractVersion { major: 0, minor: 2 });
    assert_eq!(Tick::ZERO.0, 0);
    let cap = RenderCapability { format: PixelFormat::Rgba8, bit_depth: 8, place: Place::Cpu, supported: false, reason: "no".into() };
    assert_eq!(decide(&cap, Context::Export), Support::FailClosed);
}

/// ADR "render worker output index" (0.2): the progress total is an estimate that may move while a job
/// runs (an IVTC or a 2x deinterlace changes the frame count); an event stays well formed and the
/// fraction stays bounded in every case.
#[test]
fn the_progress_total_may_move_while_a_job_runs() {
    use crate::RenderEvent;
    for (done, total) in [(4, 8), (4, 10), (9, 8), (0, 0)] {
        let e = RenderEvent::progress("j", done, total, None, "");
        assert!(e.is_consistent(), "{done}/{total}");
        let f = e.progress.map(|p| p.fraction()).unwrap_or(-1.0);
        assert!((0.0..=1.0).contains(&f), "{done}/{total}: {f}");
    }
}
