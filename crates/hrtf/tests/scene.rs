// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! The scene renderer (B-02): moving objects without clicks, beds and objects where they belong,
//! the headphone modes, size, the room, and the TrueHD Atmos and IAB adapters (a real Atmos stream
//! when `CFA4_ATMOS_SAMPLE` names one).

use std::f32::consts::TAU;
use std::sync::Arc;

use cfa4_hrtf::atmos::{IAB_BED_FEEDS, IabScene, schedule_truehd, truehd_state};
use cfa4_hrtf::{HeadphoneMode, HrtfSet, Placement, SceneConfig, SceneRenderer, SceneState, Speaker, SpeakerLayout, VirtualSpeakers, VirtualSpeakersConfig};

const RATE: u32 = 48_000;

fn hrtf() -> Arc<HrtfSet> {
    Arc::new(HrtfSet::builtin(RATE).unwrap())
}

fn sine(n: usize, freq: f32, amp: f32) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * freq * TAU / RATE as f32).sin() * amp).collect()
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum()
}

fn db(a: f64, b: f64) -> f64 {
    10.0 * (a / b).log10()
}

fn room(position: [f32; 3], size: f32, mode: HeadphoneMode) -> SceneState {
    SceneState { placement: Placement::Room { position, size }, gain: 1.0, mode }
}

fn render(scene: &mut SceneRenderer, objects: &[&[f32]], frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    let mut done = 0;
    while done < frames {
        let n = block.min(frames - done);
        let slices: Vec<&[f32]> = objects.iter().map(|o| o.get(done..(done + n).min(o.len())).unwrap_or(&[])).collect();
        scene.render(done as u64, &slices, &mut l[done..done + n], &mut r[done..done + n]);
        done += n;
    }
    (l, r)
}

/// The largest second difference relative to the peak: a click stands out against a smooth tone.
fn roughness(x: &[f32]) -> f32 {
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let d2 = x.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()).fold(0.0f32, f32::max);
    d2 / peak
}

#[test]
fn an_object_circling_the_listener_has_no_clicks() {
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig { room: false, ..SceneConfig::default() }).unwrap();
    let frames = 2 * RATE as usize;
    let x = sine(frames, 300.0, 0.5);
    // a full circle around the room's walls in 40 updates, each ramping to the next point
    let steps = 40u64;
    let ramp = frames as u64 / steps;
    for k in 0..=steps {
        let a = k as f32 / steps as f32 * TAU;
        let p = [0.5 - 0.5 * a.sin(), 0.5 - 0.5 * a.cos(), 0.3];
        scene.schedule(k * ramp, 0, room(p, 0.0, HeadphoneMode::Near), ramp).unwrap();
    }
    let (l, r) = render(&mut scene, &[&x], frames, 480);
    // a 300 Hz sine's second difference is 0.0015 of its peak; HRTF colouring raises it a little
    for (ear, y) in [("left", &l), ("right", &r)] {
        let rough = roughness(&y[2_000..]);
        assert!(rough < 0.02, "{ear} ear rough {rough}");
    }
    assert!(energy(&l) > 0.0 && energy(&r) > 0.0);
}

#[test]
fn objects_play_from_their_side() {
    let x = sine(RATE as usize / 2, 1_000.0, 0.3);
    let side = |p: [f32; 3]| {
        let mut scene = SceneRenderer::new(hrtf(), SceneConfig { room: false, ..SceneConfig::default() }).unwrap();
        scene.schedule(0, 0, room(p, 0.0, HeadphoneMode::Near), 0).unwrap();
        let (l, r) = render(&mut scene, &[&x], x.len(), 1024);
        db(energy(&l), energy(&r))
    };
    let left = side([0.0, 0.5, 0.0]);
    let right = side([1.0, 0.5, 0.0]);
    assert!(left > 6.0, "left object: {left} dB");
    assert!(right < -6.0, "right object: {right} dB");
}

#[test]
fn a_bed_channel_sounds_like_its_virtual_loudspeaker() {
    let x = sine(RATE as usize / 2, 700.0, 0.3);
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig { room: false, ..SceneConfig::default() }).unwrap();
    scene.schedule(0, 0, SceneState { placement: Placement::Speaker(Speaker::Lss), gain: 1.0, mode: HeadphoneMode::Near }, 0).unwrap();
    let (l, r) = render(&mut scene, &[&x], x.len(), 512);
    let mut vs = VirtualSpeakers::new(hrtf(), SpeakerLayout(vec![Speaker::Lss]), VirtualSpeakersConfig::default()).unwrap();
    let (mut vl, mut vr) = (vec![0.0; x.len()], vec![0.0; x.len()]);
    vs.process(&[&x], &mut vl, &mut vr);
    for (a, b) in [(&l, &vl), (&r, &vr)] {
        let d = db(energy(a), energy(b)).abs();
        assert!(d < 0.5, "scene bed vs virtual loudspeaker: {d} dB");
    }
}

