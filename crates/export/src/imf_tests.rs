//! IMF App 2E export (#1011): the package read back with our MXF reader and JPEG 2000 decoder,
//! the playlist, packing list and asset map checked against the track files, and ffprobe as an
//! external oracle.

use std::path::Path;
use std::process::Command;

use super::*;
use crate::dcp_out::sha1;
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::TICKS_PER_SECOND;

fn project(rate: FrameRate) -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("imf");
    let white = GeneratorSource::new(Generator::ColorMatte { color: [1.0, 1.0, 1.0, 1.0] }, 320, 180, rate, Tick(2 * TICKS_PER_SECOND));
    let tone = GeneratorSource::new(Generator::Tone { hz: 440.0, db: -6.0 }, 320, 180, rate, Tick(2 * TICKS_PER_SECOND));
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
    let seq = p.new_sequence("IMF <Test> & Co", SequenceSettings { width: 320, height: 180, frame_rate: rate, ..Default::default() }, 1, 1, None);
    let len = rate.tick_of(40);
    let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, len), rate).unwrap();
    let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, len), rate).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(vi);
    q.audio_tracks[0].items.push(ai);
    let mut m = SourceMap::default();
    m.0.insert(v, Arc::new(white));
    m.0.insert(a, Arc::new(tone));
    (Arc::new(p), seq, m)
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("fc-export-imf-{}", std::process::id())).join(name);
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

/// From `<tag` to the matching close of the first `tag` element.
fn block<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}"))?;
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

/// The one file of the package whose name starts with `prefix`.
fn file(dir: &str, prefix: &str) -> String {
    let mut found: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(prefix)).collect();
    assert_eq!(found.len(), 1, "{prefix} in {dir}");
    Path::new(dir).join(found.remove(0)).to_string_lossy().to_string()
}

