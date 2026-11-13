//! MXF export (OP1a / OP-Atom): read back with our demuxer and decoders, and against ffprobe /
//! ffmpeg as external oracles (decode must succeed and match the same essence exported as MOV;
//! PCM sample-exact).

use std::path::Path;
use std::process::Command;

use super::*;
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{FrameRequest, Generator, MediaSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::TICKS_PER_SECOND;

/// 1 s at 24 fps, 320×180: the counting leader over V1, a 440 Hz tone on A1 (stereo mix), start
/// timecode 01:00:00:00.
fn project() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("mxf");
    let r = FrameRate::FPS_24;
    let leader = GeneratorSource::new(Generator::CountingLeader, 320, 180, r, Tick(2 * TICKS_PER_SECOND));
    let tone = GeneratorSource::new(Generator::Tone { hz: 440.0, db: -6.0 }, 320, 180, r, Tick(2 * TICKS_PER_SECOND));
    let add = |p: &mut Project, g: &GeneratorSource| {
        let info = g.info().clone();
        p.add_item(
            &info.name.clone(),
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(g.generator.clone()),
                info,
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        )
    };
    let v = add(&mut p, &leader);
    let a = add(&mut p, &tone);
    let seq = p.new_sequence("MXF Test", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
    let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(24)), r).unwrap();
    let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(24)), r).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(vi);
    q.audio_tracks[0].items.push(ai);
    q.start_timecode = 24 * 3600;
    let mut m = SourceMap::default();
    m.0.insert(v, Arc::new(leader));
    m.0.insert(a, Arc::new(tone));
    (Arc::new(p), seq, m)
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("fc-export-mxf-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().to_string()
}

fn settings(format: Format, codec: MxfVideoCodec, path: &str) -> ExportSettings {
    ExportSettings {
        format,
        path: path.into(),
        mxf_video_codec: codec,
        audio: AudioSettings { bits: 24, sample_rate: Some(48_000), channels: 2, ..Default::default() },
        ..Default::default()
    }
}

