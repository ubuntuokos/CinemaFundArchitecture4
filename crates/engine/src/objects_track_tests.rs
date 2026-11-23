use serde_json::json;

use crate::Session;
use crate::masks_tests::{tracking_session, truth};
use cfa4_geom::{Rect, Vec2};
use cfa4_project::ItemId;
use cfa4_project::object::{ObjectId, ObjectSample, ObjectSource, ObjectTrack};
use cfa4_time::{FrameRate, Tick};

const RATE: FrameRate = FrameRate::FPS_24;

/// An object with one manual box around the disc at `frame`; returns (item, object id).
fn with_object(s: &mut Session, frame: i64, half: f64) -> (ItemId, u64) {
    let item = s.active_sequence().unwrap().video_tracks[0].items[0].item;
    let c = truth(frame as f64).apply(Vec2::ZERO);
    let id = s
        .edit("Create Object", move |pr, _| {
            let id = ObjectId(pr.alloc_id());
            let mut t = ObjectTrack::new(id, "Disc", ObjectSource::Manual);
            t.insert_sample(ObjectSample {
                time: RATE.tick_of(frame),
                bbox: Rect::new(c.x - half, c.y - half, 2.0 * half, 2.0 * half),
                score: 1.0,
                mask: None,
            })
            .unwrap();
            pr.objects.entry(item).or_default().push(t);
            Ok(id.0)
        })
        .unwrap();
    (item, id)
}

fn centre(r: &Rect) -> Vec2 {
    Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn samples(s: &Session, id: u64) -> Vec<ObjectSample> {
    s.project.objects.values().flatten().find(|t| t.id.0 == id).unwrap().samples().to_vec()
}

#[test]
fn a_box_follows_known_motion_forward_and_backward() {
    for tracker in ["points", "correlation"] {
        let (mut s, _) = tracking_session();
        let (_, id) = with_object(&mut s, 10, 40.0);
        let r = s.execute("objects.track", json!({"id": id, "frames": 10, "wait": true, "tracker": tracker})).unwrap();
        assert_eq!(r["frames"], 10);
        assert!(s.object_jobs.is_empty());
        s.execute("objects.track", json!({"id": id, "direction": "backward", "frames": 6, "mediaTime": RATE.tick_of(10).0, "wait": true, "tracker": tracker}))
            .unwrap();
        let v = samples(&s, id);
        assert_eq!(v.len(), 17, "frames 4..=20 with {tracker}");
        let mut worst = 0.0f64;
        for x in &v {
            let f = RATE.frame_at(x.time) as f64;
            worst = worst.max((centre(&x.bbox) - truth(f).apply(Vec2::ZERO)).length());
        }
        eprintln!("object tracking ({tracker}): max centre error {worst:.3} px");
        assert!(worst < 3.0, "{tracker}: centre error {worst} px");
        let h = s.execute("history.list", json!({})).unwrap();
        let undo: Vec<&str> = h["undo"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(&undo[undo.len() - 2..], ["Track Object", "Track Object"], "{undo:?}");
        s.undo();
        s.undo();
        assert_eq!(samples(&s, id).len(), 1);
    }
}

#[test]
fn tracking_runs_in_the_background_and_can_be_cancelled() {
    let (mut s, _) = tracking_session();
    let (_, id) = with_object(&mut s, 10, 40.0);
    let r = s.execute("objects.track", json!({"id": id})).unwrap();
    let job = r["job"].as_u64().unwrap();
    assert_eq!(r["frames"], 37);
    assert!(s.execute("objects.track", json!({"id": id})).is_err(), "one job per object");
    s.execute("jobs.cancel", json!({"job": job})).unwrap();
    let t0 = std::time::Instant::now();
    while !s.object_jobs.is_empty() && t0.elapsed().as_secs() < 60 {
        s.poll_persistence();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(s.object_jobs.is_empty());
}

#[test]
fn hostile_parameters_are_refused() {
    let (mut s, _) = tracking_session();
    let (_, id) = with_object(&mut s, 10, 40.0);
    for p in [
        json!({}),
        json!({"id": "x"}),
        json!({"id": 424242}),
        json!({"id": id, "direction": "sideways"}),
        json!({"id": id, "method": "spin"}),
        json!({"id": id, "tracker": "magic"}),
        json!({"id": id, "frames": -1}),
        json!({"id": id, "frames": "all"}),
        json!({"id": id, "frames": 0}),
        json!({"id": id, "mediaTime": "now"}),
    ] {
        assert!(s.execute("objects.track", p.clone()).is_err(), "{p}");
    }
    assert!(s.object_jobs.is_empty());
    assert_eq!(samples(&s, id).len(), 1);
    // an object with no box cannot start
    let item = s.active_sequence().unwrap().video_tracks[0].items[0].item;
    let empty = s
        .edit("x", move |pr, _| {
            let id = ObjectId(pr.alloc_id());
            pr.objects.entry(item).or_default().push(ObjectTrack::new(id, "Empty", ObjectSource::Manual));
            Ok(id.0)
        })
        .unwrap();
    assert!(s.execute("objects.track", json!({"id": empty})).is_err());
    let _ = Tick::ZERO;
}
