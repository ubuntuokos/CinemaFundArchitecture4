//! Media Cutter K13: the complete stream list (`cfa4_codecs::streams`) against `ffprobe`.
//!
//! Each case muxes a file with ffmpeg (video, several audio streams, subtitles, a timecode track,
//! chapter text, attachments, chapters, languages and titles) into `target/fixtures/streams/` and
//! checks that CFA4 lists the same streams in the same order, with the same type, codec, id,
//! language, title, picture size, channels and sample rate, and the same chapters.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cfa4_codecs::streams::{StreamList, StreamType, stream_list};
use serde_json::Value;

struct Case {
    /// The fixture's file name (its extension picks ffmpeg's muxer).
    file: &'static str,
    /// Encoders the case needs; a missing one skips the case.
    encoders: &'static [&'static str],
    /// ffmpeg arguments between the global options and the output (`{srt}`, `{chapters}` and
    /// `{font}` name the side files).
    args: &'static str,
}

/// Encoders a case may lack and still pass when oracles are required: the CI ffmpeg is built
/// without them (`scripts/ci/install-ffmpeg-9.0.2.sh`), and another case covers the container.
const OPTIONAL_ENCODERS: &[&str] = &["libtheora"];

const V: &str = "-f lavfi -i testsrc2=size=320x240:rate=25:duration=1";

fn cases() -> Vec<Case> {
    vec![
        Case {
            file: "full.mp4",
            encoders: &["libx264", "aac", "mov_text"],
            args: "{V} {A48} {A44} -i {srt} -i {chapters} -map 0 -map 1 -map 2 -map 3 -map_chapters 4 -c:v libx264 -c:a aac -ac:a:0 2 -c:s mov_text \
                   -metadata:s:a:0 language=hun -metadata:s:a:1 language=eng -metadata:s:s:0 language=ger -timecode 01:00:00:00",
        },
        Case {
            file: "full.mov",
            encoders: &["mpeg4", "pcm_s24le", "mov_text"],
            args: "{V} {A48} -i {srt} -i {chapters} -map 0 -map 1 -map 2 -map_chapters 3 -c:v mpeg4 -c:a pcm_s24le -c:s mov_text \
                   -metadata:s:a:0 language=hun -timecode 10:00:00:00",
        },
        Case {
            file: "full.mkv",
            encoders: &["libx264", "flac", "srt", "ass", "ac3"],
            args: "{V} {A48} {A44} -i {srt} -i {chapters} -map 0 -map 1 -map 2 -map 3 -map 3 -map_chapters 4 -c:v libx264 -c:a:0 ac3 -ac:a:0 2 -c:a:1 flac \
                   -c:s:0 srt -c:s:1 ass -metadata:s:a:0 language=hun -metadata:s:s:0 title=Magyar -metadata:s:s:1 language=fre \
                   -disposition:s:1 forced -attach {font} -metadata:s:t mimetype=application/x-truetype-font",
        },
        Case {
            file: "full.webm",
            encoders: &["libvpx-vp9", "libopus"],
            args: "{V} {A48} -map 0 -map 1 -c:v libvpx-vp9 -c:a libopus -metadata:s:a:0 language=hun",
        },
        Case {
            file: "full.ts",
            encoders: &["mpeg2video", "mp2", "ac3"],
            args: "{V} {A48} {A44} -map 0 -map 1 -map 2 -c:v mpeg2video -c:a:0 mp2 -c:a:1 ac3 -ac:a:1 2 -metadata:s:a:0 language=hun",
        },
        Case { file: "h264.ts", encoders: &["libx264", "aac"], args: "{V} {A48} -map 0 -map 1 -c:v libx264 -c:a aac -ac 2" },
        Case { file: "full.mpg", encoders: &["mpeg2video", "mp2"], args: "{V} {A48} -map 0 -map 1 -c:v mpeg2video -c:a mp2 -f vob" },
        Case {
            file: "full.avi",
            encoders: &["mjpeg", "pcm_s16le"],
            args: "{V} {A48} -map 0 -map 1 -c:v mjpeg -c:a pcm_s16le -ac 2 -metadata:s:a:0 title=Hang",
        },
        // ffmpeg's own Vorbis and FLAC encoders, so CI's ffmpeg (no libvorbis) makes it too
        Case { file: "audio.ogg", encoders: &["vorbis", "flac"], args: "{A44} {A48} -map 0 -map 1 -c:a:0 vorbis -strict -2 -ac:a:0 2 -c:a:1 flac" },
        Case { file: "theora.ogg", encoders: &["libtheora"], args: "{V} -c:v libtheora" },
        Case { file: "full.opus", encoders: &["libopus"], args: "{A48} -c:a libopus -ac 2" },
        Case { file: "flac.oga", encoders: &["flac"], args: "{A44} -c:a flac -ac 2 -f ogg" },
        Case {
            file: "full.mxf",
            encoders: &["mpeg2video", "pcm_s16le"],
            args: "-f lavfi -i testsrc2=size=720x576:rate=25:duration=1 {A48} -map 0 -map 1 -map 1 -c:v mpeg2video -pix_fmt yuv422p -c:a pcm_s16le -timecode 01:00:00:00",
        },
        Case { file: "full.wav", encoders: &["pcm_s24le"], args: "{A44} -c:a pcm_s24le -ac 2" },
        Case { file: "full.flac", encoders: &["flac"], args: "{A48} -c:a flac" },
    ]
}

