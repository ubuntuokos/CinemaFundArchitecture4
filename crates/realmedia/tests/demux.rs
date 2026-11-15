//! The demuxer on hand-made RealMedia files: header chunks, whole frames, frames split over
//! packets (slice tables as FFmpeg's demuxer builds them), several frames in one packet, seeking
//! into the middle of a split frame, and damaged files (no panic, bounded work).

use std::io::Cursor;

use cfa4_realmedia::{Demuxer, StreamKind, probe};

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// A data packet of stream 0.
fn packet(ts: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::new();
    be16(&mut p, 0);
    be16(&mut p, (12 + payload.len()) as u16);
    be16(&mut p, 0);
    be32(&mut p, ts);
    p.push(0);
    p.push(flags);
    p.extend_from_slice(payload);
    p
}

/// `get_num` coding of a small number.
fn num(v: &mut Vec<u8>, x: u16) {
    be16(v, 0x4000 | x);
}

fn file(packets: &[Vec<u8>]) -> Vec<u8> {
    let mut f = b".RMF".to_vec();
    be32(&mut f, 18);
    be16(&mut f, 0);
    be32(&mut f, 0);
    be32(&mut f, 4);
    // PROP
    f.extend_from_slice(b"PROP");
    be32(&mut f, 50);
    be16(&mut f, 0);
    for x in [0, 0, 0, 0, packets.len() as u32, 1000, 0, 0, 0] {
        be32(&mut f, x);
    }
    be16(&mut f, 1);
    be16(&mut f, 0);
    // MDPR with VIDO type-specific data
    let mut ts = Vec::new();
    be32(&mut ts, 34);
    ts.extend_from_slice(b"VIDORV20");
    be16(&mut ts, 176);
    be16(&mut ts, 144);
    be16(&mut ts, 12);
    be32(&mut ts, 0);
    be32(&mut ts, 15 << 16);
    ts.extend_from_slice(&[0, 0, 0, 8, 0x20, 0x10, 0x30, 0x01]);
    let mime = b"video/x-pn-realvideo";
    let mut m = b"MDPR".to_vec();
    be32(&mut m, 0);
    be16(&mut m, 0);
    be16(&mut m, 0);
    for x in [0, 0, 0, 0, 0, 0, 1000] {
        be32(&mut m, x);
    }
    m.push(5);
    m.extend_from_slice(b"Video");
    m.push(mime.len() as u8);
    m.extend_from_slice(mime);
    be32(&mut m, ts.len() as u32);
    m.extend_from_slice(&ts);
    let n = m.len() as u32;
    m[4..8].copy_from_slice(&n.to_be_bytes());
    f.extend_from_slice(&m);
    // DATA
    let body: usize = packets.iter().map(Vec::len).sum();
    f.extend_from_slice(b"DATA");
    be32(&mut f, (18 + body) as u32);
    be16(&mut f, 0);
    be32(&mut f, packets.len() as u32);
    be32(&mut f, 0);
    for p in packets {
        f.extend_from_slice(p);
    }
    f
}

/// FFmpeg's packet layout for one frame of `slices` (offset of each).
fn assembled(offsets: &[u32], data: &[u8]) -> Vec<u8> {
    let mut v = vec![(offsets.len() - 1) as u8];
    for &o in offsets {
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&o.to_le_bytes());
    }
    v.extend_from_slice(data);
    v
}

