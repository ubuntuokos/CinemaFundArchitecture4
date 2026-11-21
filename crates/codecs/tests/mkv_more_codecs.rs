//! Matroska tracks whose decoders exist in other containers: AVS1 (`V_CAVS`), RealVideo
//! (`V_REAL/RV10` / `RV20`), JPEG 2000 (`V_JPEG2000`, and ffmpeg's VfW `mjp2`) and ALAC
//! (`A_ALAC`), read through the media layer. Video is checked against the same pictures decoded
//! outside Matroska, ALAC against ffmpeg's decode.

mod common;

use std::io::Cursor;
use std::sync::Arc;

use cfa4_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};
use cfa4_media::{FrameRequest, SharedSource};
use common::*;

fn mux(track: TrackSpec, frames: &[(i64, bool, Vec<u8>)], dur_ns: u64) -> Vec<u8> {
    let mut w = MkvWriter::new(Cursor::new(Vec::new()), vec![track], MuxOptions::default()).expect("writer");
    for (pts, key, data) in frames {
        w.write_frame(0, *pts, *key, data, Some(dur_ns)).expect("frame");
    }
    w.finish().expect("finish").into_inner()
}

fn open(name: &str, data: Vec<u8>) -> SharedSource {
    cfa4_codecs::open_bytes(name, Arc::<[u8]>::from(data)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn video_track(id: &str, w: u32, h: u32, private: Vec<u8>) -> TrackSpec {
    let mut t = TrackSpec::new(TrackKind::Video, id);
    t.video_size = Some((w, h));
    t.codec_private = private;
    t.default_duration_ns = Some(40_000_000);
    t
}

fn at(i: i64) -> cfa4_time::Tick {
    cfa4_time::FrameRate::new(25, 1).tick_of(i)
}

#[test]
fn avs1_in_matroska_decodes_like_the_raw_stream() {
    use cfa4_cavs::synth::{Params, clean_stream};
    let p = Params { width: 96, height: 64, low_delay: true, seed: 31, pictures: 12 };
    let (data, _) = clean_stream(p).expect("a stream");
    let packets: Vec<Vec<u8>> = cfa4_cavs::split_packets(&data).into_iter().map(<[u8]>::to_vec).collect();
    // a sequence header (and any extension) belongs to the picture that follows it
    let mut frames = Vec::new();
    let mut head: Vec<u8> = Vec::new();
    for pk in packets {
        match cfa4_cavs::picture_type(&pk) {
            Some(t) => {
                head.extend_from_slice(&pk);
                let n = frames.len() as i64;
                frames.push((n * 40_000_000, t == cfa4_cavs::PictureType::I, std::mem::take(&mut head)));
            }
            None => head.extend_from_slice(&pk),
        }
    }
    // anything after the last picture (a sequence end code) stays with it
    if let Some(last) = frames.last_mut() {
        last.2.extend_from_slice(&head);
    }
    let mut d = cfa4_cavs::Decoder::new();
    let mut want: Vec<Vec<u8>> = Vec::new();
    for (_, _, f) in &frames {
        for pk in cfa4_cavs::split_packets(f) {
            if let Some(o) = d.decode(pk).expect("decodes") {
                want.push(o.planes.concat());
            }
        }
    }
    want.extend(d.flush().map(|f| f.planes.concat()));
    let mkv = mux(video_track("V_CAVS", 96, 64, Vec::new()), &frames, 40_000_000);
    let src = open("avs1.mkv", mkv);
    let v = src.info().video.clone().expect("video");
    assert_eq!((v.width, v.height, v.codec.as_str()), (96, 64, cfa4_codecs::cavs::NAME));
    for (i, w) in want.iter().enumerate().rev() {
        let f = src.video_frame(FrameRequest::full(at(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        let got: Vec<u8> = planes(&f).iter().flat_map(|p| p.iter().map(|&x| x as u8)).collect();
        assert!(&got == w, "frame {i} differs");
    }
}

#[test]
fn jpeg2000_in_matroska_decodes_like_its_codestreams() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let Some(mkv_ff) =
        fixture(&ff, "mkv_j2k.mkv", &["-f", "lavfi", "-i", "testsrc2=size=96x64:rate=25:duration=0.6", "-c:v", "jpeg2000", "-pix_fmt", "yuv444p"])
    else {
        cfa4_testkit::oracle::skip("jpeg2000", "ffmpeg can't write it");
        return;
    };
    // the codestreams, as ffmpeg's own VfW track holds them
    let reader: Vec<u8> = std::fs::read(&mkv_ff).unwrap();
    let file = cfa4_matroska::open(reader.clone()).expect("matroska");
    let t = &file.tracks[0];
    assert_eq!(t.codec_id, "V_MS/VFW/FOURCC");
    let src_ff = open("j2k_vfw.mkv", reader.clone());
    let n = t.samples.len();
    assert_eq!(n, 15);
    let frames: Vec<(i64, bool, Vec<u8>)> = (0..n).map(|i| (i as i64 * 40_000_000, true, file.read_sample(&reader, 0, i).expect("sample"))).collect();
    let spec_mkv = open("j2k.mkv", mux(video_track("V_JPEG2000", 96, 64, Vec::new()), &frames, 40_000_000));
    for i in (0..n as i64).rev() {
        let a = src_ff.video_frame(FrameRequest::full(at(i))).expect("vfw frame");
        let b = spec_mkv.video_frame(FrameRequest::full(at(i))).expect("V_JPEG2000 frame");
        assert!(planes(&a) == planes(&b) && !planes(&a).is_empty(), "frame {i}");
    }
    assert_eq!(src_ff.info().video.as_ref().map(|v| v.codec.as_str()), Some("JPEG 2000"));
    assert_eq!(spec_mkv.info().video.as_ref().map(|v| v.codec.as_str()), Some("JPEG 2000"));
}

#[test]
fn realvideo_in_matroska_decodes_like_realmedia() {
    let ff = cfa4_testkit::require_ffmpeg!();
    for (codec, id, size) in [("rv10", "V_REAL/RV10", "176x144"), ("rv20", "V_REAL/RV20", "320x240")] {
        let name = format!("mkv_real_{codec}.rm");
        let src = format!("testsrc2=size={size}:rate=25:duration=1,noise=alls=8:allf=t");
        let Some(p) = fixture(&ff, &name, &["-f", "lavfi", "-i", &src, "-c:v", codec, "-g", "12", "-q:v", "4", "-f", "rm"]) else {
            cfa4_testkit::oracle::skip(codec, "ffmpeg can't write it");
            continue;
        };
        let rm = std::fs::read(&p).unwrap();
        let mut dm = cfa4_realmedia::Demuxer::new(Cursor::new(&rm)).expect("realmedia");
        let (fourcc, w, h, fps, extra) = dm
            .header()
            .streams
            .iter()
            .find_map(|s| match &s.kind {
                cfa4_realmedia::StreamKind::Video { fourcc, width, height, fps_16_16, extradata } => {
                    Some((*fourcc, *width, *height, *fps_16_16, extradata.clone()))
                }
                _ => None,
            })
            .expect("a video stream");
        // real_video_props_t: size, `VIDO`, FourCC, width, height, then the frame rate at 22
        let mut props = vec![0u8; 26];
        props[..4].copy_from_slice(&(26 + extra.len() as u32).to_be_bytes());
        props[4..8].copy_from_slice(b"VIDO");
        props[8..12].copy_from_slice(&fourcc);
        props[12..14].copy_from_slice(&w.to_be_bytes());
        props[14..16].copy_from_slice(&h.to_be_bytes());
        props[22..26].copy_from_slice(&fps.to_be_bytes());
        props.extend_from_slice(&extra);
        let mut frames = Vec::new();
        while let Some(pk) = dm.next_packet().expect("packet") {
            if !pk.partial && matches!(dm.header().streams[pk.stream].kind, cfa4_realmedia::StreamKind::Video { .. }) {
                frames.push((i64::from(pk.timestamp_ms) * 1_000_000, pk.keyframe, pk.data));
            }
        }
        assert_eq!(frames.len(), 25, "{codec}");
        let rm_src = open(&name, rm.clone());
        let mkv_src = open("real.mkv", mux(video_track(id, u32::from(w), u32::from(h), props), &frames, 40_000_000));
        assert_eq!(mkv_src.info().video.as_ref().map(|v| (v.width, v.height)), Some((u32::from(w), u32::from(h))));
        for i in (0..25).rev() {
            let a = rm_src.video_frame(FrameRequest::full(at(i))).expect("rm frame");
            let b = mkv_src.video_frame(FrameRequest::full(at(i))).unwrap_or_else(|e| panic!("{codec} frame {i}: {e}"));
            assert!(planes(&a) == planes(&b), "{codec}: frame {i} differs");
        }
    }
}

#[test]
fn alac_in_matroska_matches_ffmpeg() {
    let ff = cfa4_testkit::require_ffmpeg!();
    // (name, ffmpeg sample format, rate, channels)
    for (name, fmt, rate, ch) in [("s16", "s16p", 44100u32, 2usize), ("s24", "s32p", 48000, 2), ("mono", "s16p", 32000, 1)] {
        let src = format!("aevalsrc=exprs='0.5*sin(2*PI*440*t)|0.3*sin(2*PI*220*t)+(random(0)-0.5)*0.2':s={rate}:d=2.5");
        let layout = if ch == 1 { format!("pan=mono|c0=c0,aformat=sample_fmts={fmt}") } else { format!("aformat=sample_fmts={fmt}") };
        let file = format!("mkv_alac_{name}.mka");
        let Some(path) = fixture(&ff, &file, &["-f", "lavfi", "-i", &src, "-af", &layout, "-c:a", "alac"]) else {
            cfa4_testkit::oracle::skip(name, "ffmpeg can't write it");
            continue;
        };
        let want = ffmpeg_audio_f32(&ff, &path, &[]);
        let s = open(&file, std::fs::read(&path).unwrap());
        let info = s.info().audio().expect("audio");
        assert_eq!((info.sample_rate, info.channels as usize, info.codec.as_str()), (rate, ch, "ALAC"), "{name}");
        // ffmpeg's output is interleaved
        let frames = (want.len() / ch).min(rate as usize * 2);
        let chan = |c: usize, from: usize, n: usize| -> Vec<f32> { (from..from + n).map(|i| want[i * ch + c]).collect() };
        let b = s.audio(0, frames, rate).unwrap_or_else(|e| panic!("{name}: {e}"));
        for c in 0..ch {
            let w = chan(c, 0, frames);
            if let Some(i) = b.channels[c].iter().zip(&w).position(|(a, w)| (a - w).abs() > 1e-6) {
                panic!("{name}: channel {c} sample {i} differs: ours {}, ffmpeg {}", b.channels[c][i], w[i]);
            }
        }
        let mid = frames / 2 + 77;
        let part = s.audio(mid as i64, 3000, rate).unwrap();
        assert!(part.channels[0] == chan(0, mid, 3000), "{name}: read from the middle differs");
    }
}
