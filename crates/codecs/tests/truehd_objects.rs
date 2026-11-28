//! Dolby TrueHD Atmos objects from a Matroska file (`MediaSource::object_audio`, binaural monitor
//! plan B-03). Needs a real Atmos file, which is never committed: set `CFA4_ATMOS_MKV` to one
//! (e.g. the Altitude16 Dolby Atmos sample); without it the test says so and passes.

use std::sync::Arc;

use cfa4_media::MediaSource;

fn source() -> Option<cfa4_codecs::mkv::MkvSource> {
    let Ok(path) = std::env::var("CFA4_ATMOS_MKV") else {
        eprintln!("CFA4_ATMOS_MKV is not set: skipped");
        return None;
    };
    let bytes: Arc<[u8]> = std::fs::read(&path).expect("read the Atmos file").into();
    Some(cfa4_codecs::mkv::MkvSource::open("atmos.mkv", bytes).expect("open the Atmos file"))
}

#[test]
fn atmos_objects_play_on_and_after_a_seek_alike() {
    let Some(src) = source() else { return };
    let rate = src.info().audio().expect("audio").sample_rate;
    assert_eq!(rate, 48_000);
    let at = 10 * i64::from(rate);
    let n = 9_600;
    // straight through from the start of the stream: 0 → 10 s, then two blocks
    let mut t = 0;
    while t < at {
        let b = src.object_audio(t, 48_000).expect("objects").expect("decode");
        assert_eq!(b.start, t);
        t += 48_000;
    }
    let a = src.object_audio(at, n).expect("objects").expect("decode");
    let a2 = src.object_audio(at + n as i64, n).expect("objects").expect("decode");
    assert!(a.objects.len() >= 10, "an Atmos programme has beds and objects: {}", a.objects.len());
    assert!(a.objects.iter().all(|o| o.len() == n));
    assert!(a.objects.iter().any(|o| o.iter().any(|x| x.abs() > 1e-4)), "sound at 10 s");
    assert!(!a.current.is_empty(), "every object has a state at 10 s");
    // a jump back to the same place decodes from an earlier major sync: the same samples
    let _ = src.object_audio(0, 480).expect("objects").expect("decode");
    let b = src.object_audio(at, n).expect("objects").expect("decode");
    assert_eq!(a.objects, b.objects, "the same samples after a seek");
    assert_eq!(a.events, b.events, "the same updates after a seek");
    let current = |e: &[cfa4_truehd::render::Event]| e.iter().map(|e| (e.object, e.state)).collect::<Vec<_>>();
    assert_eq!(current(&a.current), current(&b.current), "the same states after a seek");
    let b2 = src.object_audio(at + n as i64, n).expect("objects").expect("decode");
    assert_eq!(a2.objects, b2.objects);
    // updates fall within their block
    for e in a.events.iter().chain(&a2.events) {
        assert!(e.offset >= at as u64 && e.offset < (at + 2 * n as i64) as u64);
    }
    println!("{} objects, {} updates in 0.4 s at 10 s", a.objects.len(), a.events.len() + a2.events.len());
}
