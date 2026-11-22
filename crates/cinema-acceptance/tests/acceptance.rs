//! CineCert acceptance tests on real CFA4 output (plan ch. 8, items 6, 8 and 10).
//!
//! A short DCP and IMF package are exported with `cfa4-export`, then:
//! - every track file gets an MXF-DIGEST, read twice (stable), and the IMF composition's virtual
//!   tracks get fingerprints: a resource split into two continuations keeps the fingerprint, a
//!   changed duration changes it (item 6);
//! - the DCP's content title is checked against the ISDCF naming convention;
//! - both packages are delivered with verification and archived as BagIt bags (item 8);
//! - the external test oracles of `scripts/dev/setup-cinema-oracles.sh` check them when their
//!   variables are set: `asdcp-info` (CFA4_ASDCPLIB_PREFIX), Photon (CFA4_PHOTON_CP), Clairmeta
//!   (CFA4_CLAIRMETA). `CFA4_ACCEPTANCE_REQUIRE_ORACLES=1` makes a missing oracle a failure, for the
//!   release run (item 10).
//!
//! Every run writes its evidence, a provenance report with each check, to
//! `target/tmp/cinema-acceptance/<package>.provenance.json`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cfa4_cinema_pkg::archive::{self, Check, FileHash, Provenance, Tool};
use cfa4_cinema_pkg::isdcf::{Registry, dcnc};
use cfa4_cinema_pkg::{delivery, mxf_digest, vtfp};
use cfa4_export::{ExportSettings, Format, Progress, export};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{Generator, MediaSource};
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::SourceMap;
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const FRAMES: i64 = 48;

fn project(rate: FrameRate) -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("acceptance");
    let bars = GeneratorSource::new(Generator::ColorMatte { color: [0.2, 0.5, 0.8, 1.0] }, 320, 180, rate, Tick(3 * TICKS_PER_SECOND));
    let tone = GeneratorSource::new(Generator::Tone { hz: 1000.0, db: -20.0 }, 320, 180, rate, Tick(3 * TICKS_PER_SECOND));
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
    let v = add(&mut p, &bars);
    let a = add(&mut p, &tone);
    let seq = p.new_sequence("Acceptance", SequenceSettings { width: 320, height: 180, frame_rate: rate, ..Default::default() }, 1, 1, None);
    let len = rate.tick_of(FRAMES);
    let vi = p.make_track_item(v, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, len), rate).unwrap();
    let ai = p.make_track_item(a, TrackKind::Audio, Tick::ZERO, TimeRange::new(Tick::ZERO, len), rate).unwrap();
    let q = p.sequence_mut(seq).unwrap();
    q.video_tracks[0].items.push(vi);
    q.audio_tracks[0].items.push(ai);
    let mut m = SourceMap::default();
    m.0.insert(v, Arc::new(bars));
    m.0.insert(a, Arc::new(tone));
    (Arc::new(p), seq, m)
}

fn work(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cinema-acceptance-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn utc_now() -> chrono::DateTime<chrono::Utc> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    chrono::DateTime::from_timestamp(i64::try_from(secs).unwrap_or(0), 0).unwrap_or_default()
}

