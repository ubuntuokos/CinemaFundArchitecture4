use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;
use crate::sequence_profiles::{SequenceProfile, builtin_profiles, description_key, display_aspect_for, file_stem, free_file, pixel_aspect_for};

fn session(name: &str) -> (Session, std::path::PathBuf) {
    let dir = tmp_dir(name);
    let s = Session { prefs_path: Some(dir.join("preferences.json")), ..Session::default() };
    (s, dir.join("sequence-profiles"))
}

fn profile(description: &str) -> Value {
    json!({"description": description, "width": 1440, "height": 1080, "displayAspect": [16, 9], "pixelAspect": [4, 3], "fps": [25, 1]})
}

fn descriptions(s: &mut Session) -> Vec<String> {
    let v = s.execute("sequenceProfiles.list", json!({})).unwrap();
    v["profiles"].as_array().unwrap().iter().map(|p| p["description"].as_str().unwrap().to_string()).collect()
}

fn user(s: &mut Session) -> Vec<(String, String)> {
    let v = s.execute("sequenceProfiles.list", json!({})).unwrap();
    v["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["builtin"] == json!(false))
        .map(|p| {
            let file = std::path::Path::new(p["file"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
            (p["description"].as_str().unwrap().to_string(), file)
        })
        .collect()
}

/// Either aspect follows from the other: 1440 x 1080 with 4:3 pixels shows 16:9, and a 720 x 576
/// frame shown 16:9 has 64:45 pixels. Sizes that overflow give `None`, never a wrapped ratio.
#[test]
fn aspects_follow_each_other() {
    assert_eq!(display_aspect_for(1920, 1080, (1, 1)), Some((16, 9)));
    assert_eq!(display_aspect_for(1440, 1080, (4, 3)), Some((16, 9)));
    assert_eq!(pixel_aspect_for(720, 576, (16, 9)), Some((64, 45)));
    assert_eq!(pixel_aspect_for(720, 480, (4, 3)), Some((8, 9)));
    assert_eq!(pixel_aspect_for(1920, 1080, (16, 9)), Some((1, 1)));
    assert_eq!(display_aspect_for(0, 1080, (1, 1)), None);
    assert_eq!(pixel_aspect_for(1920, 1080, (0, 9)), None);
    // u32::MAX x u32::MAX fits u64; the ratio of two primes near u32::MAX does not fit u32 once multiplied
    assert_eq!(display_aspect_for(u32::MAX, u32::MAX, (u32::MAX, u32::MAX)), Some((1, 1)));
    assert_eq!(display_aspect_for(4_294_967_291, 4_294_967_279, (4_294_967_291, 1)), None);
}

/// File names come from the description: ASCII letters and digits, other runs as `_`, capped.
#[test]
fn file_stems_are_safe() {
    assert_eq!(file_stem("HD 1080p 25 fps"), "hd_1080p_25_fps");
    assert_eq!(file_stem("  ../../etc/passwd "), "etc_passwd");
    assert_eq!(file_stem("360° 4K"), "360_4k");
    assert_eq!(file_stem("ÁÉŐ"), "profile");
    assert_eq!(file_stem(""), "profile");
    assert!(file_stem(&"a".repeat(500)).len() <= 64);
    assert_eq!(description_key("  HD   1080p "), "hd 1080p");
    let dir = tmp_dir("profile-stems");
    assert_eq!(free_file(&dir, "My Format", None).unwrap(), dir.join("my_format.json"));
    std::fs::write(dir.join("my_format.json"), b"{}").unwrap();
    assert_eq!(free_file(&dir, "My Format", None).unwrap(), dir.join("my_format_2.json"));
    assert_eq!(free_file(&dir, "My Format", Some(&dir.join("my_format.json"))).unwrap(), dir.join("my_format.json"));
}

/// The built-in profiles are valid, already tidy and have distinct descriptions.
#[test]
fn builtin_profiles_are_valid_and_distinct() {
    let b = builtin_profiles();
    for p in &b {
        assert_eq!(&p.validated().unwrap(), p, "{}", p.description);
    }
    let mut keys: Vec<String> = b.iter().map(|p| description_key(&p.description)).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), b.len());
}

/// Hostile profiles are refused with a message: empty or overlong descriptions, control
/// characters, zero or huge sizes, stretched or zero pixel aspects, a display aspect that does
/// not match, zero or absurd frame rates. Every truncation of a good file is refused cleanly.
#[test]
fn bad_profiles_are_refused() {
    let good: SequenceProfile = serde_json::from_value(profile("Anamorphic HDV")).unwrap();
    assert_eq!(good.validated().unwrap(), good);
    let tidy = SequenceProfile { description: "  Wide   one ".into(), fps: (50, 2), pixel_aspect: (8, 6), ..good.clone() }.validated().unwrap();
    assert_eq!((tidy.description.as_str(), tidy.fps, tidy.pixel_aspect), ("Wide one", (25, 1), (4, 3)));
    for (why, p) in [
        ("empty", SequenceProfile { description: "   ".into(), ..good.clone() }),
        ("long", SequenceProfile { description: "x".repeat(129), ..good.clone() }),
        ("control", SequenceProfile { description: "a\u{7}b".into(), ..good.clone() }),
        ("zero width", SequenceProfile { width: 0, display_aspect: (0, 1), ..good.clone() }),
        ("huge", SequenceProfile { width: 40_000, ..good.clone() }),
        ("zero par", SequenceProfile { pixel_aspect: (0, 1), ..good.clone() }),
        ("stretched par", SequenceProfile { pixel_aspect: (9, 1), ..good.clone() }),
        ("dar mismatch", SequenceProfile { display_aspect: (4, 3), ..good.clone() }),
        ("zero dar", SequenceProfile { display_aspect: (0, 0), ..good.clone() }),
        ("zero fps", SequenceProfile { fps: (0, 1), ..good.clone() }),
        ("zero fps den", SequenceProfile { fps: (25, 0), ..good.clone() }),
        ("fast", SequenceProfile { fps: (1001, 1), ..good.clone() }),
        ("slow", SequenceProfile { fps: (1, u32::MAX), ..good.clone() }),
    ] {
        assert!(p.validated().is_err(), "{why} should be refused");
    }
    let bytes = serde_json::to_vec(&json!({"format": "cfa4.sequence-profile", "version": 1, "description": "A", "width": 1920, "height": 1080, "displayAspect": [16, 9], "pixelAspect": [1, 1], "fps": [25, 1]})).unwrap();
    assert!(SequenceProfile::parse(&bytes).is_ok());
    for n in 0..bytes.len() {
        assert!(SequenceProfile::parse(&bytes[..n]).is_err());
    }
    let wrong_format = serde_json::to_vec(&json!({"format": "other", "version": 1, "description": "A"})).unwrap();
    assert!(SequenceProfile::parse(&wrong_format).is_err());
    let future = String::from_utf8(bytes.clone()).unwrap().replace("\"version\":1", "\"version\":99");
    assert!(SequenceProfile::parse(future.as_bytes()).is_err());
    assert!(SequenceProfile::parse(&[0xff, 0xfe]).is_err());
}

/// Save writes `<data dir>/sequence-profiles/<stem>.json`; descriptions are unique against
/// built-in and user profiles; a clashing file name is numbered; editing renames the file;
/// editing a built-in profile makes a user copy; delete removes user profiles only.
#[test]
fn save_edit_duplicate_and_delete() {
    let (mut s, dir) = session("sequence-profiles");
    let builtins = descriptions(&mut s).len();
    assert_eq!(builtins, builtin_profiles().len());

    let r = s.execute("sequenceProfiles.save", json!({"profile": profile("My HDV")})).unwrap();
    assert_eq!(r["profile"]["description"], "My HDV");
    assert!(dir.join("my_hdv.json").is_file());
    // the same description (any case or spacing) is taken; so is a built-in one
    for taken in [" my  hdv ", "HD 1080p 25 FPS"] {
        let e = s.execute("sequenceProfiles.save", json!({"profile": profile(taken)})).unwrap_err().to_string();
        assert!(e.contains("already called"), "{e}");
    }
    // another description with the same file stem gets a numbered file
    s.execute("sequenceProfiles.save", json!({"profile": profile("My-HDV")})).unwrap();
    assert_eq!(user(&mut s), [("My HDV".to_string(), "my_hdv.json".to_string()), ("My-HDV".into(), "my_hdv_2.json".into())]);

    // edit: same description keeps the file, a new one renames it
    let mut changed = profile("My HDV");
    changed["fps"] = json!([50, 1]);
    s.execute("sequenceProfiles.save", json!({"profile": changed, "original": "my hdv"})).unwrap();
    assert_eq!(SequenceProfile::read(&dir.join("my_hdv.json")).unwrap().fps, (50, 1));
    s.execute("sequenceProfiles.save", json!({"profile": profile("Studio HDV"), "original": "My HDV"})).unwrap();
    assert!(!dir.join("my_hdv.json").exists());
    assert!(dir.join("studio_hdv.json").is_file());
    // renaming onto another profile's description is refused and changes nothing
    assert!(s.execute("sequenceProfiles.save", json!({"profile": profile("My-HDV"), "original": "Studio HDV"})).is_err());
    assert!(dir.join("studio_hdv.json").is_file());
    // editing a built-in profile keeps it and saves a user profile
    let mut hd = profile("HD 1080p 25 fps (mine)");
    hd["width"] = json!(1920);
    hd["pixelAspect"] = json!([1, 1]);
    s.execute("sequenceProfiles.save", json!({"profile": hd, "original": "HD 1080p 25 fps"})).unwrap();
    assert!(descriptions(&mut s).contains(&"HD 1080p 25 fps".to_string()));
    assert!(s.execute("sequenceProfiles.save", json!({"profile": profile("X"), "original": "nobody"})).is_err());

    // duplicate: "<description> copy", then "copy 2"; built-in ones too
    let r = s.execute("sequenceProfiles.duplicate", json!({"description": "Studio HDV"})).unwrap();
    assert_eq!(r["profile"]["description"], "Studio HDV copy");
    let r = s.execute("sequenceProfiles.duplicate", json!({"description": "Studio HDV"})).unwrap();
    assert_eq!(r["profile"]["description"], "Studio HDV copy 2");
    let r = s.execute("sequenceProfiles.duplicate", json!({"description": "UHD 2160p 24 fps"})).unwrap();
    assert_eq!(r["profile"]["builtin"], false);
    assert!(s.execute("sequenceProfiles.duplicate", json!({"description": "nobody"})).is_err());

    // delete: user profiles only
    s.execute("sequenceProfiles.delete", json!({"description": "studio hdv copy"})).unwrap();
    assert!(!descriptions(&mut s).contains(&"Studio HDV copy".to_string()));
    let e = s.execute("sequenceProfiles.delete", json!({"description": "HD 1080p 25 fps"})).unwrap_err().to_string();
    assert!(e.contains("can't be deleted"), "{e}");
    assert!(s.execute("sequenceProfiles.delete", json!({})).is_err());
    assert!(s.execute("sequenceProfiles.save", json!({"profile": "nonsense"})).is_err());
    assert!(s.execute("sequenceProfiles.save", json!({"profile": profile("Y"), "original": 5})).is_err());
}

/// Damaged files and files whose description is already taken are left out of the list; a
/// session without a data directory lists the built-in profiles and refuses to save.
#[test]
fn listing_skips_bad_and_duplicate_files() {
    let (mut s, dir) = session("sequence-profiles-bad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.json"), b"{\"format\":").unwrap();
    std::fs::write(dir.join("big.json"), vec![b' '; 70 * 1024]).unwrap();
    let mut dup = serde_json::from_value::<Value>(profile("HD 1080p 25 fps")).unwrap();
    dup["format"] = json!("cfa4.sequence-profile");
    dup["version"] = json!(1);
    dup["width"] = json!(1920);
    dup["pixelAspect"] = json!([1, 1]);
    std::fs::write(dir.join("dup.json"), serde_json::to_vec(&dup).unwrap()).unwrap();
    assert_eq!(descriptions(&mut s).len(), builtin_profiles().len());
    assert!(user(&mut s).is_empty());

    let mut bare = Session::default();
    assert_eq!(descriptions(&mut bare).len(), builtin_profiles().len());
    assert!(bare.execute("sequenceProfiles.save", json!({"profile": profile("Z")})).is_err());
}

