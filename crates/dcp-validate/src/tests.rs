use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::rules::*;

const AM_NS: &str = "http://www.smpte-ra.org/schemas/429-9/2007/AM";
const H: &str = "2jmj7l5rSw0yVb/vlWAYkK/YBwk=";

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-validate-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn uuid(n: u32) -> String {
    format!("urn:uuid:{n:08x}-2222-4333-8444-555555555555")
}

/// A small SMPTE package: an asset map, a packing list, a playlist and a picture file. `paths`
/// replaces the chunk paths (asset 1 PKL, 2 CPL, 3 picture).
fn package(root: &Path, paths: [&str; 3]) {
    package_with(root, paths, Edit::default());
}

/// Text replacements applied to the documents before they are written; the packing list's hash
/// and size of the CPL are of the edited CPL.
#[derive(Default)]
struct Edit {
    am: Vec<(&'static str, String)>,
    pkl: Vec<(&'static str, String)>,
    cpl: Vec<(&'static str, String)>,
    /// The picture file (`MXF!` when `None`).
    picture: Option<Vec<u8>>,
}

fn edit(mut text: String, edits: &[(&'static str, String)]) -> String {
    for (from, to) in edits {
        assert!(text.contains(from), "{from} is not in the document");
        text = text.replacen(from, to, 1);
    }
    text
}

fn package_with(root: &Path, paths: [&str; 3], e: Edit) {
    let am = format!(
        r#"<AssetMap xmlns="{AM_NS}"><Id>{}</Id><Creator>t</Creator><VolumeCount>1</VolumeCount><IssueDate>2026-11-09T00:00:00Z</IssueDate><Issuer>t</Issuer><AssetList>
<Asset><Id>{}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>{}</Path></Chunk></ChunkList></Asset>
<Asset><Id>{}</Id><ChunkList><Chunk><Path>{}</Path></Chunk></ChunkList></Asset>
<Asset><Id>{}</Id><ChunkList><Chunk><Path>{}</Path></Chunk></ChunkList></Asset>
</AssetList></AssetMap>"#,
        uuid(9),
        uuid(1),
        paths[0],
        uuid(2),
        paths[1],
        uuid(3),
        paths[2]
    );
    let am = edit(am, &e.am);
    let cpl = format!(
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"><Id>{}</Id><IssueDate>2026-11-09T00:00:00Z</IssueDate><ContentTitleText>T</ContentTitleText><ContentKind>test</ContentKind><ContentVersion><Id>urn:uuid:00000005-2222-4333-8444-555555555555</Id><LabelText>T</LabelText></ContentVersion><ReelList><Reel><Id>{}</Id><AssetList>
<MainMarkers><Id>{}</Id><EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><MarkerList><Marker><Label>FFOC</Label><Offset>0</Offset></Marker><Marker><Label>LFOC</Label><Offset>0</Offset></Marker></MarkerList></MainMarkers>
<MainPicture><Id>{}</Id><EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><Duration>1</Duration><FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>
</AssetList></Reel></ReelList></CompositionPlaylist>"#,
        uuid(2),
        uuid(4),
        uuid(6),
        uuid(3)
    );
    let cpl = edit(cpl, &e.cpl);
    let picture: &[u8] = e.picture.as_deref().unwrap_or(b"MXF!");
    let digest = |b: &[u8]| cfa4_dcp::sha1::base64(&cfa4_dcp::sha1::sha1(b));
    let pkl = format!(
        r#"<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL"><Id>{}</Id><IssueDate>2026-11-09T00:00:00Z</IssueDate><Issuer>t</Issuer><Creator>t</Creator><AssetList>
<Asset><Id>{}</Id><Hash>{}</Hash><Size>{}</Size><Type>text/xml</Type></Asset>
<Asset><Id>{}</Id><Hash>{}</Hash><Size>{}</Size><Type>application/mxf</Type></Asset>
</AssetList></PackingList>"#,
        uuid(1),
        uuid(2),
        digest(cpl.as_bytes()),
        cpl.len(),
        uuid(3),
        digest(picture),
        picture.len()
    );
    let pkl = edit(pkl, &e.pkl);
    std::fs::write(root.join("ASSETMAP.xml"), am).unwrap();
    std::fs::write(root.join("pkl.xml"), pkl).unwrap();
    std::fs::write(root.join("cpl.xml"), cpl).unwrap();
    std::fs::write(root.join("picture.mxf"), picture).unwrap();
}

const GOOD: [&str; 3] = ["pkl.xml", "cpl.xml", "picture.mxf"];

fn validate(root: &Path, profile: &str) -> ValidationReport {
    let req = ValidationRequest::new(root, Profile::builtin(profile).unwrap());
    NativeValidator.validate(&req, &ValidationControl::new()).unwrap()
}

fn statuses(r: &ValidationReport, rule: &str) -> Vec<Status> {
    r.findings.iter().filter(|f| f.rule == rule).map(|f| f.status).collect()
}

#[test]
fn quick_scan_of_a_good_package_passes() {
    let d = dir("good");
    package(&d, GOOD);
    let r = validate(&d, "quick-scan");
    assert_eq!(r.verdict, Verdict::Pass, "{:?}", outcome(&r));
    assert_eq!(r.standard, Some(cfa4_dcp::Standard::Smpte));
    assert_eq!(r.documents.packing_lists, [cfa4_dcp::Uuid::parse(&uuid(1)).unwrap()]);
    assert_eq!(r.documents.compositions, [cfa4_dcp::Uuid::parse(&uuid(2)).unwrap()]);
    assert_eq!(r.assets_checked, 3);
    assert_eq!(r.counts.pass, 6);
    assert!(r.findings.iter().all(|f| f.status == Status::Pass));
}

#[test]
fn incomplete_checks_never_pass() {
    let d = dir("incomplete");
    package_with(&d, GOOD, Edit { picture: Some(good_track_file()), ..Edit::default() });
    let r = validate(&d, "security");
    assert_eq!(r.verdict, Verdict::Incomplete, "{:?}", outcome(&r));
    assert_eq!(statuses(&r, SIG_SIGNATURE), [Status::Unchecked]);
    assert!(r.findings.iter().any(|f| f.rule == SIG_SIGNATURE && f.message.contains("DCP-10")));
    // every profile rule is reported
    let r = validate(&d, "smpte-cinema");
    let p = Profile::builtin("smpte-cinema").unwrap();
    for pr in &p.rules {
        assert!(r.findings.iter().any(|f| f.rule == pr.rule), "{} missing", pr.rule);
    }
}

#[test]
fn standard_of_the_profile() {
    let d = dir("standard");
    package(&d, GOOD);
    assert_eq!(statuses(&validate(&d, "smpte-cinema"), STD_CONSISTENT), [Status::Pass]);
    let r = validate(&d, "interop-cinema");
    assert_eq!(statuses(&r, STD_CONSISTENT), [Status::Fail; 3]);
    assert_eq!(r.verdict, Verdict::Fail);
}

#[test]
fn missing_asset_map() {
    let d = dir("no-am");
    let r = validate(&d, "quick-scan");
    assert_eq!(r.verdict, Verdict::Fail);
    assert_eq!(statuses(&r, AM_PRESENT), [Status::Fail]);
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Unchecked]);
}

#[test]
fn interop_asset_map_name_and_both_names() {
    let d = dir("both");
    package(&d, GOOD);
    std::fs::copy(d.join("ASSETMAP.xml"), d.join("ASSETMAP")).unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, AM_PRESENT), [Status::Warning]);
    assert_eq!(r.verdict, Verdict::Pass, "a warning does not fail");
    // a SMPTE asset map under the Interop name
    std::fs::remove_file(d.join("ASSETMAP.xml")).unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, AM_PRESENT), [Status::Fail]);
    assert_eq!(r.verdict, Verdict::Fail);
    // an Interop asset map under ASSETMAP passes; under ASSETMAP.xml it is a warning
    let am = std::fs::read_to_string(d.join("ASSETMAP"))
        .unwrap()
        .replace(AM_NS, cfa4_dcp::ns::INTEROP_AM)
        .replace("<PackingList>true</PackingList>", "<PackingList/>");
    std::fs::write(d.join("ASSETMAP"), &am).unwrap();
    assert_eq!(statuses(&validate(&d, "quick-scan"), AM_PRESENT), [Status::Pass]);
    std::fs::rename(d.join("ASSETMAP"), d.join("ASSETMAP.xml")).unwrap();
    assert_eq!(statuses(&validate(&d, "quick-scan"), AM_PRESENT), [Status::Warning]);
}

