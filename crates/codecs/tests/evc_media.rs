//! MPEG-5 EVC Baseline (`cfa4-evc`) through the media layer: raw `.evc` streams opened by
//! content with frames asked for in a shuffled order, and `evc1` samples through the MP4 decoder
//! factory, against the pictures the decoder gives when it runs through the stream directly.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cfa4_codecs::VideoDecoder;
use cfa4_frame::PixelData;
use cfa4_isobmff::{CodecConfig, SampleEntry};
use cfa4_media::FrameRequest;

#[path = "../../evc/tests/synth/mod.rs"]
mod synth;

use synth::{Content, PicCfg, Rng, SLICE_B, SLICE_I, SLICE_P, SeqCfg};

fn pic(slice_type: u32, idr: bool, tid: u32, num_refp: [u32; 2]) -> PicCfg {
    PicCfg { slice_type, idr, tid, num_refp, qp: 28, qp_u_offset: 1, qp_v_offset: -1, deblock: true, content: Content::Random }
}

/// IDR, sub-GOPs of two B pictures, a second IDR and more sub-GOPs.
fn random_access() -> Vec<PicCfg> {
    let mut v = Vec::new();
    for _ in 0..2 {
        v.push(pic(SLICE_I, true, 0, [0, 0]));
        for _ in 0..3 {
            v.push(pic(SLICE_B, false, 0, [1, 1]));
            v.push(pic(SLICE_B, false, 1, [1, 1]));
        }
    }
    v
}

fn low_delay(n: u32) -> Vec<PicCfg> {
    let mut v = vec![pic(SLICE_I, true, 0, [0, 0])];
    for k in 1..n {
        v.push(pic(SLICE_P, false, 0, [k.min(2), 0]));
    }
    v
}

fn nal_units(seq: &SeqCfg, pics: &[PicCfg], seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng::new(seed);
    let mut nals = vec![synth::sps(seq), synth::pps(seq)];
    for p in pics {
        nals.push(synth::slice(seq, p, &mut rng));
    }
    nals
}

/// The decoder's pictures in output order, each as all its samples.
fn reference(data: &[u8]) -> Vec<Vec<u16>> {
    let mut d = cfa4_evc::Decoder::new();
    let mut out = Vec::new();
    for nal in cfa4_evc::split_nal_units(data) {
        if let Some(f) = d.decode_nal(nal).expect("decodes") {
            out.push(f.planes.concat());
        }
    }
    out.extend(d.flush().into_iter().map(|f| f.planes.concat()));
    out
}

fn samples(data: &PixelData, chroma: bool) -> Vec<u16> {
    let planes: Vec<Vec<u16>> = match data {
        PixelData::Yuv8 { planes, .. } => planes.iter().map(|p| p.iter().map(|&v| u16::from(v)).collect()).collect(),
        PixelData::Yuv16 { planes, .. } => planes.iter().map(|p| p.to_vec()).collect(),
        _ => panic!("not a YUV frame"),
    };
    planes.into_iter().take(if chroma { 3 } else { 1 }).flatten().collect()
}

#[test]
fn raw_streams_through_the_media_layer() {
    let cases = [
        (SeqCfg { width: 120, height: 72, log2_sub_gop_length: 1, max_num_ref_pics: 1, ..SeqCfg::default() }, random_access()),
        (SeqCfg { width: 80, height: 64, bit_depth: 10, max_num_ref_pics: 2, cu_qp_delta: true, ..SeqCfg::default() }, low_delay(9)),
        (SeqCfg { width: 64, height: 40, chroma: false, max_num_ref_pics: 2, ..SeqCfg::default() }, low_delay(5)),
    ];
    for (n, (seq, pics)) in cases.iter().enumerate() {
        let data = synth::stream(&nal_units(seq, pics, 40 + n as u64));
        let want = reference(&data);
        assert_eq!(want.len(), pics.len());
        let bytes: Arc<[u8]> = data.into();
        let src = cfa4_codecs::openers()
            .iter()
            .find_map(|o| o("clip.evc", bytes.clone()))
            .expect("recognised by content")
            .unwrap_or_else(|e| panic!("case {n}: {e}"));
        let v = src.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (seq.width, seq.height));
        assert_eq!(v.codec, cfa4_codecs::evc::NAME);
        assert_eq!(src.info().duration, v.frame_rate.tick_of(pics.len() as i64));
        let count = pics.len();
        let mut order: Vec<usize> = (0..count).rev().collect();
        order.swap(0, count / 2);
        for i in order.into_iter().chain(0..count) {
            let f = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("case {n} frame {i}: {e}"));
            assert_eq!(samples(&f.data, seq.chroma), want[i], "case {n} frame {i}");
        }
    }
}

