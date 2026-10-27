use super::*;
use crate::structs::oamd::ProgramAssignment;

fn point(position: [f32; 3]) -> ObjectState {
    ObjectState { source: Source::Point { position, size: 0.0, zone: 0, snap: false, elevation: true }, gain: 1.0 }
}

fn idx(layout: &Layout, label: ChannelLabel) -> usize {
    layout.speakers.iter().position(|s| s.label == label).unwrap()
}

fn power(g: &[f32]) -> f32 {
    g.iter().map(|v| v * v).sum()
}

#[test]
fn an_object_at_a_loudspeaker_plays_from_it_alone() {
    for layout in [Layout::stereo(), Layout::surround_5_1(), Layout::surround_7_1(), Layout::surround_7_1_4()] {
        for (i, s) in layout.speakers.iter().enumerate().filter(|(_, s)| !s.lfe) {
            let g = speaker_gains(&layout, &point(s.position));
            assert!(g[i] > 0.999, "{} {:?}: {g:?}", layout.name, s.label);
        }
    }
}

#[test]
fn panning_keeps_constant_power_and_never_feeds_the_lfe() {
    let layout = Layout::surround_7_1_4();
    let lfe = idx(&layout, ChannelLabel::LFE);
    let mut seed = 1u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1 << 24) as f32
    };
    for _ in 0..500 {
        let p = [rnd(), rnd(), rnd()];
        let size = rnd();
        let st = ObjectState { source: Source::Point { position: p, size, zone: 0, snap: false, elevation: true }, gain: 1.0 };
        let g = speaker_gains(&layout, &st);
        assert!((power(&g) - 1.0).abs() < 1e-4, "{p:?}: {g:?}");
        assert_eq!(g[lfe], 0.0);
    }
}

#[test]
fn a_centred_object_splits_evenly_left_and_right() {
    let layout = Layout::surround_7_1_4();
    let g = speaker_gains(&layout, &point([0.5, 0.5, 1.0]));
    for (a, b) in [(ChannelLabel::Tfl, ChannelLabel::Tfr), (ChannelLabel::Tbl, ChannelLabel::Tbr), (ChannelLabel::Tfl, ChannelLabel::Tbl)] {
        assert!((g[idx(&layout, a)] - g[idx(&layout, b)]).abs() < 1e-6, "{g:?}");
    }
    assert!(g[idx(&layout, ChannelLabel::Tfl)] > 0.4);
}