#[test]
fn unsafe_and_missing_paths() {
    let d = dir("paths");
    package(&d, ["pkl.xml", "../cpl.xml", "/etc/hostname"]);
    let r = validate(&d, "quick-scan");
    assert_eq!(r.verdict, Verdict::Fail);
    assert_eq!(statuses(&r, AM_PATH_SAFE), [Status::Fail, Status::Fail]);
    let f = r.findings.iter().find(|f| f.rule == AM_PATH_SAFE).unwrap();
    assert_eq!(f.location.file.as_deref(), Some("ASSETMAP.xml"));
    assert_eq!(f.location.line, Some(3));
    assert_eq!(f.location.asset, cfa4_dcp::Uuid::parse(&uuid(2)));
    // the CPL was refused, so it could not be opened
    assert_eq!(statuses(&r, CPL_DOCUMENT), [Status::Unchecked]);

    let d = dir("missing");
    package(&d, ["pkl.xml", "cpl.xml", "gone.mxf"]);
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, AM_FILE_EXISTS), [Status::Fail]);
    assert_eq!(statuses(&r, AM_PATH_SAFE), [Status::Pass]);

    let d = dir("dir");
    package(&d, ["pkl.xml", "cpl.xml", "sub"]);
    std::fs::create_dir(d.join("sub")).unwrap();
    assert_eq!(statuses(&validate(&d, "quick-scan"), AM_FILE_EXISTS), [Status::Fail]);
}

#[cfg(unix)]
#[test]
fn symbolic_links_stay_inside() {
    let outside = dir("outside");
    std::fs::write(outside.join("secret.mxf"), b"x").unwrap();
    let d = dir("links");
    package(&d, ["pkl.xml", "cpl.xml", "out.mxf"]);
    std::os::unix::fs::symlink(outside.join("secret.mxf"), d.join("out.mxf")).unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, AM_PATH_SAFE), [Status::Fail]);
    // a link inside the package is fine
    let d = dir("links-in");
    package(&d, ["pkl.xml", "cpl.xml", "link.mxf"]);
    std::os::unix::fs::symlink(d.join("picture.mxf"), d.join("link.mxf")).unwrap();
    assert_eq!(validate(&d, "quick-scan").verdict, Verdict::Pass);
}

#[test]
fn broken_documents_are_findings() {
    let d = dir("broken");
    package(&d, GOOD);
    let pkl = std::fs::read_to_string(d.join("pkl.xml")).unwrap();
    std::fs::write(d.join("pkl.xml"), pkl.replace("<Size>4</Size>", "<Size>four</Size><Extra/>")).unwrap();
    std::fs::write(d.join("cpl.xml"), "<CompositionPlaylist").unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Fail, Status::Warning]);
    let f = r.findings.iter().find(|f| f.rule == PKL_DOCUMENT).unwrap();
    assert_eq!(f.location.element.as_deref(), Some("PackingList/AssetList/Asset[2]/Size"));
    assert!(f.location.line.is_some());
    assert!(statuses(&r, CPL_DOCUMENT).iter().all(|s| *s == Status::Fail));
    assert_eq!(r.verdict, Verdict::Fail);
}

#[test]
fn no_packing_list() {
    let d = dir("no-pkl");
    package(&d, GOOD);
    let am = std::fs::read_to_string(d.join("ASSETMAP.xml")).unwrap();
    std::fs::write(d.join("ASSETMAP.xml"), am.replace("<PackingList>true</PackingList>", "")).unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Fail]);
}

#[test]
fn cancellation_interrupts() {
    let d = dir("cancel");
    package(&d, GOOD);
    let c = ValidationControl::new();
    c.cancel();
    let r = NativeValidator.validate(&ValidationRequest::new(&d, Profile::builtin("quick-scan").unwrap()), &c).unwrap();
    assert_eq!(r.verdict, Verdict::Interrupted);
    assert!(r.counts.interrupted > 0);
    // a cancel is reported even when discovery stops early
    let r = NativeValidator.validate(&ValidationRequest::new(dir("cancel-no-am"), Profile::builtin("quick-scan").unwrap()), &c).unwrap();
    assert_eq!(r.verdict, Verdict::Interrupted);
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Interrupted]);
}

#[test]
fn progress_is_reported() {
    let d = dir("progress");
    package(&d, GOOD);
    let n = std::sync::Arc::new(AtomicUsize::new(0));
    let stages = std::sync::Arc::new(Mutex::new(Vec::new()));
    let (n2, s2) = (n.clone(), stages.clone());
    let c = ValidationControl::new().with_progress(move |p| {
        n2.fetch_add(1, Ordering::Relaxed);
        s2.lock().unwrap().push(p.stage);
    });
    NativeValidator.validate(&ValidationRequest::new(&d, Profile::builtin("quick-scan").unwrap()), &c).unwrap();
    assert!(n.load(Ordering::Relaxed) >= 2);
    assert!(stages.lock().unwrap().iter().any(|s| s == "documents"));
}

