use serde_json::json;

use crate::Session;
use cfa4_project::ClipId;
use cfa4_time::{TICKS_PER_SECOND, Tick};

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

#[test]
fn captions_import_edit_export_and_burn_in() {
    let dir = std::env::temp_dir().join(format!("fc-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let srt = dir.join("talk.srt");
    std::fs::write(
        &srt,
        "\u{feff}1\r\n00:00:00,500 --> 00:00:02,000\r\nHello there\r\n\r\n2\r\n00:00:02,500 --> 00:00:04,000\r\nSecond line\r\nwith two rows\r\n",
    )
    .unwrap();
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [srt.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["captions"], 2, "{r}");
    let q = s.active_sequence().unwrap();
    let rate = q.settings.frame_rate;
    let tr = q.caption_tracks[0].clone();
    assert_eq!(tr.name, "talk");
    assert_eq!(tr.captions[0].start, rate.snap_nearest(Tick(TICKS_PER_SECOND / 2)), "snapped to frames");
    let first = tr.captions[0].id;
    let second = tr.captions[1].id;

    // edit text, split, merge, trim, move, add at playhead, delete — each one undoable
    s.execute("captions.setText", json!({"caption": first.0, "text": "Hi!", "speaker": "Ann"})).unwrap();
    let split_at = rate.tick_of(rate.frame_at(tr.captions[1].start) + 10);
    let r = s.execute("captions.split", json!({"caption": second.0, "time": split_at.0})).unwrap();
    let right = ClipId(r["captions"][0].as_u64().unwrap());
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions.len(), 3);
    s.execute("captions.merge", json!({"captions": [second.0, right.0]})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions.len(), 2);
    s.execute("captions.trim", json!({"caption": second.0, "edge": "out", "deltaFrames": 12})).unwrap();
    s.execute("captions.move", json!({"captions": [second.0], "deltaFrames": 24})).unwrap();
    s.execute("playhead.set", json!({"seconds": 8.0})).unwrap();
    let r = s.execute("captions.add", json!({"text": "Added"})).unwrap();
    let added = ClipId(r["caption"].as_u64().unwrap());
    assert_eq!(s.state.caption_selection, vec![added]);
    s.execute("edit.clear", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_caption(added).is_none(), "Clear deletes selected captions");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_caption(added).is_some());
    let list = s.execute("captions.list", json!({})).unwrap();
    assert_eq!(list["tracks"][0]["captions"][0]["text"], "Hi!");
    assert_eq!(list["tracks"][0]["captions"][0]["speaker"], "Ann");
    s.active_sequence().unwrap().check().unwrap();

    // export in all three formats and read back: same texts
    for ext in ["srt", "vtt", "scc"] {
        let path = dir.join(format!("out.{ext}")).to_string_lossy().to_string();
        let r = s.execute("captions.export", json!({"path": path})).unwrap();
        assert_eq!(r["captions"], 3, "{ext}: {r}");
        let back = cfa4_captions::parse(&std::fs::read(&path).unwrap(), cfa4_captions::Format::from_name(ext).unwrap()).unwrap();
        let texts: Vec<String> = back.cues.iter().map(|c| cfa4_project::plain_text(&c.text)).collect();
        assert_eq!(texts.len(), 3, "{ext}");
        assert_eq!(texts[0], "Hi!", "{ext}");
    }

    // the program renders captions; hiding the track removes them
    s.execute("playhead.set", json!({"seconds": 1.0})).unwrap();
    let with = s.render_program(0.25).unwrap();
    s.execute("captions.hideAll", json!({})).unwrap();
    let without = s.render_program(0.25).unwrap();
    let diff = with.px.iter().zip(&without.px).filter(|(a, b)| (*a - *b).abs() > 0.05).count();
    assert!(diff > 200, "captions change the picture ({diff} samples)");
    let half = with.w * 4 * (with.h / 2);
    let top_diff = with.px[..half].iter().zip(&without.px[..half]).filter(|(a, b)| (*a - *b).abs() > 0.05).count();
    assert_eq!(top_diff, 0, "bottom-anchored captions stay in the lower half");
    s.execute("captions.showAll", json!({})).unwrap();
    let plan = cfa4_render::plan::plan_frame(
        &s.project,
        s.state.active_sequence.unwrap(),
        s.playhead(),
        cfa4_render::RenderOptions { scale: 0.25, captions: true, ..Default::default() },
        &s.media.provider(s.project.clone(), s.services.clone()),
    );
    let cpu = cfa4_render::plan::execute_cpu(&plan);
    let pd = cpu.px.iter().zip(&with.px).filter(|(a, b)| (*a - *b).abs() > 0.02).count();
    assert!(pd < cpu.px.len() / 200, "the GPU plan matches the CPU render ({pd})");

    // save/open keeps caption tracks
    let proj = dir.join("cap.fcproj").to_string_lossy().to_string();
    s.execute("file.saveAs", json!({"path": proj})).unwrap();
    let before = s.active_sequence().unwrap().caption_tracks.clone();
    s.execute("file.open", json!({"path": proj})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks, before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn caption_track_follows_sync_locked_insert() {
    let mut s = demo();
    s.execute("captions.newTrack", json!({"format": "CEA-608"})).unwrap();
    s.execute("playhead.set", json!({"seconds": 4.0})).unwrap();
    s.execute("captions.add", json!({"text": "later"})).unwrap();
    let start = s.active_sequence().unwrap().caption_tracks[0].captions[0].start;
    let item = s.project.items.values().find(|i| i.as_media().is_some_and(|m| m.info.has_video())).unwrap().id;
    s.execute("source.open", json!({"item": item.0})).unwrap();
    s.execute("playhead.set", json!({"seconds": 0.0})).unwrap();
    s.execute("source.insert", json!({})).unwrap();
    let after = s.active_sequence().unwrap().caption_tracks[0].captions[0].start;
    assert!(after > start, "caption moved right with the insert");
    s.active_sequence().unwrap().check().unwrap();
}

#[test]
fn caption_commands_need_tracks_and_navigate() {
    let mut s = demo();
    assert!(s.execute("captions.split", json!({})).is_err(), "disabled without captions");
    for (sec, text) in [(1.0, "one"), (3.0, "two"), (6.0, "three")] {
        s.execute("playhead.set", json!({"seconds": sec})).unwrap();
        s.execute("captions.add", json!({"text": text, "durationSeconds": 1.0})).unwrap();
    }
    s.execute("playhead.set", json!({"seconds": 0.0})).unwrap();
    s.execute("captions.next", json!({})).unwrap();
    s.execute("captions.next", json!({})).unwrap();
    let id = s.state.caption_selection[0];
    assert_eq!(s.active_sequence().unwrap().find_caption(id).unwrap().1.text, "two");
    s.execute("captions.previous", json!({})).unwrap();
    let id = s.state.caption_selection[0];
    assert_eq!(s.active_sequence().unwrap().find_caption(id).unwrap().1.text, "one");
    s.execute("captions.setStyle", json!({"color": "#ffff00", "anchor": "top", "size": 40})).unwrap();
    let st = &s.active_sequence().unwrap().caption_tracks[0].style;
    assert_eq!(st.color, [255, 255, 0, 255]);
    assert!(s.execute("captions.setStyle", json!({"anchor": "sideways"})).is_err());
    s.execute("captions.setTimes", json!({"caption": id.0, "startSeconds": 0.5, "endSeconds": 2.5})).unwrap();
    assert!(s.execute("captions.setTimes", json!({"caption": id.0, "endSeconds": 3.5})).is_err(), "would overlap `two`");
    s.execute("captions.deleteTrack", json!({"track": "C1"})).unwrap();
    assert!(s.active_sequence().unwrap().caption_tracks.is_empty());
}

#[test]
fn caption_list_track_filter_never_falls_back_to_every_track() {
    let mut s = demo();
    s.execute("captions.newTrack", json!({})).unwrap();
    s.execute("captions.newTrack", json!({})).unwrap();
    let all = s.execute("captions.list", json!({})).unwrap();
    let ids: Vec<u64> = all["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2, "{all}");
    for (i, id) in ids.iter().enumerate() {
        for sel in [json!(id), json!(format!("C{}", i + 1))] {
            let r = s.execute("captions.list", json!({"track": sel})).unwrap();
            let got: Vec<u64> = r["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_u64().unwrap()).collect();
            assert_eq!(got, vec![*id], "{sel}");
        }
    }
    let missing = ids.iter().max().unwrap() + 999;
    for sel in [json!(missing), json!("C3"), json!("C0"), json!("nope")] {
        assert!(s.execute("captions.list", json!({"track": sel})).is_err(), "{sel} names no caption track");
    }
}

fn caption_spans(s: &Session) -> Vec<(f64, f64, String)> {
    let secs = |t: Tick| (t.seconds() * 10.0).round() / 10.0; // the demo sequence snaps to 23.976 fps frames
    s.active_sequence().unwrap().caption_tracks[0].captions.iter().map(|c| (secs(c.start), secs(c.end()), c.text.clone())).collect()
}

#[test]
fn captions_duplicate_copies_and_overwrites() {
    let mut s = demo();
    for (sec, text) in [(1.0, "one"), (3.0, "two")] {
        s.execute("playhead.set", json!({"seconds": sec})).unwrap();
        s.execute("captions.add", json!({"text": text, "durationSeconds": 1.0})).unwrap();
    }
    let one = s.active_sequence().unwrap().caption_tracks[0].captions[0].id;
    // Option-drag: a copy 5 s later, the original stays; the copy is selected
    let r = s.execute("captions.duplicate", json!({"captions": [one.0], "delta": 5 * TICKS_PER_SECOND})).unwrap();
    assert_eq!(caption_spans(&s), vec![(1.0, 2.0, "one".into()), (3.0, 4.0, "two".into()), (6.0, 7.0, "one".into())]);
    assert_eq!(s.state.caption_selection.len(), 1);
    assert_eq!(r["captions"][0].as_u64(), Some(s.state.caption_selection[0].0));
    // a copy dropped half over `two` overwrites that half
    s.execute("captions.duplicate", json!({"captions": [one.0], "delta": TICKS_PER_SECOND * 3 / 2})).unwrap();
    assert_eq!(caption_spans(&s)[1..3], [(2.5, 3.5, "one".into()), (3.5, 4.0, "two".into())]);
    // never before zero; zero delta is nothing
    s.execute("captions.duplicate", json!({"captions": [one.0], "delta": -100 * TICKS_PER_SECOND})).unwrap();
    assert_eq!(caption_spans(&s)[0], (0.0, 1.0, "one".into()));
    assert!(s.execute("captions.duplicate", json!({"captions": [one.0], "delta": 0})).is_err());
    assert!(s.execute("captions.duplicate", json!({"captions": [one.0]})).is_err(), "needs a delta");
    assert!(s.execute("captions.duplicate", json!({"captions": [], "delta": TICKS_PER_SECOND})).is_err(), "needs a caption");
    let before = s.active_sequence().unwrap().caption_tracks[0].captions.len();
    s.execute("captions.duplicate", json!({"captions": [one.0, one.0], "delta": 20 * TICKS_PER_SECOND})).unwrap();
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].captions.len(), before + 1, "a caption named twice is copied once");
    s.active_sequence().unwrap().check().unwrap();
}

#[test]
fn captions_copy_cut_and_paste() {
    let mut s = demo();
    // Copy / Paste with only captions selected copies captions, pasted at the playhead
    s.execute("playhead.set", json!({"seconds": 1.0})).unwrap();
    s.execute("captions.add", json!({"text": "hi", "durationSeconds": 1.0})).unwrap();
    assert!(s.state.selection.is_empty());
    assert_eq!(s.execute("edit.copy", json!({})).unwrap()["copied"], 1);
    s.execute("playhead.set", json!({"seconds": 10.0})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(caption_spans(&s), vec![(1.0, 2.0, "hi".into()), (10.0, 11.0, "hi".into())]);
    assert!((s.playhead().seconds() - 11.0).abs() < 0.05, "the playhead moves past the paste");
    // Cut takes it away and Paste puts it back
    s.execute("edit.cut", json!({})).unwrap();
    assert_eq!(caption_spans(&s), vec![(1.0, 2.0, "hi".into())]);
    s.execute("playhead.set", json!({"seconds": 4.0})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    assert_eq!(caption_spans(&s)[1], (4.0, 5.0, "hi".into()));
    // copying clips afterwards makes Paste place clips again, not captions
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
    s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.state.caption_clipboard.is_empty() && !s.state.clipboard.is_empty());
    // undo takes back a caption paste as one step
    let mut s = demo();
    s.execute("captions.add", json!({"text": "x", "durationSeconds": 1.0})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("playhead.set", json!({"seconds": 5.0})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(caption_spans(&s).len(), 1);
}

const DCP_REEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <Id>urn:uuid:7c5bd5b8-3b0f-4a3b-9b1e-1c2d3e4f5a6b</Id>
  <ContentTitleText>Reel</ContentTitleText>
  <Language>hu</Language>
  <EditRate>24 1</EditRate>
  <TimeCodeRate>24</TimeCodeRate>
  <StartTime>01:00:00:00</StartTime>
  <SubtitleList>
    <Font Italic="no">
      <Subtitle SpotNumber="1" TimeIn="01:00:01:00" TimeOut="01:00:03:00"><Text Valign="bottom" Vposition="10">Jó estét!</Text></Subtitle>
      <Subtitle SpotNumber="2" TimeIn="01:00:04:00" TimeOut="01:00:06:00"><Text Valign="bottom" Vposition="10"><Font Italic="yes">Második</Font></Text></Subtitle>
    </Font>
  </SubtitleList>
</SubtitleReel>
"#;

#[test]
fn dcp_timed_text_mxf_and_dcst_xml_import_as_caption_tracks() {
    let dir = std::env::temp_dir().join(format!("fc-dcst-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mxf = dir.join("sub_reel1.mxf");
    std::fs::write(&mxf, cfa4_mxf::testing::build_timed_text(DCP_REEL.as_bytes(), &[("application/x-font-opentype", b"font")])).unwrap();
    let xml = dir.join("reel2.xml");
    std::fs::write(&xml, DCP_REEL).unwrap();
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [mxf.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["captions"], 2, "{r}");
    assert_eq!(r["documents"][0]["format"], "D-Cinema subtitles (ST 428-7)");
    let q = s.active_sequence().unwrap();
    let tr = &q.caption_tracks[0];
    assert_eq!(tr.name, "sub_reel1 (hu)");
    assert_eq!(tr.captions[0].start, q.settings.frame_rate.snap_nearest(Tick(TICKS_PER_SECOND)), "one second after StartTime");
    assert_eq!(tr.captions[0].text, "Jó estét!");
    assert_eq!(tr.captions[1].text, "<i>Második</i>");
    let r = s.execute("file.import", json!({"paths": [xml.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["captions"], 2, "{r}");
    assert_eq!(s.active_sequence().unwrap().caption_tracks[0].name, "reel2");
    // a damaged timed text file is an error, not a crash or a media item
    let mut bad = std::fs::read(&mxf).unwrap();
    let at = bad.windows(9).position(|w| w == b"<Subtitle").unwrap();
    bad[at + 1] = b'!';
    let broken = dir.join("broken.mxf");
    std::fs::write(&broken, bad).unwrap();
    assert!(s.execute("file.import", json!({"paths": [broken.to_string_lossy()]})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// #929: closed captions in an MXF file's ANC data (ST 436-1 elements carrying ST 334-2 CDPs)
/// import as a caption track with the same cues as the MCC file of those CDPs.
#[test]
fn mxf_anc_closed_captions_import_as_a_caption_track() {
    let doc = cfa4_captions::Document {
        cues: vec![
            cfa4_captions::Cue { start: Tick(TICKS_PER_SECOND), end: Tick(3 * TICKS_PER_SECOND), text: "Hello ANC".into(), ..Default::default() },
            cfa4_captions::Cue { start: Tick(4 * TICKS_PER_SECOND), end: Tick(6 * TICKS_PER_SECOND), text: "Second line".into(), ..Default::default() },
        ],
        ..Default::default()
    };
    let mcc = cfa4_captions::mcc::write_with(&doc, false, true, true);
    let expected = cfa4_captions::mcc::parse(&mcc).unwrap();
    // the MCC file's CDPs, frame by frame (non-drop 30 fps labels on the 29.97 grid)
    let mut frames: Vec<Vec<u8>> = Vec::new();
    for l in mcc.lines().filter(|l| !l.starts_with("//") && !l.contains('=')) {
        let Some((tc, data)) = l.split_once('\t') else { continue };
        let p: Vec<usize> = tc.split([':', ';']).map(|x| x.parse().unwrap()).collect();
        let frame = ((p[0] * 60 + p[1]) * 60 + p[2]) * 30 + p[3];
        frames.resize(frame + 1, cfa4_mxf::testing::anc_element(&[], false));
        frames[frame] = cfa4_mxf::testing::anc_element(&[cfa4_captions::mcc::expand(data).unwrap()], false);
    }
    assert!(frames.len() > 150, "{} frames", frames.len());
    let dir = std::env::temp_dir().join(format!("fc-anc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mxf = dir.join("xdcam_cc.mxf");
    std::fs::write(&mxf, cfa4_mxf::testing::build_anc((30000, 1001), &frames)).unwrap();
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [mxf.to_string_lossy()]}));
    let r = match r {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(r["documents"][0]["format"], "MXF ANC captions", "{r}");
    assert_eq!(r["documents"][0]["captions"], 2, "{r}");
    // #1068: CEA-608 CC1 and CEA-708 service 1 each become a track
    let q = s.active_sequence().unwrap();
    let mut names: Vec<&str> = q.caption_tracks.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["xdcam_cc (CC1)", "xdcam_cc (SERVICE1)"]);
    for tr in &q.caption_tracks {
        assert_eq!(tr.captions.len(), expected.cues.len());
        for (c, e) in tr.captions.iter().zip(&expected.cues) {
            assert_eq!(c.text, e.text);
            assert!((c.start.0 - q.settings.frame_rate.snap_nearest(e.start).0).abs() <= q.settings.frame_rate.tick_of(1).0, "{}", tr.name);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An MXF file without caption distribution packets: CC1 in digitised line 21 VI lines (10-bit,
/// luma and colour difference), CC3 in ST 334-1 CEA-608 packets on field 2. Each channel
/// becomes its own caption track with the right texts and times.
#[test]
fn mxf_line21_and_cea608_packets_import_per_channel() {
    use cfa4_captions::cea608::{line21_samples, with_parity};
    use cfa4_mxf::testing::{anc_element, build_anc, build_vi, vi_element};
    let cue = |a: i64, b: i64, t: &str| cfa4_captions::Cue {
        start: Tick(a * TICKS_PER_SECOND),
        end: Tick(b * TICKS_PER_SECOND),
        text: t.into(),
        ..Default::default()
    };
    let cc1 = cfa4_captions::Document { cues: vec![cue(1, 3, "From line 21"), cue(4, 6, "Second one")], ..Default::default() };
    let cc3 = cfa4_captions::Document { cues: vec![cue(2, 4, "Field two")], ..Default::default() };
    let rate = cfa4_time::FrameRate::new(30000, 1001);
    let expect = |d: &cfa4_captions::Document| {
        let s = cfa4_captions::scc::schedule(d);
        cfa4_captions::scc::decode_pairs(s.into_iter().map(|(f, (a, b))| (f, a, b)), rate)
    };
    let s1 = cfa4_captions::scc::schedule(&cc1);
    // field 2 carries CC3 with the misc control codes 15h instead of 14h
    let s3: std::collections::BTreeMap<i64, (u8, u8)> =
        cfa4_captions::scc::schedule(&cc3).into_iter().map(|(f, (a, b))| (f, (if a == 0x14 { 0x15 } else { a }, b))).collect();
    let frames = s1.keys().chain(s3.keys()).max().map_or(0, |&f| f as usize + 2);
    let dir = std::env::temp_dir().join(format!("fc-l21-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // line 21 as 4:2:2 Cb Y Cr Y samples (coding 9), chroma at 512
    let vi: Vec<Vec<u8>> = (0..frames as i64)
        .map(|f| {
            let (a, b) = s1.get(&f).copied().unwrap_or((0, 0));
            let luma = line21_samples(with_parity(a), with_parity(b), 720, 20.0);
            let both: Vec<u16> = luma.iter().flat_map(|&y| [512, y]).collect();
            vi_element(&[(21, 2, 9, both), (284, 3, 7, vec![64; 720])])
        })
        .collect();
    let anc: Vec<Vec<u8>> = (0..frames as i64)
        .map(|f| match s3.get(&f) {
            Some(&(a, b)) => anc_element(&[vec![0x61, 0x02, 3, 0x09, with_parity(a), with_parity(b)]], false),
            None => anc_element(&[], false),
        })
        .collect();
    let mut s = demo();
    for (name, bytes, want) in [
        ("vbi_cc.mxf", build_vi((30000, 1001), &vi), vec![("vbi_cc (CC1)", expect(&cc1))]),
        ("anc608.mxf", build_anc((30000, 1001), &anc), vec![("anc608 (CC3)", expect(&cc3))]),
    ] {
        let mxf = dir.join(name);
        std::fs::write(&mxf, bytes).unwrap();
        let r = match s.execute("file.import", json!({"paths": [mxf.to_string_lossy()]})) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(r["documents"][0]["format"], "MXF line 21 captions", "{r}");
        let q = s.active_sequence().unwrap();
        for (track, cues) in want {
            assert!(!cues.is_empty());
            let tr = q.caption_tracks.iter().find(|t| t.name == track).unwrap_or_else(|| panic!("no {track}: {r}"));
            assert_eq!(tr.format, cfa4_project::CaptionFormat::Cea608);
            assert_eq!(tr.captions.len(), cues.len(), "{track}");
            for (c, e) in tr.captions.iter().zip(&cues) {
                assert_eq!(c.text, e.text);
                assert_eq!(c.start, q.settings.frame_rate.snap_nearest(e.start));
            }
        }
    }
    // #1068: CDPs with CC1 next to ST 334-1 packets with CC3: both channels come in, CC3 from
    // the 334-1 packets
    let both: Vec<Vec<u8>> = (0..frames as i64)
        .map(|f| {
            let (a, b) = s1.get(&f).copied().unwrap_or((0x80, 0x80));
            let cc = [0xfc, with_parity(a), with_parity(b)];
            let mut cdp = vec![0x96, 0x69, 16, 0x4f, 0x43, 0, f as u8, 0x72, 0xe1];
            cdp.extend_from_slice(&cc);
            cdp.extend_from_slice(&[0x74, 0, f as u8, 0]);
            let mut packets = vec![[vec![0x61, 0x01, cdp.len() as u8], cdp].concat()];
            if let Some(&(a, b)) = s3.get(&f) {
                packets.push(vec![0x61, 0x02, 3, 0x09, with_parity(a), with_parity(b)]);
            }
            anc_element(&packets, false)
        })
        .collect();
    let mxf = dir.join("mixed.mxf");
    std::fs::write(&mxf, build_anc((30000, 1001), &both)).unwrap();
    let r = s.execute("file.import", json!({"paths": [mxf.to_string_lossy()]})).unwrap();
    let formats: Vec<&str> = r["documents"].as_array().unwrap().iter().filter_map(|d| d["format"].as_str()).collect();
    assert_eq!(formats, ["MXF ANC captions", "MXF line 21 captions"], "{r}");
    let q = s.active_sequence().unwrap();
    for (track, cues) in [("mixed (CC1)", expect(&cc1)), ("mixed (CC3)", expect(&cc3))] {
        let tr = q.caption_tracks.iter().find(|t| t.name == track).unwrap_or_else(|| panic!("no {track}: {r}"));
        let got: Vec<&str> = tr.captions.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(got, cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), "{track}");
    }
    assert!(!q.caption_tracks.iter().any(|t| t.name.starts_with("mixed (SERVICE")));
    let _ = std::fs::remove_dir_all(&dir);
}
