//! MPEG-2 export (program stream, transport streams, elementary stream): read back with our demuxer
//! and decoder, and against ffprobe / ffmpeg as external oracles (the pictures decode to what our
//! decoder gives, the LPCM is sample-exact, the AAC decodes).

use std::path::Path;
use std::process::Command;

use super::*;
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::TICKS_PER_SECOND;

/// 1 s at 25 fps, 320×180: colour bars over V1, a 440 Hz tone on A1 (stereo mix).
fn project() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("mpeg2");
    let r = FrameRate::new(25, 1);
    let bars = GeneratorSource::new(Generator::CountingLeader, 320, 180, r, Tick(2 * TICKS_PER_SECOND));
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
    let v = add(&mut p, &bars);
    let a = add(&mut p, &tone);
    let seq = p.new_sequence("MPEG-2 Test", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
    let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(25)), r).unwrap();
    let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(25)), r).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(vi);
    q.audio_tracks[0].items.push(ai);
    let mut m = SourceMap::default();
    m.0.insert(v, Arc::new(bars));
    m.0.insert(a, Arc::new(tone));
    (Arc::new(p), seq, m)
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("fc-export-mpeg2-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().to_string()
}

fn settings(path: &str) -> ExportSettings {
    ExportSettings {
        format: Format::Mpeg2,
        path: path.into(),
        bitrate_kbps: 4000,
        keyframe_distance: Some(12),
        audio: AudioSettings { bits: 16, sample_rate: Some(48_000), channels: 2, bitrate_kbps: 192, ..Default::default() },
        ..Default::default()
    }
}

/// The video elementary stream of an exported file (every access unit, concatenated), the PTS of
/// each, and the stream list.
fn demux(path: &str) -> (Vec<u8>, Vec<i64>, cfa4_mpegts::File, Vec<u8>) {
    let data = std::fs::read(path).unwrap();
    let f = cfa4_mpegts::open(&data).unwrap();
    assert!(f.warnings.is_empty(), "{path}: {:?}", f.warnings);
    let v = f.find(cfa4_mpegts::Kind::Video, |c| *c == cfa4_mpegts::Codec::Mpeg2Video).unwrap();
    let es: Vec<u8> = (0..f.streams[v].units.len()).flat_map(|k| f.read_unit(&data, v, k).unwrap()).collect();
    let pts = f.streams[v].units.iter().map(|u| u.pts.unwrap()).collect();
    (es, pts, f, data)
}

/// Our decoder's pictures of `es`.
fn decode(es: &[u8]) -> Vec<cfa4_mpeg2v::Picture> {
    let mut d = cfa4_mpeg2v::Decoder::new();
    let mut out = Vec::new();
    for (k, r) in cfa4_mpeg2v::access_units(es).into_iter().enumerate() {
        out.extend(d.decode(&es[r], k as i64).unwrap());
    }
    out.extend(d.flush());
    assert_eq!(d.errors(), 0, "{:?}", d.last_error());
    out
}

/// The 16-bit mix as a WAV export gives it (little-endian interleaved samples).
fn wav_pcm(p: &Arc<Project>, seq: ItemId, m: &SourceMap) -> Vec<u8> {
    let path = tmp("mix.wav");
    let s = ExportSettings { format: Format::Wav, ..settings(&path) };
    export(p, seq, &s, m, &Progress::default()).unwrap();
    let wav = std::fs::read(&path).unwrap();
    let at = wav.windows(4).position(|w| w == b"data").unwrap();
    wav[at + 8..].to_vec()
}

