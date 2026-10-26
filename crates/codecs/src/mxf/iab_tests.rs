//! IAB tracks in MXF, end to end: a hand-built OP1a file around frames from `cfa4_iab::write`.

use super::*;
use cfa4_iab::{AudioData, Bed, BedChannel, Coding, Description, Frame, Gain, Layout};

fn ber(n: usize) -> Vec<u8> {
    let mut v = vec![0x83];
    v.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
    v
}

fn klv(out: &mut Vec<u8>, key: [u8; 16], v: &[u8]) {
    out.extend_from_slice(&key);
    out.extend_from_slice(&ber(v.len()));
    out.extend_from_slice(v);
}

/// A header metadata set of type `t`.
fn set(t: u8, uid: u8, props: &[(u16, Vec<u8>)]) -> Vec<u8> {
    local_set([0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x01, 0x01, 0x01, 0x01, t, 0x00], uid, props)
}

const INDEX_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00];

fn local_set(key: [u8; 16], uid: u8, props: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut v = Vec::new();
    let mut put = |tag: u16, val: &[u8]| {
        v.extend_from_slice(&tag.to_be_bytes());
        v.extend_from_slice(&(val.len() as u16).to_be_bytes());
        v.extend_from_slice(val);
    };
    put(0x3C0A, &uuid(uid));
    for (tag, val) in props {
        put(*tag, val);
    }
    let mut out = Vec::new();
    klv(&mut out, key, &v);
    out
}

fn uuid(n: u8) -> [u8; 16] {
    let mut u = [0xAA; 16];
    u[15] = n;
    u
}

fn umid(n: u8) -> Vec<u8> {
    let mut u = vec![0x55; 32];
    u[31] = n;
    u
}

fn batch(items: &[&[u8]]) -> Vec<u8> {
    let mut v = (items.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(&(items.first().map_or(0, |i| i.len()) as u32).to_be_bytes());
    for i in items {
        v.extend_from_slice(i);
    }
    v
}

const SOUND_DEF: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x02, 0, 0, 0];
const OP1A: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x01, 0x09, 0x00];
const IAB_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x16, 0x01, 0x0D, 0x01];

fn partition(kind: u8, this: u64, body_sid: u32, index_sid: u32, hbc: u64) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&this.to_be_bytes());
    v.extend_from_slice(&[0; 16]);
    v.extend_from_slice(&hbc.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&index_sid.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&body_sid.to_be_bytes());
    v.extend_from_slice(&OP1A);
    v.extend_from_slice(&batch(&[]));
    let mut out = Vec::new();
    klv(&mut out, [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, kind, 0x04, 0x00], &v);
    out
}

/// An audio-only OP1a file: an IAB track of `frames`, clip-wrapped with a VBE index (ST 2067-201
/// style), edit rate `rate`.
fn iab_mxf(frames: &[Vec<u8>], rate: (u32, u32)) -> Vec<u8> {
    let d = (frames.len() as i64).to_be_bytes().to_vec();
    let er = [rate.0.to_be_bytes(), rate.1.to_be_bytes()].concat();
    let track = |uid: u8, id: u32, number: u32, seq: u8| {
        set(
            0x3B,
            uid,
            &[
                (0x4801, id.to_be_bytes().to_vec()),
                (0x4804, number.to_be_bytes().to_vec()),
                (0x4B01, er.clone()),
                (0x4B02, vec![0; 8]),
                (0x4803, uuid(seq).to_vec()),
            ],
        )
    };
    let seq = |uid: u8, comp: u8| set(0x0F, uid, &[(0x0201, SOUND_DEF.to_vec()), (0x0202, d.clone()), (0x1001, batch(&[&uuid(comp)]))]);
    let clip = |uid: u8, pkg: Vec<u8>, t: u32| {
        set(0x11, uid, &[(0x0201, SOUND_DEF.to_vec()), (0x0202, d.clone()), (0x1201, vec![0; 8]), (0x1101, pkg), (0x1102, t.to_be_bytes().to_vec())])
    };
    let mut md = Vec::new();
    klv(&mut md, [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00], &batch(&[]));
    md.extend(set(0x2F, 1, &[(0x3B03, uuid(2).to_vec()), (0x3B09, OP1A.to_vec())]));
    md.extend(set(0x18, 2, &[(0x1901, batch(&[&uuid(3), &uuid(4)]))]));
    md.extend(set(0x36, 3, &[(0x4401, umid(1)), (0x4403, batch(&[&uuid(12)]))]));
    md.extend(track(12, 2, 0, 22));
    md.extend(seq(22, 32));
    md.extend(clip(32, umid(2), 2));
    md.extend(set(0x37, 4, &[(0x4401, umid(2)), (0x4403, batch(&[&uuid(14)])), (0x4701, uuid(42).to_vec())]));
    md.extend(track(14, 2, 0x1601_0D01, 24));
    md.extend(seq(24, 34));
    md.extend(clip(34, vec![0; 32], 0));
    md.extend(set(0x7B, 42, &[(0x3006, 2u32.to_be_bytes().to_vec()), (0x3001, er.clone()), (0x3D03, vec![0, 0, 0xBB, 0x80, 0, 0, 0, 1])]));
    md.extend(set(0x23, 5, &[(0x2701, umid(2)), (0x3F07, 1u32.to_be_bytes().to_vec()), (0x3F06, 2u32.to_be_bytes().to_vec())]));
    let mut f = partition(2, 0, 0, 0, md.len() as u64);
    f.extend_from_slice(&md);
    let body_at = f.len() as u64;
    f.extend_from_slice(&partition(3, body_at, 1, 0, 0));
    let mut all = Vec::new();
    let mut entries = Vec::new();
    for fr in frames {
        let mut e = vec![0, 0, 0x80];
        e.extend_from_slice(&(all.len() as u64).to_be_bytes());
        entries.push(e);
        all.extend_from_slice(fr);
    }
    klv(&mut f, IAB_KEY, &all);
    let footer_at = f.len() as u64;
    f.extend_from_slice(&partition(4, footer_at, 0, 2, 0));
    let refs: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();
    f.extend(local_set(
        INDEX_KEY,
        6,
        &[
            (0x3F0B, er.clone()),
            (0x3F0C, vec![0; 8]),
            (0x3F0D, d.clone()),
            (0x3F06, 2u32.to_be_bytes().to_vec()),
            (0x3F07, 1u32.to_be_bytes().to_vec()),
            (0x3F0A, batch(&refs)),
        ],
    ));
    f
}

