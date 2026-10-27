//! DCP export (#115): the package read back with our MXF reader and JPEG 2000 decoder, the
//! hashes in the packing list checked, and ffprobe / ffmpeg as external oracles.

use std::path::Path;
use std::process::Command;

use super::*;
use crate::dcp_out::{Sha1, sha1};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::TICKS_PER_SECOND;

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

#[test]
fn sha1_test_vectors() {
    // FIPS 180-4 examples
    assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    // a million `a`, fed in uneven pieces
    let mut s = Sha1::new();
    let mut left = 1_000_000usize;
    let mut k = 1;
    while left > 0 {
        let n = (k * 37 % 1000 + 1).min(left);
        s.update(&vec![b'a'; n]);
        left -= n;
        k += 1;
    }
    assert_eq!(hex(&s.finish()), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
}

#[test]
fn dcdm_encoding_matches_st428_1() {
    // ST 428-1 annex B: the RP 431-2 calibration white (X 42.94, Y 48, Z 45.82 cd/m²)
    assert_eq!([dcdm_code(42.94 / 48.0), dcdm_code(1.0), dcdm_code(45.82 / 48.0)], [3794, 3960, 3890]);
    // and one JND above black of a 2000:1 projector (X 0.00043, Y 0.00048, Z 0.00046 cd/m²)
    // (the annex rounds: Z 0.00046 is code 46.53)
    assert_eq!([dcdm_code(0.00043 / 48.0), dcdm_code(0.00048 / 48.0), dcdm_code(0.00046 / 48.0)], [45, 47, 47]);
    assert_eq!(dcdm_code(0.0), 0);
    assert_eq!(dcdm_code(-1.0), 0);
    assert_eq!(dcdm_code(10.0), 4095);
    assert_eq!(dcdm_code(f32::NAN), 0);
    // sRGB white is D65: Y' 3960, X' and Z' from x 0.3127 y 0.3290
    let [x, y, z] = rgba_to_xyz12(&[255, 255, 255, 255, 0, 0, 0, 255], 2, 1);
    assert_eq!((x[0], y[0], z[0]), (dcdm_code(0.950_47), 3960, dcdm_code(1.088_83)));
    assert_eq!((x[1], y[1], z[1]), (0, 0, 0));
    // mid grey: sRGB 128 is 21.6 % linear
    let [_, y, _] = rgba_to_xyz12(&[128, 128, 128, 255], 1, 1);
    assert_eq!(y[0], dcdm_code(0.215_861));
    // short input never panics
    let [x, _, _] = rgba_to_xyz12(&[255, 255], 2, 2);
    assert_eq!(x.len(), 4);
}

#[test]
fn rates_containers_and_names() {
    let r = |n, d| FrameRate::new(n, d);
    assert_eq!(dcp_rate(DcpResolution::TwoK, r(24000, 1001)), r(24, 1));
    assert_eq!(dcp_rate(DcpResolution::TwoK, r(30000, 1001)), r(30, 1));
    assert_eq!(dcp_rate(DcpResolution::TwoK, r(60000, 1001)), r(60, 1));
    assert_eq!(dcp_rate(DcpResolution::TwoK, r(50, 1)), r(50, 1));
    assert_eq!(dcp_rate(DcpResolution::FourK, r(60, 1)), r(30, 1));
    assert_eq!(dcp_rate(DcpResolution::FourK, r(0, 0)), r(24, 1), "a broken rate falls back to the default (24)");
    for res in DcpResolution::ALL {
        assert_eq!(DcpResolution::from_name(res.id()), Some(res));
    }
    for a in DcpAspect::ALL {
        assert_eq!(DcpAspect::from_name(a.id()), Some(a));
    }
    assert_eq!(DcpAspect::from_name("2.39:1"), Some(DcpAspect::Scope));
    assert_eq!(DcpAspect::Scope.size(DcpResolution::FourK), (4096, 1716));
    assert_eq!(DcpAspect::Flat.size(DcpResolution::TwoK), (1998, 1080));
    assert_eq!(Format::from_name("DCP"), Some(Format::Dcp));
    assert_eq!(Format::Dcp.extension(), "");
    let mut s = ExportSettings { format: Format::Dcp, ..Default::default() };
    assert!(s.validate().is_ok());
    s.dcp_resolution = "8k".into();
    assert!(s.validate().is_err());
    s.dcp_resolution = "4K".into();
    s.dcp_aspect = "imax".into();
    assert!(s.validate().is_err());
    s.dcp_aspect = "scope".into();
    s.dcp_content_kind = "Trailer".into();
    assert!(s.validate().is_ok());
    assert_eq!(dcp_content_kind(&s), Some("trailer"));
    s.dcp_content_kind = "movie".into();
    assert!(s.validate().is_err());
    // the summary and estimate use what the export writes, whatever the settings ask
    s.dcp_content_kind.clear();
    s.frame_rate = Some(r(24000, 1001));
    s.audio.channels = 2;
    let sum = s.summary(1920, 1080, r(25, 1), 48_000, Tick(10 * TICKS_PER_SECOND));
    assert!(sum.video.starts_with("4096x1716 (1.0), 24 fps"), "{}", sum.video);
    assert!(sum.video.contains("JPEG 2000 DCI 4K"), "{}", sum.video);
    assert_eq!(sum.audio, "Uncompressed 24-bit PCM, 48000 Hz, 5.1");
    assert!(sum.format.contains("Scope"), "{}", sum.format);
    assert!(sum.estimated_bytes > 200_000_000, "{}", sum.estimated_bytes);
}

/// 320×180 at 25 fps: a white matte over V1, a 440 Hz tone on A1.
fn project() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("dcp");
    let r = FrameRate::new(25, 1);
    let white = GeneratorSource::new(Generator::ColorMatte { color: [1.0, 1.0, 1.0, 1.0] }, 320, 180, r, Tick(2 * TICKS_PER_SECOND));
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
    let v = add(&mut p, &white);
    let a = add(&mut p, &tone);
    let seq = p.new_sequence("DCP <Test> & Co", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
    let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(50)), r).unwrap();
    let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(50)), r).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(vi);
    q.audio_tracks[0].items.push(ai);
    let mut m = SourceMap::default();
    m.0.insert(v, Arc::new(white));
    m.0.insert(a, Arc::new(tone));
    (Arc::new(p), seq, m)
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("fc-export-dcp-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&d);
    d.to_string_lossy().to_string()
}