#[test]
fn the_lfe_reaches_both_ears_low_passed() {
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig::default()).unwrap();
    scene.schedule(0, 0, SceneState { placement: Placement::Speaker(Speaker::Lfe), gain: 1.0, mode: HeadphoneMode::Mid }, 0).unwrap();
    let low = sine(RATE as usize / 2, 40.0, 0.5);
    let (l, r) = render(&mut scene, &[&low], low.len(), 1024);
    assert_eq!(l, r);
    // -10 dB by default
    let d = db(energy(&l[4_800..]), energy(&low[4_800..]));
    assert!((d + 10.0).abs() < 0.5, "LFE level {d} dB");
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig::default()).unwrap();
    scene.schedule(0, 0, SceneState { placement: Placement::Speaker(Speaker::Lfe), gain: 1.0, mode: HeadphoneMode::Mid }, 0).unwrap();
    let high = sine(RATE as usize / 2, 2_000.0, 0.5);
    let (l, _) = render(&mut scene, &[&high], high.len(), 1024);
    assert!(db(energy(&l), energy(&high)) < -40.0);
}

#[test]
fn off_mode_pans_without_hrtf() {
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig::default()).unwrap();
    scene.schedule(0, 0, room([0.0, 0.5, 0.0], 0.0, HeadphoneMode::Off), 0).unwrap();
    let x = sine(4_800, 500.0, 0.5);
    let (l, r) = render(&mut scene, &[&x], x.len(), 256);
    assert!(r.iter().all(|v| v.abs() < 1e-6), "a hard-left object in plain stereo leaves the right ear silent");
    assert!(l.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));
}

#[test]
fn the_room_adds_a_tail_and_can_be_turned_off() {
    let burst = sine(4_800, 800.0, 0.5);
    let tail = |mode: HeadphoneMode, room_on: bool| {
        let mut scene = SceneRenderer::new(hrtf(), SceneConfig { room: room_on, ..SceneConfig::default() }).unwrap();
        scene.schedule(0, 0, room([0.3, 0.2, 0.0], 0.0, mode), 0).unwrap();
        let (l, r) = render(&mut scene, &[&burst], RATE as usize / 2, 512);
        // after the burst and the HRTF's own length
        energy(&l[6_000..]) + energy(&r[6_000..])
    };
    let dry = tail(HeadphoneMode::Near, true);
    let mid = tail(HeadphoneMode::Mid, true);
    let far = tail(HeadphoneMode::Far, true);
    let off = tail(HeadphoneMode::Far, false);
    assert!(dry < 1e-9, "near is dry: {dry}");
    assert!(off < 1e-9, "the room turned off: {off}");
    assert!(mid > 1e-4 && far > mid, "mid {mid}, far {far}");
}

#[test]
fn size_widens_an_object() {
    let x: Vec<f32> = {
        // noise, so that the ears' correlation means something
        let mut s = 1u32;
        (0..RATE as usize / 2)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect()
    };
    let correlation = |size: f32| {
        let mut scene = SceneRenderer::new(hrtf(), SceneConfig { room: false, ..SceneConfig::default() }).unwrap();
        scene.schedule(0, 0, room([0.5, 0.0, 0.0], size, HeadphoneMode::Near), 0).unwrap();
        let (l, r) = render(&mut scene, &[&x], x.len(), 1024);
        // interaural coherence: the largest normalised cross-correlation within ±1 ms
        let lag = |k: isize| -> f64 {
            let (a, b) = if k >= 0 { (&l[k as usize..], &r[..]) } else { (&l[..], &r[(-k) as usize..]) };
            a.iter().zip(b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum()
        };
        let peak = (-48..=48).map(|k| lag(k).abs()).fold(0.0, f64::max);
        (peak / (energy(&l) * energy(&r)).sqrt(), energy(&l) + energy(&r))
    };
    let (point, e_point) = correlation(0.0);
    let (wide, e_wide) = correlation(1.0);
    eprintln!("interaural coherence: point {point:.3}, size 1 {wide:.3}");
    assert!(point > 0.85, "a point straight ahead: {point}");
    assert!(wide < point - 0.2, "size 1: {wide} against {point}");
    assert!(db(e_wide, e_point).abs() < 2.0, "the level stays: {} dB", db(e_wide, e_point));
}

#[test]
fn hostile_states_and_signals_never_panic() {
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig::default()).unwrap();
    for (i, p) in [[f32::NAN; 3], [f32::INFINITY, -1e30, 5.0], [0.5, 0.5, 0.0]].into_iter().enumerate() {
        let s = SceneState { placement: Placement::Room { position: p, size: f32::NAN }, gain: f32::INFINITY, mode: HeadphoneMode::Far };
        scene.schedule(u64::MAX - i as u64, i, s, u64::MAX).unwrap();
        scene.schedule(0, i, s, 7).unwrap();
    }
    assert!(scene.schedule(0, usize::MAX, room([0.0; 3], 0.0, HeadphoneMode::Near), 0).is_err());
    let x = vec![f32::MAX; 300];
    let (mut l, mut r) = (vec![0.0; 300], vec![0.0; 299]);
    scene.render(u64::MAX - 100, &[&x, &[], &x[..5]], &mut l, &mut r);
    assert!(SceneRenderer::new(hrtf(), SceneConfig { room_half_width: 0.0, ..SceneConfig::default() }).is_err());
    assert!(SceneRenderer::new(hrtf(), SceneConfig { max_frames: 0, ..SceneConfig::default() }).is_err());
}

