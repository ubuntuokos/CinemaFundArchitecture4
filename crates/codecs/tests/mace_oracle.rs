//! Apple MACE 3:1 / 6:1 in QuickTime, read through the media sources, against ffmpeg's decode:
//! the FATE `qt/surge-*-MAC{3,6}.mov` samples (when present) and random MACE bytes remuxed into
//! MOV, front to back and from the middle (a seek).

mod common;

use common::*;
use std::path::Path;

fn compare(name: &str, path: &Path) {
    let ff = cfa4_testkit::require_ffmpeg!();
    let want = ffmpeg_audio_f32(&ff, path, &[]);
    let s = cfa4_codecs::open_bytes(name, bytes(path)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = s.info().audio().unwrap_or_else(|| panic!("{name}: no audio"));
    let (c, rate) = (info.channels as usize, info.sample_rate);
    let frames = want.len() / c;
    let read = |from: usize, n: usize| -> Vec<f32> {
        let mut out = Vec::with_capacity(n * c);
        let mut at = from;
        while at < from + n {
            let k = 1000.min(from + n - at);
            let b = s.audio(at as i64, k, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
            for i in 0..k {
                out.extend(b.channels.iter().map(|p| p[i]));
            }
            at += k;
        }
        out
    };
    let check = |from: usize, ours: &[f32]| {
        let theirs = &want[from * c..from * c + ours.len()];
        if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
            panic!("{name}: frame {} channel {} differs: ours {}, ffmpeg {}", from + i / c, i % c, ours[i], theirs[i]);
        }
    };
    check(0, &read(0, frames));
    // a jump into the middle, then on
    let mid = frames / 2 + 777;
    check(mid, &read(mid, frames / 4));
}

#[test]
fn fate_samples() {
    let dir = cfa4_testkit::fixtures::fixtures_root().join("fate/qt");
    for n in ["surge-1-8-MAC3.mov", "surge-1-8-MAC6.mov", "surge-2-8-MAC3.mov", "surge-2-8-MAC6.mov"] {
        let p = dir.join(n);
        if p.exists() {
            compare(n, &p);
        } else {
            cfa4_testkit::oracle::skip(n, "FATE sample not fetched");
        }
    }
}

fn aifc(tag: &[u8; 4], channels: u16, data: &[u8]) -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&channels.to_be_bytes());
    comm.extend_from_slice(&0u32.to_be_bytes());
    comm.extend_from_slice(&8u16.to_be_bytes());
    comm.extend_from_slice(&0x400Du16.to_be_bytes()); // 22050 Hz, 80-bit extended
    comm.extend_from_slice(&(22050u64 << 49).to_be_bytes());
    comm.extend_from_slice(tag);
    comm.extend_from_slice(&[0, 0]);
    let mut ssnd = vec![0u8; 8];
    ssnd.extend_from_slice(data);
    let mut body = b"AIFC".to_vec();
    for (id, b) in [(b"FVER", &0xA280_5140u32.to_be_bytes()[..]), (b"COMM", &comm[..]), (b"SSND", &ssnd[..])] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(b.len() as u32).to_be_bytes());
        body.extend_from_slice(b);
        if b.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Random MACE bytes (ffmpeg has no encoder) remuxed from AIFF-C into QuickTime.
#[test]
fn random_streams_in_mov() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let mut x = 9u32;
    for (tag, ch, unit) in [(b"MAC3", 1u16, 2usize), (b"MAC3", 2, 4), (b"MAC6", 1, 1), (b"MAC6", 2, 2)] {
        let data: Vec<u8> = (0..unit * 20000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 16) as u8
            })
            .collect();
        let name = format!("mace_{}{ch}", if tag == b"MAC3" { 3 } else { 6 });
        let src = dir().join(format!("{name}.aifc"));
        std::fs::write(&src, aifc(tag, ch, &data)).expect("write");
        let Some(mov) = fixture(&ff, &format!("{name}.mov"), &["-i", src.to_str().unwrap(), "-c:a", "copy", "-f", "mov"]) else {
            cfa4_testkit::oracle::skip(&name, "ffmpeg can't write MACE into QuickTime");
            continue;
        };
        compare(&name, &mov);
    }
}
