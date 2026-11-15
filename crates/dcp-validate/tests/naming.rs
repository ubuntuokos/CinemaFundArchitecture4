//! DCP-09: `DCP.NAME.001` on small SMPTE packages written here: a 2K flat picture and a
//! 6-channel sound track file (`cfa4-mxf`'s writer), named in the CPL's `ContentTitleText`.

use std::path::{Path, PathBuf};

use cfa4_dcp_validate::rules::NAME_ISDCF;
use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest};
use cfa4_export::{ExportSettings, Format};
use cfa4_mxf::{FrameInfo, MxfWriter, PackageIds, Pattern, PictureDesc, Rational, SoundDesc, Umid, WriterConfig};

fn uuid(n: u32) -> String {
    format!("{n:08x}-1111-4222-8333-444444444444")
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-validate-name-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn writer(id: u32, rate: i32, cfg: impl FnOnce(&mut WriterConfig)) -> MxfWriter<std::io::Cursor<Vec<u8>>> {
    let asset = cfa4_dcp::Uuid::parse(&format!("urn:uuid:{}", uuid(id))).unwrap().0;
    let ids = PackageIds { material: Umid::dcinema([7; 16]), file: Umid::dcinema(asset), material_name: "T".into(), file_name: "T".into() };
    let mut c = WriterConfig::new(Pattern::DCinema, Rational::new(rate, 1), ids);
    cfg(&mut c);
    MxfWriter::new(std::io::Cursor::new(Vec::new()), c).unwrap()
}

fn picture(rate: i32, size: (u32, u32)) -> Vec<u8> {
    let mut w = writer(3, rate, |c| c.picture = Some(PictureDesc::dcinema(3, size.0, size.1)));
    w.push_picture(&[0xFF, 0x4F, 0xFF, 0xD9], FrameInfo::intra()).unwrap();
    w.finish().unwrap().into_inner()
}

fn sound(rate: i32, channels: u32) -> Vec<u8> {
    let mut w = writer(5, rate, |c| c.sound = vec![SoundDesc { sample_rate: 48_000, channels, bits: 24 }]);
    let per_unit = 48_000 / rate as usize;
    w.push_sound(0, &vec![0u8; per_unit * channels as usize * 3]).unwrap();
    w.finish().unwrap().into_inner()
}

struct Pkg {
    title: String,
    kind: &'static str,
    rate: i32,
    size: (u32, u32),
    channels: Option<u32>,
    /// Extra reel asset XML (subtitles, a track file of another package).
    extra: String,
}

impl Pkg {
    fn new(title: &str) -> Pkg {
        Pkg { title: title.into(), kind: "feature", rate: 24, size: (1998, 1080), channels: Some(6), extra: String::new() }
    }
}

fn package(root: &Path, p: &Pkg) {
    let rate = p.rate;
    let mut files: Vec<(String, String, Vec<u8>)> = vec![(uuid(3), "picture.mxf".into(), picture(rate, p.size))];
    let mut assets = format!(
        "<MainPicture><Id>urn:uuid:{}</Id><EditRate>{rate} 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><Duration>1</Duration><FrameRate>{rate} 1</FrameRate><ScreenAspectRatio>{} {}</ScreenAspectRatio></MainPicture>",
        uuid(3),
        p.size.0,
        p.size.1
    );
    if let Some(ch) = p.channels {
        files.push((uuid(5), "sound.mxf".into(), sound(rate, ch)));
        assets.push_str(&format!(
            "<MainSound><Id>urn:uuid:{}</Id><EditRate>{rate} 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><Duration>1</Duration><Language>en</Language></MainSound>",
            uuid(5)
        ));
    }
    assets.push_str(&p.extra);
    let cpl = format!(
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"><Id>urn:uuid:{}</Id><IssueDate>2026-11-15T00:00:00Z</IssueDate><ContentTitleText>{}</ContentTitleText><ContentKind>{}</ContentKind><ReelList><Reel><Id>urn:uuid:{}</Id><AssetList>{assets}</AssetList></Reel></ReelList></CompositionPlaylist>"#,
        uuid(2),
        p.title,
        p.kind,
        uuid(4)
    );
    files.push((uuid(2), "cpl.xml".into(), cpl.into_bytes()));
    let digest = |b: &[u8]| cfa4_dcp::sha1::base64(&cfa4_dcp::sha1::sha1(b));
    let pkl_assets: String = files
        .iter()
        .map(|(id, path, b)| {
            let ty = if path.ends_with(".mxf") { "application/mxf" } else { "text/xml" };
            format!("<Asset><Id>urn:uuid:{id}</Id><Hash>{}</Hash><Size>{}</Size><Type>{ty}</Type></Asset>", digest(b), b.len())
        })
        .collect();
    let pkl = format!(
        r#"<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL"><Id>urn:uuid:{}</Id><IssueDate>2026-11-15T00:00:00Z</IssueDate><Issuer>t</Issuer><Creator>t</Creator><AssetList>{pkl_assets}</AssetList></PackingList>"#,
        uuid(1)
    );
    let am_assets: String =
        files.iter().map(|(id, path, _)| format!("<Asset><Id>urn:uuid:{id}</Id><ChunkList><Chunk><Path>{path}</Path></Chunk></ChunkList></Asset>")).collect();
    let am = format!(
        r#"<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Id>urn:uuid:{}</Id><Creator>t</Creator><VolumeCount>1</VolumeCount><IssueDate>2026-11-15T00:00:00Z</IssueDate><Issuer>t</Issuer><AssetList><Asset><Id>urn:uuid:{}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>pkl.xml</Path></Chunk></ChunkList></Asset>{am_assets}</AssetList></AssetMap>"#,
        uuid(9),
        uuid(1)
    );
    std::fs::write(root.join("ASSETMAP.xml"), am).unwrap();
    std::fs::write(root.join("pkl.xml"), pkl).unwrap();
    for (_, path, b) in &files {
        std::fs::write(root.join(path), b).unwrap();
    }
}

fn validate(name: &str, p: &Pkg) -> ValidationReport {
    let d = dir(name);
    package(&d, p);
    let req = ValidationRequest::new(&d, Profile::builtin("smpte-cinema").unwrap());
    NativeValidator.validate(&req, &ValidationControl::new()).unwrap()
}

fn findings(r: &ValidationReport) -> Vec<(Status, String)> {
    r.findings.iter().filter(|f| f.rule == NAME_ISDCF).map(|f| (f.status, f.message.clone())).collect()
}

const GOOD: &str = "RoundTrip_FTR-1_F_EN-XX_51_2K_20261115_SMPTE_OV";

#[test]
fn a_name_that_matches_its_package_passes() {
    let r = validate("good", &Pkg::new(GOOD));
    assert_eq!(findings(&r), [(Status::Pass, "1 ISDCF names agree with their packages".to_string())]);
}

#[test]
fn the_cfa4_export_name_matches_a_cfa4_like_package() {
    let s = ExportSettings { format: Format::Dcp, ..Default::default() };
    let name = cfa4_export::dcp_isdcf_name(&s, "Round Trip", "2026-11-15");
    let n = cfa4_dcp::isdcf::parse(&name).unwrap();
    assert!(n.unknown.is_empty() && n.missing.is_empty() && !n.bad_date, "{name}: {n:?}");
    let r = validate("export-name", &Pkg::new(&name));
    assert_eq!(findings(&r).len(), 1, "{name}: {:?}", findings(&r));
    assert_eq!(findings(&r)[0].0, Status::Pass, "{name}: {:?}", findings(&r));
}

#[test]
fn claims_that_differ_from_the_package_are_warnings() {
    let mut p = Pkg::new("RoundTrip_TLR-1-25-3D_S_EN-DE-CCAP_71-ATMOS_4K_20261115_IOP_VF");
    p.extra =
        format!("<MainSound><Id>urn:uuid:{}</Id><EditRate>24 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><Duration>1</Duration></MainSound>", uuid(77));
    let r = validate("bad", &p);
    let f = findings(&r);
    assert!(f.iter().all(|(s, _)| *s == Status::Warning), "{f:?}");
    for m in [
        "the content type TLR is a trailer, the ContentKind is feature",
        "the name claims 25 fps, the picture is 24 fps",
        "the name claims 3D, the picture is 2D",
        "the aspect ratio S is not the picture's container 1998×1080 (F)",
        "the name claims 4K, the picture is 2K (1998×1080)",
        "the name claims IOP, the composition is SMPTE",
        "the name claims 71 (8 channels), the sound has 6",
        "the name claims immersive audio",
        "the name claims DE subtitles in their own track file, the composition has none",
        "the name claims CCAP, the composition has no caption track",
    ] {
        assert!(f.iter().any(|(_, x)| x.contains(m)), "{m}: {f:?}");
    }
    // one track file is outside the package, so VF is right
    assert!(!f.iter().any(|(_, x)| x.contains("claims VF")), "{f:?}");
}

#[test]
fn package_type_languages_and_frame_rate() {
    let mut p = Pkg::new("RoundTrip_FTR-1_F_FR-XX_51_2K_20261115_SMPTE_VF");
    p.rate = 25;
    p.extra = format!(
        "<MainSubtitle><Id>urn:uuid:{}</Id><EditRate>25 1</EditRate><IntrinsicDuration>1</IntrinsicDuration><Language>hu</Language></MainSubtitle>",
        uuid(78)
    );
    let f = findings(&validate("vf", &p));
    for m in [
        "the name claims 24 fps (no frame rate in the name means 24), the picture is 25 fps",
        "the audio language FR is not the sound's Language en",
        "the composition has subtitles, the name says XX",
    ] {
        assert!(f.iter().any(|(_, x)| x.contains(m)), "{m}: {f:?}");
    }
    // the subtitle track file is not in the package
    assert!(!f.iter().any(|(_, x)| x.contains("claims VF")), "{f:?}");

    let mut p = Pkg::new("RoundTrip_FTR-1_F_EN-XX_MOS_2K_20261115_SMPTE_VF");
    p.channels = None;
    let f = findings(&validate("vf-mos", &p));
    assert_eq!(
        f,
        [(
            Status::Warning,
            "'RoundTrip_FTR-1_F_EN-XX_MOS_2K_20261115_SMPTE_VF': the name claims VF, the package holds every track file the composition plays".to_string()
        )]
    );

    let mut p = Pkg::new(GOOD);
    p.channels = Some(2);
    let f = findings(&validate("stereo-sound", &p));
    assert!(f.iter().any(|(_, x)| x.contains("the name claims 51 (6 channels), the sound has 2")), "{f:?}");
}

#[test]
fn titles_outside_the_convention() {
    let f = findings(&validate("plain", &Pkg::new("Round Trip")));
    assert_eq!(f, [(Status::Warning, "'Round Trip' does not follow the ISDCF naming convention (its second field is not a content type)".to_string())]);
    let f = findings(&validate("fields", &Pkg::new("RoundTrip_FTR-1_EN-XX_F_51_2K_20261399_SMPTE")));
    for m in
        ["lacks the fields aspect ratio, package type", "has fields out of place or not in the convention: 'F'", "the date '20261399' is not a calendar date"]
    {
        assert!(f.iter().any(|(_, x)| x.contains(m)), "{m}: {f:?}");
    }
}

#[test]
fn profiles_without_the_rule_do_not_read_names() {
    let d = dir("quick");
    package(&d, &Pkg::new("Round Trip"));
    let req = ValidationRequest::new(&d, Profile::builtin("full-integrity").unwrap());
    let r = NativeValidator.validate(&req, &ValidationControl::new()).unwrap();
    assert!(findings(&r).is_empty());
}