#[test]
fn truehd_objects_take_their_place() {
    use cfa4_truehd::render::{Event, ObjectState, Source};
    use cfa4_truehd::structs::oamd::SpeakerLabels;
    let bed = ObjectState { source: Source::Bed(SpeakerLabels::Lrh), gain: 1.0 };
    assert_eq!(truehd_state(&bed, HeadphoneMode::Mid).placement, Placement::Speaker(Speaker::Ltr));
    let point = |snap, elevation| ObjectState { source: Source::Point { position: [0.1, 0.2, 0.8], size: 0.3, zone: 0, snap, elevation }, gain: 0.5 };
    assert_eq!(truehd_state(&point(false, true), HeadphoneMode::Near).placement, Placement::Room { position: [0.1, 0.2, 0.8], size: 0.3 });
    assert_eq!(truehd_state(&point(false, false), HeadphoneMode::Near).placement, Placement::Room { position: [0.1, 0.2, 0.0], size: 0.3 });
    // snapped to the 7.1.4 top front left loudspeaker
    assert_eq!(truehd_state(&point(true, true), HeadphoneMode::Near).placement, Placement::Room { position: [0.0, 0.25, 1.0], size: 0.0 });
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig::default()).unwrap();
    let events = [Event { offset: 0, object: 0, state: bed, ramp: 0 }, Event { offset: 100, object: 1, state: point(false, true), ramp: 480 }];
    schedule_truehd(&mut scene, &events, HeadphoneMode::Mid);
    let x = sine(4_800, 440.0, 0.3);
    let (l, r) = render(&mut scene, &[&x, &x], x.len(), 512);
    assert!(energy(&l) > 0.0 && energy(&r) > 0.0);
}

#[test]
fn an_iab_frame_renders_its_beds_and_objects() {
    use cfa4_iab::{AudioData, Bed, BedChannel, Coding, Decorrelation, Description, Frame, FrameRate, Gain, Object, Pan, Snap, Spread};
    let n = FrameRate::R24.samples(false, 0);
    let tone = |freq: f32| sine(n, freq, 0.25).into_iter().map(|v| (v * 2_147_483_647.0) as i32).collect::<Vec<i32>>();
    let pan =
        |x: u16, y: u16| Pan { gain: Gain::Unity, position: [x, y, 0], snap: Snap::Off, zones: None, spread: Spread::None, decorrelation: Decorrelation::None };
    let mut pans = vec![None; FrameRate::R24.sub_blocks()];
    // hard left (code 32767 is the room's left wall), then hard right
    pans[0] = Some(pan(32767, 49151));
    let frame = |pans: Vec<Option<Pan>>, bed_audio: u32| Frame {
        version: 1,
        sample_rate: RATE,
        bit_depth: 24,
        frame_rate: FrameRate::R24,
        frame_index: 0,
        max_rendered: 2,
        beds: vec![Bed {
            meta_id: 1,
            use_case: None,
            channels: vec![BedChannel { id: 0x02, audio_id: bed_audio, gain: Gain::Unity, decorrelation: None }],
            description: Description::default(),
            beds: Vec::new(),
            remaps: Vec::new(),
        }],
        objects: vec![Object { meta_id: 7, audio_id: 2, use_case: None, pan: pans, description: Description::default(), objects: Vec::new(), zones19: None }],
        audio: vec![AudioData { id: 1, coding: Coding::Pcm, samples: tone(300.0) }, AudioData { id: 2, coding: Coding::Pcm, samples: tone(1_000.0) }],
        authoring_tool: None,
        user_data: Vec::new(),
    };
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig { room: false, ..SceneConfig::default() }).unwrap();
    let mut iab = IabScene::new(HeadphoneMode::Near);
    // the object alone, at the left wall
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    iab.render(&mut scene, &frame(pans.clone(), 0), 0, &mut l, &mut r).unwrap();
    assert!(db(energy(&l), energy(&r)) > 6.0, "IAB object on the left: {} dB", db(energy(&l), energy(&r)));
    // the next frame: the same object (MetaID 7) moved right, and the centre bed playing
    let mut right = vec![None; FrameRate::R24.sub_blocks()];
    right[0] = Some(pan(65535, 49151));
    iab.render(&mut scene, &frame(right, 1), n as u64, &mut l, &mut r).unwrap();
    assert!(energy(&l) > 0.0 && energy(&r) > energy(&l), "moved right: {} vs {}", energy(&r), energy(&l));
    assert_eq!(IAB_BED_FEEDS, 12);
    // a frame at another rate is refused
    let mut f96 = frame(pans, 1);
    f96.sample_rate = 96_000;
    assert!(iab.render(&mut scene, &f96, 0, &mut l, &mut r).is_err());
}