fn side_files(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let srt = dir.join("sub.srt");
    let chapters = dir.join("chapters.txt");
    let font = dir.join("font.ttf");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:00,500\nHello\n\n2\n00:00:00,600 --> 00:00:00,900\nWorld\n").expect("write srt");
    std::fs::write(
        &chapters,
        ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=400\ntitle=Intro\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=400\nEND=1000\ntitle=Fő rész\n",
    )
    .expect("write chapters");
    std::fs::write(&font, b"\x00\x01\x00\x00not really a font").expect("write font");
    (srt, chapters, font)
}

fn make(ff: &Path, case: &Case, dir: &Path) -> Option<PathBuf> {
    let (srt, chapters, font) = side_files(dir);
    let args = case
        .args
        .replace("{V}", V)
        .replace("{A48}", "-f lavfi -i sine=frequency=440:sample_rate=48000:duration=1")
        .replace("{A44}", "-f lavfi -i sine=frequency=880:sample_rate=44100:duration=1");
    let out = dir.join(case.file);
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        let mut c = Command::new(ff);
        c.args(["-hide_banner", "-v", "error", "-y"]);
        for a in args.split_whitespace() {
            match a {
                "{srt}" => c.arg(&srt),
                "{chapters}" => c.arg(&chapters),
                "{font}" => c.arg(&font),
                a => c.arg(a),
            };
        }
        let o = c.arg(tmp).output().expect("run ffmpeg");
        if !o.status.success() {
            eprintln!("ffmpeg failed for {}: {}", case.file, String::from_utf8_lossy(&o.stderr));
        }
        o.status.success()
    })
}