#[test]
fn root_must_be_a_directory() {
    let d = dir("file-root");
    std::fs::write(d.join("f"), b"").unwrap();
    for root in [d.join("f"), d.join("nothing")] {
        let req = ValidationRequest::new(root, Profile::builtin("quick-scan").unwrap());
        assert!(matches!(NativeValidator.validate(&req, &ValidationControl::new()), Err(ValidationError::NotADirectory(_))));
    }
}

#[test]
fn verdict_rules() {
    let p = Profile {
        id: "t".into(),
        version: 1,
        title: "t".into(),
        standard: None,
        rules: vec![ProfileRule { rule: "A".into(), required: true }, ProfileRule { rule: "B".into(), required: false }],
    };
    let f = |rule: &str, s| Finding::new(rule, s, "");
    assert_eq!(Verdict::of(&p, &[f("A", Status::Pass)]), Verdict::Pass);
    assert_eq!(Verdict::of(&p, &[f("A", Status::NotApplicable), f("B", Status::Fail)]), Verdict::Pass, "optional rules never decide");
    assert_eq!(Verdict::of(&p, &[f("A", Status::Pass), f("A", Status::Fail)]), Verdict::Fail);
    assert_eq!(Verdict::of(&p, &[f("A", Status::Unchecked)]), Verdict::Incomplete);
    assert_eq!(Verdict::of(&p, &[f("A", Status::Skipped)]), Verdict::Incomplete);
    assert_eq!(Verdict::of(&p, &[]), Verdict::Incomplete, "a required rule without a finding is unchecked");
    assert_eq!(Verdict::of(&p, &[f("A", Status::Fail), f("B", Status::Interrupted)]), Verdict::Interrupted);
}

#[test]
fn qc_mapping() {
    assert_eq!(Status::Pass.qc(), Some(QcStatus::Pass));
    assert_eq!(Status::Warning.qc(), Some(QcStatus::Warn));
    assert_eq!(Status::Fail.qc(), Some(QcStatus::Fail));
    for s in [Status::Skipped, Status::Unchecked, Status::Interrupted] {
        assert_eq!(s.qc(), Some(QcStatus::NotRun));
    }
    assert_eq!(Status::NotApplicable.qc(), None);
}

#[test]
fn builtin_profiles() {
    let all = Profile::all();
    assert_eq!(all.len(), 8);
    for p in &all {
        let mut ids: Vec<&str> = p.rules.iter().map(|r| r.rule.as_str()).collect();
        for id in &ids {
            assert!(rules::find(id).is_some(), "{}: unknown rule {id}", p.id);
        }
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "{}: a rule twice", p.id);
        assert!(p.rules.iter().any(|r| r.rule == AM_PRESENT && r.required));
    }
    assert!(Profile::builtin("nope").is_none());
    let quick = Profile::builtin("quick-scan").unwrap();
    assert!(!quick.contains(PKL_HASH), "Quick Scan never claims full integrity");
    let mut ids: Vec<&str> = rules::RULES.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), rules::RULES.len());
}

#[test]
fn report_json_and_markdown() {
    let d = dir("report");
    package(&d, ["pkl.xml", "cpl.xml", "a|b.mxf"]);
    let r = validate(&d, "full-integrity");
    let json = r.to_json();
    assert!(json.contains("\"schema\": \"cfa4-dcp-report/1\""));
    assert!(json.contains("\"verdict\": \"FAIL\""));
    assert!(json.contains("\"status\": \"UNCHECKED\""));
    assert_eq!(ValidationReport::from_json(&json), Some(r.clone()));
    assert_eq!(ValidationReport::from_json(&json.replace("cfa4-dcp-report/1", "other/1")), None);
    let md = r.to_markdown();
    assert!(md.starts_with("# DCP validation: Fail"));
    assert!(md.contains("a\\|b.mxf"));
    let first = md.lines().position(|l| l.starts_with("| Fail")).unwrap();
    let pass = md.lines().position(|l| l.starts_with("| Pass")).unwrap();
    assert!(first < pass, "failures first");
    assert!(r.by_rule().contains_key(AM_FILE_EXISTS));
}

#[test]
fn timestamps() {
    assert_eq!(report::rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(report::rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(report::rfc3339(1_794_258_022), "2026-11-09T21:00:22Z");
    assert_eq!(report::now().len(), 20);
}

#[test]
fn capabilities() {
    let c = NativeValidator.capabilities();
    assert_eq!(c.id, "native");
    assert!(c.rules.iter().all(|r| rules::find(r).is_some()));
}

#[test]
fn documents_over_the_limit_are_unchecked_not_passed() {
    let d = dir("doc-limit");
    package(&d, GOOD);
    let run = |max_documents| {
        let mut req = ValidationRequest::new(&d, Profile::builtin("quick-scan").unwrap());
        req.limits.max_documents = max_documents;
        NativeValidator.validate(&req, &ValidationControl::new()).unwrap()
    };
    let r = run(1);
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Pass]);
    assert_eq!(statuses(&r, CPL_DOCUMENT), [Status::Unchecked]);
    assert_eq!(r.verdict, Verdict::Incomplete);
    let r = run(0);
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Unchecked]);
    assert_eq!(r.verdict, Verdict::Incomplete);
}

#[test]
fn limits_and_unopened_documents_are_unchecked() {
    let d = dir("asset-limit");
    package(&d, GOOD);
    let mut req = ValidationRequest::new(&d, Profile::builtin("quick-scan").unwrap());
    req.limits.max_assets = 2;
    let r = NativeValidator.validate(&req, &ValidationControl::new()).unwrap();
    assert_eq!(statuses(&r, AM_PATH_SAFE), [Status::Unchecked]);
    assert_eq!(statuses(&r, AM_FILE_EXISTS), [Status::Unchecked]);
    assert_ne!(r.verdict, Verdict::Pass);

    let d = dir("pkl-gone");
    package(&d, ["gone.xml", "cpl.xml", "picture.mxf"]);
    std::fs::remove_file(d.join("pkl.xml")).unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, AM_FILE_EXISTS), [Status::Fail]);
    assert_eq!(statuses(&r, PKL_DOCUMENT), [Status::Unchecked]);
    assert_eq!(r.verdict, Verdict::Fail);
}