#[test]
fn evc1_samples_through_the_decoder_factory() {
    let seq = SeqCfg { width: 96, height: 64, log2_sub_gop_length: 1, max_num_ref_pics: 1, ..SeqCfg::default() };
    let pics = random_access();
    let nals = nal_units(&seq, &pics, 5);
    let want = reference(&synth::stream(&nals));
    // evcC with the parameter sets, then one sample (a 4-byte length and the slice) per picture
    let mut rec = vec![1, 0, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 96, 0, 64, 3, 2];
    for (t, nal) in [(24u8, &nals[0]), (25, &nals[1])] {
        rec.extend_from_slice(&[0x80 | t, 0, 1]);
        rec.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        rec.extend_from_slice(nal);
    }
    let mut raw = vec![0u8; 78];
    raw.extend_from_slice(&((rec.len() + 8) as u32).to_be_bytes());
    raw.extend_from_slice(b"evcC");
    raw.extend_from_slice(&rec);
    let entry = SampleEntry::video(cfa4_codecs::evc::EVC1, CodecConfig::Unknown { fourcc: cfa4_codecs::evc::EVC1, raw }, 96, 64);
    let mut d: Box<dyn VideoDecoder> = cfa4_codecs::software_video_decoder(&entry).expect("an evc1 decoder");
    // display positions of the pictures in decoding order: I0 B2 B1 B4 B3 B6 B5, twice
    let shown = [0i64, 2, 1, 4, 3, 6, 5];
    let mut out = Vec::new();
    for (k, slice) in nals[2..].iter().enumerate() {
        let pts = (k / 7 * 7) as i64 + shown[k % 7];
        let sample = synth::stream(std::slice::from_ref(slice));
        assert_eq!(d.is_random_access(&sample), (k % 7 == 0).then_some(true));
        out.extend(d.decode(&sample, pts).expect("decodes"));
    }
    out.extend(d.flush());
    let pts: Vec<i64> = out.iter().map(|f| f.pts).collect();
    assert_eq!(pts, (0..14).collect::<Vec<_>>());
    for (f, w) in out.iter().zip(&want) {
        assert_eq!(&samples(&f.frame.data, true), w);
    }
    // after a reset the stored parameter sets let decoding start again at the second IDR
    d.reset();
    let sample = synth::stream(std::slice::from_ref(&nals[2 + 7]));
    let mut again = d.decode(&sample, 7).expect("decodes after a reset");
    again.extend(d.flush());
    assert_eq!(again.len(), 1);
    assert_eq!(samples(&again[0].frame.data, true), want[7]);
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/evc")
}

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// ffmpeg has no EVC decoder of its own, but remuxes raw EVC into MP4 (`evc1` with `evcC`).
/// Its raw demuxer gives the packets their decoding order as timestamps, so the stream is
/// low-delay (no reordering).
#[test]
fn mp4_written_by_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let dir = fixtures();
    std::fs::create_dir_all(&dir).unwrap();
    let seq = SeqCfg { width: 112, height: 80, bit_depth: 10, max_num_ref_pics: 2, ..SeqCfg::default() };
    let pics = low_delay(10);
    let data = synth::stream(&nal_units(&seq, &pics, 77));
    let want = reference(&data);
    let raw = dir.join("ld10.evc");
    let mp4 = dir.join("ld10.mp4");
    std::fs::write(&raw, &data).unwrap();
    let o = Command::new("ffmpeg").args(["-v", "error", "-y", "-i"]).arg(&raw).args(["-c", "copy"]).arg(&mp4).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let bytes: Arc<[u8]> = std::fs::read(&mp4).unwrap().into();
    let src = cfa4_codecs::openers().iter().find_map(|o| o("ld10.mp4", bytes.clone())).expect("an MP4").expect("opens");
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height), (112, 80));
    for i in (0..pics.len()).rev().chain(0..pics.len()) {
        let f = src.video_frame(FrameRequest::full(v.frame_rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("frame {i}: {e}"));
        assert_eq!(samples(&f.data, true), want[i], "frame {i}");
    }
}