fn now() -> String {
    utc_now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn check(name: &str, version: &str, profile: &str, result: &str, summary: String) -> Check {
    Check { tool: Tool { name: name.into(), version: version.into() }, profile: profile.into(), result: result.into(), at: now(), summary }
}

fn require_oracles() -> bool {
    std::env::var("CFA4_ACCEPTANCE_REQUIRE_ORACLES").as_deref() == Ok("1")
}

fn missing_oracle(var: &str) {
    assert!(!require_oracles(), "{var} is not set and CFA4_ACCEPTANCE_REQUIRE_ORACLES=1");
    eprintln!("oracle skipped: {var} is not set (bash scripts/dev/setup-cinema-oracles.sh --print-env)");
}

fn run(cmd: &mut Command) -> (bool, String) {
    match cmd.output() {
        Ok(o) => (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(e) => (false, e.to_string()),
    }
}

fn tail(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    lines[lines.len().saturating_sub(5)..].join(" | ")
}

fn files_with(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = delivery::package_files(dir).unwrap().into_iter().filter(|f| f.ends_with(ext)).map(|f| dir.join(f)).collect();
    v.sort();
    v
}

/// MXF-DIGEST of every track file, twice; the digests go into the evidence as sources.
fn digests(dir: &Path, checks: &mut Vec<Check>) -> Vec<FileHash> {
    let mxfs = files_with(dir, ".mxf");
    assert!(!mxfs.is_empty(), "no track files in {}", dir.display());
    let mut out = Vec::new();
    for f in mxfs {
        let (a, n) = mxf_digest(std::fs::File::open(&f).unwrap()).unwrap();
        let (b, _) = mxf_digest(std::fs::File::open(&f).unwrap()).unwrap();
        assert_eq!(a, b, "{}", f.display());
        assert!(n > 3);
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        checks.push(check("cfa4-cinema-pkg", env!("CARGO_PKG_VERSION"), "mxf-digest", "pass", format!("{name}: {} ({n} KLV packets)", a.to_urn())));
        out.push(FileHash { path: name, sha256: delivery::sha256_file(&f).unwrap() });
    }
    out
}

/// Deliver with verification and bag the package; both must check out.
fn deliver_and_bag(pkg: &Path, root: &Path, prov: &Provenance, checks: &mut Vec<Check>) {
    let target = root.join("delivery");
    std::fs::create_dir_all(&target).unwrap();
    let r = delivery::deliver(pkg, &target).unwrap();
    let bad = delivery::verify(&r.package, &r.manifest).unwrap();
    assert!(bad.is_empty(), "{bad:?}");
    for f in delivery::package_files(pkg).unwrap() {
        assert!(std::fs::read(pkg.join(&f)).unwrap() == std::fs::read(r.package.join(&f)).unwrap(), "{f} differs after delivery");
    }
    checks.push(check("cfa4-cinema-pkg", env!("CARGO_PKG_VERSION"), "delivery", "pass", format!("{} files, {} bytes, bit-identical", r.files, r.bytes)));
    let bag = root.join("archive").join(format!("{}-bag", prov.package));
    std::fs::create_dir_all(bag.parent().unwrap()).unwrap();
    let date = utc_now().date_naive();
    archive::make_bag(pkg, &bag, date, prov, "CFA4 CineCert acceptance package").unwrap();
    let problems = archive::validate_bag(&bag).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    checks.push(check("cfa4-cinema-pkg", env!("CARGO_PKG_VERSION"), "bagit", "pass", "valid BagIt 1.0 bag".into()));
}

fn evidence(prov: &Provenance) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cinema-acceptance");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.provenance.json", prov.package));
    std::fs::write(&path, prov.to_json().unwrap()).unwrap();
    eprintln!("evidence: {}", path.display());
}

fn provenance(package: &str, profile: &str, sources: Vec<FileHash>, checks: Vec<Check>) -> Provenance {
    Provenance {
        package: package.into(),
        created: now(),
        producer: Tool { name: "cfa4".into(), version: env!("CARGO_PKG_VERSION").into() },
        profile: profile.into(),
        transcoders: vec![Tool { name: "cfa4-export".into(), version: env!("CARGO_PKG_VERSION").into() }],
        sources,
        checks,
    }
}

fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