/// Rule and status of every finding, for assertion messages. The report text is not printed: it
/// holds the package's ids, and test logs are not the place for them.
fn outcome(r: &ValidationReport) -> Vec<(&str, Status)> {
    r.findings.iter().map(|f| (f.rule.as_str(), f.status)).collect()
}

#[test]
fn duplicate_chunk_paths_fail() {
    let d = dir("dup-path");
    package(&d, ["pkl.xml", "cpl.xml", "cpl.xml"]);
    let r = validate(&d, "quick-scan");
    assert_eq!(statuses(&r, AM_DOCUMENT), [Status::Fail]);
    assert_eq!(r.verdict, Verdict::Fail);
}

#[test]
fn xml_media_type_is_case_insensitive() {
    let d = dir("mime-case");
    package(&d, GOOD);
    let pkl = std::fs::read_to_string(d.join("pkl.xml")).unwrap();
    std::fs::write(d.join("pkl.xml"), pkl.replace("text/xml", "Text/XML;charset=UTF-8")).unwrap();
    let r = validate(&d, "quick-scan");
    assert_eq!(r.verdict, Verdict::Pass, "{:?}", outcome(&r));
    assert_eq!(r.documents.compositions, [cfa4_dcp::Uuid::parse(&uuid(2)).unwrap()]);
}

fn edited(name: &str, e: Edit, profile: &str) -> ValidationReport {
    let d = dir(name);
    package_with(&d, GOOD, e);
    validate(&d, profile)
}

#[test]
fn full_integrity_of_a_good_package_passes() {
    let r = edited("integrity-good", Edit::default(), "full-integrity");
    assert_eq!(r.verdict, Verdict::Pass, "{:?}", outcome(&r));
    for rule in [AM_UNIQUE_IDS, AM_HAS_PKL, PKL_IN_ASSETMAP, PKL_SIZE, PKL_HASH, CPL_IN_PKL, CPL_ASSETS_IN_PKL, CPL_DURATIONS, CPL_REEL_SYNC] {
        assert_eq!(statuses(&r, rule), [Status::Pass], "{rule}");
    }
    assert!(r.findings.iter().any(|f| f.rule == PKL_HASH && f.message.starts_with("2 files")));
    // Quick Scan does not hash
    let quick = edited("integrity-quick", Edit::default(), "quick-scan");
    assert!(statuses(&quick, PKL_HASH).is_empty());
}

#[test]
fn size_and_hash_mismatches_fail() {
    let d = dir("hash");
    package(&d, GOOD);
    std::fs::write(d.join("picture.mxf"), b"MXF?").unwrap();
    let r = validate(&d, "full-integrity");
    assert_eq!(statuses(&r, PKL_SIZE), [Status::Pass]);
    assert_eq!(statuses(&r, PKL_HASH), [Status::Fail]);
    assert_eq!(r.verdict, Verdict::Fail);
    let f = r.findings.iter().find(|f| f.rule == PKL_HASH).unwrap();
    assert_eq!(f.location.file.as_deref(), Some("picture.mxf"));
    assert_eq!(f.location.asset, cfa4_dcp::Uuid::parse(&uuid(3)));
    std::fs::write(d.join("picture.mxf"), b"MXF!!").unwrap();
    let r = validate(&d, "full-integrity");
    assert_eq!(statuses(&r, PKL_SIZE), [Status::Fail]);
    assert_eq!(statuses(&r, PKL_HASH), [Status::Fail]);
}

#[test]
fn asset_map_and_packing_list_consistency() {
    // the same id twice in the asset map, and a packing list asset the asset map lacks
    let dup = format!("<Asset><Id>{}</Id><ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>\n</AssetList>", uuid(2));
    let extra = format!("<Asset><Id>{}</Id><Hash>{H}</Hash><Size>1</Size><Type>application/mxf</Type></Asset>\n</AssetList>", uuid(7));
    let r = edited("consistency", Edit { am: vec![("</AssetList>", dup)], pkl: vec![("</AssetList>", extra)], ..Edit::default() }, "full-integrity");
    assert_eq!(statuses(&r, AM_UNIQUE_IDS), [Status::Fail]);
    assert_eq!(statuses(&r, PKL_IN_ASSETMAP), [Status::Fail]);
    // its size and hash cannot be checked without a file
    assert_eq!(statuses(&r, PKL_SIZE), [Status::Unchecked]);
    assert_eq!(statuses(&r, PKL_HASH), [Status::Unchecked]);
    assert_eq!(r.verdict, Verdict::Fail);

    // no packing list at all
    let r = edited("no-pkl-flag", Edit { am: vec![("<PackingList>true</PackingList>", String::new())], ..Edit::default() }, "full-integrity");
    assert_eq!(statuses(&r, AM_HAS_PKL), [Status::Fail]);
    assert!(statuses(&r, PKL_HASH).contains(&Status::Unchecked));
}

#[test]
fn composition_assets_and_packing_lists() {
    // the picture is in the asset map but no packing list names it
    let r = edited(
        "cpl-assets",
        Edit {
            pkl: vec![("application/mxf", "application/mxf".into()), ("<Asset><Id>urn:uuid:00000003", "<Asset><Id>urn:uuid:00000008".into())],
            ..Edit::default()
        },
        "full-integrity",
    );
    assert_eq!(statuses(&r, CPL_ASSETS_IN_PKL), [Status::Fail], "{:?}", outcome(&r));
    // a reel asset that is not in the package at all: a version file
    let sound = format!("<MainSound><Id>{}</Id><EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration></MainSound>\n</AssetList>", uuid(10));
    let r = edited("cpl-vf", Edit { cpl: vec![("</AssetList>", sound)], ..Edit::default() }, "full-integrity");
    assert_eq!(statuses(&r, CPL_ASSETS_IN_PKL), [Status::Warning]);
    assert_eq!(statuses(&r, CPL_REEL_SYNC), [Status::Pass]);
    assert_eq!(r.verdict, Verdict::Pass, "a version file is not a failure: {:?}", outcome(&r));
}

#[test]
fn composition_outside_the_packing_lists_fails() {
    let d = dir("cpl-outside");
    let extra = format!("<Asset><Id>{}</Id><ChunkList><Chunk><Path>cpl2.xml</Path></Chunk></ChunkList></Asset>\n</AssetList>", uuid(8));
    package_with(&d, GOOD, Edit { am: vec![("</AssetList>", extra)], ..Edit::default() });
    std::fs::copy(d.join("cpl.xml"), d.join("cpl2.xml")).unwrap();
    let r = validate(&d, "full-integrity");
    assert_eq!(statuses(&r, CPL_IN_PKL), [Status::Fail]);
    assert_eq!(r.findings.iter().find(|f| f.rule == CPL_IN_PKL).unwrap().location.file.as_deref(), Some("cpl2.xml"));
}

