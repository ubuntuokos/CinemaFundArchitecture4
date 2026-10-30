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

// ---------------------------------------------------------------------------------------------
// Immersive audio (#343)

/// Demodulate the 48 kHz FSK of ST 430-12: four samples a bit, a one changes sign in the symbol.
fn fsk_bits(s: &[f32]) -> Vec<bool> {
    s.chunks_exact(4).map(|c| (c[0] > 0.0) != (c[3] > 0.0)).collect()
}

fn bits_value(b: &[bool]) -> u64 {
    b.iter().fold(0, |v, x| v << 1 | u64::from(*x))
}

/// The UUID in an asset `<Id>urn:uuid:…</Id>`.
fn asset_uuid(xml: &str) -> [u8; 16] {
    let id = element(xml, "Id").unwrap().trim_start_matches("urn:uuid:").replace('-', "");
    let mut u = [0u8; 16];
    for (i, b) in u.iter_mut().enumerate() {
        *b = u8::from_str_radix(&id[2 * i..2 * i + 2], 16).unwrap();
    }
    u
}

/// The text between `<tag …>` and `</tag>` (a tag with attributes or a prefix).
fn block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}"))?;
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

/// Checks shared by the immersive packages: the IAB track file, the 16-channel main sound with
/// the FSK, the `AuxData` in the playlist and the packing list. Returns (IAB file bytes, main
/// sound PCM).
fn check_immersive_package(dir: &str, frames: usize, spu: usize) -> (Vec<u8>, Vec<Vec<f32>>) {
    let iab_path = file(dir, "iab_");
    let iab = std::fs::read(&iab_path).unwrap();
    let m = cfa4_mxf::open(&iab).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, cfa4_mxf::OperationalPattern::Atom);
    let t = &m.tracks[0];
    assert_eq!(t.codec, cfa4_mxf::Codec::Iab);
    assert_eq!(t.samples.len(), frames);
    for i in 0..frames {
        let f = cfa4_iab::parse(&m.read_sample(&iab, 0, i).unwrap()).unwrap();
        // ST 429-18 7.2: DLC only, 24 bit, no UserData; Profile 1: no AuthoringToolInfo, 48 kHz
        assert!(f.audio.iter().all(|a| a.coding == cfa4_iab::Coding::Dlc), "frame {i}");
        assert_eq!((f.bit_depth, f.sample_rate), (24, 48_000));
        assert!(f.user_data.is_empty() && f.authoring_tool.is_none());
        assert_eq!(f.samples(), spu);
    }

    let cpl = std::fs::read_to_string(file(dir, "cpl_")).unwrap();
    let aux = block(&cpl, "axd:AuxData").expect("an AuxData asset");
    assert!(aux.contains("xmlns:axd=\"http://www.dolby.com/schemas/2012/AD\""));
    assert_eq!(element(aux, "axd:DataType"), Some("urn:smpte:ul:060e2b34.04010105.0e090604.00000000"));
    assert_eq!(element(aux, "Duration"), Some(frames.to_string().as_str()));
    assert_eq!(element(aux, "IntrinsicDuration"), Some(frames.to_string().as_str()));
    assert!(iab_path.ends_with(&format!("iab_{}.mxf", element(aux, "Id").unwrap().trim_start_matches("urn:uuid:"))));
    let pkl = std::fs::read_to_string(file(dir, "pkl_")).unwrap();
    assert_eq!(pkl.matches("<Asset>").count(), 4, "CPL, picture, sound, IAB");
    let iab_asset = pkl.split("<Asset>").find(|a| a.contains(element(aux, "Id").unwrap())).unwrap();
    assert_eq!(element(aux, "Hash"), element(iab_asset, "Hash"));
    assert_eq!(element(iab_asset, "Hash").unwrap(), cfa4_project::gtemplate::base64_encode(&sha1(&iab)));
    let am = std::fs::read_to_string(Path::new(dir).join("ASSETMAP.xml")).unwrap();
    assert_eq!(am.matches("<Asset>").count(), 5);
    if Command::new("xmllint").arg("--version").output().is_ok() {
        let out = Command::new("xmllint").args(["--noout", &file(dir, "cpl_")]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    // main sound: 16 channels, Configuration 4, FSK sync in channel 14 naming the sound asset
    let snd = std::fs::read(file(dir, "pcm_")).unwrap();
    let ms = cfa4_mxf::open(&snd).unwrap();
    let info = ms.tracks[0].sound.as_ref().unwrap();
    assert_eq!((info.channels, info.bits), (16, 24));
    assert!(snd.windows(16).any(|w| w == crate::dcp_iab::CHANNEL_CONFIG_4));
    assert_eq!(ms.tracks[0].stored_sample_frames(), (frames * spu) as u64);
    let pcm = ms.read_pcm(&snd, 0, 0, frames * spu).unwrap();
    let sound_id = asset_uuid(block(&cpl, "MainSound").unwrap());
    let bits = fsk_bits(&pcm[13]);
    let per_unit = bits.len() / frames;
    let packets = if spu >= 1600 { 4 } else { 2 };
    for (p, pk) in bits.chunks_exact(per_unit / packets).enumerate() {
        assert_eq!(bits_value(&pk[..16]), 0x4D56, "packet {p}");
        let sub = bits_value(&pk[22..24]) as usize;
        assert_eq!(sub, p % 4);
        assert_eq!(bits_value(&pk[24..56]), u64::from(u32::from_be_bytes(sound_id[sub * 4..sub * 4 + 4].try_into().unwrap())));
        assert_eq!(bits_value(&pk[56..80]), (p / packets) as u64);
    }
    for c in [6, 7, 8, 9, 10, 11, 12, 14, 15] {
        assert!(pcm[c].iter().all(|v| *v == 0.0), "channel {}", c + 1);
    }
    (iab, pcm)
}

/// `bed`: the 5.1 mix as a 7.1 DS bed, sample for sample the 5.1 fallback in channels 1-6.
#[test]
fn immersive_bed_package() {
    let (p, seq, m) = project();
    let dir = tmp("iab-bed");
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.clone(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(6))),
        dcp_immersive: "bed".into(),
        ..Default::default()
    };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    assert_eq!(r.extra_files.len(), 7, "{:?}", r.extra_files);
    let (iab, pcm) = check_immersive_package(&dir, 6, 1920);
    let mi = cfa4_mxf::open(&iab).unwrap();
    for i in 0..6 {
        let f = cfa4_iab::parse(&mi.read_sample(&iab, 0, i).unwrap()).unwrap();
        assert_eq!(f.beds.len(), 1);
        assert!(f.objects.is_empty());
        let ids: Vec<u32> = f.beds[0].channels.iter().map(|c| c.id).collect();
        assert_eq!(ids, [0x00, 0x04, 0x02, 0x0D, 0x05, 0x09, 0x07, 0x08], "7.1 DS");
        // bed L and R are the main sound's L and R
        for (asset, ch) in [(0, 0), (1, 1)] {
            let bed: Vec<f32> = f.audio[asset].samples.iter().map(|v| *v as f32 / 2_147_483_648.0).collect();
            assert_eq!(&bed[..], &pcm[ch][i * 1920..(i + 1) * 1920], "frame {i} channel {ch}");
        }
    }
    let rms = |c: &[f32]| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
    assert!(rms(&pcm[0]) > 0.05 && rms(&pcm[1]) > 0.05, "the tone is on L and R");
    // the descriptor counts the bed's channels
    let sub = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x05, 0x0E, 0x09, 0x06, 0x06];
    assert!(iab.windows(12).any(|w| w == sub));
}