fn uuid_bytes(urn: &str) -> Vec<u8> {
    let hex = urn.trim_start_matches("urn:uuid:").replace('-', "");
    (0..16).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn symbols(xml: &str) -> Vec<String> {
    xml.split("<r1:MCATagSymbol>").skip(1).filter_map(|t| t.split('<').next()).map(str::to_string).collect()
}

/// A 25 fps stereo package of ten frames: track files, playlist, packing list and asset map agree;
/// the picture is 10-bit full-range white R'G'B'; the sound 24-bit 48 kHz with Standard Stereo labels.
#[test]
fn stereo_package() {
    let rate = FrameRate::new(25, 1);
    let (p, seq, m) = project(rate);
    let dir = tmp("stereo");
    let s = ExportSettings {
        format: Format::Imf,
        path: dir.clone(),
        range: Some(TimeRange::new(Tick::ZERO, rate.tick_of(10))),
        imf_content_kind: "episode".into(),
        dcp_language: "hu-HU".into(),
        ..Default::default()
    };
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    assert_eq!((r.path.as_str(), r.frames), (dir.as_str(), 10));
    assert_eq!(r.extra_files.len(), 6, "{:?}", r.extra_files);
    let total: u64 = r.extra_files.iter().map(|f| std::fs::metadata(f).unwrap().len()).sum();
    assert_eq!(r.bytes, total);

    // image track file (ST 2067-5): OP1a, frame-wrapped JPEG 2000 in the 2K IMF lossy profile
    let pic_path = file(&dir, "VIDEO_");
    let pic = std::fs::read(&pic_path).unwrap();
    let mxf = cfa4_mxf::open(&pic).unwrap();
    assert!(mxf.warnings.is_empty(), "{:?}", mxf.warnings);
    assert_eq!(mxf.operational_pattern, cfa4_mxf::OperationalPattern::Generalized { item: 1, package: 1 });
    let t = &mxf.tracks[0];
    assert_eq!(t.codec, cfa4_mxf::Codec::Jpeg2000);
    assert_eq!(t.wrapping, cfa4_mxf::Wrapping::Frame);
    assert_eq!(t.edit_rate, cfa4_mxf::Rational::new(25, 1));
    assert_eq!(t.duration, Some(10));
    assert!(t.picture.as_ref().unwrap().rgba, "an RGBA descriptor");
    let cs = mxf.read_sample(&pic, 0, 3).unwrap();
    assert_eq!(u16::from_be_bytes([cs[6], cs[7]]) >> 8, 0x04, "2K IMF Single Tile Lossy profile");
    let img = cfa4_j2k::decode(&cs).unwrap();
    assert_eq!((img.width, img.height, img.components.len()), (320, 180, 3));
    for c in &img.components {
        assert_eq!(c.precision, 10);
        assert!((c.data[180 * 90 + 160] - 1023).abs() <= 4, "white: {}", c.data[180 * 90 + 160]);
    }

    // audio track file: clip-wrapped, 24-bit 48 kHz stereo, ten frames of sound
    let snd_path = file(&dir, "AUDIO_");
    let snd = std::fs::read(&snd_path).unwrap();
    let mxf_s = cfa4_mxf::open(&snd).unwrap();
    assert!(mxf_s.warnings.is_empty(), "{:?}", mxf_s.warnings);
    let a = &mxf_s.tracks[0];
    assert_eq!(a.kind, cfa4_mxf::TrackKind::Sound);
    assert_eq!(a.wrapping, cfa4_mxf::Wrapping::Clip);
    assert_eq!(a.stored_sample_frames(), 10 * 1920);
    let info = a.sound.as_ref().unwrap();
    assert_eq!((info.bits, info.channels, info.sample_rate), (24, 2, cfa4_mxf::Rational::new(48_000, 1)));
    let pcm = mxf_s.read_pcm(&snd, 0, 0, 10 * 1920).unwrap();
    let rms = |c: &[f32]| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
    assert!(rms(&pcm[0]) > 0.05 && rms(&pcm[1]) > 0.05, "the tone is on L and R");

    // composition playlist
    let cpl_path = file(&dir, "CPL_");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    assert!(cpl.contains("xmlns=\"http://www.smpte-ra.org/schemas/2067-3/2016\""));
    assert_eq!(element(&cpl, "ContentTitle"), Some("IMF &lt;Test&gt; &amp; Co"));
    assert_eq!(element(&cpl, "ContentKind"), Some("episode"));
    assert_eq!(element(&cpl, "EditRate"), Some("25 1"));
    assert_eq!(element(&cpl, "TotalRunningTime"), Some("00:00:00"));
    assert_eq!(element(&cpl, "Language"), Some("hu-HU"));
    assert_eq!(element(&cpl, "cc:ApplicationIdentification"), Some("http://www.smpte-ra.org/ns/2067-21/5ED"));
    let image = block(&cpl, "cc:MainImageSequence").expect("main image");
    let audio = block(&cpl, "cc:MainAudioSequence").expect("main audio");
    assert_eq!(element(image, "EditRate"), Some("25 1"));
    assert_eq!((element(image, "IntrinsicDuration"), element(image, "SourceDuration")), (Some("10"), Some("10")));
    assert_eq!(element(audio, "EditRate"), Some("48000 1"));
    assert_eq!((element(audio, "IntrinsicDuration"), element(audio, "SourceDuration")), (Some("19200"), Some("19200")));
    // each resource names its track file (the file package UMID's material number), hash and descriptor
    let descriptors = block(&cpl, "EssenceDescriptorList").unwrap();
    for (seq_xml, data, group) in [(image, &pic, "r0:RGBADescriptor"), (audio, &snd, "r0:WAVEPCMDescriptor")] {
        let id = element(seq_xml, "TrackFileId").unwrap();
        let bytes = uuid_bytes(id);
        assert!(data.windows(32).any(|w| w[..12] == [0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0F, 0x20] && w[16..] == bytes[..]));
        assert_eq!(element(seq_xml, "Hash").unwrap(), cfa4_project::gtemplate::base64_encode(&sha1(data)));
        let enc = element(seq_xml, "SourceEncoding").unwrap();
        let d = descriptors.split("<EssenceDescriptor>").find(|d| d.contains(&format!("<Id>{enc}</Id>"))).expect("the descriptor");
        assert!(d.contains(&format!("<{group}")), "{group}");
        let want = cfa4_mxf::regxml::essence_descriptor(data).unwrap();
        assert!(d.contains(want.trim_end()), "the track file's own descriptor");
    }
    assert!(image.contains("HashAlgorithm Algorithm=\"http://www.w3.org/2000/09/xmldsig#sha1\""));
    // the stereo labels with their ST 2067-2 titles, and the full-range 10-bit R'G'B'
    assert_eq!(symbols(descriptors), ["sgST", "chL", "chR"]);
    for item in [
        "<r1:MCATitle>IMF &lt;Test&gt; &amp; Co</r1:MCATitle>",
        "<r1:MCATitleVersion>1</r1:MCATitleVersion>",
        "<r1:MCAAudioContentKind>PRM</r1:MCAAudioContentKind>",
        "<r1:MCAAudioElementKind>FCMP</r1:MCAAudioElementKind>",
    ] {
        assert!(descriptors.contains(item), "{item}");
    }
    assert_eq!(descriptors.matches("<r1:RFC5646SpokenLanguage>hu-HU</r1:RFC5646SpokenLanguage>").count(), 3);
    assert!(descriptors.contains("<r1:ComponentMaxRef>1023</r1:ComponentMaxRef>") && descriptors.contains("<r1:ComponentMinRef>0</r1:ComponentMinRef>"));

    // packing list and asset map
    let pkl_path = file(&dir, "PKL_");
    let pkl = std::fs::read_to_string(&pkl_path).unwrap();
    assert!(pkl.contains("xmlns=\"http://www.smpte-ra.org/schemas/2067-2/2016/PKL\""));
    let assets: Vec<&str> = pkl.split("<Asset>").skip(1).collect();
    assert_eq!(assets.len(), 3, "the CPL and both track files");
    for a in &assets {
        let name = element(a, "OriginalFileName").unwrap();
        let data = std::fs::read(Path::new(&dir).join(name)).unwrap();
        assert_eq!(element(a, "Size").unwrap(), data.len().to_string(), "{name}");
        assert_eq!(element(a, "Hash").unwrap(), cfa4_project::gtemplate::base64_encode(&sha1(&data)), "{name}");
        assert_eq!(element(a, "Type").unwrap(), if name.ends_with(".xml") { "text/xml" } else { "application/mxf" });
        assert!(a.contains("<HashAlgorithm Algorithm=\"http://www.w3.org/2000/09/xmldsig#sha1\"/>"));
    }
    for seq_xml in [image, audio] {
        assert!(assets.iter().any(|a| element(a, "Id") == element(seq_xml, "TrackFileId")), "the PKL lists each track file by its id");
    }
    let am = std::fs::read_to_string(Path::new(&dir).join("ASSETMAP.xml")).unwrap();
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
    // the official XML schemas (ST 2067-3, ST 2067-2 2016 PKL, ST 429-9), when a folder holding
    // them is given (Photon's `src/main/resources/org`: `smpte_ra/schemas/...`, `w3/...`)
    if let Ok(org) = std::env::var("CFA4_IMF_SCHEMAS") {
        let schemas = Path::new(&org).join("smpte_ra/schemas");
        let at = |p: &str| schemas.join(p).to_string_lossy().to_string();
        let wrapper = Path::new(&dir).join("cpl-schemas.xsd.tmp");
        std::fs::write(
            &wrapper,
            format!(
                "<?xml version=\"1.0\"?>\n<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" targetNamespace=\"urn:cfa4-test\">\n<xs:import namespace=\"http://www.w3.org/2000/09/xmldsig#\" schemaLocation=\"{}\"/>\n<xs:import namespace=\"http://www.smpte-ra.org/schemas/2067-3/2016\" schemaLocation=\"{}\"/>\n<xs:import namespace=\"http://www.smpte-ra.org/ns/2067-2/2020\" schemaLocation=\"{}\"/>\n</xs:schema>\n",
                Path::new(&org).join("w3/_2000_09/xmldsig/xmldsig-core-schema.xsd").display(),
                at("st2067_3_2016/imf-cpl-20160411.xsd"),
                at("st2067_2_2020/imf-core-constraints-2020.xsd")
            ),
        )
        .unwrap();
        let am_path = Path::new(&dir).join("ASSETMAP.xml").to_string_lossy().to_string();
        for (xsd, f) in [
            (wrapper.to_string_lossy().to_string(), &cpl_path),
            (at("st2067_2_2016/PKL/packingList_schema.xsd"), &pkl_path),
            (at("st0429_9_2007/AM/assetMap_schema.xsd"), &am_path),
        ] {
            let out = Command::new("xmllint").args(["--noout", "--schema", &xsd, f]).output().unwrap();
            assert!(out.status.success(), "{f}: {}", String::from_utf8_lossy(&out.stderr));
        }
        std::fs::remove_file(&wrapper).unwrap();
    }
    // ffprobe (oracle only)
    if Command::new("ffprobe").arg("-version").output().is_ok() {
        let out =
            Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_name,width,height", "-of", "csv=p=0", &pic_path]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "jpeg2000,320,180");
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,sample_rate,channels", "-of", "csv=p=0", &snd_path])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "pcm_s24le,48000,2");
    }
}

/// 29.97 fps (1601.6 samples a frame): the composition plays a multiple of 5 frames (ST 2067-21
/// 8.3.2) and exactly their sound; lossless pictures decode to the exact 10-bit values; 5.1 has
/// the ST 428-12 labels.
#[test]
fn ntsc_lossless_51_package() {
    let rate = FrameRate::new(30000, 1001);
    let (p, seq, m) = project(rate);
    let dir = tmp("ntsc");
    let mut s = ExportSettings {
        format: Format::Imf,
        path: dir.clone(),
        range: Some(TimeRange::new(Tick::ZERO, rate.tick_of(12))),
        imf_lossless: true,
        ..Default::default()
    };
    s.audio.channels = 6;
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    assert_eq!(r.frames, 12);
    let pic = std::fs::read(file(&dir, "VIDEO_")).unwrap();
    let mxf = cfa4_mxf::open(&pic).unwrap();
    assert_eq!(mxf.tracks[0].edit_rate, cfa4_mxf::Rational::new(30000, 1001));
    let cs = mxf.read_sample(&pic, 0, 0).unwrap();
    assert_eq!(u16::from_be_bytes([cs[6], cs[7]]) >> 8, 0x07, "2K IMF reversible profile");
    let img = cfa4_j2k::decode(&cs).unwrap();
    assert!(img.components.iter().all(|c| c.data.iter().all(|&v| v == 1023)), "lossless white");
    let cpl = std::fs::read_to_string(file(&dir, "CPL_")).unwrap();
    assert_eq!(element(&cpl, "ContentKind"), Some("feature"));
    assert_eq!(element(&cpl, "EditRate"), Some("30000 1001"));
    assert!(!cpl.contains("<LocaleList>"), "no language set");
    let image = block(&cpl, "cc:MainImageSequence").unwrap();
    let audio = block(&cpl, "cc:MainAudioSequence").unwrap();
    assert_eq!((element(image, "IntrinsicDuration"), element(image, "SourceDuration")), (Some("12"), Some("10")));
    assert_eq!(element(audio, "SourceDuration"), Some("16016"), "10 frames of 1601.6 samples");
    let intrinsic: u64 = element(audio, "IntrinsicDuration").unwrap().parse().unwrap();
    assert!(intrinsic >= 16016);
    assert_eq!(symbols(block(&cpl, "EssenceDescriptorList").unwrap()), ["sg51", "chL", "chR", "chC", "chLFE", "chLs", "chRs"]);
    let snd = std::fs::read(file(&dir, "AUDIO_")).unwrap();
    assert_eq!(cfa4_mxf::open(&snd).unwrap().tracks[0].sound.as_ref().unwrap().channels, 6);
}

/// Fewer than 5 frames at 29.97 make no composition; a broken content kind or language is an
/// error before anything is written.
#[test]
fn rejects_what_imf_cannot_hold() {
    let rate = FrameRate::new(30000, 1001);
    let (p, seq, m) = project(rate);
    let dir = tmp("short");
    let s = ExportSettings { format: Format::Imf, path: dir.clone(), range: Some(TimeRange::new(Tick::ZERO, rate.tick_of(3))), ..Default::default() };
    let e = export(&p, seq, &s, &m, &Progress::default()).unwrap_err();
    assert!(e.to_string().contains("at least 5 frames"), "{e}");
    let bad = ExportSettings { imf_content_kind: "policy".into(), ..s.clone() };
    assert!(bad.validate().unwrap_err().to_string().contains("IMF content kind"));
    let bad = ExportSettings { dcp_language: "not a tag".into(), ..s };
    assert!(bad.validate().unwrap_err().to_string().contains("IMF language"));
}

/// Mono is a Discrete Numbered Sources group of one channel; into memory, every file goes to
/// the sink and nothing to disk.
#[test]
fn mono_into_a_sink() {
    let rate = FrameRate::new(24, 1);
    let (p, seq, m) = project(rate);
    let got: Arc<Mutex<Vec<(String, Vec<u8>)>>> = Arc::default();
    let g = got.clone();
    let mut s = ExportSettings {
        format: Format::Imf,
        path: "imp".into(),
        range: Some(TimeRange::new(Tick::ZERO, rate.tick_of(2))),
        sink: Some(OutputSink(Arc::new(move |path: &str, data: Vec<u8>| {
            g.lock().unwrap().push((path.to_string(), data));
            Ok(())
        }))),
        ..Default::default()
    };
    s.audio.channels = 1;
    let r = export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 6);
    assert_eq!(r.bytes, got.iter().map(|(_, d)| d.len() as u64).sum::<u64>());
    assert!(!Path::new("imp").exists(), "nothing written to disk");
    let cpl = got.iter().find(|(p, _)| p.contains("CPL_")).map(|(_, d)| String::from_utf8_lossy(d).to_string()).unwrap();
    assert_eq!(symbols(&cpl), ["sgDNS", "chNSC001"]);
    assert_eq!(element(block(&cpl, "cc:MainAudioSequence").unwrap(), "SourceDuration"), Some("4000"));
}