#[test]
fn durations_and_reel_sync() {
    let r = edited(
        "entry",
        Edit { cpl: vec![("<Duration>1</Duration>", "<EntryPoint>1</EntryPoint><Duration>1</Duration>".into())], ..Edit::default() },
        "full-integrity",
    );
    assert_eq!(statuses(&r, CPL_DURATIONS), [Status::Fail]);
    let f = r.findings.iter().find(|f| f.rule == CPL_DURATIONS).unwrap();
    assert_eq!(f.location.element.as_deref(), Some("ReelList/Reel[1]/AssetList/MainPicture"));
    assert!(f.location.line.is_some());

    // a sound track two edit units long next to a one-unit picture
    let sound = format!("<MainSound><Id>{}</Id><EditRate>24 1</EditRate><IntrinsicDuration>2</IntrinsicDuration></MainSound>\n</AssetList>", uuid(10));
    let r = edited("sync", Edit { cpl: vec![("</AssetList>", sound)], ..Edit::default() }, "full-integrity");
    assert_eq!(statuses(&r, CPL_REEL_SYNC), [Status::Fail]);
    // another edit rate
    let sound = format!("<MainSound><Id>{}</Id><EditRate>48 1</EditRate><IntrinsicDuration>2</IntrinsicDuration></MainSound>\n</AssetList>", uuid(10));
    let r = edited("rate", Edit { cpl: vec![("</AssetList>", sound)], ..Edit::default() }, "full-integrity");
    assert!(r.findings.iter().any(|f| f.rule == CPL_REEL_SYNC && f.status == Status::Fail && f.message.contains("EditRate")));
}

#[test]
fn composition_markers() {
    let markers = |m: &str| Edit {
        cpl: vec![
            ("<IntrinsicDuration>1</IntrinsicDuration><MarkerList>", "<IntrinsicDuration>3</IntrinsicDuration><MarkerList>".to_string()),
            ("<Marker><Label>FFOC</Label><Offset>0</Offset></Marker><Marker><Label>LFOC</Label><Offset>0</Offset></Marker>", m.to_string()),
            ("<IntrinsicDuration>1</IntrinsicDuration><Duration>1</Duration>", "<IntrinsicDuration>3</IntrinsicDuration><Duration>3</Duration>".into()),
        ],
        ..Edit::default()
    };
    let m = |l: &str, o: u32| format!("<Marker><Label>{l}</Label><Offset>{o}</Offset></Marker>");
    let run = |name: &str, list: String| edited(name, markers(&list), "smpte-cinema");
    let good = run("mark-good", format!("{}{}{}", m("FFOC", 0), m("FFEC", 1), m("LFOC", 2)));
    assert_eq!(statuses(&good, MARK_MARKERS), [Status::Pass], "{:?}", outcome(&good));
    let r = run("mark-range", format!("{}{}", m("FFOC", 0), m("LFOC", 3)));
    assert!(r.findings.iter().any(|f| f.rule == MARK_MARKERS && f.status == Status::Fail && f.message.contains("outside the reel")));
    let r = run("mark-dup", format!("{}{}{}", m("FFOC", 0), m("FFOC", 1), m("LFOC", 2)));
    assert!(statuses(&r, MARK_MARKERS).contains(&Status::Fail));
    let r = run("mark-order", format!("{}{}{}{}", m("FFOC", 0), m("LFEC", 1), m("FFEC", 2), m("LFOC", 2)));
    assert!(r.findings.iter().any(|f| f.rule == MARK_MARKERS && f.status == Status::Fail && f.message.contains("FFEC")));
    let r = run("mark-label", format!("{}{}{}", m("FFOC", 0), m("XXXX", 1), m("LFOC", 2)));
    assert_eq!(statuses(&r, MARK_MARKERS), [Status::Warning]);
    let r = run("mark-outside", format!("{}{}{}", m("FFTC", 0), m("FFOC", 1), m("LFOC", 2)));
    assert!(r.findings.iter().any(|f| f.rule == MARK_MARKERS && f.message.contains("outside FFOC")));
    let r = run("mark-last", format!("{}{}", m("FFOC", 1), m("LFOC", 1)));
    assert_eq!(statuses(&r, MARK_MARKERS), [Status::Warning], "FFOC at 1 is accepted, LFOC before the last frame is not: {:?}", outcome(&r));
    assert!(r.findings.iter().any(|f| f.message.contains("not at the last frame")));
}

