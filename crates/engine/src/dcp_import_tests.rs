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