#[test]
fn beds_play_from_their_loudspeaker_or_are_panned() {
    let l = Layout::surround_7_1_4();
    let bed = |s| speaker_gains(&l, &ObjectState { source: Source::Bed(s), gain: 1.0 });
    use SpeakerLabels as S;
    for (s, label) in [
        (S::L, ChannelLabel::L),
        (S::Rss, ChannelLabel::Rs),
        (S::Lrs, ChannelLabel::Lb),
        (S::Lfh, ChannelLabel::Tfl),
        (S::Rrh, ChannelLabel::Tbr),
        (S::LFE2, ChannelLabel::LFE),
    ] {
        let g = bed(s);
        assert_eq!(g[idx(&l, label)], 1.0, "{s:?}: {g:?}");
        assert!((power(&g) - 1.0).abs() < 1e-6);
    }
    // top side has no loudspeaker in 7.1.4: it sits between the front and back tops
    let g = bed(S::Lts);
    assert!((g[idx(&l, ChannelLabel::Tfl)] - g[idx(&l, ChannelLabel::Tbl)]).abs() < 1e-6, "{g:?}");
    assert!(g[idx(&l, ChannelLabel::Tfl)] > 0.6);

    let st = Layout::stereo();
    let c = speaker_gains(&st, &ObjectState { source: Source::Bed(S::C), gain: 1.0 });
    assert!((c[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5 && (c[1] - c[0]).abs() < 1e-6, "{c:?}");
    let lfe = speaker_gains(&st, &ObjectState { source: Source::Bed(S::LFE), gain: 1.0 });
    assert_eq!(lfe, vec![0.0, 0.0], "an LFE without an LFE loudspeaker is dropped");
}

#[test]
fn zones_snap_elevation_and_gain_are_honoured() {
    let l = Layout::surround_7_1_4();
    let at =
        |position, zone, snap, elevation, gain| speaker_gains(&l, &ObjectState { source: Source::Point { position, size: 0.0, zone, snap, elevation }, gain });
    // screen only: a back object plays from the screen loudspeakers
    let g = at([0.0, 1.0, 0.0], 4, false, true, 1.0);
    for (i, s) in l.speakers.iter().enumerate() {
        let screen = matches!(s.label, ChannelLabel::L | ChannelLabel::C | ChannelLabel::R);
        assert!(screen || g[i] == 0.0, "{:?} {g:?}", s.label);
    }
    assert!(g[idx(&l, ChannelLabel::L)] > g[idx(&l, ChannelLabel::C)] && g[idx(&l, ChannelLabel::C)] > g[idx(&l, ChannelLabel::R)], "{g:?}");
    // no back
    let g = at([0.0, 1.0, 0.0], 1, false, true, 1.0);
    assert_eq!(g[idx(&l, ChannelLabel::Lb)], 0.0);
    assert!(g[idx(&l, ChannelLabel::Ls)] > 0.5, "{g:?}");
    // snap goes to the nearest loudspeaker alone
    let g = at([0.1, 0.45, 0.0], 0, true, true, 1.0);
    assert_eq!(g[idx(&l, ChannelLabel::Ls)], 1.0);
    assert_eq!(power(&g), 1.0);
    // elevation off flattens to ear height
    let g = at([0.0, 0.25, 1.0], 0, false, false, 1.0);
    assert!(l.speakers.iter().zip(&g).all(|(s, v)| s.position[2] < 0.5 || *v == 0.0), "{g:?}");
    // gain scales every loudspeaker
    let g = at([0.3, 0.3, 0.3], 0, false, true, 0.5);
    assert!((power(&g) - 0.25).abs() < 1e-5);
    // hostile numbers give silence or a clamped position, never NaN
    let g = at([f32::NAN, f32::INFINITY, -7.0], 0, false, true, f32::NAN);
    assert!(g.iter().all(|v| *v == 0.0));
    let g = at([f32::NAN, 9.0, -7.0], 9, false, true, 1.0);
    assert!(g.iter().all(|v| v.is_finite()) && (power(&g) - 1.0).abs() < 1e-5);
}

#[test]
fn updates_ramp_linearly_from_their_sample_across_calls() {
    let l = Layout::stereo();
    let mut r = Renderer::new(l);
    let left = ObjectState { source: Source::Bed(SpeakerLabels::L), gain: 1.0 };
    // a silent first state, so the update at 10 ramps up from silence
    r.schedule(0, 0, &ObjectState { gain: 0.0, ..left }, 0);
    r.schedule(10, 0, &left, 20);
    let ones = vec![1.0f32; 40];
    let mut out = vec![Vec::new(), Vec::new()];
    r.render(0, &[&ones], &mut out);
    assert!(out[0][..=10].iter().all(|v| *v == 0.0), "{:?}", &out[0][..12]);
    for (k, v) in out[0].iter().enumerate().skip(10) {
        let want = ((k - 10) as f32 / 20.0).min(1.0);
        assert!((v - want).abs() < 1e-6, "sample {k}: {v} vs {want}");
    }
    assert!(out[1].iter().all(|v| *v == 0.0));

    // a hard cut to the right lands in the next call, at its exact sample
    let right = ObjectState { source: Source::Bed(SpeakerLabels::R), gain: 0.5 };
    r.schedule(65, 0, &right, 0);
    r.render(40, &[&ones], &mut out);
    assert_eq!(out[0].len(), 80);
    assert!(out[0][40..65].iter().all(|v| *v == 1.0));
    assert!(out[0][65..].iter().all(|v| *v == 0.0));
    assert!(out[1][65..].iter().all(|v| *v == 0.5) && out[1][..65].iter().all(|v| *v == 0.0));

    // objects without an update are silent; reset forgets everything
    let mut out = vec![Vec::new(), Vec::new()];
    r.render(80, &[&ones, &ones], &mut out);
    assert!(out[1].iter().all(|v| *v == 0.5));
    r.reset();
    let mut out = vec![Vec::new(), Vec::new()];
    r.render(120, &[&ones], &mut out);
    assert!(out.iter().flatten().all(|v| *v == 0.0));
}

/// An object's first state applies at once, whatever its ramp: ramping in from silence would fade
/// in the start of the stream (a real Atmos stream's first OAMD, 1000 samples in, asks for 1536).
#[test]
fn an_objects_first_state_is_not_ramped_in() {
    let mut r = Renderer::new(Layout::stereo());
    let left = ObjectState { source: Source::Bed(SpeakerLabels::L), gain: 1.0 };
    r.schedule(10, 0, &left, 1536);
    let ones = vec![1.0f32; 40];
    let mut out = vec![Vec::new(), Vec::new()];
    r.render(0, &[&ones], &mut out);
    assert!(out[0][..10].iter().all(|v| *v == 0.0) && out[0][10..].iter().all(|v| *v == 1.0), "{:?}", out[0]);
    // after a reset the next state is a first state again
    r.reset();
    r.schedule(45, 0, &left, 1536);
    r.render(40, &[&ones], &mut out);
    assert!(out[0][45..].iter().all(|v| *v == 1.0));
}

/// An MSB-first bit writer for building OAMD payloads.
#[derive(Default)]
struct Bits(Vec<bool>);

impl Bits {
    fn put(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.0.push((v >> i) & 1 == 1);
        }
    }
    fn pad(&mut self) {
        while !self.0.len().is_multiple_of(8) {
            self.0.push(false);
        }
    }
    fn bytes(&self) -> Vec<u8> {
        self.0.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (u8::from(b) << (7 - i)))).collect()
    }
}