#[test]
fn cancel_while_hashing_interrupts() {
    let d = dir("cancel-hash");
    package(&d, GOOD);
    let c = ValidationControl::new();
    let flag = c.cancel_flag();
    let c = c.with_progress(move |p| {
        if p.stage == "hash" {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    let r = NativeValidator.validate(&ValidationRequest::new(&d, Profile::builtin("full-integrity").unwrap()), &c).unwrap();
    assert_eq!(r.verdict, Verdict::Interrupted);
    assert_eq!(statuses(&r, PKL_HASH), [Status::Interrupted]);
    assert_eq!(statuses(&r, PKL_SIZE), [Status::Pass], "rules finished before the cancel keep their result");
}

#[test]
fn chunk_length_is_the_file_length() {
    let e =
        |l: u32| Edit { am: vec![("<Path>picture.mxf</Path></Chunk>", format!("<Path>picture.mxf</Path><Length>{l}</Length></Chunk>"))], ..Edit::default() };
    let r = edited("chunk-length", e(5), "quick-scan");
    assert_eq!(statuses(&r, AM_FILE_EXISTS), [Status::Fail]);
    let r = edited("chunk-length-ok", e(4), "quick-scan");
    assert_eq!(statuses(&r, AM_FILE_EXISTS), [Status::Pass]);
}

/// One 1998 × 1080 frame of the 2K DCI profile. `cfa4-j2k` writes 2 guard bits where the 2K
/// profile has 1, so the QCD is set to 1 here, and it codes cinema frames without the multiple
/// component transform, so the COD's MCT flag is set (the checks do not decode the samples).
fn frame_2k() -> Vec<u8> {
    static FRAME: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FRAME
        .get_or_init(|| {
            let mut cs = encoded_2k();
            set_marker_byte(&mut cs, 0xFF5C, 0, 1 << 5 | 2);
            set_marker_byte(&mut cs, 0xFF52, 4, 1);
            cs
        })
        .clone()
}

/// The frame as `cfa4-j2k` writes it.
fn encoded_2k() -> Vec<u8> {
    let (w, h) = (1998, 1080);
    let plane = cfa4_j2k::Plane { width: w, height: h, precision: 12, signed: false, dx: 1, dy: 1, data: vec![2048; (w * h) as usize] };
    let image = cfa4_j2k::Image { width: w, height: h, components: vec![plane.clone(), plane.clone(), plane], mct: false };
    cfa4_j2k::encode(&image, &cfa4_j2k::EncodeOptions::cinema_2k(24)).unwrap()
}

/// Sets byte `at` of the segment of the first main-header marker `m` (SIZ: 0 is Rsiz's high byte).
fn set_marker_byte(cs: &mut [u8], m: u16, at: usize, value: u8) {
    let mut p = 2;
    loop {
        let marker = u16::from_be_bytes([cs[p], cs[p + 1]]);
        assert_ne!(marker, 0xFF90, "marker {m:04X} not in the main header");
        let len = usize::from(u16::from_be_bytes([cs[p + 2], cs[p + 3]]));
        if marker == m {
            cs[p + 4 + at] = value;
            return;
        }
        p += 2 + len;
    }
}

/// A D-Cinema picture track file (ST 429-3) for the asset `uuid(id)` with `frames`.
fn track_file(id: u32, frames: &[Vec<u8>], rate: i32, size: (u32, u32)) -> Vec<u8> {
    use cfa4_mxf::{FrameInfo, MxfWriter, PackageIds, Pattern, PictureDesc, Rational, Umid, WriterConfig};
    let asset = cfa4_dcp::Uuid::parse(&uuid(id)).unwrap().0;
    let ids = PackageIds { material: Umid::dcinema([7; 16]), file: Umid::dcinema(asset), material_name: "T".into(), file_name: "T picture".into() };
    let mut c = WriterConfig::new(Pattern::DCinema, Rational::new(rate, 1), ids);
    c.picture = Some(PictureDesc::dcinema(3, size.0, size.1));
    let mut w = MxfWriter::new(std::io::Cursor::new(Vec::new()), c).unwrap();
    for f in frames {
        w.push_picture(f, FrameInfo::intra()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn good_track_file() -> Vec<u8> {
    track_file(3, &[frame_2k()], 24, (1998, 1080))
}

fn media(name: &str, picture: Vec<u8>, cpl: Vec<(&'static str, String)>) -> ValidationReport {
    edited(name, Edit { picture: Some(picture), cpl, ..Edit::default() }, "smpte-cinema")
}

fn messages(r: &ValidationReport, rule: &str, status: Status) -> Vec<String> {
    r.findings.iter().filter(|f| f.rule == rule && f.status == status).map(|f| f.message.clone()).collect()
}

/// The picture's EditRate, IntrinsicDuration and Duration (the markers have no Duration).
const PICTURE_TIMING: &str = "<EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><Duration>1</Duration>";

#[test]
fn a_dci_track_file_passes() {
    let r = media("media-good", good_track_file(), vec![]);
    for rule in [MXF_TRACK_FILES, J2K_CODESTREAM, PIC_FORMAT] {
        assert_eq!(statuses(&r, rule), [Status::Pass], "{rule}: {:?}", outcome(&r));
    }
    assert!(messages(&r, J2K_CODESTREAM, Status::Pass)[0].starts_with("1 frames in 1 picture track files"));
    // Quick Scan and Full Integrity do not open the track files
    let quick = edited("media-quick", Edit { picture: Some(good_track_file()), ..Edit::default() }, "full-integrity");
    assert!(statuses(&quick, MXF_TRACK_FILES).is_empty());
}

#[test]
fn track_files_must_match_their_reel_assets() {
    // not MXF: the codestreams and the format cannot be checked
    let r = media("media-not-mxf", b"MXF!".to_vec(), vec![]);
    assert_eq!(messages(&r, MXF_TRACK_FILES, Status::Fail), ["'picture.mxf' is not an MXF file"]);
    assert_eq!(statuses(&r, J2K_CODESTREAM), [Status::Unchecked]);
    assert_eq!(statuses(&r, PIC_FORMAT), [Status::Unchecked]);

    // another asset's file
    let r = media("media-id", track_file(8, &[frame_2k()], 24, (1998, 1080)), vec![]);
    let m = messages(&r, MXF_TRACK_FILES, Status::Fail);
    assert!(m.len() == 1 && m[0].contains("is the track file urn:uuid:00000008"), "{m:?}");

    // edit rate and duration
    let r = media("media-rate", track_file(3, &[frame_2k()], 25, (1998, 1080)), vec![]);
    assert!(messages(&r, MXF_TRACK_FILES, Status::Fail).iter().any(|m| m.contains("has the edit rate 25 1, the reel asset 24 1")));
    let r = media("media-duration", track_file(3, &[frame_2k(), frame_2k()], 24, (1998, 1080)), vec![]);
    assert!(messages(&r, MXF_TRACK_FILES, Status::Fail).iter().any(|m| m.contains("is 2 edit units long, the IntrinsicDuration is 1")));

    // a mono file as stereoscopic pictures (ST 429-10: two frames per edit unit)
    let stereo = vec![
        ("<MainPicture>", "<MainStereoscopicPicture>".to_string()),
        ("<FrameRate>24 1</FrameRate>", "<FrameRate>48 1</FrameRate>".to_string()),
        ("</MainPicture>", "</MainStereoscopicPicture>".to_string()),
    ];
    let r = media("media-stereo", good_track_file(), stereo);
    assert!(messages(&r, MXF_TRACK_FILES, Status::Fail).iter().any(|m| m.contains("has 1 frames for 1 edit units")), "{:?}", outcome(&r));
    assert_eq!(statuses(&r, PIC_FORMAT), [Status::Pass], "48 is twice the edit rate");

    // a KeyId for a file in the clear
    let key = vec![(PICTURE_TIMING, format!("{PICTURE_TIMING}<KeyId>{}</KeyId>", uuid(11)))];
    let r = media("media-key", good_track_file(), key);
    assert!(messages(&r, MXF_TRACK_FILES, Status::Fail).iter().any(|m| m.contains("is not encrypted")));
    assert_eq!(r.verdict, Verdict::Fail);
}

#[test]
fn codestreams_follow_the_dci_profile() {
    // as cfa4-j2k writes it: 2 guard bits
    let r = media("j2k-guard", track_file(3, &[encoded_2k()], 24, (1998, 1080)), vec![]);
    assert_eq!(messages(&r, J2K_CODESTREAM, Status::Fail), ["'picture.mxf' frame 0: 2 guard bits; the 2K profile has 1"]);
    assert_eq!(statuses(&r, MXF_TRACK_FILES), [Status::Pass]);

    // without the multiple component transform: a warning, not a failure (plan CFA4-DCPVAL-LIBDCP-001, D2)
    let mut no_mct = frame_2k();
    set_marker_byte(&mut no_mct, 0xFF52, 4, 0);
    let r = media("j2k-no-mct", track_file(3, &[no_mct], 24, (1998, 1080)), vec![]);
    assert!(messages(&r, J2K_CODESTREAM, Status::Fail).is_empty(), "{:?}", outcome(&r));
    let w = messages(&r, J2K_CODESTREAM, Status::Warning);
    assert!(w.len() == 1 && w[0].contains("no multiple component transform"), "{w:?}");

    // not the cinema profile, in both frames
    let mut part1 = frame_2k();
    set_marker_byte(&mut part1, 0xFF51, 1, 0);
    let r = media("j2k-rsiz", track_file(3, &[part1.clone(), part1], 24, (1998, 1080)), vec![(PICTURE_TIMING, PICTURE_TIMING.replace(">1<", ">2<"))]);
    assert_eq!(messages(&r, J2K_CODESTREAM, Status::Fail), ["'picture.mxf' frame 0: Rsiz is 0, the 2K digital cinema profile is 3 (2 frames)"]);

    // the descriptor says another size
    let r = media("j2k-size", track_file(3, &[frame_2k()], 24, (2048, 1080)), vec![]);
    assert!(messages(&r, J2K_CODESTREAM, Status::Fail).iter().any(|m| m.contains("the codestream is 1998 × 1080, the MXF picture descriptor 2048 × 1080")));

    // not a codestream at all
    let r = media("j2k-none", track_file(3, &[vec![0; 64]], 24, (1998, 1080)), vec![]);
    assert!(messages(&r, J2K_CODESTREAM, Status::Fail)[0].contains("no SOC marker"));
}

#[test]
fn picture_formats() {
    let r = media("pic-sar", good_track_file(), vec![("1998 1080", "2048 858".into())]);
    assert_eq!(messages(&r, PIC_FORMAT, Status::Warning), ["the ScreenAspectRatio 2048 858 is not the 1998 × 1080 of 'picture.mxf'"]);
    let r = media("pic-frame-rate", good_track_file(), vec![("<FrameRate>24 1</FrameRate>", "<FrameRate>48 1</FrameRate>".into())]);
    assert_eq!(messages(&r, PIC_FORMAT, Status::Fail), ["the FrameRate 48 1 is not the edit rate (24 1)"]);
    // 25 fps is SMPTE but not everywhere; 30000/1001 is not a cinema rate
    let rate = |n: &str| {
        vec![
            (PICTURE_TIMING, PICTURE_TIMING.replace("24 1", n)),
            ("<FrameRate>24 1</FrameRate>", format!("<FrameRate>{n}</FrameRate>")),
            (
                "<EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><MarkerList>",
                format!("<EditRate>{n}</EditRate><IntrinsicDuration>1</IntrinsicDuration><MarkerList>"),
            ),
        ]
    };
    let r = media("pic-25", track_file(3, &[frame_2k()], 25, (1998, 1080)), rate("25 1"));
    assert_eq!(messages(&r, PIC_FORMAT, Status::Warning), ["25 fps 2K is in the Smpte standard but not every cinema server plays it"]);
    let r = media("pic-ntsc", track_file(3, &[frame_2k()], 24, (1998, 1080)), rate("30000 1001"));
    assert!(
        messages(&r, PIC_FORMAT, Status::Fail).contains(&"the edit rate 30000 1001 is not a Smpte cinema frame rate for 2K".to_string()),
        "{:?}",
        outcome(&r)
    );
    // a size outside the DCI containers
    let r = media("pic-size", track_file(3, &[frame_2k()], 24, (1920, 1080)), vec![]);
    assert!(messages(&r, PIC_FORMAT, Status::Warning).iter().any(|m| m.contains("1920 × 1080, not a DCI image size")));
}

#[test]
fn codestream_structure() {
    use crate::codestream::{component_bytes, main_header, tile_parts};
    let cs = frame_2k();
    let main = main_header(&cs).unwrap().unwrap();
    let parts = tile_parts(cs.len() as u64, main.end as u64, |at, n| Ok(cs[at as usize..at as usize + n].to_vec())).unwrap();
    assert_eq!(parts.lengths.len(), 3, "one tile-part per component");
    assert!(parts.problems.is_empty() && !parts.no_eoc, "{parts:?}");
    assert_eq!(main.end as u64 + parts.lengths.iter().sum::<u64>() + 2, cs.len() as u64);
    assert_eq!(component_bytes(&[1, 2, 3, 4, 5, 6], true), Some([5, 7, 9]));
    assert_eq!(component_bytes(&[1, 2], false), None);
    // cut short: no SOT yet
    assert!(main_header(&cs[..main.end]).unwrap().is_none());
    // DCI limits at 24 fps, and for stereoscopic pictures at 24 per eye
    let r = |n| cfa4_dcp::Rational { num: n, den: 1 };
    assert_eq!(crate::media::dci_budget(r(24), false), Some((1_302_083, 1_041_666)));
    assert_eq!(crate::media::dci_budget(r(24), true), Some((651_041, 520_833)));
    assert_eq!(crate::media::dci_budget(r(0), false), None);
}

#[test]
fn cancel_while_reading_frames_interrupts() {
    let d = dir("cancel-media");
    package_with(&d, GOOD, Edit { picture: Some(good_track_file()), ..Edit::default() });
    let c = ValidationControl::new();
    let flag = c.cancel_flag();
    let c = c.with_progress(move |p| {
        if p.stage == "media" {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    let r = NativeValidator.validate(&ValidationRequest::new(&d, Profile::builtin("smpte-cinema").unwrap()), &c).unwrap();
    assert_eq!(r.verdict, Verdict::Interrupted);
    assert_eq!(statuses(&r, MXF_TRACK_FILES), [Status::Interrupted]);
}

/// A D-Cinema track file for the asset `uuid(3)` at 24 fps, made by `cfg` and filled by `fill`.
fn dcinema_file(cfg: impl FnOnce(&mut cfa4_mxf::WriterConfig), fill: impl FnOnce(&mut cfa4_mxf::MxfWriter<std::io::Cursor<Vec<u8>>>)) -> Vec<u8> {
    use cfa4_mxf::{MxfWriter, PackageIds, Pattern, Rational, Umid, WriterConfig};
    let asset = cfa4_dcp::Uuid::parse(&uuid(3)).unwrap().0;
    let ids = PackageIds { material: Umid::dcinema([7; 16]), file: Umid::dcinema(asset), material_name: "T".into(), file_name: "T".into() };
    let mut c = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg(&mut c);
    let mut w = MxfWriter::new(std::io::Cursor::new(Vec::new()), c).unwrap();
    fill(&mut w);
    w.finish().unwrap().into_inner()
}

/// `units` edit units of `channels`-channel PCM at 48 kHz and `bits`: a 1 kHz sine of amplitude
/// `level` on the first channel, silence on the others.
fn sound_file(units: usize, channels: u32, bits: u32, level: f32) -> Vec<u8> {
    let n = units * 2000;
    let mut planar = vec![vec![0f32; n]; channels as usize];
    for (i, s) in planar[0].iter_mut().enumerate() {
        *s = level * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin();
    }
    let pcm = cfa4_mxf::encode_pcm(&planar, bits);
    dcinema_file(|c| c.sound = vec![cfa4_mxf::SoundDesc { sample_rate: 48_000, channels, bits }], |w| w.push_sound(0, &pcm).unwrap())
}

/// The CPL's picture as a `units`-long main sound.
fn as_sound(units: usize) -> Vec<(&'static str, String)> {
    vec![
        ("<MainPicture>", "<MainSound>".into()),
        (PICTURE_TIMING, format!("<EditRate>24 1</EditRate><IntrinsicDuration>{units}</IntrinsicDuration><Duration>{units}</Duration>")),
        ("<FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>", "</MainSound>".into()),
    ]
}

fn sound(name: &str, file: Vec<u8>, units: usize) -> ValidationReport {
    edited(name, Edit { picture: Some(file), cpl: as_sound(units), ..Edit::default() }, "delivery-preflight")
}

#[test]
fn sound_track_files() {
    let r = sound("aud-good", sound_file(48, 6, 24, 0.1), 48);
    assert_eq!(statuses(&r, MXF_TRACK_FILES), [Status::Pass], "{:?}", outcome(&r));
    assert_eq!(statuses(&r, AUD_PCM), [Status::Pass]);
    assert_eq!(statuses(&r, IAB_TRACK), [Status::NotApplicable]);
    // the reel has no picture now, so the picture rules have nothing to check
    assert_eq!(statuses(&r, J2K_CODESTREAM), [Status::Pass]);

    let r = sound("aud-16", sound_file(1, 6, 16, 0.1), 1);
    assert_eq!(messages(&r, AUD_PCM, Status::Fail), ["'picture.mxf' has 16 bits a sample; D-Cinema sound has 24"]);
    let r = sound("aud-4ch", sound_file(1, 4, 24, 0.1), 1);
    assert_eq!(messages(&r, AUD_PCM, Status::Warning), ["'picture.mxf' has 4 channels, not one of the ST 429-2 configurations (6, 8, 16)"]);
    // a picture where the composition expects sound
    // a composition without sound
    let r = edited("aud-silent", Edit { picture: Some(good_track_file()), ..Edit::default() }, "smpte-cinema");
    assert_eq!(statuses(&r, AUD_PCM), [Status::NotApplicable]);
    let r = sound("aud-picture", good_track_file(), 1);
    assert_eq!(statuses(&r, AUD_PCM), [Status::Unchecked]);
    assert!(statuses(&r, MXF_TRACK_FILES).contains(&Status::Fail));
}

#[test]
fn loudness_is_measured() {
    // BS.1770: a 1 kHz sine at -20 dBFS peak on one front channel is -23.0 LUFS
    let r = sound("aud-loud", sound_file(48, 6, 24, 0.1), 48);
    let f = r.findings.iter().find(|f| f.rule == AUD_LOUDNESS).unwrap();
    assert_eq!(f.status, Status::Pass, "{f:?}");
    let lufs: f64 = f.message.strip_prefix("integrated ").and_then(|m| m.split(' ').next()).unwrap().parse().unwrap();
    assert!((lufs + 23.0).abs() < 0.2, "{lufs}");
    assert!(f.evidence.as_deref().unwrap().contains("2.0 s, 6 channels at 48000 Hz"));
    // the LFE does not count
    assert_eq!(crate::audio::weights(8), [1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 0.0, 0.0]);
    // a composition without sound
    let r = edited("aud-none", Edit { picture: Some(good_track_file()), ..Edit::default() }, "delivery-preflight");
    assert_eq!(statuses(&r, AUD_LOUDNESS), [Status::NotApplicable]);
}

#[test]
fn immersive_audio_frames() {
    use cfa4_iab::{Frame, FrameRate};
    let frame = |rate| Frame {
        version: 1,
        sample_rate: 48_000,
        bit_depth: 24,
        frame_rate: rate,
        frame_index: 0,
        max_rendered: 0,
        beds: vec![],
        objects: vec![],
        audio: vec![],
        authoring_tool: None,
        user_data: vec![],
    };
    let file = |frames: Vec<Vec<u8>>| {
        dcinema_file(
            |c| c.iab = Some(cfa4_mxf::IabDesc { version: 1, max_channels: 0, max_objects: 0, first_frame: 0, sample_rate: 48_000 }),
            |w| {
                for f in &frames {
                    w.push_data(f).unwrap();
                }
            },
        )
    };
    let aux = vec![
        ("<MainPicture>", "<AuxData>".into()),
        ("<FrameRate>24 1</FrameRate><ScreenAspectRatio>1998 1080</ScreenAspectRatio></MainPicture>", "</AuxData>".into()),
    ];
    let good = cfa4_iab::write(&frame(FrameRate::R24)).unwrap();
    let r = edited("iab-good", Edit { picture: Some(file(vec![good])), cpl: aux.clone(), ..Edit::default() }, "delivery-preflight");
    assert_eq!(statuses(&r, IAB_TRACK), [Status::Pass], "{:?}", outcome(&r));
    let wrong = cfa4_iab::write(&frame(FrameRate::R25)).unwrap();
    let r = edited("iab-rate", Edit { picture: Some(file(vec![wrong])), cpl: aux.clone(), ..Edit::default() }, "delivery-preflight");
    assert_eq!(messages(&r, IAB_TRACK, Status::Fail), ["'picture.mxf' frame 0: the IAFrame rate is 25/1, the track's edit rate 24 1"]);
    let r = edited("iab-junk", Edit { picture: Some(file(vec![vec![0; 32]])), cpl: aux, ..Edit::default() }, "delivery-preflight");
    assert!(messages(&r, IAB_TRACK, Status::Fail)[0].contains("is not an IABitstream frame"));
}