/// A real Dolby Atmos TrueHD stream (a raw `.thd`, none committed): every object with an update
/// takes a place in the scene, the bed objects included, and the binaural render is finite and
/// carries about the energy of the stream's 7.1.4 loudspeaker render.
#[test]
fn a_real_atmos_stream_renders_binaurally() {
    let Some(path) = std::env::var_os("CFA4_ATMOS_SAMPLE") else {
        eprintln!("skipped: set CFA4_ATMOS_SAMPLE to a Dolby Atmos .thd stream");
        return;
    };
    let data = std::fs::read(&path).unwrap();
    let mut dec = cfa4_truehd::cfa4::ObjectDecoder::new();
    let mut objects: Vec<Vec<f32>> = Vec::new();
    let mut events = Vec::new();
    let mut rate = 0;
    // the first 20 seconds or so
    for piece in data.chunks(4096).take(1_500) {
        let Ok(chunk) = dec.decode(piece) else { continue };
        if chunk.objects.is_empty() {
            continue;
        }
        rate = chunk.sample_rate;
        objects.resize_with(objects.len().max(chunk.objects.len()), Vec::new);
        let before = objects.first().map_or(0, Vec::len);
        for (o, c) in objects.iter_mut().zip(&chunk.objects) {
            o.resize(before, 0.0);
            o.extend(c);
        }
        events.extend(chunk.events);
    }
    assert!(rate == RATE, "the sample is at {rate} Hz");
    let mut scene = SceneRenderer::new(hrtf(), SceneConfig::default()).unwrap();
    schedule_truehd(&mut scene, &events, HeadphoneMode::Mid);
    let mut placed: Vec<usize> = events.iter().map(|e| e.object).collect();
    placed.sort_unstable();
    placed.dedup();
    let beds =
        events.iter().filter(|e| matches!(e.state.source, cfa4_truehd::render::Source::Bed(_))).map(|e| e.object).collect::<std::collections::BTreeSet<_>>();
    eprintln!("{} objects ({} beds), {} events, {} samples", placed.len(), beds.len(), events.len(), objects[0].len());
    assert!(placed.len() == objects.len() && placed.len() > beds.len(), "every object takes a place");
    let refs: Vec<&[f32]> = objects.iter().map(|o| o.as_slice()).collect();
    let frames = objects[0].len();
    let (l, r) = render(&mut scene, &refs, frames, 1024);
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    let mut speakers = cfa4_truehd::render::Renderer::new(cfa4_truehd::render::Layout::surround_7_1_4());
    for e in &events {
        speakers.schedule(e.offset, e.object, &e.state, e.ramp);
    }
    let mut out = vec![Vec::new(); 12];
    speakers.render(0, &refs, &mut out);
    let e_speakers: f64 = out.iter().map(|c| energy(c)).sum();
    let e_ears = energy(&l) + energy(&r);
    eprintln!("ears {e_ears:.1}, 7.1.4 loudspeakers {e_speakers:.1} ({:.1} dB)", db(e_ears, e_speakers));
    assert!(db(e_ears, e_speakers).abs() < 6.0);
}