/// Apply sets the active sequence's frame size, rate and pixel aspect in one undo step, and says
/// which parts CFA4 sequences can't take yet (interlaced scan, 360°).
#[test]
fn apply_sets_the_sequence() {
    let (mut s, _dir) = session("sequence-profiles-apply");
    assert!(s.execute("sequenceProfiles.apply", json!({"description": "HD 720p 50 fps"})).is_err(), "no sequence yet");
    s.execute("file.newSequence", json!({})).unwrap();
    s.execute("sequenceProfiles.save", json!({"profile": profile("My HDV")})).unwrap();
    let r = s.execute("sequenceProfiles.apply", json!({"description": "my hdv"})).unwrap();
    assert_eq!(r["notApplied"], json!([]));
    let q = &s.active_sequence().unwrap().settings;
    assert_eq!((q.width, q.height, q.par, q.frame_rate.num, q.frame_rate.den), (1440, 1080, (4, 3), 25, 1));
    assert_eq!(q.preset, "My HDV");
    let r = s.execute("sequenceProfiles.apply", json!({"description": "SD PAL 16:9 interlaced 25 fps"})).unwrap();
    assert_eq!(r["notApplied"], json!(["interlaced"]));
    assert_eq!(s.active_sequence().unwrap().settings.par, (64, 45));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_sequence().unwrap().settings.width, 1440);
    assert!(s.execute("sequenceProfiles.apply", json!({"description": "nobody"})).is_err());
    assert!(s.execute("sequenceProfiles.apply", json!({})).is_err());
}