#[test]
fn dcp_acceptance() {
    let rate = FrameRate::new(24, 1);
    let (p, seq, m) = project(rate);
    let root = work("dcp");
    let dir = root.join("Acceptance_TST-1_F_EN-XX_INT-TL_51_2K_NULL_20261115_NUL_SMPTE_OV");
    let s = ExportSettings {
        format: Format::Dcp,
        path: dir.to_string_lossy().into(),
        range: Some(TimeRange::new(Tick::ZERO, rate.tick_of(FRAMES))),
        dcp_content_kind: "test".into(),
        dcp_language: "en".into(),
        ..Default::default()
    };
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let mut checks = Vec::new();
    let sources = digests(&dir, &mut checks);

    // ISDCF naming of the export's folder name and of the content title it writes into the CPL
    // (the sequence name). Reported, not enforced: the convention is a recommendation, and the
    // export's names are PROCODEC-002's (its builder leaves out empty territory, studio and
    // facility fields, as DCP-o-matic does, where the convention keeps all twelve).
    let reg = Registry::builtin().unwrap();
    let dcnc_result = |what: &str, name: &str, checks: &mut Vec<Check>| {
        let f = dcnc::check(name, reg);
        let summary =
            if f.is_empty() { format!("{name}: no findings") } else { format!("{name}: {}", f.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")) };
        eprintln!("DCNC {what}: {summary}");
        checks.push(check("cfa4-cinema-pkg", env!("CARGO_PKG_VERSION"), &format!("isdcf-dcnc ({what})"), if f.is_empty() { "pass" } else { "warn" }, summary));
    };
    dcnc_result("export folder name", &cfa4_export::dcp_isdcf_name(&s, "Acceptance", "20261115"), &mut checks);
    let cpl = files_with(&dir, ".xml").into_iter().find(|f| f.file_name().unwrap().to_string_lossy().to_ascii_lowercase().starts_with("cpl")).unwrap();
    let xml = std::fs::read_to_string(&cpl).unwrap();
    let title = element(&xml, "ContentTitleText").unwrap().to_string();
    dcnc_result("ContentTitleText", &title, &mut checks);

    // asdcplib: every track file must be readable by asdcp-info.
    match std::env::var("CFA4_ASDCPLIB_PREFIX") {
        Ok(prefix) => {
            let bin = Path::new(&prefix).join("bin/asdcp-info");
            for f in files_with(&dir, ".mxf") {
                let (ok, log) = run(Command::new(&bin).arg("-i").arg(&f));
                assert!(ok, "asdcp-info {}: {log}", f.display());
                checks.push(check(
                    "asdcplib",
                    "2.13.2",
                    "asdcp-info -i",
                    if ok { "pass" } else { "fail" },
                    format!("{}: {}", f.file_name().unwrap().to_string_lossy(), tail(&log)),
                ));
            }
        }
        Err(_) => missing_oracle("CFA4_ASDCPLIB_PREFIX"),
    }
    // Clairmeta: the whole DCP.
    match std::env::var("CFA4_CLAIRMETA") {
        Ok(cm) => {
            let (ok, log) = run(Command::new(&cm).args(["check", "-type", "dcp"]).arg(&dir));
            checks.push(check("clairmeta", "1.6.3", "check -type dcp", if ok { "pass" } else { "fail" }, tail(&log)));
            assert!(ok, "clairmeta: {log}");
        }
        Err(_) => missing_oracle("CFA4_CLAIRMETA"),
    }

    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    let prov = provenance(&name, "DCP SMPTE 2K flat 24 fps, 48 frames", sources, checks.clone());
    deliver_and_bag(&dir, &root, &prov, &mut checks);
    evidence(&provenance(&name, &prov.profile, prov.sources.clone(), checks));
    std::fs::remove_dir_all(&root).unwrap();
}

/// Rewrite the first resource of the CPL's `MainImageSequence`: `parts` gives (entry point,
/// duration) pairs, one resource each, all pointing at the same track file.
fn resplit(cpl: &str, parts: &[(i64, i64)]) -> String {
    let seq = cpl.find("MainImageSequence>").unwrap();
    let start = seq + [cpl[seq..].find("<Resource "), cpl[seq..].find("<Resource>")].into_iter().flatten().min().unwrap();
    let end = start + cpl[start..].find("</Resource>").unwrap() + "</Resource>".len();
    let mut res = cpl[start..end].to_string();
    for tag in ["EntryPoint", "SourceDuration"] {
        if let (Some(a), Some(b)) = (res.find(&format!("<{tag}>")), res.find(&format!("</{tag}>"))) {
            res.replace_range(a..b + tag.len() + 3, "");
        }
    }
    let at = res.find("</IntrinsicDuration>").unwrap() + "</IntrinsicDuration>".len();
    let pieces: String = parts
        .iter()
        .map(|(e, d)| {
            let mut r = res.clone();
            r.insert_str(at, &format!("<EntryPoint>{e}</EntryPoint><SourceDuration>{d}</SourceDuration>"));
            r
        })
        .collect();
    format!("{}{pieces}{}", &cpl[..start], &cpl[end..])
}

#[test]
fn imf_acceptance() {
    let rate = FrameRate::new(24, 1);
    let (p, seq, m) = project(rate);
    let root = work("imf");
    let dir = root.join("Acceptance_IMF");
    let s = ExportSettings {
        format: Format::Imf,
        path: dir.to_string_lossy().into(),
        range: Some(TimeRange::new(Tick::ZERO, rate.tick_of(FRAMES))),
        imf_content_kind: "test".into(),
        dcp_language: "en".into(),
        ..Default::default()
    };
    export(&p, seq, &s, &m, &Progress::default()).unwrap();
    let mut checks = Vec::new();
    let sources = digests(&dir, &mut checks);

    // Virtual track fingerprints of CFA4's own composition (item 6).
    let cpl_path = files_with(&dir, ".xml").into_iter().find(|f| f.file_name().unwrap().to_string_lossy().to_ascii_lowercase().starts_with("cpl")).unwrap();
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    let tracks = vtfp::virtual_tracks(&cpl).unwrap();
    assert!(tracks.iter().any(|t| t.kind == "MainImageSequence"), "{tracks:?}");
    for t in &tracks {
        let fp = vtfp::fingerprint(&cpl, &t.track_id).unwrap();
        checks.push(check("cfa4-cinema-pkg", env!("CARGO_PKG_VERSION"), "imf-vtfp", "pass", format!("{} {}: {}", t.kind, t.track_id, fp.to_urn())));
    }
    let image = tracks.iter().find(|t| t.kind == "MainImageSequence").unwrap();
    let whole = vtfp::fingerprint(&resplit(&cpl, &[(0, FRAMES)]), &image.track_id).unwrap();
    assert_eq!(vtfp::fingerprint(&cpl, &image.track_id).unwrap(), whole, "explicit defaults keep the fingerprint");
    let split = vtfp::fingerprint(&resplit(&cpl, &[(0, 20), (20, FRAMES - 20)]), &image.track_id).unwrap();
    assert_eq!(split, whole, "the same timeline in two resources");
    let trimmed = vtfp::fingerprint(&resplit(&cpl, &[(0, FRAMES - 1)]), &image.track_id).unwrap();
    assert_ne!(trimmed, whole, "a shorter timeline");
    let shifted = vtfp::fingerprint(&resplit(&cpl, &[(1, FRAMES - 1)]), &image.track_id).unwrap();
    assert_ne!(shifted, trimmed, "other edit units");

    // Photon: the whole IMP.
    match std::env::var("CFA4_PHOTON_CP") {
        Ok(cp) => {
            let (_, log) = run(Command::new("java").args(["-cp", &cp, "com.netflix.imflibrary.app.IMPAnalyzer"]).arg(&dir));
            let ok = log.contains("has no errors") && !log.contains("ERROR");
            checks.push(check("photon", "5.0.1", "IMPAnalyzer", if ok { "pass" } else { "fail" }, tail(&log)));
            assert!(ok, "photon: {log}");
        }
        Err(_) => missing_oracle("CFA4_PHOTON_CP"),
    }

    let prov = provenance("Acceptance_IMF", "IMF App 2E 24 fps, 48 frames", sources, checks.clone());
    deliver_and_bag(&dir, &root, &prov, &mut checks);
    evidence(&provenance("Acceptance_IMF", &prov.profile, prov.sources.clone(), checks));
    std::fs::remove_dir_all(&root).unwrap();
}
