//! The real engine on a package written by CFA4's own DCP export. Runs only where dcp_inspect is
//! installed (`scripts/dev/setup-dcp-inspect.sh --install`, or `CFA4_DCP_INSPECT` naming its
//! wrapper); elsewhere it says so and passes, since the engine is optional.

use std::sync::Arc;

use cfa4_dcp_inspect::{BACKEND_ID, DcpInspect, ENGINE_VERSION};
use cfa4_dcp_validate::rules::{AM_DOCUMENT, AM_PRESENT, CPL_DOCUMENT, PKL_DOCUMENT, PKL_HASH};
use cfa4_dcp_validate::{DcpValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest, Verdict};
use cfa4_export::{ExportSettings, Format, Progress, export};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

/// 320×180 at 25 fps: a white matte on V1, a 440 Hz tone on A1.
fn project() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("dcp-inspect");
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
    let seq = p.new_sequence("Reference", SequenceSettings { width: 320, height: 180, frame_rate: r, ..Default::default() }, 1, 1, None);
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
fn reference_engine_reads_a_cfa4_export() {
    let Some(engine) = DcpInspect::locate() else {
        eprintln!("dcp_inspect is not installed (scripts/dev/setup-dcp-inspect.sh --install); skipping");
        return;
    };
    assert_eq!(engine.version().unwrap(), ENGINE_VERSION, "the adapter reads the result of the pinned engine only");

    let (p, seq, m) = project();
    let dir = std::env::temp_dir().join(format!("cfa4-dcp-inspect-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.to_string_lossy().into_owned(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(5))),
        dcp_content_kind: "test".into(),
        ..Default::default()
    };
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let before = listing(&dir);

    let r = engine.validate(&ValidationRequest::new(&dir, Profile::builtin("full-integrity").unwrap()), &ValidationControl::new()).unwrap();
    assert_eq!(r.backend.id, BACKEND_ID);
    assert_eq!(r.standard, Some(cfa4_dcp::Standard::Smpte));
    assert_eq!(r.documents.compositions.len(), 1);
    for rule in [AM_PRESENT, AM_DOCUMENT, PKL_DOCUMENT, CPL_DOCUMENT, PKL_HASH] {
        assert!(r.findings.iter().any(|f| f.rule == rule && f.status == Status::Pass), "{rule}: {:?}", outcome(&r));
        assert!(r.findings.iter().all(|f| f.rule != rule || f.status == Status::Pass), "{rule}: {:?}", outcome(&r));
    }
    assert_ne!(r.verdict, Verdict::Interrupted);
    let root = dir.canonicalize().unwrap().to_string_lossy().into_owned();
    assert!(r.findings.iter().all(|f| !f.message.contains(&root)), "absolute paths in the report");
    assert_eq!(listing(&dir), before, "the engine wrote into the package");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Names and sizes of the files in `dir`.
fn listing(dir: &std::path::Path) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().len())
        })
        .collect();
    v.sort();
    v
}

/// Rule and status of every finding, for assertion messages (not the messages: they hold the
/// package's ids).
fn outcome(r: &ValidationReport) -> Vec<(&str, Status)> {
    r.findings.iter().map(|f| (f.rule.as_str(), f.status)).collect()
}
