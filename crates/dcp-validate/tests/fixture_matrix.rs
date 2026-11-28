//! DCP-16: known-result regression matrix. One valid SMPTE package is written by CFA4's own DCP
//! export; each case copies it, damages it in one named way (the plan's §12 list) and states which
//! rule must fail and which must keep passing. No media is committed: the package is generated
//! into the temp directory (`AGENTS.md`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest, Verdict};
use cfa4_export::{ExportSettings, Format, Progress, export};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const PROFILE: &str = "delivery-preflight";

/// A tiny valid SMPTE package (5 frames, 320×180 at 25 fps, 440 Hz tone) from the exporter.
fn export_valid(dir: &Path) {
    let mut p = Project::new("dcp-fixture");
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
    let seq = p.new_sequence("Fixture", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
    let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(50)), r).unwrap();
    let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, r.tick_of(50)), r).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(vi);
    q.audio_tracks[0].items.push(ai);
    let mut m = SourceMap::default();
    m.0.insert(v, Arc::new(white));
    m.0.insert(a, Arc::new(tone));
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.to_string_lossy().into_owned(),
        range: Some(TimeRange::new(Tick::ZERO, r.tick_of(5))),
        dcp_content_kind: "test".into(),
        ..Default::default()
    };
    export(&Arc::new(p), seq, &s, &m, &Progress::default()).unwrap();
}

fn base() -> &'static Path {
    static BASE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("cfa4-dcp-fixture-matrix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        export_valid(&d);
        d
    })
}

/// A fresh copy of the valid package.
fn copy(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-fixture-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for e in std::fs::read_dir(base()).unwrap().flatten() {
        std::fs::copy(e.path(), d.join(e.file_name())).unwrap();
    }
    d
}

fn file(d: &Path, prefix: &str) -> PathBuf {
    let hit = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with(prefix));
    d.join(hit.unwrap_or_else(|| panic!("no {prefix}* in the package")))
}

fn edit(path: &Path, f: impl FnOnce(String) -> String) {
    let t = std::fs::read_to_string(path).unwrap();
    std::fs::write(path, f(t)).unwrap();
}

fn validate(d: &Path) -> ValidationReport {
    NativeValidator.validate(&ValidationRequest::new(d, Profile::builtin(PROFILE).unwrap()), &ValidationControl::new()).unwrap()
}

fn failed(r: &ValidationReport) -> Vec<&str> {
    let mut v: Vec<&str> = r.findings.iter().filter(|f| f.status == Status::Fail).map(|f| f.rule.as_str()).collect();
    v.dedup();
    v
}

