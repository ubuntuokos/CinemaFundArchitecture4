//! An AMV file (made by ffmpeg's AMV encoder) read through the media source: every picture equals
//! what `cfa4_amv::Decoder` gives, whichever order the pictures are asked for in, and the sound
//! reads from the middle.

use std::process::{Command, Stdio};
use std::sync::Arc;

use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_time::{FrameRate, Tick};

#[test]
fn amv_file_reads_like_the_decoder() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("amv") else { return };
    let path = std::env::temp_dir().join(format!("cfa4-amv-file-{}.amv", std::process::id()));
    let st = Command::new(&ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=96x80:rate=15", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050", "-t", "3"])
        .args(["-c:v", "amv", "-c:a", "adpcm_ima_amv", "-block_size", "1470", "-ac", "1", "-ar", "22050", "-f", "amv"])
        .arg(&path)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    if !st.success() {
        eprintln!("skipped: this ffmpeg cannot write AMV");
        return;
    }
    let data = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let amv = cfa4_amv::Amv::parse(&data).unwrap();
    assert_eq!((amv.width, amv.height, amv.video.len()), (96, 80, 45));
    let dec = cfa4_amv::Decoder::new(96, 80).unwrap();
    let want: Vec<[Vec<u8>; 3]> = amv.video.iter().map(|c| dec.decode(&data[c.offset..c.offset + c.len]).unwrap().planes).collect();
    assert!(want[3] != want[4]);

    let src = cfa4_codecs::open_bytes("clip.amv", Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, "AMV");
    let rate = FrameRate::new(i64::from(amv.frame_rate.0), i64::from(amv.frame_rate.1));
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.frame_rate), (96, 80, rate));
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (22050, 1, "IMA ADPCM (AMV)"));

    let at = |k: usize| FrameRequest::full(rate.tick_of(k as i64) + Tick::from_rational(1, 1, 120));
    for k in [0, 1, 2, 44, 5, 30, 29, 31, 0, 20, 19, 43, 42, 10] {
        let f = src.video_frame(at(k)).unwrap();
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("not 8-bit YUV") };
        for p in 0..3 {
            assert_eq!(*planes[p], want[k][p], "frame {k}, plane {p}");
        }
    }

    let mut pcm = Vec::new();
    for c in &amv.audio {
        pcm.extend(cfa4_amv::decode_audio(&data[c.offset..c.offset + c.len]).unwrap());
    }
    let got = src.audio(10_000, 2_000, 22050).unwrap();
    let exp: Vec<f32> = pcm[10_000..12_000].iter().map(|&v| f32::from(v) / 32768.0).collect();
    assert_eq!(got.channels[0], exp);
}
