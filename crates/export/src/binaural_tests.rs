//! Binaural stereo export (B-05): the Mix through the headphone monitor, sample for sample the
//! Program monitor's sound, and the file says which HRTF set made it (WAV `LIST/INFO` `ICMT`,
//! FLAC `COMMENT`, MP4 `©cmt`). ffmpeg / ffprobe are external test oracles only (skipped when
//! absent).

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource};
use cfa4_project::{AudioChannels, ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::TICKS_PER_SECOND;

use super::*;

const SR: u32 = 48_000;
const HZ: [f64; 6] = [500.0, 700.0, 900.0, 60.0, 1300.0, 1700.0];

struct Tones6 {
    info: MediaInfo,
}

fn tone(c: usize, i: i64) -> f32 {
    (0.2 * (2.0 * std::f64::consts::PI * HZ[c] * i as f64 / SR as f64).sin()) as f32
}

impl MediaSource for Tones6 {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _: FrameRequest) -> cfa4_media::Result<Arc<VideoFrame>> {
        Err(MediaError::Unsupported("audio only".into()))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> cfa4_media::Result<AudioBuffer> {
        Ok(AudioBuffer { sample_rate, channels: (0..6).map(|c| (0..frames).map(|i| tone(c, start + i as i64)).collect()).collect() })
    }
}

/// A sequence with a 5.1 Mix and one 5.1 track holding 1 s of six tones.
fn surround() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("binaural");
    let info = MediaInfo {
        name: "tones51".into(),
        kind: MediaKind::AudioOnly,
        duration: Tick(2 * TICKS_PER_SECOND),
        video: None,
        audio_streams: vec![AudioStreamInfo { sample_rate: SR, channels: 6, codec: "test".into(), bits_per_sample: Some(32), speakers: Vec::new() }],
        container: "test".into(),
        start_timecode: None,
        file_size: None,
    };
    let id = p.add_item(
        "tones51",
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: "/tones51.wav".into() },
            info: info.clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    let r = FrameRate::FPS_24;
    let mut st = SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() };
    st.audio_master = AudioChannels::Surround51;
    let seq = p.new_sequence("s", st, 1, 1, None);
    let ai = p.make_track_item(id, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(24)), r).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.audio_tracks[0].channels = AudioChannels::Surround51;
    q.audio_tracks[0].items.push(ai);
    let mut m = SourceMap::default();
    m.0.insert(id, Arc::new(Tones6 { info }) as Arc<dyn MediaSource>);
    (Arc::new(p), seq, m)
}

