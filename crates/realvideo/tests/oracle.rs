//! RealVideo 1.0/2.0 streams from ffmpeg's `rv10`/`rv20` encoders (and ffmpeg's FATE sample
//! `G2_with_SVT_320_240.rm` when present locally), decoded by us through `cfa4-realmedia`, against
//! ffmpeg's decoder run with the same IDCT (`-idct simple -flags +bitexact`).

use std::path::Path;
use std::process::Command;

use cfa4_realmedia::{Demuxer, StreamKind};
use cfa4_realvideo::{Codec, Decoder, Picture};

/// Our frames, in output order, as tight YUV 4:2:0.
fn decode_ours(data: &[u8]) -> Result<Vec<Picture>, String> {
    let mut dm = Demuxer::new(std::io::Cursor::new(data)).map_err(|e| e.to_string())?;
    let (si, codec, w, h, extra) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Video { fourcc, width, height, extradata, .. } => {
                let c = match fourcc {
                    b"RV10" => Codec::Rv10,
                    b"RV20" => Codec::Rv20,
                    _ => return None,
                };
                Some((i, c, u32::from(*width), u32::from(*height), extradata.clone()))
            }
            _ => None,
        })
        .ok_or("no RV10/RV20 stream")?;
    let mut dec = Decoder::new(codec, w, h, &extra).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(p) = dm.next_packet().map_err(|e| e.to_string())? {
        if p.stream != si {
            continue;
        }
        match dec.decode(&p.data) {
            Ok(Some(pic)) => out.push(pic),
            Ok(None) => {}
            Err(e) => return Err(format!("frame {}: {e}", out.len())),
        }
    }
    Ok(out)
}

fn yuv(p: &Picture) -> Vec<u8> {
    let mut v = p.y.clone();
    v.extend_from_slice(&p.u);
    v.extend_from_slice(&p.v);
    v
}

