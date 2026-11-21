//! id RoQ against ffmpeg: files its own `roq` muxer writes (`roqvideo` and `roq_dpcm`
//! encoders: motion, vector quantisation, mono and stereo sound), and streams whose every
//! syntax element is random within the bitstream's rules (every block code, motion bias,
//! partial codebooks, codebook updates) written by this test, decoded by ffmpeg and by
//! `cfa4_roq` to the same pictures and samples. Damaged files must not panic.

use cfa4_roq::{Decoder, decode_audio, parse};
use std::path::Path;
use std::process::Command;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn chunk(kind: u16, size: u32, arg: u16, body: &[u8]) -> Vec<u8> {
    let mut v = kind.to_le_bytes().to_vec();
    v.extend_from_slice(&size.to_le_bytes());
    v.extend_from_slice(&arg.to_le_bytes());
    v.extend_from_slice(body);
    v
}

/// The file header and the info chunk.
fn start(w: u16, h: u16, fps: u16) -> Vec<u8> {
    let mut v = vec![0x84, 0x10, 0xFF, 0xFF, 0xFF, 0xFF];
    v.extend_from_slice(&fps.to_le_bytes());
    let mut info = w.to_le_bytes().to_vec();
    info.extend_from_slice(&h.to_le_bytes());
    info.extend_from_slice(&[8, 0, 4, 0]);
    v.extend(chunk(0x1001, 8, 0, &info));
    v
}

/// One video packet whose every element is random: `first` has only vector blocks (a picture
/// made from nothing else is defined); `codebook` (nv1, nv2) loads codebooks first.
fn random_packet(w: usize, h: usize, first: bool, codebook: Option<(usize, usize)>, loaded: (usize, usize), rng: &mut Rng) -> (Vec<u8>, (usize, usize)) {
    let (nv1, nv2) = codebook.unwrap_or(loaded);
    let loaded = if codebook.is_some() { (nv1.max(1), nv2.max(1)) } else { loaded };
    let mut out = Vec::new();
    if let Some((n1, n2)) = codebook {
        let mut body = Vec::new();
        for _ in 0..n1 {
            (0..6).for_each(|_| body.push(rng.below(256) as u8));
        }
        for _ in 0..n2 {
            (0..4).for_each(|_| body.push(rng.below(n1 as u64) as u8));
        }
        // a count of 256 is written as 0 (and 0 quad entries means 256 when the chunk is big enough)
        out.extend(chunk(0x1002, body.len() as u32, ((n1 % 256) << 8 | n2 % 256) as u16, &body));
    }
    let (n1, n2) = loaded;
    let mut data = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    let mut code = |data: &mut Vec<u8>, rng: &mut Rng, first: bool| -> u8 {
        if queue.is_empty() {
            let mut word = 0u16;
            for i in 0..8 {
                let c = if first { 2 + rng.below(2) as u8 } else { rng.below(4) as u8 };
                word |= u16::from(c) << ((7 - i) * 2);
                queue.push_back(c);
            }
            data.extend_from_slice(&word.to_le_bytes());
        }
        queue.pop_front().unwrap()
    };
    for _mb in 0..(w / 16) * (h / 16) {
        for _blk in 0..4 {
            match code(&mut data, rng, first) {
                0 => {}
                1 => data.push(rng.below(256) as u8),
                2 => data.push(rng.below(n2 as u64) as u8),
                _ => {
                    for _ in 0..4 {
                        match code(&mut data, rng, first) {
                            0 => {}
                            1 => data.push(rng.below(256) as u8),
                            2 => data.push(rng.below(n2 as u64) as u8),
                            _ => (0..4).for_each(|_| data.push(rng.below(n1 as u64) as u8)),
                        }
                    }
                }
            }
        }
    }
    let arg = if first { 0 } else { rng.below(0x10000) as u16 };
    out.extend(chunk(0x1011, data.len() as u32, arg, &data));
    (out, loaded)
}

fn random_file(w: usize, h: usize, frames: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut file = start(w as u16, h as u16, 30);
    let mut loaded = (1, 1);
    for f in 0..frames {
        let cb = (f == 0 || rng.below(4) == 0).then(|| {
            let n1 = if rng.below(5) == 0 { 256 } else { 1 + rng.below(255) as usize };
            let n2 = if rng.below(5) == 0 { 256 } else { 1 + rng.below(255) as usize };
            (n1, n2)
        });
        let (p, l) = random_packet(w, h, f == 0, cb, loaded, &mut rng);
        loaded = l;
        file.extend(p);
    }
    file
}

fn run(ff: &Path, data: &[u8], tag: &str, args: &[&str]) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("cfa4-roq-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join("in.roq");
    std::fs::write(&input, data).ok()?;
    let out = Command::new(ff).args(["-v", "error", "-f", "roq", "-i"]).arg(&input).args(args).arg("-").output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    out.status.success().then_some(out.stdout)
}