struct Obj {
    active: bool,
    /// dB; `None` is -inf.
    gain: Option<i8>,
    /// x and y codes 0 ..= 62, z code -15 ..= 15.
    pos: [i8; 3],
    zone: u8,
    elevation: bool,
    snap: bool,
}

fn put_gain(w: &mut Bits, gain: Option<i8>) {
    match gain {
        Some(0) => w.put(0, 2),
        None => w.put(1, 2),
        Some(g) => {
            w.put(2, 2);
            w.put(if g > 0 { 15 - g as i64 } else { 14 - g as i64 } as u64, 6);
        }
    }
}

/// An OAMD payload: a standard bed (`bed` bits of `STD_BED_LIST`) or, with `lfe_only`, a dynamic
/// object program with an LFE; then `objs` dynamic objects; one block.
fn oamd(bed: u16, lfe_only: bool, objs: &[Obj], sample_offset: u8, ramp: u16) -> Vec<u8> {
    let beds = if lfe_only { 1 } else { crate::structs::oamd::BedAssignment::from_std(bed).count_beds() };
    let mut w = Bits::default();
    w.put(0, 2); // version
    w.put((beds + objs.len() - 1) as u64, 5);
    if lfe_only {
        w.put(1, 1);
        w.put(1, 1); // b_lfe_present
    } else {
        w.put(0, 1);
        w.put(1 | 4, 4); // beds and dynamic objects
        w.put(0, 1); // b_bed_chan_distribute
        w.put(0, 1); // one bed instance
        w.put(0, 1); // not LFE only
        w.put(1, 1); // standard assignment
        w.put(u64::from(bed), 10);
        w.put((objs.len() - 1) as u64, 5);
    }
    w.put(0, 1); // no alternate object data
    w.put(1, 4); // one element

    let mut e = Bits::default();
    e.put(0, 1); // b_discard_unknown_element
    if sample_offset == 0 {
        e.put(0, 2);
    } else {
        e.put(2, 2);
        e.put(u64::from(sample_offset), 5);
    }
    e.put(0, 3); // one block
    e.put(0, 6); // block offset
    if ramp == 0 {
        e.put(0, 2);
    } else {
        e.put(3, 2);
        e.put(0, 1);
        e.put(u64::from(ramp), 11);
    }
    e.put(1, 1); // b_reserved_data_not_present
    for _ in 0..beds {
        e.put(0, 1); // active
        put_gain(&mut e, Some(0));
        e.put(1, 1); // default priority
        e.put(0, 1); // no additional table data
    }
    for o in objs {
        e.put(u64::from(!o.active), 1);
        if o.active {
            put_gain(&mut e, o.gain);
            e.put(1, 1);
            e.put(o.pos[0] as u64, 6);
            e.put(o.pos[1] as u64, 6);
            e.put(u64::from(o.pos[2] >= 0), 1);
            e.put(u64::from(o.pos[2].unsigned_abs()), 4);
            e.put(0, 1); // no distance
            e.put(u64::from(o.zone), 3);
            e.put(u64::from(o.elevation), 1);
            e.put(0, 2); // no size
            e.put(0, 1); // no screen reference
            e.put(u64::from(o.snap), 1);
        }
        e.put(0, 1);
    }
    e.pad();
    let size = e.0.len() / 8 - 1;
    w.put(1, 4); // object element
    if size < 16 {
        w.put(size as u64, 4);
        w.put(0, 1);
    } else {
        w.put((size / 16 - 1) as u64, 4);
        w.put(1, 1);
        w.put((size % 16) as u64, 4);
        w.put(0, 1);
    }
    w.0.extend(e.0);
    w.pad();
    w.bytes()
}