fn out_path(name: &str) -> String {
    let d = cfa4_testkit::fixtures::workspace_root().join("target").join("export-tests").join(format!("binaural-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().to_string()
}

fn settings(format: Format, path: &str) -> ExportSettings {
    let mut s = ExportSettings { format, path: path.to_string(), ..Default::default() };
    s.audio.channels = 6; // binaural overrides it: two ears
    s.audio.bits = 24;
    s.audio.sample_rate = Some(SR);
    s.audio.binaural = Some(BinauralAudio::default());
    s
}

/// The reference: the whole range's Mix through one `ChannelMonitor` call.
fn reference(p: &Project, seq: ItemId, m: &SourceMap, frames: usize) -> (Vec<f32>, Vec<f32>, String) {
    let q = p.sequence(seq).unwrap();
    let mix = cfa4_render::mixer::mix_graph(p, q, 0, frames, m, None);
    assert_eq!(mix.channels.len(), 6, "the Mix is 5.1");
    let set = Arc::new(cfa4_hrtf::HrtfSet::builtin(SR).unwrap());
    let mut mon = cfa4_hrtf::ChannelMonitor::new(set.clone(), cfa4_hrtf::MonitorConfig::default()).unwrap();
    let refs: Vec<&[f32]> = mix.channels.iter().map(Vec::as_slice).collect();
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    mon.process(0, &refs, &mut l, &mut r).unwrap();
    (l, r, set.name().to_owned())
}

fn run(p: &Arc<Project>, seq: ItemId, m: &SourceMap, s: &ExportSettings) {
    export(p, seq, s, m, &Progress::default()).unwrap();
}

#[test]
fn the_audio_path_is_the_monitor_sound_in_two_channels() {
    let (p, seq, m) = surround();
    let s = settings(Format::Wav, &out_path("unused.wav"));
    let range = export_range(&p, seq, &s).unwrap();
    let mut a = audio_out::AudioOut::new(p.clone(), seq, &s, range).unwrap();
    assert_eq!(a.channels, 2, "binaural is two channels whatever `channels` says");
    let frames = a.remaining() as usize;
    // in uneven pieces: the renderer keeps its history from block to block
    let mut got: Vec<Vec<f32>> = vec![Vec::new(), Vec::new()];
    for step in [1000, 4096, 777, 48_000].iter().cycle() {
        let Some(b) = a.pull(a.out + step, &m) else { break };
        got[0].extend(&b[0]);
        got[1].extend(&b[1]);
    }
    let (l, r, _) = reference(&p, seq, &m, frames);
    assert_eq!(got[0].len(), frames);
    assert!(l.iter().any(|s| s.abs() > 0.01), "the reference is silent");
    for (ear, want) in got.iter().zip([&l, &r]) {
        for (i, (g, w)) in ear.iter().zip(want.iter()).enumerate() {
            assert!((g - w).abs() <= 1e-6, "sample {i}: {g} vs {w}");
        }
    }
}

#[test]
fn a_binaural_wav_says_so_and_holds_the_monitor_sound() {
    let (p, seq, m) = surround();
    let path = out_path("binaural.wav");
    let s = settings(Format::Wav, &path);
    run(&p, seq, &m, &s);
    let bytes = std::fs::read(&path).unwrap();
    // RIFF size counts everything, the LIST/INFO is the last chunk
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
    let (l, r, name) = reference(&p, seq, &m, 48_000);
    let list = bytes.windows(4).rposition(|w| w == b"LIST").expect("a LIST chunk");
    assert_eq!(&bytes[list + 8..list + 16], b"INFOICMT");
    let text = String::from_utf8_lossy(&bytes[list + 20..]).trim_end_matches('\0').to_owned();
    assert_eq!(text, format!("binaural, HRTF: {name}"));
    // the samples: 24-bit stereo after the fmt chunk
    let data = bytes.windows(4).position(|w| w == b"data").unwrap();
    let n = u32::from_le_bytes(bytes[data + 4..data + 8].try_into().unwrap()) as usize;
    assert_eq!(n, 48_000 * 2 * 3);
    let pcm = &bytes[data + 8..data + 8 + n];
    let q = |s: f32| SampleFormat::Int24.quantise(s);
    for (i, f) in pcm.chunks_exact(6).enumerate() {
        let v = |b: &[u8]| i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
        assert!((v(&f[..3]) - q(l[i])).abs() <= 1 && (v(&f[3..]) - q(r[i])).abs() <= 1, "frame {i}");
    }
    if let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("binaural WAV") {
        let out =
            std::process::Command::new(ffprobe).args(["-v", "error", "-show_entries", "stream=channels:format_tags", "-of", "json", &path]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(j["streams"][0]["channels"], 2);
        assert_eq!(j["format"]["tags"]["comment"], format!("binaural, HRTF: {name}"), "{j}");
    }
}

#[test]
fn a_binaural_flac_says_so() {
    let (p, seq, m) = surround();
    let path = out_path("binaural.flac");
    run(&p, seq, &m, &settings(Format::Flac, &path));
    let bytes = std::fs::read(&path).unwrap();
    let (.., name) = reference(&p, seq, &m, 16);
    let head = String::from_utf8_lossy(&bytes[..512]).into_owned();
    assert!(head.contains(&format!("COMMENT=binaural, HRTF: {name}")), "{head}");
}

#[test]
fn the_mp4_comment_names_the_set_after_the_users_comment() {
    let mut s = settings(Format::H264, "x.mp4");
    let items = binaural::udta(&s);
    let note = items.iter().find(|(k, _)| k == "©cmt").map(|(_, v)| v.clone()).unwrap();
    assert!(note.starts_with("binaural, HRTF: SADIE II"), "{note}");
    assert_eq!(items.last().map(|(k, _)| k.as_str()), Some("©too"));
    s.metadata.comment = "Final mix".into();
    let items = binaural::udta(&s);
    let c = items.iter().find(|(k, _)| k == "©cmt").map(|(_, v)| v.clone()).unwrap();
    assert_eq!(c, format!("Final mix\n{note}"));
    s.audio.binaural = None;
    assert_eq!(binaural::udta(&s), s.metadata.udta());
}

#[test]
fn a_set_that_cannot_be_made_is_an_export_error() {
    let (p, seq, m) = surround();
    let path = out_path("gone.wav");
    let mut s = settings(Format::Wav, &path);
    s.audio.binaural = Some(BinauralAudio { hrtf: "/no/such/set.maudhrtf".into(), ..BinauralAudio::default() });
    let e = export(&p, seq, &s, &m, &Progress::default()).unwrap_err().to_string();
    assert!(e.contains("/no/such/set.maudhrtf"), "{e}");
    s.audio.binaural = Some(BinauralAudio { room: 3.0, ..BinauralAudio::default() });
    assert!(export(&p, seq, &s, &m, &Progress::default()).unwrap_err().to_string().contains("room"));
    s.audio.binaural = Some(BinauralAudio { hrtf: "x.wav".into(), ..BinauralAudio::default() });
    assert!(export(&p, seq, &s, &m, &Progress::default()).unwrap_err().to_string().contains(".maudhrtf"));
}

#[test]
fn binaural_settings_round_trip_and_stay_out_of_older_json() {
    let s = AudioSettings::default();
    assert!(!serde_json::to_string(&s).unwrap().contains("binaural"), "absent when off");
    let on: AudioSettings = serde_json::from_str(r#"{"channels":2,"binaural":{"hrtf":"builtin","room":0.75}}"#).unwrap();
    let b = on.binaural.unwrap();
    assert_eq!((b.hrtf.as_str(), b.room, b.lfe_db), ("builtin", 0.75, cfa4_hrtf::MonitorConfig::default().lfe_db));
}