fn ffmpeg_video(ff: &Path, data: &[u8], tag: &str) -> Option<Vec<u8>> {
    run(ff, data, tag, &["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "yuvj444p", "-fps_mode", "passthrough"])
}

fn ours_video(data: &[u8]) -> Vec<u8> {
    let s = parse(data).expect("parse");
    let mut d = Decoder::new(s.width, s.height).expect("decoder");
    let mut out = Vec::new();
    for v in &s.video {
        let f = d.decode(v.of(data));
        for p in &f.planes {
            out.extend_from_slice(p);
        }
    }
    out
}

/// Planar frames to ffmpeg's yuvj444p layout are the same (Y, U, V planes per frame).
fn compare(tag: &str, ours: &[u8], theirs: &[u8], frame_bytes: usize) {
    assert_eq!(ours.len(), theirs.len(), "{tag}: length");
    if let Some(i) = ours.iter().zip(theirs).position(|(a, b)| a != b) {
        let n = ours.iter().zip(theirs).filter(|(a, b)| a != b).count();
        panic!("{tag}: first difference at byte {i} (frame {}, plane {}), {n} differ", i / frame_bytes, (i % frame_bytes) / (frame_bytes / 3));
    }
}

#[test]
fn random_syntax_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("roq oracle") else { return };
    for (i, &(w, h, frames)) in [(16, 16, 8), (32, 16, 10), (48, 32, 12), (64, 48, 9), (160, 96, 6)].iter().enumerate() {
        for round in 0..8u64 {
            let tag = format!("r{i}-{round}");
            let data = random_file(w, h, frames, 0x40_0000 + i as u64 * 100 + round);
            let theirs = ffmpeg_video(&ff, &data, &tag).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
            compare(&tag, &ours_video(&data), &theirs, w * h * 3);
        }
    }
}

fn ffmpeg_make(ff: &Path, tag: &str, args: &[&str]) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("cfa4-roq-make-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join("x.roq");
    let ok = Command::new(ff).args(["-v", "error", "-y"]).args(args).arg(&out).status().ok()?.success();
    let data = ok.then(|| std::fs::read(&out).ok()).flatten();
    let _ = std::fs::remove_dir_all(&dir);
    data
}

#[test]
fn files_ffmpeg_writes_decode_the_same() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("roq oracle") else { return };
    let mut made = 0;
    for (tag, size, audio) in [("mono", "128x96", 1), ("stereo", "256x144", 2), ("silent", "64x64", 0)] {
        let video = format!("testsrc2=size={size}:rate=30:duration=2");
        let sound = "sine=frequency=440:sample_rate=22050:duration=2";
        let mut args = vec!["-f", "lavfi", "-i", video.as_str()];
        if audio > 0 {
            args.extend(["-f", "lavfi", "-i", sound]);
        }
        let ch = audio.to_string();
        args.extend(["-c:v", "roqvideo", "-g", "8"]);
        if audio > 0 {
            args.extend(["-c:a", "roq_dpcm", "-ac", ch.as_str(), "-ar", "22050"]);
        }
        let Some(data) = ffmpeg_make(&ff, tag, &args) else {
            eprintln!("roq: ffmpeg cannot write {tag}; skipped");
            continue;
        };
        made += 1;
        let s = parse(&data).unwrap_or_else(|e| panic!("{tag}: {e}"));
        assert!(!s.damaged, "{tag}");
        assert!(s.video.len() >= 30, "{tag}: {} video packets", s.video.len());
        let theirs = ffmpeg_video(&ff, &data, tag).unwrap_or_else(|| panic!("{tag}: ffmpeg video"));
        compare(tag, &ours_video(&data), &theirs, s.width as usize * s.height as usize * 3);
        if let Some(ch) = s.audio_channels {
            let mut pcm = Vec::new();
            for a in &s.audio {
                pcm.extend(decode_audio(a.of(&data), ch).unwrap());
            }
            let want = run(&ff, &data, tag, &["-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le"]).expect("ffmpeg audio");
            let ours: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
            compare(&format!("{tag} audio"), &ours, &want, 3);
        }
    }
    assert!(made > 0 || std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "ffmpeg wrote no RoQ file");
}

#[test]
fn random_audio_matches_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("roq oracle") else { return };
    let mut rng = Rng(0xA0D10);
    for (tag, stereo) in [("a-mono", false), ("a-stereo", true)] {
        let mut file = start(16, 16, 30);
        let (p, _) = random_packet(16, 16, true, Some((10, 10)), (1, 1), &mut rng);
        file.extend(p);
        for _ in 0..6 {
            let n = 100 + 2 * rng.below(900) as usize;
            let body: Vec<u8> = (0..n).map(|_| rng.below(256) as u8).collect();
            file.extend(chunk(if stereo { 0x1021 } else { 0x1020 }, n as u32, rng.below(0x10000) as u16, &body));
        }
        let want = run(&ff, &file, tag, &["-map", "0:a:0", "-f", "s16le", "-c:a", "pcm_s16le"]).unwrap_or_else(|| panic!("{tag}: ffmpeg failed"));
        let s = parse(&file).unwrap();
        let mut pcm = Vec::new();
        for a in &s.audio {
            pcm.extend(decode_audio(a.of(&file), s.audio_channels.unwrap()).unwrap());
        }
        let ours: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        compare(tag, &ours, &want, 3);
    }
}

#[test]
fn damaged_files_do_not_panic() {
    let mut rng = Rng(0xBAD0_0101);
    for round in 0..300u64 {
        let mut data = random_file(32, 32, 5, 0x77_0000 + round);
        for _ in 0..1 + rng.below(10) {
            let at = rng.below(data.len() as u64) as usize;
            data[at] = rng.below(256) as u8;
        }
        if round % 5 == 0 {
            data.truncate(rng.below(data.len() as u64) as usize);
        }
        let Ok(s) = parse(&data) else { continue };
        let Ok(mut d) = Decoder::new(s.width, s.height) else { continue };
        for v in &s.video {
            let _ = d.decode(v.of(&data));
        }
        for a in &s.audio {
            let _ = decode_audio(a.of(&data), s.audio_channels.unwrap_or(1));
        }
    }
}

#[test]
fn probe_and_bad_headers() {
    assert!(!cfa4_roq::probe(b"RIFF"));
    assert!(parse(b"RIFF....").is_err());
    let mut f = start(20, 16, 30); // not a multiple of 16
    f.extend(chunk(0x1011, 2, 0, &[0, 0]));
    assert!(parse(&f).is_err());
}
