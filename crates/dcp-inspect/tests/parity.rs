//! DCP-17: native and reference engine side by side. The same damaged packages (structural cases,
//! as in the DCP-16 matrix) go through the native backend and through dcp_inspect, and the verdict and
//! the rules both of them judge are compared. The comparison is written to
//! `target/dcp-parity/report.md`; it is a measurement, so only the valid package is asserted (neither
//! engine may fail it). A case where one engine fails and the other passes is listed as a divergence
//! for the maintainer to read; it is not a test failure. Runs only where dcp_inspect is installed
//! (`scripts/dev/setup-dcp-inspect.sh --install`, or `CFA4_DCP_INSPECT`); elsewhere it says so and passes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfa4_dcp_inspect::DcpInspect;
use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest};
use cfa4_export::{ExportSettings, Format, Progress, export};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const PROFILE: &str = "full-integrity";

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

fn export_valid(dir: &Path) {
    let (p, seq, m) = project();
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.to_string_lossy().into_owned(),
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::new(25, 1).tick_of(5))),
        dcp_content_kind: "test".into(),
        ..Default::default()
    };
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
}

fn file(d: &Path, prefix: &str) -> PathBuf {
    let hit = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with(prefix));
    d.join(hit.unwrap_or_else(|| panic!("no {prefix}* in the package")))
}

type Damage = fn(&Path);

/// Structural damage cases (the media cases need the engine's asdcplib).
const CASES: &[(&str, Damage)] = &[
    ("valid package", |_| {}),
    ("asset map is not XML", |d| std::fs::write(d.join("ASSETMAP.xml"), "<AssetMap><Id>").unwrap()),
    ("asset map is missing", |d| std::fs::remove_file(d.join("ASSETMAP.xml")).unwrap()),
    ("packing list is missing", |d| std::fs::remove_file(file(d, "pkl_")).unwrap()),
    ("packing list is not XML", |d| std::fs::write(file(d, "pkl_"), "not xml <<").unwrap()),
    ("composition is not XML", |d| std::fs::write(file(d, "cpl_"), "<CompositionPlaylist>").unwrap()),
    ("sound track file is missing", |d| std::fs::remove_file(file(d, "pcm_")).unwrap()),
    ("a changed byte (bad hash)", |d| {
        let p = file(d, "pcm_");
        let mut b = std::fs::read(&p).unwrap();
        let i = b.len() - 1;
        b[i] ^= 0xFF;
        std::fs::write(p, b).unwrap();
    }),
    ("a truncated track file (bad size)", |d| {
        let p = file(d, "pcm_");
        let b = std::fs::read(&p).unwrap();
        std::fs::write(p, &b[..b.len() - 16]).unwrap();
    }),
];

fn copy(from: &Path, name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-parity-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        std::fs::copy(e.path(), d.join(e.file_name())).unwrap();
    }
    d
}

fn failed(r: &ValidationReport) -> Vec<String> {
    let mut v: Vec<String> = r.findings.iter().filter(|f| f.status == Status::Fail).map(|f| f.rule.clone()).collect();
    v.dedup();
    v
}

#[test]
fn native_and_reference_engine_are_compared() {
    let Some(engine) = DcpInspect::locate() else {
        eprintln!("dcp_inspect is not installed (scripts/dev/setup-dcp-inspect.sh --install); skipping");
        return;
    };
    let base = std::env::temp_dir().join(format!("cfa4-dcp-parity-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    export_valid(&base);

    let profile = || Profile::builtin(PROFILE).unwrap();
    let mut md = String::from(
        "# DCP parity: native backend and dcp_inspect\n\n| Case | Native | Reference | Rules failed by native only | Rules failed by reference only |\n|---|---|---|---|---|\n",
    );
    let mut diverged = 0;
    for (name, damage) in CASES {
        let d = copy(&base, "case");
        damage(&d);
        let req = || ValidationRequest::new(&d, profile());
        let native = NativeValidator.validate(&req(), &ValidationControl::new()).unwrap();
        let reference = engine.validate(&req(), &ValidationControl::new());
        let _ = std::fs::remove_dir_all(&d);
        let Ok(reference) = reference else {
            md.push_str(&format!("| {name} | {:?} | not run ({}) | | |\n", native.verdict, reference.err().map(|e| e.to_string()).unwrap_or_default()));
            continue;
        };
        let (n, r) = (failed(&native), failed(&reference));
        let only_n: Vec<&String> = n.iter().filter(|x| !r.contains(x)).collect();
        let only_r: Vec<&String> = r.iter().filter(|x| !n.contains(x)).collect();
        if (native.verdict == cfa4_dcp_validate::Verdict::Fail) != (reference.verdict == cfa4_dcp_validate::Verdict::Fail) {
            diverged += 1;
        }
        md.push_str(&format!("| {name} | {:?} | {:?} | {only_n:?} | {only_r:?} |\n", native.verdict, reference.verdict));
        if *name == "valid package" {
            assert!(n.is_empty() && r.is_empty(), "neither engine may fail the valid package: native {n:?}, reference {r:?}");
        }
    }
    md.push_str(&format!("\nVerdicts that differ (one FAIL, the other not): {diverged}\n"));
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target").join("dcp-parity");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("report.md"), &md).unwrap();
    eprintln!("{md}");
    let _ = std::fs::remove_dir_all(&base);
}