/// A frame of a 5.1 Bed whose channel k carries a ramp scaled by k + 1.
fn bed_frame(rate: cfa4_iab::FrameRate, index: u8, frame_no: usize, coding: Coding) -> Frame {
    let n = rate.samples(false, index);
    let ids = [0x00u32, 0x04, 0x02, 0x0D, 0x06, 0x0A];
    Frame {
        version: 1,
        sample_rate: 48_000,
        bit_depth: 24,
        frame_rate: rate,
        frame_index: index,
        max_rendered: 6,
        beds: vec![Bed {
            meta_id: 1,
            use_case: None,
            channels: ids.iter().enumerate().map(|(k, &id)| BedChannel { id, audio_id: k as u32 + 1, gain: Gain::Unity, decorrelation: None }).collect(),
            description: Description::default(),
            beds: Vec::new(),
            remaps: Vec::new(),
        }],
        objects: Vec::new(),
        audio: (0..6i32)
            .map(|k| AudioData {
                id: k as u32 + 1,
                coding,
                samples: (0..n).map(|i| (((frame_no * 2002 + i) % 1000) as i32 - 500) * (k + 1) * 1000 * 256).collect(),
            })
            .collect(),
        authoring_tool: None,
        user_data: Vec::new(),
    }
}

#[test]
fn an_iab_track_plays_as_7_1() {
    let rate = cfa4_iab::FrameRate::R24;
    let frames: Vec<Frame> = (0..5).map(|i| bed_frame(rate, 0, i, Coding::Dlc)).collect();
    let bytes: Vec<Vec<u8>> = frames.iter().map(|f| cfa4_iab::write(f).unwrap()).collect();
    let src = MxfSource::open("iab.mxf", Arc::from(iab_mxf(&bytes, (24, 1)))).unwrap();
    let a = src.info().audio().unwrap().clone();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (48_000, 8, "IAB"));
    assert_eq!(src.info().duration, Tick::from_units(10_000, 48_000));
    // across the boundary of frames 1 and 2
    let got = src.audio(3_900, 300, 48_000).unwrap();
    let layout = Layout::surround_7_1();
    let want: Vec<Vec<f32>> = {
        let r1 = cfa4_iab::render(&frames[1], &layout).unwrap();
        let r2 = cfa4_iab::render(&frames[2], &layout).unwrap();
        r1.iter().zip(&r2).map(|(a, b)| a[1_900..].iter().chain(&b[..200]).copied().collect()).collect()
    };
    assert_eq!(got.channels, want);
    // 5.1 Bed on 7.1: L R C LFE straight through
    let l: Vec<f32> = frames[1].audio[0].samples[1_900..].iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
    assert_eq!(got.channels[0][..100], l[..]);
    // past the end: silence
    let tail = src.audio(9_990, 20, 48_000).unwrap();
    assert!(tail.channels.iter().all(|c| c[10..].iter().all(|&v| v == 0.0)));
}

#[test]
fn the_29_97_cadence_maps_samples_to_frames() {
    let ia = IabAudio {
        track: 0,
        rate: cfa4_iab::FrameRate::R29_97,
        rate96: false,
        index0: 3,
        first: 2,
        count: 12,
        layout: Layout::surround_7_1(),
        cache: Mutex::new(None),
    };
    // stored frame 2 has FrameIndex 0: 1602 1601 1602 1601 1602 …
    assert_eq!(ia.start_of(1), 1602);
    assert_eq!(ia.start_of(5), 8008);
    assert_eq!(ia.start_of(7), 8008 + 1602 + 1601);
    for s in [0, 1601, 1602, 8007, 8008, 11_210, 11_211] {
        let k = ia.frame_of(s);
        assert!(ia.start_of(k) <= s && s < ia.start_of(k + 1), "{s} → {k}");
    }
}