fn ffmpeg_frames(ff: &Path, file: &Path, w: usize, h: usize) -> Option<Vec<Vec<u8>>> {
    let out = Command::new(ff)
        .args(["-v", "error", "-idct", "simple", "-flags", "+bitexact", "-i"])
        .arg(file)
        .args(["-an", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let fs = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
    Some(out.stdout.chunks(fs).map(<[u8]>::to_vec).collect())
}

fn compare(name: &str, ff: &Path, file: &Path) {
    let data = std::fs::read(file).unwrap();
    let ours = decode_ours(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!ours.is_empty(), "{name}: no frames");
    let (w, h) = (ours[0].width as usize, ours[0].height as usize);
    let theirs = ffmpeg_frames(ff, file, w, h).unwrap_or_else(|| panic!("{name}: ffmpeg failed"));
    assert!(ours.len() >= theirs.len(), "{name}: {} frames, ffmpeg {}", ours.len(), theirs.len());
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        let a = yuv(a);
        if a != *b {
            let first = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(0);
            let diffs = a.iter().zip(b).filter(|(x, y)| x != y).count();
            panic!("{name}: frame {i} ({:?}) differs: {diffs} bytes, first at {first} (y plane {})", ours[i].pict_type, w * h);
        }
    }
}

fn encode(ff: &Path, dir: &Path, codec: &str, w: u32, h: u32, extra: &[&str]) -> std::path::PathBuf {
    let out = dir.join(format!("{codec}-{w}x{h}-{}.rm", extra.join("").replace(['-', ':', '+', ' '], "")));
    let st = Command::new(ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={w}x{h}:rate=25:duration=2,noise=alls=8:allf=t"))
        .args(["-c:v", codec])
        .args(extra)
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg encode {codec}");
    out
}

#[test]
fn ffmpeg_encoded_streams_match() {
    let ff = cfa4_testkit::oracle::ffmpeg_or_skip("realvideo oracle");
    let Some(ff) = ff else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-rv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases: &[(&str, u32, u32, &[&str])] = &[
        ("rv10", 176, 144, &["-q:v", "4"]),
        ("rv10", 320, 240, &["-b:v", "300k", "-g", "12"]),
        ("rv20", 176, 144, &["-q:v", "4"]),
        ("rv20", 352, 288, &["-b:v", "400k", "-g", "25"]),
        ("rv20", 320, 240, &["-q:v", "2", "-mbd", "2", "-trellis", "1"]),
        ("rv20", 128, 96, &["-q:v", "1"]),
        ("rv10", 160, 128, &["-q:v", "1"]),
        ("rv20", 200, 152, &["-q:v", "3"]),
    ];
    for (codec, w, h, extra) in cases {
        let f = encode(&ff, &dir, codec, *w, *h, extra);
        compare(&f.display().to_string(), &ff, &f);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fate_g2_sample_matches() {
    let ff = cfa4_testkit::oracle::ffmpeg_or_skip("realvideo FATE");
    let Some(ff) = ff else { return };
    let f = cfa4_testkit::fixtures::fixtures_dir("real").join("G2_with_SVT_320_240.rm");
    if !f.exists() {
        return;
    }
    compare("G2_with_SVT_320_240", &ff, &f);
}

/// The packets of the first video stream and the decoder for it.
fn packets(data: &[u8]) -> (Decoder, Vec<Vec<u8>>, Codec, u32, u32, Vec<u8>) {
    let mut dm = Demuxer::new(std::io::Cursor::new(data)).unwrap();
    let (si, codec, w, h, extra) = dm
        .header()
        .streams
        .iter()
        .enumerate()
        .find_map(|(i, s)| match &s.kind {
            StreamKind::Video { fourcc, width, height, extradata, .. } => {
                let c = if fourcc == b"RV10" { Codec::Rv10 } else { Codec::Rv20 };
                Some((i, c, u32::from(*width), u32::from(*height), extradata.clone()))
            }
            _ => None,
        })
        .unwrap();
    let mut out = Vec::new();
    while let Some(p) = dm.next_packet().unwrap() {
        if p.stream == si {
            out.push(p.data);
        }
    }
    (Decoder::new(codec, w, h, &extra).unwrap(), out, codec, w, h, extra)
}

/// Random byte changes and cuts in every packet: errors or concealed pictures, never a panic.
fn hammer(data: &[u8], rounds: usize) {
    let (_, pk, codec, w, h, extra) = packets(data);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..rounds {
        let mut dec = Decoder::new(codec, w, h, &extra).unwrap();
        for p in &pk {
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
                assert_eq!(pic.y.len(), (w * h) as usize);
            }
        }
        let _ = dec.flush();
    }
}

#[test]
fn damaged_streams_do_not_panic() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("realvideo damage") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-rv-dmg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for codec in ["rv10", "rv20"] {
        let f = encode(&ff, &dir, codec, 96, 64, &["-q:v", "5", "-g", "6"]);
        hammer(&std::fs::read(&f).unwrap(), 60);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let g2 = cfa4_testkit::fixtures::fixtures_dir("real").join("G2_with_SVT_320_240.rm");
    if let Ok(d) = std::fs::read(g2) {
        hammer(&d, 3);
    }
}

#[test]
fn garbage_and_bad_parameters_are_errors() {
    for codec in [Codec::Rv10, Codec::Rv20] {
        assert!(Decoder::new(codec, 0, 16, &[0, 0, 0, 8, 0x10, 0, 0, 0]).is_err());
        assert!(Decoder::new(codec, 16, 16, &[1, 2]).is_err());
    }
    let mut d = Decoder::new(Codec::Rv20, 64, 48, &[0, 0, 0, 8, 0x20, 0x10, 0x30, 0x01]).unwrap();
    let mut seed = 7u64;
    for len in 0..400usize {
        let pkt: Vec<u8> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (seed >> 56) as u8
            })
            .collect();
        let _ = d.decode(&pkt);
    }
    assert!(d.decode(&[]).is_ok_and(|p| p.is_none()));
}