#[test]
fn rates_kinds_and_profiles() {
    let r = FrameRate::new;
    assert_eq!(imf_rate(r(24000, 1001)), r(24000, 1001));
    assert_eq!(imf_rate(r(48, 1)), r(50, 1));
    assert_eq!(imf_rate(r(120, 1)), r(60, 1));
    assert_eq!(imf_rate(r(15, 1)), r(24000, 1001));
    let broken = imf_rate(r(0, 0));
    assert!(IMF_RATES.iter().any(|&(n, d)| r(n, d) == broken), "a broken rate becomes an App 2E rate: {broken:?}");
    assert_eq!(Format::from_name("IMF"), Some(Format::Imf));
    assert_eq!(Format::from_name(Format::Imf.id()), Some(Format::Imf));
    assert_eq!(Format::Imf.extension(), "");
    let kind = |k: &str| imf_content_kind(&ExportSettings { imf_content_kind: k.into(), ..Default::default() });
    assert_eq!((kind(""), kind("Episode"), kind("policy")), (Some("feature"), Some("episode"), None));
    // the profile follows the size; lossy sublevels hold an eighth of the raw rate
    let p = imf_profile(1920, 1080, r(24, 1), false).unwrap();
    assert_eq!((p.size, p.reversible, p.sublevel), (cfa4_j2k::ImfSize::TwoK, false, 1));
    assert_eq!(imf_profile(3840, 2160, r(24, 1), true).unwrap().size, cfa4_j2k::ImfSize::FourK);
    assert_eq!(imf_profile(8192, 6224, r(24, 1), false).unwrap().size, cfa4_j2k::ImfSize::EightK);
    assert!(imf_profile(8194, 100, r(24, 1), false).is_none());
    assert_eq!(rgba_to_rgb10(&[255, 128, 0, 255], 1, 1), [vec![1023], vec![514], vec![0]]);
    assert_eq!(rgba_to_rgb10(&[1, 2], 2, 2)[0].len(), 4, "short input never panics");
}