#[test]
fn program_stream_carries_mpeg2_video_and_lpcm() {
    let (p, seq, m) = project();
    let path = tmp("out.mpg");
    let r = export(&p, seq, &settings(&path), &m, &Progress::default()).unwrap();
    assert_eq!(r.frames, 25);
    let (es, pts, f, data) = demux(&path);
    assert_eq!(f.format, cfa4_mpegts::Format::Ps { mpeg1: false });
    assert_eq!(pts, (0..25).map(|k| 63_000 + k * 3600).collect::<Vec<_>>());
    assert!(es.ends_with(&[0, 0, 1, 0xB7]));
    let pics = decode(&es);
    assert_eq!(pics.len(), 25);
    assert!(pics.iter().all(|p| p.width == 320 && p.height == 180));
    // a sequence header (I picture) every 12 pictures
    let keys: Vec<usize> =
        f.streams.iter().find(|s| s.codec == cfa4_mpegts::Codec::Mpeg2Video).unwrap().units.iter().enumerate().filter(|(_, u)| u.key).map(|(k, _)| k).collect();
    assert_eq!(keys, [0, 12, 24]);
    // the LPCM is the WAV export's 16-bit mix, byte-swapped
    let a = f.find(cfa4_mpegts::Kind::Audio, |c| *c == cfa4_mpegts::Codec::LpcmDvd).unwrap();
    assert_eq!(f.streams[a].lpcm, Some(cfa4_mpegts::LpcmFormat { sample_rate: 48_000, channels: 2, bits: 16 }));
    let be: Vec<u8> = (0..f.streams[a].units.len()).flat_map(|k| f.read_unit(&data, a, k).unwrap()).collect();
    let le: Vec<u8> = be.chunks_exact(2).flat_map(|s| [s[1], s[0]]).collect();
    assert_eq!(le, wav_pcm(&p, seq, &m));
}

#[test]
fn transport_streams_carry_mpeg2_video_and_aac() {
    let (p, seq, m) = project();
    for (name, size) in [("out.ts", 188), ("out.m2ts", 192)] {
        let path = tmp(name);
        export(&p, seq, &settings(&path), &m, &Progress::default()).unwrap();
        let (es, pts, f, data) = demux(&path);
        assert!(matches!(f.format, cfa4_mpegts::Format::Ts { packet_size, .. } if packet_size == size), "{:?}", f.format);
        assert_eq!(data.len() % size, 0);
        assert_eq!(pts.len(), 25);
        assert_eq!(decode(&es).len(), 25);
        let a = f.find(cfa4_mpegts::Kind::Audio, |c| *c == cfa4_mpegts::Codec::AacAdts).unwrap();
        let units = &f.streams[a].units;
        // 1 s of 1024-sample frames plus the encoder delay; the first starts the delay early
        assert!((47..=50).contains(&units.len()), "{}", units.len());
        assert_eq!(units[1].pts.unwrap() - units[0].pts.unwrap(), 1920);
        assert!(units[0].pts.unwrap() < 63_000);
    }
}

#[test]
fn elementary_stream_is_the_bare_video() {
    let (p, seq, m) = project();
    let path = tmp("out.m2v");
    let s = settings(&path);
    assert!(!s.has_audio());
    assert_eq!(s.extension(), "m2v");
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let es = std::fs::read(&path).unwrap();
    assert!(es.starts_with(&[0, 0, 1, 0xB3]) && es.ends_with(&[0, 0, 1, 0xB7]));
    assert_eq!(decode(&es).len(), 25);
    // the same pictures as in a program stream
    let mpg = tmp("same.mpg");
    export(&p, seq, &settings(&mpg), &m, &Progress::default()).unwrap();
    assert_eq!(demux(&mpg).0, es);
}

#[test]
fn constant_quantiser_and_4_2_2() {
    let (p, seq, m) = project();
    let path = tmp("q422.m2v");
    let s = ExportSettings { mpeg2_422: true, mpeg2_quantizer: Some(2), ..settings(&path) };
    assert!(s.summary(320, 180, FrameRate::new(25, 1), 48_000, Tick(TICKS_PER_SECOND)).video.contains("4:2:2 Profile, constant quantiser 2"));
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let pics = decode(&std::fs::read(&path).unwrap());
    assert_eq!(pics.len(), 25);
    assert!(pics.iter().all(|p| p.chroma == cfa4_mpeg2v::ChromaFormat::Yuv422 && p.cb.len() == 160 * 180));
    // the 4:2:2 conversion: the first picture's luma and chroma are the 10-bit conversion rounded
    let pipe = pipeline::Pipeline::new(p.clone(), seq, &s, false).unwrap();
    let (rgba, _) = pipe.frame(0, &m);
    let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
    rgba_to_yuv422_8(&rgba, 320, 180, &mut y, &mut u, &mut v);
    let psnr = |a: &[u8], b: &[u8]| {
        let se: f64 = a.iter().zip(b).map(|(x, y)| f64::from(x.abs_diff(*y)).powi(2)).sum();
        10.0 * (255.0f64 * 255.0 / (se / a.len() as f64).max(1e-12)).log10()
    };
    assert!(psnr(&pics[0].y, &y) > 40.0 && psnr(&pics[0].cb, &u) > 40.0 && psnr(&pics[0].cr, &v) > 40.0);
}