fn ffprobe_json(ffprobe: &Path, path: &str) -> serde_json::Value {
    let out = Command::new(ffprobe).args(["-v", "error", "-show_format", "-show_streams", "-count_frames", "-of", "json", path]).output().unwrap();
    assert!(out.status.success(), "ffprobe {path}: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// ffmpeg's decode of `path` (stream `map`) as raw bytes in `fmt`.
fn ffmpeg_raw(ffmpeg: &Path, path: &str, map: &str, args: &[&str]) -> Vec<u8> {
    let out = Command::new(ffmpeg).args(["-v", "error", "-i", path, "-map", map]).args(args).arg("-").output().unwrap();
    assert!(out.status.success(), "ffmpeg {path}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn codec_name(c: MxfVideoCodec) -> &'static str {
    match c {
        MxfVideoCodec::Dnxhr => "dnxhd",
        MxfVideoCodec::ProRes => "prores",
        MxfVideoCodec::H264 => "h264",
        MxfVideoCodec::Mpeg2 | MxfVideoCodec::D10 | MxfVideoCodec::XdcamHd422 => "mpeg2video",
        MxfVideoCodec::V210 => "v210",
        MxfVideoCodec::Uyvy => "rawvideo",
    }
}

#[test]
fn op1a_reads_back_with_our_demuxer_and_decoders() {
    let (p, seq, m) = project();
    for codec in MxfVideoCodec::ALL {
        let path = tmp(&format!("op1a_{codec:?}.mxf"));
        let r = export(&p, seq, &settings(Format::MxfOp1a, codec, &path), &m, &Progress::default()).unwrap();
        assert_eq!(r.frames, 24);
        assert!(r.extra_files.is_empty());
        let bytes = std::fs::read(&path).unwrap();
        let mx = cfa4_mxf::open(&bytes).unwrap();
        assert!(mx.warnings.is_empty(), "{codec:?}: {:?}", mx.warnings);
        assert_eq!(mx.operational_pattern.name(), "OP1a");
        assert_eq!(mx.material_package_name.as_deref(), Some("MXF Test"));
        assert_eq!(mx.timecode.unwrap().format(), "01:00:00:00");
        let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
        assert_eq!(v.samples.len(), 24, "{codec:?}");
        assert_eq!(v.duration, Some(24));
        let a = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Sound).unwrap()];
        assert_eq!(a.stored_sample_frames(), 48_000, "{codec:?}");
        assert_eq!(a.sound.as_ref().unwrap().bits, 24);
        if codec == MxfVideoCodec::H264 {
            assert!(v.temporal_offsets, "B pictures: the index gives the presentation order");
            assert!(v.samples[0].key && v.samples.iter().filter(|s| s.key).count() < 24);
        }
        // our MXF source decodes it like the MOV export of the same sequence
        let src = cfa4_codecs::open_bytes("x.mxf", bytes.into()).unwrap();
        let info = src.info();
        assert_eq!(info.video.as_ref().unwrap().width, 320, "{codec:?}");
        assert!((info.duration.seconds() - 1.0).abs() < 0.05, "{codec:?}: {}", info.duration.seconds());
        let mov = tmp(&format!("op1a_{codec:?}.mov"));
        let ms = ExportSettings { format: codec.encoder_format(), multiplexer: Multiplexer::Mov, ..settings(Format::MxfOp1a, codec, &mov) };
        export(&p, seq, &ms, &m, &Progress::default()).unwrap();
        let msrc = cfa4_codecs::open_bytes("x.mov", std::fs::read(&mov).unwrap().into()).unwrap();
        for f in [0i64, 5, 13, 23, 7] {
            let t = FrameRate::FPS_24.tick_of(f) + Tick(1);
            let a = src.video_frame(FrameRequest::full(t)).unwrap().to_rgba8();
            let b = msrc.video_frame(FrameRequest::full(t)).unwrap().to_rgba8();
            assert!(a == b, "{codec:?}: frame {f} differs from the MOV export");
        }
        let pcm = src.audio(0, 48_000, 48_000).unwrap();
        assert!((pcm.peaks()[0] - 0.5).abs() < 0.05, "{codec:?}: tone at -6 dB: {}", pcm.peaks()[0]);
    }
}

#[test]
fn op_atom_writes_one_file_per_essence() {
    let (p, seq, m) = project();
    let path = tmp("atom.mxf");
    let r = export(&p, seq, &settings(Format::MxfOpAtom, MxfVideoCodec::Dnxhr, &path), &m, &Progress::default()).unwrap();
    assert_eq!(r.extra_files, opatom_audio_paths(&path, 2));
    assert!(r.extra_files[0].ends_with("atom_A1.mxf") && r.extra_files[1].ends_with("atom_A2.mxf"));
    let v = std::fs::read(&path).unwrap();
    let mv = cfa4_mxf::open(&v).unwrap();
    assert_eq!(mv.operational_pattern.name(), "OP-Atom");
    assert_eq!(mv.tracks.len(), 1);
    assert_eq!(mv.tracks[0].wrapping, cfa4_mxf::Wrapping::Clip);
    assert_eq!(mv.tracks[0].samples.len(), 24);
    for (k, a) in r.extra_files.iter().enumerate() {
        let b = std::fs::read(a).unwrap();
        let ma = cfa4_mxf::open(&b).unwrap();
        assert_eq!(ma.operational_pattern.name(), "OP-Atom");
        let t = &ma.tracks[0];
        assert_eq!(t.kind, cfa4_mxf::TrackKind::Sound);
        assert_eq!(t.track_id, 3 + k as u32);
        assert_eq!(t.sound.as_ref().unwrap().channels, 1);
        assert_eq!(t.stored_sample_frames(), 48_000);
        assert_eq!(t.duration, Some(24), "audio edit rate is the frame rate");
        assert_eq!(ma.timecode.unwrap().format(), "01:00:00:00");
        let pcm = ma.read_pcm(&b, 0, 0, 48_000).unwrap();
        assert!((pcm[0].iter().fold(0f32, |a, s| a.max(s.abs())) - 0.5).abs() < 0.05);
    }
}

#[test]
fn mxf_formats_and_presets() {
    assert_eq!(Format::from_name("mxf"), Some(Format::MxfOp1a));
    assert_eq!(Format::from_name("mxf-op1a"), Some(Format::MxfOp1a));
    assert_eq!(Format::from_name("mxf-opatom"), Some(Format::MxfOpAtom));
    assert_eq!(Format::from_name(Format::MxfOpAtom.id()), Some(Format::MxfOpAtom));
    assert_eq!(Format::MxfOp1a.extension(), "mxf");
    let s: ExportSettings = serde_json::from_value(serde_json::json!({"format": "mxf-op1a", "mxfVideoCodec": "proRes"})).unwrap();
    assert_eq!((s.format, s.video_format()), (Format::MxfOp1a, Format::ProRes));
    assert_eq!(s.audio_codec(), AudioCodec::Pcm);
    assert!(s.summary(1920, 1080, FrameRate::FPS_24, 48_000, Tick(TICKS_PER_SECOND)).format.contains("MXF OP1a (Apple ProRes)"));
    let names: Vec<String> = builtin_presets().into_iter().filter(|p| p.settings.format.is_mxf()).map(|p| p.name).collect();
    assert_eq!(names.len(), 9, "{names:?}");
    let s: ExportSettings = serde_json::from_value(serde_json::json!({"format": "mxf-op1a", "mxfVideoCodec": "d10"})).unwrap();
    assert_eq!((s.mxf_video_codec, s.video_format()), (MxfVideoCodec::D10, Format::Mpeg2));
    let s: ExportSettings = serde_json::from_value(serde_json::json!({"format": "mxf-op1a", "mxfVideoCodec": "xdcamHd422"})).unwrap();
    assert_eq!((s.mxf_video_codec, s.video_format()), (MxfVideoCodec::XdcamHd422, Format::Mpeg2));
    assert!(ExportSettings { format: Format::MxfOpAtom, ..s.clone() }.validate().is_err());
    assert_eq!(
        opatom_audio_paths("/a/b/clip.mxf", 2).iter().map(std::path::PathBuf::from).collect::<Vec<_>>(),
        vec![std::path::PathBuf::from("/a/b/clip_A1.mxf"), std::path::PathBuf::from("/a/b/clip_A2.mxf")]
    );
}

/// ffprobe identifies our files (operational pattern, codec, frame count, duration, timecode);
/// ffmpeg decodes them to exactly what it decodes from the same essence in a MOV export, and
/// the PCM is sample-exact against what we wrote.
#[test]
fn ffmpeg_oracle_op1a_and_op_atom() {
    let (Some(ffprobe), Some(ffmpeg)) = (cfa4_testkit::ffprobe_or_skip("MXF export probe"), cfa4_testkit::ffmpeg_or_skip("MXF export decode")) else {
        return;
    };
    let (p, seq, m) = project();
    for codec in MxfVideoCodec::ALL {
        let path = tmp(&format!("oracle_{codec:?}.mxf"));
        export(&p, seq, &settings(Format::MxfOp1a, codec, &path), &m, &Progress::default()).unwrap();
        let j = ffprobe_json(&ffprobe, &path);
        assert_eq!(j["format"]["format_name"], "mxf", "{codec:?}");
        let op = j["format"]["tags"]["operational_pattern_ul"].as_str().unwrap_or_default();
        assert!(op.starts_with("060e2b34.04010101.0d010201.0101"), "{codec:?}: OP1a label {op}");
        let streams = j["streams"].as_array().unwrap();
        let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
        let a = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
        assert_eq!(v["codec_name"], codec_name(codec), "{codec:?}");
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(320), Some(180)));
        assert_eq!(v["nb_read_frames"], "24", "{codec:?}");
        assert_eq!(v["r_frame_rate"], "24/1", "{codec:?}");
        let tc = j["format"]["tags"]["timecode"].as_str().or(v["tags"]["timecode"].as_str()).unwrap_or_default();
        assert_eq!(tc, "01:00:00:00", "{codec:?}");
        let dur: f64 = j["format"]["duration"].as_str().unwrap().parse().unwrap();
        assert!((dur - 1.0).abs() < 0.05, "{codec:?}: {dur}");
        assert_eq!(a["codec_name"], "pcm_s24le", "{codec:?}");
        assert_eq!(a["channels"], 2);
        assert_eq!(a["sample_rate"], "48000");
        // ffmpeg's decode of the MXF equals its decode of the same essence in MOV
        let mov = tmp(&format!("oracle_{codec:?}.mov"));
        let ms = ExportSettings { format: codec.encoder_format(), multiplexer: Multiplexer::Mov, ..settings(Format::MxfOp1a, codec, &mov) };
        export(&p, seq, &ms, &m, &Progress::default()).unwrap();
        let raw = ["-f", "rawvideo", "-pix_fmt", "yuv422p10le"];
        let a_mxf = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
        let a_mov = ffmpeg_raw(&ffmpeg, &mov, "0:v:0", &raw);
        assert_eq!(a_mxf.len(), 24 * 320 * 180 * 4, "{codec:?}: 24 frames decoded");
        assert!(a_mxf == a_mov, "{codec:?}: ffmpeg decodes the MXF and the MOV essence differently");
        // PCM exact: ffmpeg's samples are what our demuxer reads back
        let pcm = ffmpeg_raw(&ffmpeg, &path, "0:a:0", &["-f", "s32le", "-acodec", "pcm_s32le"]);
        let bytes = std::fs::read(&path).unwrap();
        let mx = cfa4_mxf::open(&bytes).unwrap();
        let at = mx.track_of_kind(cfa4_mxf::TrackKind::Sound).unwrap();
        let ours = mx.read_pcm(&bytes, at, 0, 48_000).unwrap();
        assert_eq!(pcm.len(), 48_000 * 2 * 4, "{codec:?}");
        for (i, c) in pcm.as_chunks::<4>().0.iter().enumerate() {
            let s = i32::from_le_bytes([c[0], c[1], c[2], c[3]]) >> 8;
            assert_eq!(s as f32 / 8_388_608.0, ours[i % 2][i / 2], "{codec:?}: sample {i}");
        }
    }
    // OP-Atom: picture file and one mono PCM file per channel
    let path = tmp("oracle_atom.mxf");
    let r = export(&p, seq, &settings(Format::MxfOpAtom, MxfVideoCodec::Dnxhr, &path), &m, &Progress::default()).unwrap();
    let j = ffprobe_json(&ffprobe, &path);
    let op = j["format"]["tags"]["operational_pattern_ul"].as_str().unwrap_or_default();
    assert!(op.starts_with("060e2b34.04010102.0d010201.10"), "OP-Atom label {op}");
    let v = &j["streams"][0];
    assert_eq!(v["codec_name"], "dnxhd");
    assert_eq!(v["nb_read_frames"], "24");
    assert_eq!(j["streams"].as_array().unwrap().len(), 1);
    let mov = tmp("oracle_atom.mov");
    let ms = ExportSettings { format: Format::DnxHr, multiplexer: Multiplexer::Mov, ..settings(Format::MxfOpAtom, MxfVideoCodec::Dnxhr, &mov) };
    export(&p, seq, &ms, &m, &Progress::default()).unwrap();
    let raw = ["-f", "rawvideo", "-pix_fmt", "yuv422p10le"];
    assert!(ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw) == ffmpeg_raw(&ffmpeg, &mov, "0:v:0", &raw), "OP-Atom picture decode");
    let wav = tmp("oracle_atom.wav");
    let ws = ExportSettings { format: Format::Wav, ..settings(Format::Wav, MxfVideoCodec::Dnxhr, &wav) };
    export(&p, seq, &ws, &m, &Progress::default()).unwrap();
    let stereo = ffmpeg_raw(&ffmpeg, &wav, "0:a:0", &["-f", "s32le", "-acodec", "pcm_s32le"]);
    for (k, a) in r.extra_files.iter().enumerate() {
        let j = ffprobe_json(&ffprobe, a);
        let s = &j["streams"][0];
        assert_eq!((s["codec_name"].as_str(), s["channels"].as_u64(), s["sample_rate"].as_str()), (Some("pcm_s24le"), Some(1), Some("48000")), "{a}");
        let mono = ffmpeg_raw(&ffmpeg, a, "0:a:0", &["-f", "s32le", "-acodec", "pcm_s32le"]);
        assert_eq!(mono.len(), 48_000 * 4, "{a}");
        let want: Vec<u8> = stereo.as_chunks::<8>().0.iter().flat_map(|f| f[k * 4..k * 4 + 4].to_vec()).collect();
        assert!(mono == want, "{a}: channel {k} differs from the WAV export");
    }
}