/// Frame A split over two packets (6 + 4 bytes), frame B whole, frames C and D in one packet.
fn sample() -> (Vec<u8>, Vec<Vec<u8>>) {
    let a: Vec<u8> = (1..=10).collect();
    let b = vec![0xB0, 0xB1, 0xB2];
    let c = vec![0xC0, 0xC1];
    let d = vec![0xD0, 0xD1, 0xD2, 0xD3];
    let mut p0 = vec![0x01, 1];
    num(&mut p0, 10);
    num(&mut p0, 0);
    p0.push(7);
    p0.extend_from_slice(&a[..6]);
    let mut p1 = vec![0x81, 2];
    num(&mut p1, 10);
    num(&mut p1, 4);
    p1.push(7);
    p1.extend_from_slice(&a[6..]);
    let mut p2 = vec![0x40, 0];
    p2.extend_from_slice(&b);
    let mut p3 = Vec::new();
    for (k, fr) in [&c, &d].iter().enumerate() {
        p3.push(0xC0);
        num(&mut p3, fr.len() as u16);
        num(&mut p3, 300 + k as u16);
        p3.push(0);
        p3.extend_from_slice(fr);
    }
    let packets = vec![packet(0, 2, &p0), packet(0, 2, &p1), packet(100, 0, &p2), packet(200, 0, &p3)];
    let want = vec![assembled(&[0, 6], &a), assembled(&[0], &b), assembled(&[0], &c), assembled(&[0], &d)];
    (file(&packets), want)
}

#[test]
fn header_and_frames() {
    let (f, want) = sample();
    assert!(probe(&f));
    let mut dm = Demuxer::new(Cursor::new(&f)).unwrap();
    let h = dm.header().clone();
    assert_eq!(h.duration_ms, 1000);
    assert_eq!(h.streams.len(), 1);
    assert_eq!(h.streams[0].mime, "video/x-pn-realvideo");
    match &h.streams[0].kind {
        StreamKind::Video { fourcc, width, height, fps_16_16, extradata } => {
            assert_eq!(fourcc, b"RV20");
            assert_eq!((*width, *height, *fps_16_16), (176, 144, 15 << 16));
            assert_eq!(extradata, &[0, 0, 0, 8, 0x20, 0x10, 0x30, 0x01]);
        }
        k => panic!("{k:?}"),
    }
    let mut got = Vec::new();
    while let Some(p) = dm.next_packet().unwrap() {
        assert!(!p.partial);
        got.push((p.data, p.timestamp_ms, p.keyframe));
    }
    let data: Vec<Vec<u8>> = got.iter().map(|g| g.0.clone()).collect();
    assert_eq!(data, want);
    // a frame inside a packet carries its own timestamp
    assert_eq!(got.iter().map(|g| (g.1, g.2)).collect::<Vec<_>>(), vec![(0, true), (100, false), (300, false), (301, false)]);
}

#[test]
fn seeking_into_a_split_frame_marks_it_partial() {
    let (f, want) = sample();
    let mut dm = Demuxer::new(Cursor::new(&f)).unwrap();
    let mut positions = Vec::new();
    while let Some(p) = dm.next_packet().unwrap() {
        positions.push(p.pos);
    }
    // the second packet of frame A: what comes first is the rest of A, flagged
    let second = positions[0] + 12 + 2 + 4 + 1 + 6;
    dm.seek(second).unwrap();
    let p = dm.next_packet().unwrap().unwrap();
    assert!(p.partial);
    let next = dm.next_packet().unwrap().unwrap();
    assert_eq!(next.data, want[1]);
    assert_eq!(next.pos, positions[1]);
    // back to the start: everything again
    dm.seek(dm.header().data_start).unwrap();
    assert_eq!(dm.next_packet().unwrap().unwrap().data, want[0]);
}

#[test]
fn damaged_files_do_not_panic() {
    let (f, _) = sample();
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..4000 {
        let mut g = f.clone();
        let n = 1 + (rnd() % 6) as usize;
        for _ in 0..n {
            let at = (rnd() as usize) % g.len();
            g[at] = rnd() as u8;
        }
        if round % 3 == 0 {
            let cut = (rnd() as usize) % g.len();
            g.truncate(cut);
        }
        let Ok(mut dm) = Demuxer::new(Cursor::new(&g)) else { continue };
        for _ in 0..1000 {
            match dm.next_packet() {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
    }
    assert!(Demuxer::new(Cursor::new(b".RMF")).is_err());
    assert!(Demuxer::new(Cursor::new(b"RIFF....")).is_err());
}