#[test]
fn dvd_presets_signal_16_9_and_the_dvd_rate() {
    let (p, seq, m) = project();
    for (name, w, h, rate) in [("MPEG-2 DVD PAL 16:9", 720, 576, (25, 1)), ("MPEG-2 DVD NTSC 16:9", 720, 480, (30000, 1001))] {
        let preset = builtin_presets().into_iter().find(|x| x.name == name).unwrap();
        let path = tmp(&format!("{w}x{h}.mpg"));
        let s = ExportSettings { path: path.clone(), ..preset.settings };
        export(&p, seq, &s, &m, &Progress::default()).unwrap();
        let (es, _, f, _) = demux(&path);
        assert!(f.find(cfa4_mpegts::Kind::Audio, |c| *c == cfa4_mpegts::Codec::LpcmDvd).is_some());
        let pics = decode(&es);
        let info = &pics[0].info;
        assert_eq!((info.width, info.height, info.dar, info.frame_rate), (w, h, Some((16, 9)), Some(rate)), "{name}");
        // Main Profile @ Main Level
        assert_eq!(info.profile_and_level, Some(0x48), "{name}");
    }
}

#[test]
fn mpeg2_settings() {
    assert_eq!(Format::from_name("mpeg2"), Some(Format::Mpeg2));
    assert_eq!(Format::from_name("m2ts"), Some(Format::Mpeg2));
    let mut s = settings("a/b.TS");
    assert_eq!(Mpeg2Container::of(&s), Mpeg2Container::Ts);
    assert_eq!(s.audio_codec(), settings::AudioCodec::Aac);
    s.mpeg2_container = "ps".into();
    assert_eq!(s.audio_codec(), settings::AudioCodec::Pcm);
    assert_eq!(s.extension(), "mpg");
    s.validate().unwrap();
    s.audio.bits = 24;
    assert!(s.validate().unwrap_err().to_string().contains("16-bit"));
    s.audio.bits = 16;
    s.audio.sample_rate = Some(22_050);
    assert!(s.validate().is_err());
    s.audio.sample_rate = Some(48_000);
    s.bitrate_mode = settings::BitrateMode::Crf;
    assert!(s.validate().is_err());
    s.bitrate_mode = settings::BitrateMode::Vbr1Pass;
    s.mpeg2_container = "avi".into();
    assert!(s.validate().is_err());
    assert_eq!(Mpeg2Container::of(&settings("noext")), Mpeg2Container::Ps);
    for c in Mpeg2Container::ALL {
        assert_eq!(Mpeg2Container::from_name(c.id()), Some(c));
        assert_eq!(Mpeg2Container::from_name(c.extension()), Some(c));
    }
    let sum = settings("x.ts").summary(1920, 1080, FrameRate::new(25, 1), 48_000, Tick(10 * TICKS_PER_SECOND));
    assert_eq!(sum.format, "MPEG-2 (Transport Stream)");
    assert!(sum.video.contains("MPEG-2 Main Profile"), "{}", sum.video);
    assert!(sum.audio.starts_with("AAC"), "{}", sum.audio);
    // 4 Mbps video + 192 kbps audio for 10 s
    assert_eq!(sum.estimated_bytes, (4_192_000 * 10 / 8) as u64);
}

#[test]
fn unsupported_frame_rates_and_sizes_are_errors() {
    let (p, seq, m) = project();
    let mut s = settings(&tmp("rate.mpg"));
    s.frame_rate = Some(FrameRate::new(15, 1));
    let e = export(&p, seq, &s, &m, &Progress::default()).unwrap_err();
    assert!(e.to_string().contains("no 15"), "{e}");
    let mut s = settings(&tmp("size.mpg"));
    s.frame_size = Some((4096, 2160));
    assert!(export(&p, seq, &s, &m, &Progress::default()).is_err());
}

