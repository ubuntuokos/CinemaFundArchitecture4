//! RealVideo 3.0/4.0: ffmpeg's FATE samples `rv30.rm` and `spygames-2MB.rmvb` (from
//! <https://fate-suite.ffmpeg.org/real/>, in `target/fixtures/real` when present locally; ffmpeg
//! has no RV30/RV40 encoder), decoded by us through `cfa4-realmedia` and by ffmpeg's `rv30`/`rv40`
//! decoders with `-flags +bitexact`: every plane of every frame equal.

use std::path::Path;
use std::process::Command;

use cfa4_realmedia::{Demuxer, StreamKind};
use cfa4_realvideo::Picture;
use cfa4_realvideo::rv34::{Codec, Decoder};

struct Stream {
    codec: Codec,
    width: u32,
    height: u32,
    extra: Vec<u8>,
    packets: Vec<Vec<u8>>,
}

fn stream(data: &[u8]) -> Stream {
    let mut dm = Demuxer::new(std::io::Cursor::new(data)).unwrap();
    let (si, codec, width, height, extra) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Video { fourcc, width, height, extradata, .. } => {
                let c = match fourcc {
                    b"RV30" => Codec::Rv30,
                    b"RV40" => Codec::Rv40,
                    _ => return None,
                };
                Some((i, c, u32::from(*width), u32::from(*height), extradata.clone()))
            }
            _ => None,
        })
        .expect("no RV30/RV40 stream");
    let mut packets = Vec::new();
    while let Ok(Some(p)) = dm.next_packet() {
        if p.stream == si {
            packets.push(p.data);
        }
    }
    Stream { codec, width, height, extra, packets }
}

fn decode_ours(s: &Stream) -> Vec<Picture> {
    let mut dec = Decoder::new(s.codec, s.width, s.height, &s.extra).unwrap();
    let mut out = Vec::new();
    for p in &s.packets {
        if let Ok(Some(pic)) = dec.decode(p) {
            out.push(pic);
        }
    }
    out.extend(dec.flush());
    out
}

fn yuv(p: &Picture) -> Vec<u8> {
    let mut v = p.y.clone();
    v.extend_from_slice(&p.u);
    v.extend_from_slice(&p.v);
    v
}

fn ffmpeg_frames(ff: &Path, file: &Path, w: usize, h: usize) -> Vec<Vec<u8>> {
    let out = Command::new(ff)
        .args(["-v", "error", "-flags", "+bitexact", "-threads", "1", "-i"])
        .arg(file)
        .args(["-an", "-map", "0:v:0", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .unwrap();
    let fs = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
    out.stdout.chunks_exact(fs).map(<[u8]>::to_vec).collect()
}

fn compare(name: &str) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("rv34 FATE") else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("real").join(name);
    let Ok(data) = std::fs::read(&f) else { return };
    let s = stream(&data);
    let ours = decode_ours(&s);
    assert!(!ours.is_empty(), "{name}: no frames");
    let (w, h) = (ours[0].width as usize, ours[0].height as usize);
    let theirs = ffmpeg_frames(&ff, &f, w, h);
    assert_eq!(ours.len(), theirs.len(), "{name}: {} frames, ffmpeg {}", ours.len(), theirs.len());
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        let a = yuv(a);
        if a != *b {
            let first = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(0);
            let diffs = a.iter().zip(b).filter(|(x, y)| x != y).count();
            panic!("{name}: frame {i} ({:?}) differs: {diffs} bytes, first at {first} (y plane {})", ours[i].pict_type, w * h);
        }
    }
}

#[test]
fn fate_rv30_matches() {
    compare("rv30.rm");
}

#[test]
fn fate_rv40_matches() {
    compare("spygames-2MB.rmvb");
}

/// Random byte changes and cuts in every packet: errors or concealed pictures, never a panic.
fn hammer(s: &Stream, rounds: usize) {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..rounds {
        let mut dec = Decoder::new(s.codec, s.width, s.height, &s.extra).unwrap();
        for p in s.packets.iter().take(80) {
            let mut q = p.clone();
            if rnd() % 3 == 0 && !q.is_empty() {
                for _ in 0..1 + rnd() % 4 {
                    let at = (rnd() as usize) % q.len();
                    q[at] = rnd() as u8;
                }
            }
            if rnd() % 7 == 0 {
                let cut = (rnd() as usize) % (q.len() + 1);
                q.truncate(cut);
            }
            if let Ok(Some(pic)) = dec.decode(&q) {
                assert_eq!(pic.y.len(), pic.width as usize * pic.height as usize);
            }
        }
        let _ = dec.flush();
    }
}

#[test]
fn damaged_streams_do_not_panic() {
    for name in ["rv30.rm", "spygames-2MB.rmvb"] {
        let f = cfa4_testkit::fixtures::fixtures_dir("real").join(name);
        if let Ok(d) = std::fs::read(f) {
            hammer(&stream(&d), 12);
        }
    }
}

#[test]
fn garbage_does_not_panic() {
    let mut seed = 1u64;
    for codec in [Codec::Rv30, Codec::Rv40] {
        for extra in [&[][..], &[0u8; 8][..], &[0x01, 0x08, 0x00, 0x30, 0x20, 0x00, 0x00, 0x00, 0x07, 0x0A][..]] {
            let Ok(mut dec) = Decoder::new(codec, 176, 144, extra) else { continue };
            for len in [0usize, 1, 5, 9, 13, 40, 300, 3000] {
                let pkt: Vec<u8> = (0..len)
                    .map(|_| {
                        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                        (seed >> 33) as u8
                    })
                    .collect();
                let _ = dec.decode(&pkt);
            }
            let _ = dec.flush();
        }
    }
}
