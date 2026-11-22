//! Oracle: ffmpeg decodes AMV files made by its own AMV encoder (and the FATE sample, when
//! fetched into `target/fixtures/amv/fate/`); video must match bit for bit, audio exactly.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn ffmpeg_raw(ff: &Path, file: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(args).args(["-"]).stdin(Stdio::null()).output().expect("ffmpeg runs");
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn check(ff: &Path, file: &Path) {
    let data = std::fs::read(file).unwrap();
    let amv = cfa4_amv::Amv::parse(&data).unwrap();
    let dec = cfa4_amv::Decoder::new(amv.width, amv.height).unwrap();
    let want_v = ffmpeg_raw(ff, file, &["-map", "0:v:0", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuvj420p"]);
    let (w, h) = (amv.width as usize, amv.height as usize);
    let frame = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
    assert_eq!(want_v.len() % frame, 0);
    assert_eq!(amv.video.len(), want_v.len() / frame, "{}: frame count", file.display());
    for (i, c) in amv.video.iter().enumerate() {
        let pic = dec.decode(&data[c.offset..c.offset + c.len]).unwrap();
        let got: Vec<u8> = pic.planes.concat();
        let want = &want_v[i * frame..(i + 1) * frame];
        let bad = got.iter().zip(want).filter(|(a, b)| a != b).count();
        let maxd = got.iter().zip(want).map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs()).max().unwrap_or(0);
        let first = got.iter().zip(want).position(|(a, b)| a != b);
        if bad > 0 {
            eprintln!("frame {i}: max diff {maxd}, first at {first:?} of {frame}; got {:?} want {:?}", &got[..8], &want[..8]);
        }
        assert_eq!(bad, 0, "{}: frame {i}: {bad} samples differ", file.display());
    }
    if amv.audio.is_empty() {
        return;
    }
    let want_a = ffmpeg_raw(ff, file, &["-map", "0:a:0", "-f", "s16le"]);
    let mut got_a = Vec::new();
    for c in &amv.audio {
        for s in cfa4_amv::decode_audio(&data[c.offset..c.offset + c.len]).unwrap() {
            got_a.extend_from_slice(&s.to_le_bytes());
        }
    }
    assert_eq!(got_a.len(), want_a.len(), "{}: audio length", file.display());
    assert!(got_a == want_a, "{}: audio differs", file.display());
}

#[test]
fn synthetic_files_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("amv") else { return };
    let dir = std::env::temp_dir().join(format!("cfa4-amv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (size, strict, src) in [("160x128", false, "testsrc2"), ("176x144", false, "mandelbrot"), ("130x98", true, "testsrc2"), ("64x48", false, "life")] {
        let path = dir.join(format!("{size}.amv"));
        let mut args = vec!["-v", "error", "-y", "-f", "lavfi", "-i"];
        let input = format!("{src}=size={size}:rate=15");
        args.push(&input);
        args.extend(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050", "-t", "2"]);
        if strict {
            args.extend(["-strict", "-1"]);
        }
        args.extend(["-c:v", "amv", "-c:a", "adpcm_ima_amv", "-block_size", "1470", "-ac", "1", "-ar", "22050", "-f", "amv"]);
        let st = Command::new(&ff).args(&args).arg(&path).stdin(Stdio::null()).status().unwrap();
        if !st.success() {
            eprintln!("skipped {size}: this ffmpeg cannot make it");
            continue;
        }
        check(&ff, &path);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fate_sample_matches_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("amv") else { return };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/amv/fate");
    let Ok(rd) = std::fs::read_dir(&root) else {
        eprintln!("skipped: no FATE samples in {}", root.display());
        return;
    };
    for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "amv")) {
        check(&ff, &e.path());
    }
}
