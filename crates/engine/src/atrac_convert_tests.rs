use std::path::PathBuf;

use serde_json::json;

use super::*;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fc-atrac-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Two seconds of a 48 kHz tone pair (440 Hz left, 660 Hz right) as a 16-bit WAV.
fn source_wav(d: &std::path::Path, channels: u16, rate: u32) -> PathBuf {
    let n = rate as usize * 2;
    let mut pcm = Vec::with_capacity(n * usize::from(channels));
    for i in 0..n {
        for c in 0..channels {
            let f = 440.0 * (1.0 + f32::from(c) * 0.5);
            pcm.push((i as f32 * 2.0 * std::f32::consts::PI * f / rate as f32).sin() * 0.5);
        }
    }
    let p = d.join(format!("src_{channels}_{rate}.wav"));
    std::fs::write(&p, cfa4_media::wav::write_wav16(&pcm, channels, rate)).unwrap();
    p
}

fn audio_of(path: &std::path::Path) -> (u32, usize, Vec<Vec<f32>>) {
    let src = cfa4_media::open_path(path, &cfa4_codecs::openers()).unwrap();
    let st = src.info().audio_streams[0].clone();
    let frames = (i128::from(src.info().duration.0) * i128::from(st.sample_rate) / i128::from(TICKS_PER_SECOND)) as usize;
    let buf = src.audio(0, frames, st.sample_rate).unwrap();
    (st.sample_rate, st.channels as usize, buf.channels)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn wav_to_at3_and_back() {
    let d = dir("roundtrip");
    for (channels, kbps) in [(2u16, 132u64), (2, 66), (1, 105)] {
        let src = source_wav(&d, channels, 48_000);
        let at3 = d.join(format!("out_{channels}_{kbps}.at3"));
        let mut s = Session::default();
        let r = s.execute(CMD, json!({"input": src, "output": at3, "bitrateKbps": kbps, "wait": true})).unwrap();
        assert_eq!(r["output"], json!(at3.display().to_string()));
        let (rate, ch, audio) = audio_of(&at3);
        assert_eq!((rate, ch), (44_100, usize::from(channels)), "{kbps} kbit/s");
        // the tone survives: about the source's level in the middle of the file
        let mid = &audio[0][30_000..60_000];
        assert!((rms(mid) - 0.5 / 2f32.sqrt()).abs() < 0.05, "{kbps} kbit/s {channels} ch: rms {}", rms(mid));
        // and back to WAV (24-bit), at the AT3's rate
        let wav = d.join(format!("back_{channels}_{kbps}.wav"));
        s.execute(CMD, json!({"input": at3, "output": wav, "bits": 24, "wait": true})).unwrap();
        let (rate, ch, back) = audio_of(&wav);
        assert_eq!((rate, ch), (44_100, usize::from(channels)));
        assert!(back[0].len() >= 2 * 44_100, "the whole source and the encoder delay: {}", back[0].len());
        assert!((rms(&back[0][30_000..60_000]) - rms(mid)).abs() < 0.01);
    }
    // no partial files left behind
    assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".cfa4-partial")));
}

#[test]
fn more_than_two_channels_keep_the_front_pair() {
    let d = dir("surround");
    let src = source_wav(&d, 6, 44_100);
    let at3 = d.join("six.at3");
    let mut s = Session::default();
    s.execute(CMD, json!({"input": src, "output": at3, "wait": true})).unwrap();
    assert_eq!(audio_of(&at3).1, 2);
}

#[test]
fn background_job_and_formats() {
    let d = dir("job");
    let src = source_wav(&d, 2, 44_100);
    let mut s = Session::default();
    let r = s.execute(CMD, json!({"input": src, "output": d.join("bg.at3")})).unwrap();
    let id = r["job"].as_u64().unwrap();
    let t0 = std::time::Instant::now();
    loop {
        let jobs = s.execute("jobs.list", json!({})).unwrap();
        let j = jobs.as_array().unwrap().iter().find(|j| j["id"] == json!(id)).unwrap().clone();
        if j["finished"] == json!(true) {
            assert!(j["result"]["error"].is_null(), "{j}");
            break;
        }
        assert!(t0.elapsed().as_secs() < 60, "the job did not finish");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let f = s.execute("audio.atracConvert.formats", json!({})).unwrap();
    assert_eq!(f["outputs"][1]["bitrateKbps"], json!([66, 105, 132]));
}

#[test]
fn bad_requests_are_errors() {
    let d = dir("bad");
    let src = source_wav(&d, 2, 44_100);
    let mut s = Session::default();
    for p in [
        json!({"output": d.join("x.at3")}),
        json!({"input": src}),
        json!({"input": src, "output": d.join("x.at3"), "bitrateKbps": 96}),
        json!({"input": src, "output": d.join("x.at9")}),
        json!({"input": src, "output": d.join("x.mp3")}),
        json!({"input": src, "output": d.join("x.wav"), "bits": 8}),
        json!({"input": src, "output": src}),
        json!({"input": d.join("missing.wav"), "output": d.join("x.wav"), "wait": true}),
    ] {
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
    }
    // a file that is not audio
    let junk = d.join("junk.at3");
    std::fs::write(&junk, b"RIFF\x04\x00\x00\x00WAVEjunk").unwrap();
    assert!(s.execute(CMD, json!({"input": junk, "output": d.join("y.wav"), "wait": true})).is_err());
    assert!(!d.join("y.wav").exists());
}