/// A 7.1 DS bed frame at `rate` with PCM assets and an authoring tool, as a source file.
fn source_frames(rate: cfa4_iab::FrameRate, n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            let spu = rate.samples(false, 0);
            let planar: Vec<Vec<f32>> = (0..6).map(|c| (0..spu).map(|k| ((k + i * spu) as f32 * 0.02 * (c + 1) as f32).sin() * 0.3).collect()).collect();
            let mut f = crate::dcp_iab::bed_frame(rate, &planar);
            f.audio.iter_mut().for_each(|a| a.coding = cfa4_iab::Coding::Pcm);
            f.authoring_tool = Some("https://example.org/tool".into());
            cfa4_iab::write(&f).unwrap()
        })
        .collect()
}

/// `file`: a raw IABitstream with PCM assets is re-coded (DLC, no AuthoringToolInfo), padded
/// with silence, and rendered to 5.1 for the fallback; a source already fit for a DCP (an IAB
/// MXF) is copied byte for byte.
#[test]
fn immersive_file_package() {
    let (p, seq, m) = project();
    let src_dir = tmp("iab-src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let raw = Path::new(&src_dir).join("three.iab");
    let frames = source_frames(cfa4_iab::FrameRate::R25, 3);
    std::fs::write(&raw, frames.concat()).unwrap();

    let info = probe_iab_source(&raw.to_string_lossy()).unwrap();
    assert_eq!((info.container.as_str(), info.frames, info.frame_rate, info.sample_rate), ("iab", 3, (25, 1), 48_000));
    assert_eq!((info.max_bed_channels, info.max_objects, info.scanned), (8, 0, 3));
    assert!(info.needs_conform && info.profile_notes.is_empty());

    let dir = tmp("iab-file");
    let mut s = ExportSettings {
        format: Format::Dcp,
        path: dir.clone(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(5))),
        dcp_immersive: "file".into(),
        dcp_iab_source: raw.to_string_lossy().to_string(),
        ..Default::default()
    };
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let (iab, pcm) = check_immersive_package(&dir, 5, 1920);
    let mi = cfa4_mxf::open(&iab).unwrap();
    for (i, src) in frames.iter().enumerate() {
        let mut want = cfa4_iab::parse(src).unwrap();
        want.audio.iter_mut().for_each(|a| a.coding = cfa4_iab::Coding::Dlc);
        want.authoring_tool = None;
        let got = cfa4_iab::parse(&mi.read_sample(&iab, 0, i).unwrap()).unwrap();
        assert_eq!(got, want, "frame {i}: the same audio and metadata, re-coded");
        // the fallback is the frame rendered to 5.1
        let r = cfa4_iab::render(&want, &cfa4_iab::Layout::surround_5_1()).unwrap();
        for c in 0..6 {
            for k in (0..1920).step_by(97) {
                assert!((pcm[c][i * 1920 + k] - r[c][k]).abs() < 2e-7, "frame {i} channel {c}");
            }
        }
    }
    for i in 3..5 {
        let got = cfa4_iab::parse(&mi.read_sample(&iab, 0, i).unwrap()).unwrap();
        assert!(got.audio.iter().all(|a| a.samples.iter().all(|v| *v == 0)), "padding frame {i} is silent");
        assert!(pcm.iter().take(6).all(|c| c[i * 1920..(i + 1) * 1920].iter().all(|v| *v == 0.0)));
    }

    // the package's own IAB track file as the next source: frames copied byte for byte, no fallback
    let src_mxf = Path::new(&src_dir).join("track.mxf");
    std::fs::copy(file(&dir, "iab_"), &src_mxf).unwrap();
    let dir2 = tmp("iab-file2");
    s.path = dir2.clone();
    s.dcp_iab_source = src_mxf.to_string_lossy().to_string();
    s.dcp_iab_fallback = "none".into();
    s.range = Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(4)));
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let (iab2, pcm2) = check_immersive_package(&dir2, 4, 1920);
    let m2 = cfa4_mxf::open(&iab2).unwrap();
    for i in 0..4 {
        assert_eq!(m2.read_sample(&iab2, 0, i).unwrap(), mi.read_sample(&iab, 0, i).unwrap(), "frame {i}");
    }
    assert!(pcm2.iter().take(6).all(|c| c.iter().all(|v| *v == 0.0)), "fallback off: channels 1-6 silent");
    let info = probe_iab_source(&src_mxf.to_string_lossy()).unwrap();
    assert_eq!((info.container.as_str(), info.frames, info.needs_conform), ("mxf", 5, false));

    // the material package's program: pre-charge skipped, a trimmed tail dropped, a duration
    // longer than the stored frames refused
    let mut t = m2.tracks[0].clone();
    assert_eq!(crate::dcp_iab::program(&t), Ok((0, 4)));
    (t.origin, t.start_position, t.duration) = (1, 1, Some(2));
    assert_eq!(crate::dcp_iab::program(&t), Ok((2, 2)));
    t.duration = Some(3);
    assert!(crate::dcp_iab::program(&t).unwrap_err().contains("declares 3 frames"));
    t.origin = 9;
    assert!(crate::dcp_iab::program(&t).is_err());
    // a track file cut inside its essence is refused, not padded with silence
    let bytes = std::fs::read(&src_mxf).unwrap();
    let cut = Path::new(&src_dir).join("cut.mxf");
    let at = (0..bytes.len().saturating_sub(16))
        .rev()
        .find(|&i| bytes[i..i + 16] == [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x05, 0x0E, 0x09, 0x06, 0x01, 0x00, 0x00, 0x00, 0x01])
        .unwrap();
    std::fs::write(&cut, &bytes[..at + 40]).unwrap();
    let e = probe_iab_source(&cut.to_string_lossy()).unwrap_err().to_string();
    assert!(e.contains("cut.mxf") && e.contains("truncated"), "{e}");

    // a source at another rate is refused
    std::fs::write(&raw, source_frames(cfa4_iab::FrameRate::R24, 2).concat()).unwrap();
    s.dcp_iab_source = raw.to_string_lossy().to_string();
    s.path = tmp("iab-rate");
    let e = export(&p, seq, &s, &m, &Progress::default()).unwrap_err();
    assert!(e.to_string().contains("24 fps"), "{e}");
}