/// The text of the first `<tag>` element in `xml`.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

/// The files of the package by prefix.
fn file(dir: &str, prefix: &str) -> String {
    let mut found: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(prefix)).collect();
    assert_eq!(found.len(), 1, "{prefix} in {dir}");
    Path::new(dir).join(found.remove(0)).to_string_lossy().to_string()
}

/// A 2K Flat DCP of ten frames: the track files, the playlist, packing list and asset map agree,
/// the picture is the pillarboxed white in X'Y'Z', the sound 5.1 24-bit.
#[test]
fn two_k_flat_package() {
    let (p, seq, m) = project();
    let dir = tmp("flat");
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.clone(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(10))),
        dcp_content_kind: "test".into(),
        ..Default::default()
    };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    assert_eq!((r.path.as_str(), r.frames), (dir.as_str(), 10));
    assert_eq!(r.extra_files.len(), 6, "{:?}", r.extra_files);
    let total: u64 = r.extra_files.iter().map(|f| std::fs::metadata(f).unwrap().len()).sum();
    assert_eq!(r.bytes, total);

    // picture track file
    let pic_path = file(&dir, "j2c_");
    let pic = std::fs::read(&pic_path).unwrap();
    let mxf = cfa4_mxf::open(&pic).unwrap();
    assert!(mxf.warnings.is_empty(), "{:?}", mxf.warnings);
    assert_eq!(mxf.operational_pattern, cfa4_mxf::OperationalPattern::Atom);
    let t = &mxf.tracks[0];
    assert_eq!(t.codec, cfa4_mxf::Codec::Jpeg2000);
    assert_eq!(t.edit_rate, cfa4_mxf::Rational::new(25, 1));
    assert_eq!(t.samples.len(), 10);
    let frame = mxf.read_sample(&pic, 0, 4).unwrap();
    assert!(frame.len() <= 250_000_000 / 8 / 25);
    let img = cfa4_j2k::decode(&frame).unwrap();
    assert_eq!((img.width, img.height, img.components.len()), (1998, 1080, 3));
    assert!(img.components.iter().all(|c| c.precision == 12));
    // 320×180 fitted into 1998×1080: 1920×1080 with 39-pixel bars left and right
    let at = |c: usize, x: usize, y: usize| img.components[c].data[y * 1998 + x];
    let white = [dcdm_code(0.950_47), 3960, dcdm_code(1.088_83)];
    for c in 0..3 {
        assert!(at(c, 10, 540) <= 8, "bar {c}: {}", at(c, 10, 540));
        assert!(at(c, 1990, 100) <= 8);
        assert!((at(c, 999, 540) - white[c]).abs() <= 4, "white {c}: {} vs {}", at(c, 999, 540), white[c]);
        assert!((at(c, 60, 20) - white[c]).abs() <= 4);
    }

    // sound track file
    let snd_path = file(&dir, "pcm_");
    let snd = std::fs::read(&snd_path).unwrap();
    let mxf_s = cfa4_mxf::open(&snd).unwrap();
    assert!(mxf_s.warnings.is_empty(), "{:?}", mxf_s.warnings);
    let a = &mxf_s.tracks[0];
    assert_eq!(a.kind, cfa4_mxf::TrackKind::Sound);
    assert_eq!(a.duration, Some(10));
    assert_eq!(a.stored_sample_frames(), 10 * 1920);
    let info = a.sound.as_ref().unwrap();
    assert_eq!((info.bits, info.channels, info.sample_rate), (24, 6, cfa4_mxf::Rational::new(48_000, 1)));
    let pcm = mxf_s.read_pcm(&snd, 0, 0, 10 * 1920).unwrap();
    let rms = |c: &[f32]| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
    assert!(rms(&pcm[0]) > 0.05 && rms(&pcm[1]) > 0.05, "the tone is on L and R");

    // playlist, packing list, asset map
    let cpl_path = file(&dir, "cpl_");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    assert!(cpl.contains("xmlns=\"http://www.smpte-ra.org/schemas/429-7/2006/CPL\""));
    assert_eq!(element(&cpl, "ContentTitleText"), Some("DCP &lt;Test&gt; &amp; Co"));
    assert_eq!(element(&cpl, "ContentKind"), Some("test"));
    assert_eq!(element(&cpl, "EditRate"), Some("25 1"));
    assert_eq!(element(&cpl, "Duration"), Some("10"));
    assert_eq!(element(&cpl, "ScreenAspectRatio"), Some("1998 1080"));
    assert!(cpl.contains("<MainSound>"));
    // the asset ids are the UUIDs of the file packages
    let pic_id = element(element(&cpl, "MainPicture").unwrap(), "Id").unwrap().trim_start_matches("urn:uuid:").replace('-', "");
    let pic_uuid: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&pic_id[2 * i..2 * i + 2], 16).unwrap()).collect();
    assert_eq!(pic_uuid[6] >> 4, 4, "version 4");
    assert!(
        pic.windows(32).any(|w| w[..16] == [0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0F, 0x20, 0x13, 0, 0, 0] && w[16..] == pic_uuid[..])
    );
    assert!(pic_path.ends_with(&format!("j2c_{}.mxf", element(element(&cpl, "MainPicture").unwrap(), "Id").unwrap().trim_start_matches("urn:uuid:"))));

    let pkl_path = file(&dir, "pkl_");
    let pkl = std::fs::read_to_string(&pkl_path).unwrap();
    assert!(pkl.contains("xmlns=\"http://www.smpte-ra.org/schemas/429-8/2007/PKL\""));
    let assets: Vec<&str> = pkl.split("<Asset>").skip(1).collect();
    assert_eq!(assets.len(), 3, "CPL and both track files");
    for a in assets {
        let name = element(a, "OriginalFileName").unwrap();
        let data = std::fs::read(Path::new(&dir).join(name)).unwrap();
        assert_eq!(element(a, "Size").unwrap(), data.len().to_string(), "{name}");
        assert_eq!(element(a, "Hash").unwrap(), cfa4_project::gtemplate::base64_encode(&sha1(&data)), "{name}");
        assert_eq!(element(a, "Type").unwrap(), if name.ends_with(".xml") { "text/xml" } else { "application/mxf" });
    }
    // the playlist's own hashes of the track files are the packing list's
    let pic_text_id = element(element(&cpl, "MainPicture").unwrap(), "Id").unwrap();
    let pic_asset = pkl.split("<Asset>").find(|a| a.contains(pic_text_id)).unwrap();
    assert_eq!(element(element(&cpl, "MainPicture").unwrap(), "Hash"), element(pic_asset, "Hash"));

    let am = std::fs::read_to_string(Path::new(&dir).join("ASSETMAP.xml")).unwrap();
    assert!(am.contains("xmlns=\"http://www.smpte-ra.org/schemas/429-9/2007/AM\""));
    assert_eq!(am.matches("<Asset>").count(), 4);
    assert_eq!(am.matches("<PackingList>true</PackingList>").count(), 1);
    for f in [&pic_path, &snd_path, &cpl_path, &pkl_path] {
        let name = Path::new(f).file_name().unwrap().to_string_lossy();
        assert!(am.contains(&format!("<Path>{name}</Path>")), "{name}");
    }
    assert!(Path::new(&dir).join("VOLINDEX.xml").exists());
    if Command::new("xmllint").arg("--version").output().is_ok() {
        for f in [&cpl_path, &pkl_path, &Path::new(&dir).join("ASSETMAP.xml").to_string_lossy().to_string()] {
            let out = Command::new("xmllint").args(["--noout", f]).output().unwrap();
            assert!(out.status.success(), "{f}: {}", String::from_utf8_lossy(&out.stderr));
        }
    }

    // ffprobe / ffmpeg (oracles only): J2K 12-bit X'Y'Z' 1998×1080, the pixels our decoder gives
    if Command::new("ffprobe").arg("-version").output().is_ok() {
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,width,height,pix_fmt,nb_frames", "-of", "csv=p=0", &pic_path])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("jpeg2000,1998,1080,xyz12le"), "{text}");
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,sample_rate,channels,bits_per_sample", "-of", "csv=p=0", &snd_path])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "pcm_s24le,48000,6,24");
        // every frame found (frame-wrapped essence, not taken for clip-wrapped OP-Atom)
        let out = Command::new("ffprobe")
            .args(["-v", "warning", "-count_frames", "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0", &pic_path])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "10");
        assert!(!String::from_utf8_lossy(&out.stderr).contains("failed"), "{}", String::from_utf8_lossy(&out.stderr));
        let raw = Command::new("ffmpeg")
            .args(["-v", "error", "-i", &pic_path, "-vf", "select=eq(n\\,4)", "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "xyz12le", "-"])
            .output()
            .unwrap();
        assert!(raw.status.success(), "{}", String::from_utf8_lossy(&raw.stderr));
        let theirs: Vec<i32> = raw.stdout.chunks_exact(2).map(|b| i32::from(u16::from_le_bytes([b[0], b[1]]) >> 4)).collect();
        assert_eq!(theirs.len(), 1998 * 1080 * 3);
        let worst = (0..1998 * 1080).map(|i| (0..3).map(|c| (theirs[i * 3 + c] - img.components[c].data[i]).abs()).max().unwrap()).max().unwrap();
        assert!(worst <= 2, "ffmpeg and our decoder differ by {worst}");
    }
}