/// MPEG-2 long GOP (ST 381-1) at 1280×720, 4:2:2 at 50 Mbit/s with IBBP: our demuxer finds the
/// MPEG video descriptor, the presentation order and the GOP structure; ffmpeg decodes the MXF
/// exactly as the same settings' elementary stream export, and our decode of the MXF matches
/// ffmpeg's within the IDCT tolerance.
#[test]
fn mpeg2_long_gop_op1a_and_op_atom() {
    let (p, seq, m) = project();
    let mpeg2 = |format: Format, path: &str| ExportSettings {
        frame_size: Some((1280, 720)),
        mpeg2_422: true,
        bitrate_kbps: 50_000,
        max_bitrate_kbps: Some(50_000),
        keyframe_distance: Some(12),
        mpeg2_b_frames: Some(2),
        ..settings(format, MxfVideoCodec::Mpeg2, path)
    };
    // standard definition is D-10, not this mapping
    let sd = ExportSettings { frame_size: Some((720, 576)), ..mpeg2(Format::MxfOp1a, &tmp("sd.mxf")) };
    assert!(matches!(export(&p, seq, &sd, &m, &Progress::default()), Err(ExportError::Unsupported(_))));
    for format in [Format::MxfOp1a, Format::MxfOpAtom] {
        let path = tmp(&format!("mpeg2_{format:?}.mxf"));
        let r = export(&p, seq, &mpeg2(format, &path), &m, &Progress::default()).unwrap();
        assert_eq!(r.frames, 24);
        let bytes = std::fs::read(&path).unwrap();
        let mx = cfa4_mxf::open(&bytes).unwrap();
        assert!(mx.warnings.is_empty(), "{format:?}: {:?}", mx.warnings);
        let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
        assert_eq!(v.codec, cfa4_mxf::Codec::Mpeg2);
        assert_eq!(v.samples.len(), 24);
        assert!(v.temporal_offsets, "{format:?}: B pictures, the index gives the presentation order");
        assert_eq!(v.samples.iter().filter(|s| s.key).count(), 2, "{format:?}: an I picture every 12");
        let pi = v.picture.as_ref().unwrap();
        assert_eq!((pi.stored_width, pi.stored_height, pi.horizontal_subsampling, pi.vertical_subsampling), (1280, 720, 2, 1));
        let coding = pi.picture_coding.unwrap().0;
        assert_eq!(coding[8..16], [0x04, 0x01, 0x02, 0x02, 0x01, 0x04, 0x03, 0x00], "422P@HL long GOP (RDD 9 Table 6)");
        // our decode, in presentation order
        let src = cfa4_codecs::open_bytes("x.mxf", bytes.clone().into()).unwrap();
        assert_eq!(src.info().video.as_ref().unwrap().width, 1280);
        let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("MXF MPEG-2 decode") else { continue };
        let raw = ["-f", "rawvideo", "-pix_fmt", "yuv422p"];
        let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
        let fsize = 1280 * 720 * 2;
        assert_eq!(theirs.len(), 24 * fsize, "{format:?}: 24 frames decoded");
        let es = tmp(&format!("mpeg2_{format:?}.m2v"));
        let ess = ExportSettings { format: Format::Mpeg2, mpeg2_container: "es".into(), ..mpeg2(Format::Mpeg2, &es) };
        export(&p, seq, &ess, &m, &Progress::default()).unwrap();
        assert!(theirs == ffmpeg_raw(&ffmpeg, &es, "0:v:0", &raw), "{format:?}: the MXF essence is the elementary stream's");
        for f in [0usize, 1, 2, 3, 11, 12, 13, 23] {
            let t = FrameRate::FPS_24.tick_of(f as i64) + Tick(1);
            let ours = src.video_frame(FrameRequest::full(t)).unwrap();
            let cfa4_frame::PixelData::Yuv8 { planes, .. } = &ours.data else { panic!("8-bit Y'CbCr") };
            let want = &theirs[f * fsize..f * fsize + 1280 * 720];
            let worst = planes[0].iter().zip(want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            assert!(worst <= 4, "{format:?}: frame {f} luma differs by {worst}");
        }
        if let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("MXF MPEG-2 probe") {
            let j = ffprobe_json(&ffprobe, &path);
            let v = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
            assert_eq!((v["codec_name"].as_str(), v["profile"].as_str(), v["pix_fmt"].as_str()), (Some("mpeg2video"), Some("4:2:2"), Some("yuv422p")));
            assert_eq!(v["nb_read_frames"], "24");
        }
    }
}

/// Type D-10 (IMX): constant SDTI-CP content packages (system item, picture padded to the
/// SMPTE 356 frame size, 8-channel AES3 element) that ffmpeg reads: the picture as 720x608
/// 4:2:2P@ML MPEG-2 equal to our decode, the vertical interval black, the PCM sample-exact.
#[test]
fn d10_imx_op1a() {
    let (p, seq, m) = project();
    let d10 = |path: &str| ExportSettings {
        frame_size: Some((720, 576)),
        frame_rate: Some(FrameRate::new(25, 1)),
        pixel_aspect: Some((16, 15)),
        bitrate_kbps: 50_000,
        ..settings(Format::MxfOp1a, MxfVideoCodec::D10, path)
    };
    let refused = |s: ExportSettings| matches!(export(&p, seq, &s, &m, &Progress::default()), Err(ExportError::Unsupported(_)));
    assert!(refused(ExportSettings { format: Format::MxfOpAtom, ..d10(&tmp("d10_atom.mxf")) }));
    assert!(refused(ExportSettings { bitrate_kbps: 45_000, ..d10(&tmp("d10_45.mxf")) }));
    assert!(refused(ExportSettings { frame_size: Some((1280, 720)), ..d10(&tmp("d10_hd.mxf")) }));
    assert!(refused(ExportSettings { frame_rate: Some(FrameRate::FPS_24), ..d10(&tmp("d10_24.mxf")) }));
    let path = tmp("d10_625.mxf");
    let r = export(&p, seq, &d10(&path), &m, &Progress::default()).unwrap();
    let n = r.frames as usize;
    assert!(n >= 24, "{n} frames");
    let bytes = std::fs::read(&path).unwrap();
    let mx = cfa4_mxf::open(&bytes).unwrap();
    assert!(mx.warnings.is_empty(), "{:?}", mx.warnings);
    assert_eq!(mx.timecode.unwrap().format(), "01:00:00:00");
    assert!(mx.partitions.iter().all(|p| p.kag_size == 512));
    let ec = mx.partitions[0].essence_containers[0].0;
    assert_eq!(ec[8..16], [0x0D, 0x01, 0x03, 0x01, 0x02, 0x01, 0x01, 0x01], "D-10 50 Mbit/s 625/50 defined template");
    let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
    assert_eq!(v.codec, cfa4_mxf::Codec::Mpeg2);
    assert_eq!(v.samples.len(), n);
    assert!(v.samples.iter().all(|s| s.key && s.size == 250_000));
    let a = mx.track_of_kind(cfa4_mxf::TrackKind::Sound).unwrap();
    assert_eq!(mx.tracks[a].codec, cfa4_mxf::Codec::Aes3Element);
    assert_eq!(mx.tracks[a].stored_sample_frames(), n as u64 * 1920);
    let pcm = mx.read_pcm(&bytes, a, 0, n * 1920).unwrap();
    assert!((pcm[0].iter().fold(0f32, |a, s| a.max(s.abs())) - 0.5).abs() < 0.05, "the -6 dB tone");
    // our decode of the MXF
    let src = cfa4_codecs::open_bytes("x.mxf", bytes.clone().into()).unwrap();
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("MXF D-10 decode") else { return };
    let raw = ["-f", "rawvideo", "-pix_fmt", "yuv422p"];
    let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
    let fsize = 720 * 608 * 2;
    assert_eq!(theirs.len(), n * fsize, "{n} frames of 720x608 decoded");
    for f in [0usize, 1, 12, n - 1] {
        let frame = &theirs[f * fsize..f * fsize + 720 * 608];
        // the 32 vertical interval lines are black, the picture below them is not
        assert!(frame[..720 * 32].iter().all(|&y| y.abs_diff(16) <= 2), "frame {f}: vertical interval");
        assert!(frame[720 * 32..].iter().any(|&y| y > 100), "frame {f}: picture");
        let t = FrameRate::new(25, 1).tick_of(f as i64) + Tick(1);
        let ours = src.video_frame(FrameRequest::full(t)).unwrap();
        let cfa4_frame::PixelData::Yuv8 { planes, .. } = &ours.data else { panic!("8-bit Y'CbCr") };
        let (h, want) = if planes[0].len() == 720 * 608 { (608, frame) } else { (576, &frame[720 * 32..]) };
        let worst = planes[0][..720 * h].iter().zip(want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 4, "frame {f}: luma differs by {worst}");
    }
    // the AES3 PCM: ffmpeg's decode is what we read back
    let theirs = ffmpeg_raw(&ffmpeg, &path, "0:a:0", &["-f", "s32le", "-c:a", "pcm_s32le"]);
    let ch = theirs.len() / 4 / (n * 1920);
    assert!(ch == 4 || ch == 8, "{ch} channels");
    for (k, w) in theirs.chunks_exact(4).enumerate().step_by(997) {
        let (s, c) = (k / ch, k % ch);
        let want = if c < 2 { (pcm[c][s] * 8_388_608.0).round() as i64 } else { 0 };
        assert_eq!(i64::from(i32::from_le_bytes(w.try_into().unwrap()) >> 8), want, "sample {s} channel {c}");
    }
    if let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("MXF D-10 probe") {
        let j = ffprobe_json(&ffprobe, &path);
        let v = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
        assert_eq!((v["codec_name"].as_str(), v["profile"].as_str(), v["pix_fmt"].as_str()), (Some("mpeg2video"), Some("4:2:2"), Some("yuv422p")));
        assert_eq!((v["width"].as_i64(), v["height"].as_i64()), (Some(720), Some(608)));
        assert_eq!(v["nb_read_frames"], n.to_string());
    }
    // 525 lines at 29.97, 30 Mbit/s, 720x480 under the 26 vertical interval lines
    let path = tmp("d10_525.mxf");
    let s = ExportSettings { frame_size: Some((720, 480)), frame_rate: Some(FrameRate::new(30000, 1001)), bitrate_kbps: 30_000, ..d10(&path) };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mx = cfa4_mxf::open(&bytes).unwrap();
    assert!(mx.warnings.is_empty(), "{:?}", mx.warnings);
    assert_eq!(mx.partitions[0].essence_containers[0].0[14], 0x06, "D-10 30 Mbit/s 525/60");
    let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
    assert_eq!(v.samples.len() as u64, r.frames);
    assert!(v.samples.iter().all(|s| s.size == 125_125));
    let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
    assert_eq!(theirs.len(), r.frames as usize * 720 * 512 * 2);
}

/// XDCAM HD422 (RDD 9): 4:2:2 MPEG-2 in system-item content packages with one mono AES3 element
/// per channel. ffmpeg decodes the picture exactly as the elementary stream export of the same
/// settings, and each audio stream sample-exact as the channel we read back.
#[test]
fn xdcam_hd422_rdd9_op1a() {
    let (p, seq, m) = project();
    let xd = |path: &str| ExportSettings {
        frame_size: Some((1280, 720)),
        bitrate_kbps: 50_000,
        max_bitrate_kbps: Some(50_000),
        keyframe_distance: Some(12),
        mpeg2_b_frames: Some(2),
        ..settings(Format::MxfOp1a, MxfVideoCodec::XdcamHd422, path)
    };
    let refused = |s: ExportSettings| matches!(export(&p, seq, &s, &m, &Progress::default()), Err(ExportError::Unsupported(_)));
    assert!(refused(ExportSettings { format: Format::MxfOpAtom, ..xd(&tmp("xd_atom.mxf")) }));
    assert!(refused(ExportSettings { frame_size: Some((1440, 1080)), ..xd(&tmp("xd_1440.mxf")) }));
    let mut a44 = xd(&tmp("xd_44k.mxf"));
    a44.audio.sample_rate = Some(44_100);
    assert!(refused(a44));
    let path = tmp("xd_720.mxf");
    let r = export(&p, seq, &xd(&path), &m, &Progress::default()).unwrap();
    let n = r.frames as usize;
    let bytes = std::fs::read(&path).unwrap();
    let mx = cfa4_mxf::open(&bytes).unwrap();
    assert!(mx.warnings.is_empty(), "{:?}", mx.warnings);
    assert!(mx.partitions.iter().all(|p| p.kag_size == 512));
    let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
    assert_eq!((v.codec, v.samples.len()), (cfa4_mxf::Codec::Mpeg2, n));
    assert!(v.temporal_offsets);
    let pi = v.picture.as_ref().unwrap();
    assert_eq!((pi.stored_width, pi.stored_height, pi.horizontal_subsampling), (1280, 720, 2));
    let sound: Vec<usize> = (0..mx.tracks.len()).filter(|&k| mx.tracks[k].sound.is_some()).collect();
    assert_eq!(sound.len(), 4, "one track per channel, four at least (the last two silent)");
    let samples = mx.tracks[sound[0]].stored_sample_frames() as usize;
    assert_eq!(samples, n * 2000, "48 kHz at 24 fps");
    let ours: Vec<Vec<f32>> = sound.iter().map(|&k| mx.read_pcm(&bytes, k, 0, samples).unwrap().remove(0)).collect();
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("MXF RDD 9 decode") else { return };
    let raw = ["-f", "rawvideo", "-pix_fmt", "yuv422p"];
    let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
    assert_eq!(theirs.len(), n * 1280 * 720 * 2);
    let es = tmp("xd_720.m2v");
    let ess = ExportSettings { format: Format::Mpeg2, mpeg2_container: "es".into(), mpeg2_422: true, ..xd(&es) };
    export(&p, seq, &ess, &m, &Progress::default()).unwrap();
    assert!(theirs == ffmpeg_raw(&ffmpeg, &es, "0:v:0", &raw), "the MXF essence is the elementary stream's");
    assert!(ours[2..].iter().all(|c| c.iter().all(|&s| s == 0.0)));
    for (c, want) in ours.iter().enumerate() {
        let pcm = ffmpeg_raw(&ffmpeg, &path, &format!("0:a:{c}"), &["-f", "s32le", "-c:a", "pcm_s32le"]);
        assert_eq!(pcm.len(), samples * 4, "channel {c}: mono");
        for (s, w) in pcm.chunks_exact(4).enumerate().step_by(331) {
            assert_eq!(i64::from(i32::from_le_bytes(w.try_into().unwrap()) >> 8), (want[s] * 8_388_608.0).round() as i64, "channel {c} sample {s}");
        }
    }
    assert!(ours[0].iter().any(|s| s.abs() > 0.1), "the tone is there");
    if let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("MXF RDD 9 probe") {
        let j = ffprobe_json(&ffprobe, &path);
        let v = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
        assert_eq!((v["codec_name"].as_str(), v["profile"].as_str(), v["pix_fmt"].as_str()), (Some("mpeg2video"), Some("4:2:2"), Some("yuv422p")));
        assert_eq!(v["nb_read_frames"], n.to_string());
    }
}

/// Interlaced MPEG-2 in MXF (#949): XDCAM HD422 1080i at 25 fps as RDD 9's separate fields (544
/// stored, 540 sampled lines per field, line map 21 / 584, field 1 first, coded content
/// interlaced) holds the interlaced elementary stream, which ffprobe (oracle) reads as top field
/// first; D-10 codes interlaced pictures upper field first. Rates, sizes and a lower-first D-10
/// that RDD 9 / ST 356 do not have are refused.
#[test]
fn interlaced_mpeg2_mxf() {
    let (p, seq, m) = project();
    let xd = |path: &str| ExportSettings {
        frame_size: Some((1920, 1080)),
        frame_rate: Some(FrameRate::new(25, 1)),
        bitrate_kbps: 50_000,
        max_bitrate_kbps: Some(50_000),
        keyframe_distance: Some(12),
        mpeg2_b_frames: Some(2),
        field_order: FieldOrder::UpperFirst,
        ..settings(Format::MxfOp1a, MxfVideoCodec::XdcamHd422, path)
    };
    let refused = |s: ExportSettings| matches!(export(&p, seq, &s, &m, &Progress::default()), Err(ExportError::Unsupported(_)));
    assert!(refused(ExportSettings { frame_rate: Some(FrameRate::FPS_24), ..xd(&tmp("xd_1080i24.mxf")) }), "RDD 9 has no 1080i at 24 fps");
    assert!(refused(ExportSettings { frame_size: Some((1280, 720)), ..xd(&tmp("xd_720i.mxf")) }), "720 lines are progressive");
    assert!(refused(ExportSettings { format: Format::ProRes, path: tmp("i.mov"), ..xd(&tmp("unused")) }), "ProRes codes progressive frames");
    let path = tmp("xd_1080i.mxf");
    let r = export(&p, seq, &xd(&path), &m, &Progress::default()).unwrap();
    let n = r.frames as usize;
    let bytes = std::fs::read(&path).unwrap();
    let mx = cfa4_mxf::open(&bytes).unwrap();
    assert!(mx.warnings.is_empty(), "{:?}", mx.warnings);
    let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
    assert_eq!((v.codec, v.samples.len()), (cfa4_mxf::Codec::Mpeg2, n));
    let pi = v.picture.as_ref().unwrap();
    assert_eq!((pi.frame_layout, pi.stored_width, pi.stored_height, pi.display_height), (1, 1920, 544, 540));
    assert_eq!((pi.frame_width(), pi.frame_height()), (1920, 1080));
    // the descriptor's field properties, as RDD 9's annex table gives them
    let find = |key: &[u8]| bytes.windows(key.len()).position(|w| w == key).map(|i| &bytes[i + key.len()..]);
    assert_eq!(find(&[0x32, 0x0D, 0x00, 0x10]).map(|b| &b[..16]), Some(&[0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0, 21, 0, 0, 2, 0x48][..]), "video line map 21, 584");
    assert_eq!(find(&[0x32, 0x12, 0x00, 0x01]).map(|b| b[0]), Some(1), "field dominance");
    assert_eq!(find(&[0x32, 0x15, 0x00, 0x01]).map(|b| b[0]), Some(4), "signal standard ST 274");
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("MXF 1080i decode") else { return };
    let raw = ["-f", "rawvideo", "-pix_fmt", "yuv422p"];
    let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
    assert_eq!(theirs.len(), n * 1920 * 1080 * 2);
    let es = tmp("xd_1080i.m2v");
    let ess = ExportSettings { format: Format::Mpeg2, mpeg2_container: "es".into(), mpeg2_422: true, ..xd(&es) };
    export(&p, seq, &ess, &m, &Progress::default()).unwrap();
    assert!(theirs == ffmpeg_raw(&ffmpeg, &es, "0:v:0", &raw), "the MXF essence is the interlaced elementary stream");
    if let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("MXF 1080i probe") {
        let j = ffprobe_json(&ffprobe, &path);
        let v = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
        assert_eq!((v["width"].as_u64(), v["height"].as_u64(), v["field_order"].as_str()), (Some(1920), Some(1080), Some("tt")), "{v}");
        assert_eq!(v["nb_read_frames"], n.to_string());
    }

    // D-10: interlaced pictures in the separate-field layout it always has
    let d10 = |path: &str| ExportSettings {
        frame_size: Some((720, 576)),
        frame_rate: Some(FrameRate::new(25, 1)),
        pixel_aspect: Some((16, 15)),
        bitrate_kbps: 50_000,
        field_order: FieldOrder::UpperFirst,
        ..settings(Format::MxfOp1a, MxfVideoCodec::D10, path)
    };
    assert!(refused(ExportSettings { field_order: FieldOrder::LowerFirst, ..d10(&tmp("d10_bff.mxf")) }), "D-10 is upper field first");
    let path = tmp("d10_i.mxf");
    let r = export(&p, seq, &d10(&path), &m, &Progress::default()).unwrap();
    let n = r.frames as usize;
    let bytes = std::fs::read(&path).unwrap();
    let mx = cfa4_mxf::open(&bytes).unwrap();
    let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
    assert!(v.samples.iter().all(|s| s.key && s.size == 250_000));
    let src = cfa4_codecs::open_bytes("x.mxf", bytes.clone().into()).unwrap();
    let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v:0", &raw);
    let fsize = 720 * 608 * 2;
    assert_eq!(theirs.len(), n * fsize);
    for f in [0usize, n / 2, n - 1] {
        let frame = &theirs[f * fsize..f * fsize + 720 * 608];
        assert!(frame[..720 * 32].iter().all(|&y| y.abs_diff(16) <= 2), "frame {f}: vertical interval");
        let t = FrameRate::new(25, 1).tick_of(f as i64) + Tick(1);
        let ours = src.video_frame(FrameRequest::full(t)).unwrap();
        let cfa4_frame::PixelData::Yuv8 { planes, .. } = &ours.data else { panic!("8-bit Y'CbCr") };
        let (h, want) = if planes[0].len() == 720 * 608 { (608, frame) } else { (576, &frame[720 * 32..]) };
        let worst = planes[0][..720 * h].iter().zip(want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 4, "frame {f}: luma differs by {worst}");
    }
    if let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("MXF D-10 interlaced probe") {
        let j = ffprobe_json(&ffprobe, &path);
        let v = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
        assert_eq!(v["field_order"].as_str(), Some("tt"), "{v}");
    }
}

/// Uncompressed MXF (#946): v210 and UYVY pictures in OP1a and OP-Atom read back with our
/// demuxer (layout, picture size, one picture per edit unit) and decode to the rendered frames
/// within the 4:2:2 Rec. 709 round trip; ffprobe / ffmpeg (oracle) name the codec and decode
/// the same pictures.
#[test]
fn uncompressed_v210_and_uyvy() {
    let (p, seq, m) = project();
    let leader = m.0.values().find(|s| s.info().video.is_some()).unwrap().clone();
    let oracle = cfa4_testkit::ffprobe_or_skip("uncompressed MXF probe").zip(cfa4_testkit::ffmpeg_or_skip("uncompressed MXF decode"));
    for (codec, layout, tol) in [(MxfVideoCodec::V210, cfa4_mxf::UncompressedLayout::V210, 6), (MxfVideoCodec::Uyvy, cfa4_mxf::UncompressedLayout::Uyvy8, 8)] {
        for format in [Format::MxfOp1a, Format::MxfOpAtom] {
            let path = tmp(&format!("unc_{codec:?}_{format:?}.mxf"));
            let s = settings(format, codec, &path);
            assert_eq!(s.video_format(), Format::MxfOp1a);
            let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
            assert_eq!(r.frames, 24);
            let bytes = std::fs::read(&path).unwrap();
            let est = s.estimate_bytes(320, 180, FrameRate::FPS_24, 48_000, Tick(TICKS_PER_SECOND));
            assert!(est >= 24 * layout.picture_bytes(320, 180), "{codec:?}: estimate {est}");
            let mx = cfa4_mxf::open(&bytes).unwrap();
            assert!(mx.warnings.is_empty(), "{codec:?} {format:?}: {:?}", mx.warnings);
            let v = &mx.tracks[mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap()];
            assert_eq!(v.samples.len(), 24, "{codec:?} {format:?}");
            assert_eq!(v.codec, cfa4_mxf::Codec::Uncompressed, "{codec:?}");
            let want = layout.picture_bytes(320, 180);
            assert!(v.samples.iter().all(|x| u64::from(x.size) == want), "{codec:?}: {want} bytes per picture");
            let src = cfa4_codecs::open_bytes("x.mxf", bytes.into()).unwrap();
            for f in [0i64, 7, 23] {
                let t = FrameRate::FPS_24.tick_of(f) + Tick(1);
                let a = src.video_frame(FrameRequest::full(t)).unwrap().to_rgba8();
                let b = leader.video_frame(FrameRequest::full(t)).unwrap().to_rgba8();
                assert_eq!(a.len(), b.len());
                // chroma is halved horizontally: compare where the picture is flat
                let worst =
                    a.chunks(4).zip(b.chunks(4)).map(|(x, y)| (0..3).map(|c| x[c].abs_diff(y[c])).max().unwrap_or(0)).filter(|d| *d <= 64).max().unwrap_or(0);
                let close = a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| (0..3).all(|c| x[c].abs_diff(y[c]) <= tol)).count();
                assert!(close * 100 >= a.len() / 4 * 95, "{codec:?} frame {f}: {close} of {} pixels within {tol} (worst {worst})", a.len() / 4);
            }
            let Some((ffprobe, ffmpeg)) = &oracle else { continue };
            let j = ffprobe_json(ffprobe, &path);
            let v = j["streams"].as_array().unwrap().iter().find(|s| s["codec_type"] == "video").unwrap().clone();
            assert_eq!(v["codec_name"], codec_name(codec), "{codec:?} {format:?}");
            assert_eq!((v["width"].as_u64(), v["height"].as_u64(), v["nb_read_frames"].as_str()), (Some(320), Some(180), Some("24")), "{codec:?}");
            let fmt = if codec == MxfVideoCodec::V210 { "yuv422p10le" } else { "uyvy422" };
            let raw = ffmpeg_raw(ffmpeg, &path, "0:v", &["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", fmt]);
            let first = &std::fs::read(&path).unwrap();
            let mx = cfa4_mxf::open(first).unwrap();
            let pic = mx.read_sample(first, mx.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap(), 0).unwrap();
            if codec == MxfVideoCodec::Uyvy {
                assert!(raw == pic, "ffmpeg reads our UYVY bytes as they are");
            } else {
                assert_eq!(raw.len(), 320 * 180 * 2 * 2, "4:2:2 10-bit planes");
            }
        }
    }
}