#[test]
fn an_oamd_payload_gives_bed_and_object_updates() {
    let objs = [
        Obj { active: true, gain: Some(-6), pos: [0, 62, 15], zone: 0, elevation: true, snap: false },
        Obj { active: false, gain: None, pos: [0, 0, 0], zone: 0, elevation: true, snap: false },
        Obj { active: true, gain: Some(3), pos: [31, 31, -15], zone: 4, elevation: false, snap: true },
    ];
    // 7.1 bed: L/R, C, LFE, Lss/Rss, Lrs/Rrs
    let bytes = oamd(0b11111, false, &objs, 12, 512);
    let mut p = ObjectAudioMetadataPayload::read(&bytes).unwrap();
    p.evo_sample_offset = 4;
    assert_eq!(p.object_count, 11);
    let ev = events(&p, 16).unwrap();
    assert_eq!(ev.len(), 11);
    assert!(ev.iter().all(|e| e.offset == 16 && e.ramp == 512));
    use SpeakerLabels as S;
    let beds: Vec<Source> = ev[..8].iter().map(|e| e.state.source).collect();
    assert_eq!(beds, [S::L, S::R, S::C, S::LFE, S::Lss, S::Rss, S::Lrs, S::Rrs].map(Source::Bed));
    assert!(ev[..8].iter().all(|e| e.state.gain == 1.0));

    let Source::Point { position, zone, snap, elevation, .. } = ev[8].state.source else { panic!() };
    assert_eq!(position, [0.0, 1.0, 1.0]);
    assert!((ev[8].state.gain - 0.501_187).abs() < 1e-5);
    assert!((zone, snap, elevation) == (0, false, true));
    assert_eq!(ev[9].state.gain, 0.0, "an inactive object is silent");
    let Source::Point { position, zone, snap, elevation, .. } = ev[10].state.source else { panic!() };
    assert!((position[0] - 0.5).abs() < 1e-6 && (position[1] - 0.5).abs() < 1e-6 && position[2] == -1.0);
    assert!((zone, snap, elevation) == (4, true, false));
    assert!((ev[10].state.gain - 1.412_538).abs() < 1e-5);

    // only the channels the presentation carries are rendered
    assert_eq!(events(&p, 9).unwrap().len(), 9);

    // the back-left object at the ceiling renders to the top back left in 7.1.4
    let l = Layout::surround_7_1_4();
    let g = speaker_gains(&l, &ev[8].state);
    let tbl = g[idx(&l, ChannelLabel::Tbl)];
    assert!(tbl * tbl > 0.5 * power(&g), "{g:?}");
}

#[test]
fn a_dynamic_object_program_has_its_lfe_as_the_first_object() {
    let objs = [Obj { active: true, gain: Some(0), pos: [62, 0, 0], zone: 0, elevation: true, snap: false }];
    let p = ObjectAudioMetadataPayload::read(&oamd(0, true, &objs, 0, 0)).unwrap();
    let ev = events(&p, 16).unwrap();
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0].state.source, Source::Bed(SpeakerLabels::LFE));
    assert!(ev.iter().all(|e| e.offset == 0 && e.ramp == 0));
    let l = Layout::surround_7_1();
    assert_eq!(speaker_gains(&l, &ev[1].state)[idx(&l, ChannelLabel::R)], 1.0);
}

#[test]
fn intermediate_spatial_format_programs_are_refused() {
    let p = ObjectAudioMetadataPayload {
        object_count: 8,
        program_assignment: ProgramAssignment { num_isf_objects: 8, ..Default::default() },
        ..Default::default()
    };
    assert_eq!(events(&p, 16), None);
    assert_eq!(events(&ObjectAudioMetadataPayload::default(), 16), Some(Vec::new()));
}

/// Mangled payloads give an error or updates, and rendering whatever comes out never panics.
#[test]
fn mangled_payloads_never_panic() {
    let objs = [
        Obj { active: true, gain: Some(-20), pos: [10, 50, 7], zone: 2, elevation: true, snap: false },
        Obj { active: true, gain: Some(15), pos: [62, 62, -3], zone: 5, elevation: true, snap: true },
    ];
    let good = oamd(0b111, false, &objs, 30, 2047);
    let mut seed = 7u32;
    let l = Layout::surround_7_1_4();
    for round in 0..3000 {
        let mut b = good.clone();
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let at = (seed >> 8) as usize % b.len();
        b[at] ^= 1 << (seed % 8);
        if round % 7 == 0 {
            b.truncate(at);
        }
        let Ok(mut p) = ObjectAudioMetadataPayload::read(&b) else { continue };
        p.evo_sample_offset = u64::MAX - u64::from(seed % 3);
        let mut r = Renderer::new(l.clone());
        for e in events(&p, 16).unwrap_or_default() {
            r.schedule(e.offset, e.object, &e.state, e.ramp);
        }
        let x = vec![0.25f32; 40];
        let mut out = vec![Vec::new(); l.channels()];
        r.render(u64::MAX - 20, &[x.as_slice(); 16], &mut out);
        assert!(out.iter().flatten().all(|v| v.is_finite()));
    }
}