/// A real IAB source (`CFA4_IAB_SOURCE`: an IAB MXF such as samples/iab/mxf/sounds_imf_iab.mxf
/// of the project samples, Dolby Atmos frames wrapped by the reference tool, or a raw
/// IABitstream) re-wrapped into a 24 fps DCP.
#[test]
fn immersive_real_iab_source() {
    let Ok(path) = std::env::var("CFA4_IAB_SOURCE") else { return };
    let info = probe_iab_source(&path).unwrap();
    assert_eq!(info.frame_rate, (24, 1));
    assert!(info.profile_notes.is_empty(), "{info:?}");
    let (p, seq, m) = project();
    let dir = tmp("iab-real");
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.clone(),
        frame_rate: Some(FrameRate::new(24, 1)),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(12))),
        dcp_immersive: "file".into(),
        dcp_iab_source: path.clone(),
        ..Default::default()
    };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let (iab, pcm) = check_immersive_package(&dir, r.frames as usize, 2000);
    let src = std::fs::read(&path).unwrap();
    let source: Vec<Vec<u8>> = match cfa4_mxf::open(&src) {
        Ok(ms) => (0..r.frames as usize).map(|i| ms.read_sample(&src, 0, i).unwrap()).collect(),
        Err(_) => cfa4_iab::split(&src).into_iter().map(<[u8]>::to_vec).collect(),
    };
    let mi = cfa4_mxf::open(&iab).unwrap();
    for i in 0..r.frames as usize {
        let mut want = cfa4_iab::parse(&source[i]).unwrap();
        want.audio.iter_mut().for_each(|a| a.coding = cfa4_iab::Coding::Dlc);
        want.authoring_tool = None;
        want.user_data.clear();
        want.bit_depth = 24;
        assert_eq!(cfa4_iab::parse(&mi.read_sample(&iab, 0, i).unwrap()).unwrap(), want, "frame {i}");
    }
    assert!(pcm.iter().take(6).any(|c| c.iter().any(|v| *v != 0.0)), "the rendered fallback is not silent");
}
