//! DCP package import (#101): CFA4's own DCP export read back as a sequence, and hand-made asset
//! maps and playlists for reels, version files, unsafe paths and encryption.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::Session;
use cfa4_time::Tick;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn scratch(name: &str) -> PathBuf {
    let d = cfa4_testkit::workspace_root().join("target").join("export-tests").join(format!("engine-dcp-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A quarter second of the demo sequence as a 2K Flat SMPTE DCP; returns the package folder.
fn export_dcp(dir: &Path) -> PathBuf {
    let mut s = demo();
    let path = dir.join("Demo").to_string_lossy().to_string();
    let r = s
        .execute("file.exportMedia", json!({"preset": "DCP 2K Flat", "path": path, "range": "custom", "startSeconds": 1.0, "endSeconds": 1.25, "wait": true}))
        .unwrap();
    PathBuf::from(r["path"].as_str().unwrap())
}

fn file_starting(dir: &Path, prefix: &str) -> String {
    std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).find(|f| f.starts_with(prefix)).unwrap()
}

/// The `urn:uuid:` of the reel asset whose `<Id>` follows `element` in the CPL text.
fn asset_id(cpl: &str, element: &str) -> String {
    let at = cpl.find(&format!("<{element}>")).unwrap();
    let id = cpl[at..].find("<Id>").unwrap() + at + 4;
    cpl[id..id + 45].to_string()
}

#[test]
fn an_exported_dcp_imports_as_a_sequence() {
    let dir = scratch("own");
    let pkg = export_dcp(&dir);
    let mut s = demo();
    let before = s.project.items.len();
    let r = s.execute("file.import", json!({"paths": [pkg.to_string_lossy()]})).unwrap();
    let doc = &r["documents"][0];
    assert_eq!(doc["format"], "DCP", "{r}");
    assert_eq!(doc["standard"], "SMPTE", "{r}");
    assert!(doc["warnings"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(doc["compositions"][0]["reels"], 1, "{r}");
    assert_eq!(doc["compositions"][0]["placed"], 2, "picture and sound: {r}");
    assert_eq!(r["sequences"].as_array().unwrap().len(), 1, "{r}");
    let q = s.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (1998, 1080));
    assert_eq!(q.settings.frame_rate.num, 24);
    let v = &q.video_tracks[0].items;
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].start, Tick::ZERO);
    assert_eq!(v[0].duration, Tick(q.settings.frame_rate.frame_duration().0 * 6), "six frames at 24 fps");
    assert!(q.audio_tracks.iter().any(|t| !t.items.is_empty()), "the sound is placed");
    // a bin with the track files and the sequence, as one undo step
    assert!(s.project.items.len() > before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.items.len(), before);
    // the asset map or the playlist itself import the same package
    let r = s.execute("file.import", json!({"paths": [pkg.join("ASSETMAP.xml").to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["compositions"][0]["placed"], 2, "{r}");
    let cpl = pkg.join(file_starting(&pkg, "cpl_"));
    let r = s.execute("file.import", json!({"paths": [cpl.to_string_lossy()]})).unwrap();
    assert_eq!(r["documents"][0]["compositions"][0]["placed"], 2, "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two reels play one after the other, each from its EntryPoint; an asset the package lacks, an
/// encrypted asset without its key and a path out of the package are reported, not placed.
#[test]
fn reels_follow_each_other_and_problems_are_reported() {
    let dir = scratch("reels");
    let pkg = export_dcp(&dir);
    let cpl_name = file_starting(&pkg, "cpl_");
    let cpl = std::fs::read_to_string(pkg.join(&cpl_name)).unwrap();
    let pic = asset_id(&cpl, "MainPicture");
    let snd = asset_id(&cpl, "MainSound");
    let reel = |id: &str, pic_extra: &str, entry: u32, dur: u32| {
        format!(
            "<Reel><Id>{id}</Id><AssetList>\
             <MainPicture><Id>{pic}</Id><EditRate>24 1</EditRate><IntrinsicDuration>6</IntrinsicDuration><EntryPoint>{entry}</EntryPoint><Duration>{dur}</Duration>{pic_extra}<FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>\
             <MainSound><Id>{snd}</Id><EditRate>24 1</EditRate><IntrinsicDuration>6</IntrinsicDuration><EntryPoint>{entry}</EntryPoint><Duration>{dur}</Duration></MainSound>\
             </AssetList></Reel>"
        )
    };
    let missing = "urn:uuid:11111111-2222-4333-8444-555555555555";
    let outside = "urn:uuid:66666666-7777-4888-8999-aaaaaaaaaaaa";
    let missing_reel = format!(
        "<Reel><Id>urn:uuid:bbbbbbbb-0000-4000-8000-000000000003</Id><AssetList>\
         <MainPicture><Id>{missing}</Id><EditRate>24 1</EditRate><IntrinsicDuration>24</IntrinsicDuration><EntryPoint>0</EntryPoint><Duration>24</Duration><FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>\
         <MainSubtitle><Id>{outside}</Id><EditRate>24 1</EditRate><IntrinsicDuration>24</IntrinsicDuration><EntryPoint>0</EntryPoint><Duration>24</Duration></MainSubtitle>\
         </AssetList></Reel>"
    );
    let encrypted = reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000004", "<KeyId>urn:uuid:cccccccc-0000-4000-8000-000000000001</KeyId>", 0, 6);
    let reels = [
        reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000001", "", 0, 6),
        reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000002", "", 2, 3),
        missing_reel,
        encrypted,
    ]
    .concat();
    let at = cpl.find("<ReelList>").unwrap() + "<ReelList>".len();
    let end = cpl.find("</ReelList>").unwrap();
    let new_cpl = format!("{}{}{}", &cpl[..at], reels, &cpl[end..]);
    // the hand-made playlist replaces the exported one; the asset map gains an asset outside
    std::fs::write(pkg.join(&cpl_name), new_cpl).unwrap();
    let map = std::fs::read_to_string(pkg.join("ASSETMAP.xml")).unwrap();
    let extra = format!("<Asset><Id>{outside}</Id><ChunkList><Chunk><Path>../secret.xml</Path></Chunk></ChunkList></Asset></AssetList>");
    std::fs::write(pkg.join("ASSETMAP.xml"), map.replacen("</AssetList>", &extra, 1)).unwrap();

    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [pkg.to_string_lossy()]})).unwrap();
    let doc = &r["documents"][0];
    let warnings: Vec<String> = doc["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect();
    assert!(warnings.iter().any(|w| w.contains("../secret.xml")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("reel 3") && w.contains("not in this package")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("reel 4") && w.contains("dcp.loadKdm")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("reel 3") && w.contains("MainSubtitle")), "{warnings:?}");
    assert_eq!(doc["compositions"][0]["reels"], 4, "{r}");
    let q = s.active_sequence().unwrap();
    let f = |n: i64| Tick(q.settings.frame_rate.frame_duration().0 * n);
    let v: Vec<(Tick, Tick, Tick)> = q.video_tracks[0].items.iter().map(|i| (i.start, i.duration, i.source_in)).collect();
    // reel 1: frames 0-5; reel 2: three frames from frame 2, right after it
    assert_eq!(v, vec![(Tick::ZERO, f(6), Tick::ZERO), (f(6), f(3), f(2))]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1245: the subtitle track file of each reel becomes one caption track, the cues of each reel
/// from its entry point, cut to its duration, at the reel's start.
#[test]
fn subtitles_become_one_caption_track() {
    let dir = scratch("subs");
    let mut s = demo();
    s.execute("playhead.set", json!({"seconds": 1.05})).unwrap();
    s.execute("captions.add", json!({"text": "Szia", "durationSeconds": 0.1})).unwrap();
    let path = dir.join("Demo").to_string_lossy().to_string();
    let r = s
        .execute(
            "file.exportMedia",
            json!({"preset": "DCP 2K Flat", "path": path, "range": "custom", "startSeconds": 1.0, "endSeconds": 1.25, "dcpSubtitles": true, "dcpSubtitleLanguage": "hu", "wait": true}),
        )
        .unwrap();
    let pkg = PathBuf::from(r["path"].as_str().unwrap());
    let cpl_name = file_starting(&pkg, "cpl_");
    let cpl = std::fs::read_to_string(pkg.join(&cpl_name)).unwrap();
    assert!(cpl.contains("<MainSubtitle>"), "{cpl}");

    // one reel as exported
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [pkg.to_string_lossy()]})).unwrap();
    let doc = &r["documents"][0];
    assert!(doc["warnings"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(doc["compositions"][0]["captions"][0]["captions"], 1, "{r}");
    let q = s.active_sequence().unwrap();
    let rate = q.settings.frame_rate;
    assert_eq!(q.caption_tracks.len(), 1);
    let c = &q.caption_tracks[0].captions;
    assert_eq!(c.len(), 1);
    assert!(c[0].text.contains("Szia"), "{:?}", c[0].text);
    assert!(q.caption_tracks[0].name.contains("(hu)"), "{}", q.caption_tracks[0].name);
    let (start, end) = (rate.frame_at(c[0].start), rate.frame_at(c[0].start + c[0].duration));
    assert!(start >= 1 && end > start && end <= 6, "{start}..{end}");

    // two reels: the second plays the file from the cue's last frame, so its part is cut there
    let pic = asset_id(&cpl, "MainPicture");
    let sub = asset_id(&cpl, "MainSubtitle");
    let reel = |id: &str, entry: i64, dur: i64| {
        format!(
            "<Reel><Id>{id}</Id><AssetList>\
             <MainPicture><Id>{pic}</Id><EditRate>24 1</EditRate><IntrinsicDuration>6</IntrinsicDuration><EntryPoint>{entry}</EntryPoint><Duration>{dur}</Duration><FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>\
             <MainSubtitle><Id>{sub}</Id><EditRate>24 1</EditRate><IntrinsicDuration>6</IntrinsicDuration><EntryPoint>{entry}</EntryPoint><Duration>{dur}</Duration><Language>hu</Language></MainSubtitle>\
             </AssetList></Reel>"
        )
    };
    let reels =
        [reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000011", 0, 6), reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000012", end - 1, 6 - (end - 1))].concat();
    let at = cpl.find("<ReelList>").unwrap() + "<ReelList>".len();
    let stop = cpl.find("</ReelList>").unwrap();
    std::fs::write(pkg.join(&cpl_name), format!("{}{}{}", &cpl[..at], reels, &cpl[stop..])).unwrap();
    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [pkg.to_string_lossy()]})).unwrap();
    assert!(r["documents"][0]["warnings"].as_array().unwrap().is_empty(), "{r}");
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.len(), 1, "one track for both reels");
    let c: Vec<(i64, i64)> = q.caption_tracks[0].captions.iter().map(|c| (rate.frame_at(c.start), rate.frame_at(c.start + c.duration))).collect();
    assert_eq!(c, vec![(start, end), (6, 7)], "reel 2 starts at frame 6 with the cue's last frame");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1245: PNG image subtitles of the reels go on a video track above the picture, each reel's
/// part from its entry point at the reel's start.
#[test]
fn image_subtitles_follow_the_reels() {
    const REEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <Language>hu</Language>
  <EditRate>24 1</EditRate>
  <TimeCodeRate>24</TimeCodeRate>
  <StartTime>00:00:00:00</StartTime>
  <SubtitleList>
    <Subtitle SpotNumber="1" TimeIn="00:00:00:02" TimeOut="00:00:00:05" FadeUpTime="00:00:00:00" FadeDownTime="00:00:00:00">
      <Image Valign="bottom" Vposition="10">urn:uuid:20202020-2020-2020-2020-202020202020</Image>
    </Subtitle>
  </SubtitleList>
</SubtitleReel>
"#;
    let dir = scratch("img-subs");
    let pkg = export_dcp(&dir);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(200, 50, image::Rgba([255, 255, 255, 200])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    std::fs::write(pkg.join("sub_img.mxf"), cfa4_mxf::testing::build_timed_text(REEL.as_bytes(), &[("image/png", &png)])).unwrap();
    let sub = "urn:uuid:dddddddd-0000-4000-8000-000000000001";
    let map = std::fs::read_to_string(pkg.join("ASSETMAP.xml")).unwrap();
    let extra = format!("<Asset><Id>{sub}</Id><ChunkList><Chunk><Path>sub_img.mxf</Path></Chunk></ChunkList></Asset></AssetList>");
    std::fs::write(pkg.join("ASSETMAP.xml"), map.replacen("</AssetList>", &extra, 1)).unwrap();
    let cpl_name = file_starting(&pkg, "cpl_");
    let cpl = std::fs::read_to_string(pkg.join(&cpl_name)).unwrap();
    let pic = asset_id(&cpl, "MainPicture");
    let reel = |id: &str, entry: u32, dur: u32| {
        format!(
            "<Reel><Id>{id}</Id><AssetList>\
             <MainPicture><Id>{pic}</Id><EditRate>24 1</EditRate><IntrinsicDuration>6</IntrinsicDuration><EntryPoint>{entry}</EntryPoint><Duration>{dur}</Duration><FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>\
             <MainSubtitle><Id>{sub}</Id><EditRate>24 1</EditRate><IntrinsicDuration>6</IntrinsicDuration><EntryPoint>{entry}</EntryPoint><Duration>{dur}</Duration></MainSubtitle>\
             </AssetList></Reel>"
        )
    };
    let reels = [reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000021", 0, 6), reel("urn:uuid:bbbbbbbb-0000-4000-8000-000000000022", 3, 3)].concat();
    let at = cpl.find("<ReelList>").unwrap() + "<ReelList>".len();
    let stop = cpl.find("</ReelList>").unwrap();
    std::fs::write(pkg.join(&cpl_name), format!("{}{}{}", &cpl[..at], reels, &cpl[stop..])).unwrap();

    let mut s = demo();
    let r = s.execute("file.import", json!({"paths": [pkg.to_string_lossy()]})).unwrap();
    let doc = &r["documents"][0];
    assert!(doc["warnings"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(doc["compositions"][0]["captions"][0]["images"], 2, "{r}");
    let q = s.active_sequence().unwrap();
    let f = |n: i64| Tick(q.settings.frame_rate.frame_duration().0 * n);
    assert!(q.caption_tracks.is_empty(), "no text, no caption track");
    let clips: Vec<(Tick, Tick)> = q.video_tracks.iter().skip(1).flat_map(|t| t.items.iter().map(|i| (i.start, i.duration))).collect();
    // reel 1: frames 2-4; reel 2 plays the file from frame 3: frames 3-4 at its start (frame 6)
    assert_eq!(clips, vec![(f(2), f(3)), (f(6), f(2))]);
    assert!(pkg.join("sub_img_20202020-2020-2020-2020-202020202020.png").exists(), "the PNG is written beside the MXF");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn not_a_dcp_is_left_to_the_other_importers() {
    let dir = scratch("none");
    let xml = dir.join("notes.xml");
    std::fs::write(&xml, "<CompositionPlaylist/>").unwrap();
    let mut s = demo();
    // no asset map beside it: not a DCP (and not a format CFA4 imports)
    assert!(crate::dcp_import::import_package(&mut s, &xml.to_string_lossy(), None).is_none());
    assert!(crate::dcp_import::import_package(&mut s, &dir.to_string_lossy(), None).is_none());
    // an asset map without playlists is an error, not a crash
    std::fs::write(dir.join("ASSETMAP.xml"), r#"<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><AssetList/></AssetMap>"#).unwrap();
    let e = crate::dcp_import::import_package(&mut s, &dir.to_string_lossy(), None).unwrap().unwrap_err().to_string();
    assert!(e.contains("no composition playlist"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
