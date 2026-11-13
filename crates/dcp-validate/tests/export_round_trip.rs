//! A package written by CFA4's own DCP export (`cfa4-export`, `dcp_out.rs`) passes the native Quick
//! Scan and Full Integrity, and every check the native backend implements passes on it.

use std::sync::Arc;

use cfa4_dcp_validate::rules::{AM_PRESENT, MARK_MARKERS, PKL_HASH, STD_CONSISTENT};
use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest, Verdict};
use cfa4_export::{ExportSettings, Format, Progress, export};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

/// 320×180 at 25 fps: a white matte on V1, a 440 Hz tone on A1 (as in the export's own DCP tests).
fn project() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("dcp-validate");
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
    let seq = p.new_sequence("Round <Trip> & Co", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
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

#[test]
fn cfa4_dcp_export_passes_the_native_quick_scan() {
    let (p, seq, m) = project();
    let dir = std::env::temp_dir().join(format!("cfa4-dcp-validate-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.to_string_lossy().into_owned(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(5))),
        dcp_content_kind: "test".into(),
        ..Default::default()
    };
    export(&p, seq, &s, &m, &Progress::default()).unwrap();

    let run = |profile: &str| NativeValidator.validate(&ValidationRequest::new(&dir, Profile::builtin(profile).unwrap()), &ValidationControl::new()).unwrap();
    let quick = run("quick-scan");
    assert_eq!(quick.verdict, Verdict::Pass, "{:?}", outcome(&quick));
    assert_eq!(quick.standard, Some(cfa4_dcp::Standard::Smpte));
    assert_eq!(quick.documents.packing_lists.len(), 1);
    assert_eq!(quick.documents.compositions.len(), 1);
    assert!(quick.documents.asset_map.is_some());
    // picture, sound, CPL, PKL
    assert!(quick.assets_checked >= 4, "{}", quick.assets_checked);
    assert!(quick.findings.iter().all(|f| f.status == Status::Pass), "{:?}", outcome(&quick));

    // DCP-05: sizes, SHA-1 hashes, the packing list and composition consistency, the markers
    let full = run("full-integrity");
    assert_eq!(full.verdict, Verdict::Pass, "{:?}", outcome(&full));
    assert!(full.findings.iter().any(|f| f.rule == PKL_HASH && f.status == Status::Pass));

    let smpte = run("smpte-cinema");
    assert!(smpte.findings.iter().any(|f| f.rule == STD_CONSISTENT && f.status == Status::Pass));
    assert!(smpte.findings.iter().any(|f| f.rule == AM_PRESENT && f.status == Status::Pass));
    assert!(smpte.findings.iter().any(|f| f.rule == MARK_MARKERS && f.status == Status::Pass));
    // the rules of later work packages are reported, so the profile is not passed yet
    assert_eq!(smpte.verdict, Verdict::Incomplete, "{:?}", outcome(&smpte));
    assert_eq!(smpte.counts.fail, 0, "{:?}", outcome(&smpte));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Rule and status of every finding, for assertion messages. The report text is not printed: it
/// holds the package's ids, and test logs are not the place for them.
fn outcome(r: &ValidationReport) -> Vec<(&str, Status)> {
    r.findings.iter().map(|f| (f.rule.as_str(), f.status)).collect()
}