type Damage = fn(&Path);
type Case = (&'static str, Damage, &'static [&'static str], &'static [&'static str]);

/// (case, damage, rules that must fail, rules that must warn)
const CASES: &[Case] = &[
    ("asset map is not XML", |d| std::fs::write(d.join("ASSETMAP.xml"), "<AssetMap><Id>").unwrap(), &["DCP.AM.001"], &[]),
    ("asset map is missing", |d| std::fs::remove_file(d.join("ASSETMAP.xml")).unwrap(), &["DCP.AM.001"], &[]),
    ("packing list is missing", |d| std::fs::remove_file(file(d, "pkl_")).unwrap(), &["DCP.AM.005"], &[]),
    ("packing list is not XML", |d| std::fs::write(file(d, "pkl_"), "not xml <<").unwrap(), &["DCP.PKL.001"], &[]),
    ("composition is not XML", |d| std::fs::write(file(d, "cpl_"), "<CompositionPlaylist>").unwrap(), &["DCP.CPL.001"], &[]),
    ("sound track file is missing", |d| std::fs::remove_file(file(d, "pcm_")).unwrap(), &["DCP.AM.005"], &[]),
    ("picture track file is missing", |d| std::fs::remove_file(file(d, "j2c_")).unwrap(), &["DCP.AM.005"], &[]),
    (
        "a changed byte (bad hash)",
        |d| {
            let p = file(d, "pcm_");
            let mut b = std::fs::read(&p).unwrap();
            let i = b.len() - 1;
            b[i] ^= 0xFF;
            std::fs::write(p, b).unwrap();
        },
        &["DCP.PKL.004"],
        &[],
    ),
    (
        "a truncated track file (bad size)",
        |d| {
            let p = file(d, "pcm_");
            let b = std::fs::read(&p).unwrap();
            std::fs::write(p, &b[..b.len() - 16]).unwrap();
        },
        &["DCP.PKL.003"],
        &[],
    ),
    (
        "a duplicated asset in the asset map",
        |d| {
            edit(&d.join("ASSETMAP.xml"), |t| {
                let a = t.find("<Asset>").unwrap();
                let b = t[a..].find("</Asset>").unwrap() + a + "</Asset>".len();
                format!("{}{}{}", &t[..b], &t[a..b], &t[b..])
            })
        },
        &["DCP.AM.003"],
        &[],
    ),
    ("a path that leaves the package", |d| edit(&d.join("ASSETMAP.xml"), |t| t.replacen("<Path>", "<Path>../", 1)), &["DCP.AM.004"], &[]),
    (
        "a composition reference to an asset the packing list does not have",
        |d| {
            edit(&file(d, "cpl_"), |t| {
                let m = t.find("<MainPicture>").unwrap();
                let at = t[m..].find("urn:uuid:").unwrap() + m + "urn:uuid:".len();
                let c = if t.as_bytes()[at] == b'0' { "1" } else { "0" };
                format!("{}{c}{}", &t[..at], &t[at + 1..])
            })
        },
        &["DCP.PKL.004"],
        &["DCP.CPL.003"],
    ),
    (
        "a reel duration that is not the track file's",
        |d| edit(&file(d, "cpl_"), |t| t.replace("<Duration>5</Duration>", "<Duration>4</Duration>")),
        &["DCP.CPL.005"],
        &[],
    ),
    (
        "a frame rate that differs from the track files'",
        |d| edit(&file(d, "cpl_"), |t| t.replace("<EditRate>25 1</EditRate>", "<EditRate>30 1</EditRate>")),
        &["DCP.MXF.001"],
        &[],
    ),
    (
        "an invalid JPEG 2000 header",
        |d| {
            let p = file(d, "j2c_");
            let mut b = std::fs::read(&p).unwrap();
            // the first codestream starts with SOC FF4F; break every SOC marker's second byte
            for i in 0..b.len().saturating_sub(1) {
                if b[i] == 0xFF && b[i + 1] == 0x4F {
                    b[i + 1] = 0x00;
                }
            }
            std::fs::write(p, b).unwrap();
        },
        &["DCP.J2K.001"],
        &[],
    ),
];

#[test]
fn the_valid_package_passes_every_check() {
    let d = copy("valid");
    let r = validate(&d);
    assert!(failed(&r).is_empty(), "{:?}", failed(&r));
    assert_eq!(r.standard, Some(cfa4_dcp::Standard::Smpte));
    assert_ne!(r.verdict, Verdict::Fail);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn every_damaged_package_fails_the_rule_that_names_the_damage() {
    let mut wrong = Vec::new();
    for (name, damage, rules, warns) in CASES {
        let d = copy("case");
        damage(&d);
        let r = validate(&d);
        let got = failed(&r);
        let warned = |x: &&str| r.findings.iter().any(|f| f.rule == *x && f.status == Status::Warning);
        if r.verdict != Verdict::Fail || !rules.iter().all(|x| got.contains(x)) || !warns.iter().all(warned) {
            wrong.push(format!("{name}: verdict {:?}, failed {got:?}, expected {rules:?}, warnings {warns:?}", r.verdict));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn damage_never_panics_or_passes_in_the_quick_scan_of_nothing() {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-fixture-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let r = validate(&d);
    assert_eq!(r.verdict, Verdict::Fail);
    let _ = std::fs::remove_dir_all(&d);
}

// DCP-19: release-gate checks of the plan's §14 acceptance criteria that need a real package.

/// Every file's name, size, modification time and bytes.
fn snapshot(d: &Path) -> Vec<(String, u64, Option<std::time::SystemTime>, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(d)
        .unwrap()
        .flatten()
        .map(|e| {
            let m = e.metadata().unwrap();
            (e.file_name().to_string_lossy().into_owned(), m.len(), m.modified().ok(), std::fs::read(e.path()).unwrap())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn validation_never_changes_the_package_under_any_profile() {
    // criterion 10: the original DCP files stay as they were, and nothing is written next to them
    let d = copy("unchanged");
    let before = snapshot(&d);
    for p in Profile::all() {
        let id = p.id.clone();
        let r = NativeValidator.validate(&ValidationRequest::new(&d, p), &ValidationControl::new()).unwrap();
        assert_ne!(r.verdict, Verdict::Interrupted, "{id}");
        assert!(snapshot(&d) == before, "profile {id} changed the package");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_report_names_the_profile_version_and_standard_and_survives_json() {
    // criterion 16: the profile and the standard that were applied can be looked up afterwards
    let d = copy("traceable");
    let r = validate(&d);
    assert_eq!(r.profile.id, PROFILE);
    assert!(r.profile.version >= 1);
    assert_eq!(r.standard, Some(cfa4_dcp::Standard::Smpte));
    let back = ValidationReport::from_json(&r.to_json()).expect("report schema");
    assert_eq!((back.profile.id, back.profile.version, back.standard), (r.profile.id, r.profile.version, r.standard));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn unchecked_properties_never_count_as_a_pass() {
    // criterion 7: a package with an unchecked or skipped rule is never reported as a plain pass
    let d = copy("unchecked");
    for p in Profile::all() {
        let id = p.id.clone();
        let r = NativeValidator.validate(&ValidationRequest::new(&d, p), &ValidationControl::new()).unwrap();
        let open = r.findings.iter().any(|f| matches!(f.status, Status::Unchecked | Status::Skipped));
        if open {
            assert_ne!(r.verdict, Verdict::Pass, "{id}: unchecked or skipped rules but a pass");
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_unreadable_track_file_fails_without_panicking() {
    // DCP-20: a read error (here a directory where the file should be; permission bits do not stop root,
    // so the test does not rely on them) is a failed asset check, never a panic or a pass
    let d = copy("unreadable");
    let p = file(&d, "pcm_");
    std::fs::remove_file(&p).unwrap();
    std::fs::create_dir(&p).unwrap();
    let r = validate(&d);
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(failed(&r).contains(&"DCP.AM.005"), "{:?}", failed(&r));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_unlisted_packing_list_or_composition_is_a_warning_not_a_failure() {
    // DCP.AM.007: copies of the PKL and CPL the asset map does not list (a left-over or a second version)
    let d = copy("unlisted");
    for prefix in ["pkl_", "cpl_"] {
        let src = file(&d, prefix);
        let name = src.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::copy(&src, d.join(format!("old_{name}"))).unwrap();
    }
    // a file that is no PKL or CPL, and one that is not XML at all, are not reported
    std::fs::write(d.join("notes.xml"), "<Notes/>").unwrap();
    std::fs::write(d.join("readme.txt"), "x").unwrap();
    let r = validate(&d);
    let warned: Vec<_> = r.findings.iter().filter(|f| f.rule == "DCP.AM.007" && f.status == Status::Warning).collect();
    assert_eq!(warned.len(), 2, "{warned:?}");
    assert!(warned.iter().all(|f| f.location.file.as_deref().is_some_and(|n| n.starts_with("old_"))));
    assert!(failed(&r).is_empty(), "{:?}", failed(&r));
    assert_ne!(r.verdict, Verdict::Fail);
    // the original, untouched package passes the rule
    let ok = validate(&copy("listed"));
    assert!(
        ok.findings.iter().all(|f| f.rule != "DCP.AM.007" || f.status == Status::Pass),
        "{:?}",
        ok.findings.iter().filter(|f| f.rule == "DCP.AM.007").collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&d);
}