fn ffprobe_json(fp: &Path, file: &Path) -> Value {
    let o = Command::new(fp).args(["-v", "error", "-show_streams", "-show_chapters", "-of", "json"]).arg(file).output().expect("run ffprobe");
    assert!(o.status.success(), "ffprobe failed on {file:?}: {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("ffprobe json")
}

fn cfa4_list(file: &Path) -> StreamList {
    let bytes: Arc<[u8]> = std::fs::read(file).expect("read fixture").into();
    let reader = cfa4_media::reader::MemReader(bytes);
    stream_list(&reader).unwrap_or_else(|| panic!("{file:?}: container not listed")).unwrap_or_else(|e| panic!("{file:?}: {e}"))
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn tag<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get("tags").and_then(|t| t.get(key)).and_then(Value::as_str)
}

fn language(l: Option<&str>) -> Option<&str> {
    l.filter(|l| *l != "und")
}

fn seconds(v: &Value, key: &str) -> Option<f64> {
    text(v, key).and_then(|s| s.parse().ok())
}

/// Every difference between the two lists (empty when they match).
fn compare(name: &str, probe: &Value, ours: &StreamList) -> Vec<String> {
    let mut diffs = Vec::new();
    let mut check = |what: String, theirs: Option<String>, mine: Option<String>| {
        if theirs != mine {
            diffs.push(format!("{name}: {what}: ffprobe {theirs:?}, CFA4 {mine:?}"));
        }
    };
    let streams = probe["streams"].as_array().cloned().unwrap_or_default();
    check("stream count".into(), Some(streams.len().to_string()), Some(ours.streams.len().to_string()));
    for (p, s) in streams.iter().zip(&ours.streams) {
        let i = s.index;
        check(format!("#{i} index"), p["index"].as_u64().map(|x| x.to_string()), Some(i.to_string()));
        let kind = match s.kind {
            StreamType::Video => "video",
            StreamType::Audio => "audio",
            StreamType::Subtitle => "subtitle",
            StreamType::Data => "data",
            StreamType::Attachment => "attachment",
        };
        check(format!("#{i} type"), text(p, "codec_type").map(str::to_string), Some(kind.into()));
        match text(p, "codec_name") {
            // ffmpeg has no codec for it: its FourCC must agree, when it has a printable one
            None | Some("unknown") => {
                if let Some(t) = text(p, "codec_tag_string").filter(|t| !t.starts_with('[')) {
                    check(format!("#{i} codec tag"), Some(t.into()), s.codec_tag.clone());
                }
            }
            Some(c) => check(format!("#{i} codec"), Some(c.into()), s.codec.clone()),
        }
        if let Some(id) = text(p, "id") {
            check(format!("#{i} id"), Some(id.into()), s.id.clone());
        }
        check(format!("#{i} language"), language(tag(p, "language")).map(str::to_string), s.language.clone());
        if s.kind != StreamType::Attachment {
            check(format!("#{i} title"), tag(p, "title").map(str::to_string), s.title.clone());
        }
        if s.kind == StreamType::Video {
            check(format!("#{i} width"), p["width"].as_u64().map(|x| x.to_string()), s.width.map(|x| x.to_string()));
            check(format!("#{i} height"), p["height"].as_u64().map(|x| x.to_string()), s.height.map(|x| x.to_string()));
        }
        if s.kind == StreamType::Audio {
            check(format!("#{i} channels"), p["channels"].as_u64().map(|x| x.to_string()), s.channels.map(|x| x.to_string()));
            check(format!("#{i} sample rate"), text(p, "sample_rate").map(str::to_string), s.sample_rate.map(|x| x.to_string()));
        }
        if s.kind == StreamType::Data && s.timecode.is_some() {
            check(format!("#{i} timecode"), tag(p, "timecode").map(str::to_string), s.timecode.clone());
        }
        if s.kind == StreamType::Attachment {
            check(format!("#{i} filename"), tag(p, "filename").map(str::to_string), s.filename.clone());
            check(format!("#{i} mimetype"), tag(p, "mimetype").map(str::to_string), s.mimetype.clone());
        }
        if ours.container == "matroska" && s.kind != StreamType::Attachment {
            let flag = |k: &str| p["disposition"][k].as_u64().map(|x| (x == 1).to_string());
            check(format!("#{i} default"), flag("default"), s.default.map(|x| x.to_string()));
            check(format!("#{i} forced"), flag("forced"), s.forced.map(|x| x.to_string()));
        }
    }
    let chapters = probe["chapters"].as_array().cloned().unwrap_or_default();
    check("chapter count".into(), Some(chapters.len().to_string()), Some(ours.chapters.len().to_string()));
    let secs = |t: cfa4_time::Tick| t.0 as f64 / cfa4_time::TICKS_PER_SECOND as f64;
    for (n, (p, c)) in chapters.iter().zip(&ours.chapters).enumerate() {
        check(format!("chapter {n} title"), tag(p, "title").map(str::to_string), Some(c.title.clone()));
        let near = |a: Option<f64>, b: Option<f64>| matches!((a, b), (Some(a), Some(b)) if (a - b).abs() < 0.002);
        if !near(seconds(p, "start_time"), Some(secs(c.start))) {
            check(format!("chapter {n} start"), text(p, "start_time").map(str::to_string), Some(format!("{:.6}", secs(c.start))));
        }
        if !near(seconds(p, "end_time"), c.end.map(secs)) {
            check(format!("chapter {n} end"), text(p, "end_time").map(str::to_string), c.end.map(|e| format!("{:.6}", secs(e))));
        }
    }
    diffs
}

#[test]
fn stream_list_matches_ffprobe() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("stream list oracle") else { return };
    let Some(fp) = cfa4_testkit::ffprobe_or_skip("stream list oracle") else { return };
    let dir = cfa4_testkit::fixtures_dir("streams");
    let mut diffs = Vec::new();
    let mut checked = 0;
    for case in cases() {
        if let Some(missing) = case.encoders.iter().find(|e| !cfa4_testkit::ffmpeg_has_encoder(&ff, e)) {
            let context = format!("stream list oracle, {}", case.file);
            if OPTIONAL_ENCODERS.contains(missing) {
                eprintln!("SKIPPED ({context}): ffmpeg has no {missing} encoder (optional case)");
            } else {
                cfa4_testkit::skip(&context, &format!("ffmpeg has no {missing} encoder"));
            }
            continue;
        }
        let Some(file) = make(&ff, &case, &dir) else {
            diffs.push(format!("{}: ffmpeg could not make the fixture", case.file));
            continue;
        };
        diffs.extend(compare(case.file, &ffprobe_json(&fp, &file), &cfa4_list(&file)));
        checked += 1;
    }
    assert!(diffs.is_empty(), "{} difference(s) from ffprobe:\n{}", diffs.len(), diffs.join("\n"));
    eprintln!("stream lists of {checked} files match ffprobe");
}