/// 4K Scope: one frame of the DCI 4K profile at 4096×1716; 23.976 fps becomes 24; no sound when
/// audio is off.
#[test]
fn four_k_scope_without_sound() {
    let (p, seq, m) = project();
    let dir = tmp("scope4k");
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.clone(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(1))),
        dcp_resolution: "4k".into(),
        dcp_aspect: "scope".into(),
        frame_rate: Some(FrameRate::new(24000, 1001)),
        include_audio: false,
        ..Default::default()
    };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    assert_eq!(r.extra_files.len(), 5, "no sound file: {:?}", r.extra_files);
    let pic = std::fs::read(file(&dir, "j2c_")).unwrap();
    let mxf = cfa4_mxf::open(&pic).unwrap();
    assert_eq!(mxf.tracks[0].edit_rate, cfa4_mxf::Rational::new(24, 1));
    let p = mxf.tracks[0].picture.as_ref().unwrap();
    assert_eq!((p.stored_width, p.stored_height), (4096, 1716));
    let cs = mxf.read_sample(&pic, 0, 0).unwrap();
    // SIZ: Rsiz 4 (DCI 4K)
    assert_eq!(&cs[..6], &[0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F]);
    assert_eq!(u16::from_be_bytes([cs[6], cs[7]]), 4);
    let img = cfa4_j2k::decode(&cs).unwrap();
    assert_eq!((img.width, img.height), (4096, 1716));
    let cpl = std::fs::read_to_string(file(&dir, "cpl_")).unwrap();
    assert_eq!(element(&cpl, "ContentKind"), Some("feature"));
    assert_eq!(element(&cpl, "EditRate"), Some("24 1"));
    assert_eq!(element(&cpl, "ScreenAspectRatio"), Some("4096 1716"));
    assert!(!cpl.contains("<MainSound>"));
}

/// Exports into memory (an output sink) hand every file of the package to the sink.
#[test]
fn sink_receives_every_file() {
    let (p, seq, m) = project();
    let got: Arc<Mutex<Vec<(String, usize)>>> = Arc::default();
    let g = got.clone();
    let s = ExportSettings {
        format: Format::Dcp,
        path: "pkg".into(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(2))),
        sink: Some(OutputSink(Arc::new(move |path: &str, data: Vec<u8>| {
            g.lock().unwrap().push((path.to_string(), data.len()));
            Ok(())
        }))),
        ..Default::default()
    };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 6);
    assert!(got.iter().all(|(p, n)| p.starts_with("pkg") && *n > 0));
    assert_eq!(r.bytes, got.iter().map(|(_, n)| *n as u64).sum::<u64>());
    assert!(!Path::new("pkg").exists(), "nothing written to disk");
}