fn ffprobe_json(ffprobe: &Path, path: &str) -> serde_json::Value {
    let out = Command::new(ffprobe).args(["-v", "error", "-show_format", "-show_streams", "-count_frames", "-of", "json", path]).output().unwrap();
    assert!(out.status.success(), "ffprobe {path}: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn ffmpeg_raw(ffmpeg: &Path, path: &str, map: &str, args: &[&str]) -> Vec<u8> {
    let out = Command::new(ffmpeg).args(["-v", "error", "-i", path, "-map", map]).args(args).arg("-").output().unwrap();
    assert!(out.status.success(), "ffmpeg {path}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

#[test]
fn ffmpeg_oracle_reads_every_container() {
    let (Some(ffprobe), Some(ffmpeg)) = (cfa4_testkit::ffprobe_or_skip("MPEG-2 export probe"), cfa4_testkit::ffmpeg_or_skip("MPEG-2 export decode")) else {
        return;
    };
    let (p, seq, m) = project();
    let pcm = wav_pcm(&p, seq, &m);
    for (name, format, audio) in
        [("o.mpg", "mpeg", Some("pcm_dvd")), ("o.ts", "mpegts", Some("aac")), ("o.m2ts", "mpegts", Some("aac")), ("o.m2v", "mpegvideo", None)]
    {
        let path = tmp(name);
        export(&p, seq, &settings(&path), &m, &Progress::default()).unwrap();
        let j = ffprobe_json(&ffprobe, &path);
        assert_eq!(j["format"]["format_name"], format, "{name}");
        let streams = j["streams"].as_array().unwrap();
        let v = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
        assert_eq!(v["codec_name"], "mpeg2video", "{name}");
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(320), Some(180)));
        assert_eq!(v["nb_read_frames"], "25", "{name}");
        assert_eq!(v["r_frame_rate"], "25/1");
        assert_eq!(v["color_primaries"], "bt709", "{name}");
        let a = streams.iter().find(|s| s["codec_type"] == "audio");
        assert_eq!(a.map(|a| a["codec_name"].as_str().unwrap()), audio, "{name}");
        // ffmpeg's pictures are our decoder's, but for ffmpeg's integer IDCT, whose difference
        // propagates through prediction (the tolerance of cfa4-mpeg2v's oracle: ±4, 58 dB)
        let ours: Vec<u8> = decode(&demux_or_es(&path)).iter().flat_map(|p| [p.y.clone(), p.cb.clone(), p.cr.clone()].concat()).collect();
        let theirs = ffmpeg_raw(&ffmpeg, &path, "0:v", &["-f", "rawvideo", "-pix_fmt", "yuv420p"]);
        assert_eq!(ours.len(), theirs.len(), "{name}");
        let worst = ours.iter().zip(&theirs).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        let se: f64 = ours.iter().zip(&theirs).map(|(a, b)| f64::from(a.abs_diff(*b)).powi(2)).sum();
        let psnr = 10.0 * (255.0f64 * 255.0 / (se / ours.len() as f64).max(1e-12)).log10();
        assert!(worst <= 4 && psnr >= 58.0, "{name}: pictures differ by {worst}, {psnr:.1} dB");
        match audio {
            Some("pcm_dvd") => assert_eq!(ffmpeg_raw(&ffmpeg, &path, "0:a", &["-f", "s16le"]), pcm, "{name}"),
            Some(_) => {
                let s16 = ffmpeg_raw(&ffmpeg, &path, "0:a", &["-f", "s16le", "-ac", "2"]);
                let secs = s16.len() as f64 / 4.0 / 48_000.0;
                assert!((0.95..1.1).contains(&secs), "{name}: {secs} s of AAC");
            }
            None => {}
        }
    }
}

fn demux_or_es(path: &str) -> Vec<u8> {
    if path.ends_with(".m2v") { std::fs::read(path).unwrap() } else { demux(path).0 }
}
