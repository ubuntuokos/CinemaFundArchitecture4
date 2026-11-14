//! DCP-08: `DCP.SUB.001` on small packages written here, an Interop one with a `DCSubtitle` XML
//! file, its font and PNG image beside it, and a SMPTE one with an ST 429-5 timed text track file.

use std::path::{Path, PathBuf};

use cfa4_dcp_validate::rules::SUB_DOCUMENTS;
use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, Status, ValidationControl, ValidationReport, ValidationRequest};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const SUB: &str = "00000003-1111-4222-8333-444444444444";
const RESOURCE: &str = "11111111-1111-1111-1111-111111111111";

fn uuid(n: u32) -> String {
    format!("{n:08x}-1111-4222-8333-444444444444")
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-validate-sub-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A package with one reel holding one subtitle asset: `file` is its path, `kind` the CPL element.
fn package(root: &Path, interop: bool, file: (&str, &[u8]), extra: &[(&str, &[u8])], asset: &str) {
    let (am_ns, pkl_ns, cpl_ns) = if interop {
        (
            "http://www.digicine.com/PROTO-ASDCP-AM-20040311#",
            "http://www.digicine.com/PROTO-ASDCP-PKL-20040311#",
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
        )
    } else {
        ("http://www.smpte-ra.org/schemas/429-9/2007/AM", "http://www.smpte-ra.org/schemas/429-8/2007/PKL", "http://www.smpte-ra.org/schemas/429-7/2006/CPL")
    };
    let cpl = format!(
        r#"<CompositionPlaylist xmlns="{cpl_ns}"><Id>urn:uuid:{}</Id><IssueDate>2026-11-14T00:00:00Z</IssueDate><ContentTitleText>T</ContentTitleText><ContentKind>test</ContentKind><ReelList><Reel><Id>urn:uuid:{}</Id><AssetList>
{asset}
</AssetList></Reel></ReelList></CompositionPlaylist>"#,
        uuid(2),
        uuid(4)
    );
    let mut files: Vec<(String, String, Vec<u8>)> = vec![(uuid(2), "cpl.xml".into(), cpl.into_bytes()), (SUB.into(), file.0.into(), file.1.to_vec())];
    for (i, (path, bytes)) in extra.iter().enumerate() {
        files.push((uuid(20 + i as u32), (*path).into(), bytes.to_vec()));
    }
    let digest = |b: &[u8]| cfa4_dcp::sha1::base64(&cfa4_dcp::sha1::sha1(b));
    let pkl_assets: String = files
        .iter()
        .map(|(id, path, b)| {
            let ty = if path.ends_with(".mxf") { "application/mxf" } else { "text/xml" };
            format!("<Asset><Id>urn:uuid:{id}</Id><Hash>{}</Hash><Size>{}</Size><Type>{ty}</Type></Asset>", digest(b), b.len())
        })
        .collect();
    let pkl = format!(
        r#"<PackingList xmlns="{pkl_ns}"><Id>urn:uuid:{}</Id><IssueDate>2026-11-14T00:00:00Z</IssueDate><Issuer>t</Issuer><Creator>t</Creator><AssetList>{pkl_assets}</AssetList></PackingList>"#,
        uuid(1)
    );
    let am_assets: String =
        files.iter().map(|(id, path, _)| format!("<Asset><Id>urn:uuid:{id}</Id><ChunkList><Chunk><Path>{path}</Path></Chunk></ChunkList></Asset>")).collect();
    let am = format!(
        r#"<AssetMap xmlns="{am_ns}"><Id>urn:uuid:{}</Id><Creator>t</Creator><VolumeCount>1</VolumeCount><IssueDate>2026-11-14T00:00:00Z</IssueDate><Issuer>t</Issuer><AssetList>
<Asset><Id>urn:uuid:{}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>pkl.xml</Path></Chunk></ChunkList></Asset>{am_assets}</AssetList></AssetMap>"#,
        uuid(9),
        uuid(1)
    );
    std::fs::write(root.join(if interop { "ASSETMAP" } else { "ASSETMAP.xml" }), am).unwrap();
    std::fs::write(root.join("pkl.xml"), pkl).unwrap();
    for (_, path, b) in &files {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b).unwrap();
    }
}

fn validate(root: &Path) -> ValidationReport {
    let req = ValidationRequest::new(root, Profile::builtin("subtitle-accessibility").unwrap());
    NativeValidator.validate(&req, &ValidationControl::new()).unwrap()
}

fn findings(r: &ValidationReport) -> Vec<(Status, String)> {
    r.findings.iter().filter(|f| f.rule == SUB_DOCUMENTS).map(|f| (f.status, f.message.clone())).collect()
}

fn passed(r: &ValidationReport) -> bool {
    r.findings.iter().any(|f| f.rule == SUB_DOCUMENTS && f.status == Status::Pass)
}

fn interop_asset(duration: u32) -> String {
    format!("<MainSubtitle><Id>urn:uuid:{SUB}</Id><EditRate>24 1</EditRate><IntrinsicDuration>{duration}</IntrinsicDuration></MainSubtitle>")
}

/// An Interop document: `body` goes inside the `Font` element that uses the font `font`.
fn dc_subtitle(id: &str, font: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DCSubtitle Version="1.0"><SubtitleID>{id}</SubtitleID><MovieTitle>T</MovieTitle><ReelNumber>1</ReelNumber><Language>hu</Language>
<LoadFont Id="Font1" URI="font.ttf"/>
<Font Id="{font}" Size="42">
{body}
</Font></DCSubtitle>"#
    )
}

const TWO: &str = r#"<Subtitle SpotNumber="1" TimeIn="00:00:01:000" TimeOut="00:00:03:000" FadeUpTime="20" FadeDownTime="20"><Text VPosition="10">Szia</Text></Subtitle>
<Subtitle SpotNumber="2" TimeIn="00:00:04.500" TimeOut="00:00:06:000"><Image>sub2.png</Image></Subtitle>"#;

fn interop(name: &str, xml: &str, png: &[u8], duration: u32) -> ValidationReport {
    let d = dir(name);
    package(&d, true, ("sub/subs.xml", xml.as_bytes()), &[("sub/font.ttf", b"\0\x01\0\0"), ("sub/sub2.png", png)], &interop_asset(duration));
    validate(&d)
}

#[test]
fn a_good_interop_subtitle_passes() {
    let r = interop("good", &dc_subtitle(SUB, "Font1", TWO), PNG, 240);
    assert!(passed(&r), "{:?}", findings(&r));
    assert_eq!(findings(&r).len(), 1, "{:?}", findings(&r));
    let pass = r.findings.iter().find(|f| f.rule == SUB_DOCUMENTS).unwrap();
    assert_eq!(pass.message, "1 subtitle documents with 2 subtitles");
}

#[test]
fn interop_ids_fonts_images_and_timing() {
    let other = "00000077-1111-4222-8333-444444444444";
    let body = r#"<Subtitle SpotNumber="2" TimeIn="00:00:05:000" TimeOut="00:00:04:000"><Text>a</Text><Font Id="Nope"><Text>b</Text></Font></Subtitle>
<Subtitle SpotNumber="1" TimeIn="00:00:01:000" TimeOut="00:00:12:000"><Image>sub2.png</Image><Image>gone.png</Image></Subtitle>
<Subtitle SpotNumber="3" TimeIn="00:00:01:000" TimeOut="00:00:02:000" FadeUpTime="200" FadeDownTime="200"></Subtitle>"#;
    let r = interop("bad", &dc_subtitle(other, "Font1", body), b"GIF89a", 240);
    let f = findings(&r);
    assert!(!passed(&r));
    let has = |s: Status, m: &str| f.iter().any(|(st, msg)| *st == s && msg.contains(m));
    assert!(has(Status::Fail, &format!("SubtitleID urn:uuid:{other} is not the reel asset's Id urn:uuid:{SUB}")), "{f:?}");
    assert!(has(Status::Fail, "a Font names 'Nope', which no LoadFont declares"), "{f:?}");
    assert!(has(Status::Fail, "the image 'sub2.png' is not PNG"), "{f:?}");
    assert!(has(Status::Fail, "the image 'gone.png' is not in the package"), "{f:?}");
    assert!(has(Status::Fail, "does not end after it starts"), "{f:?}");
    assert!(has(Status::Fail, "ends after the reel (240 edit units at 24 1)"), "{f:?}");
    assert!(has(Status::Warning, "spot numbers do not increase"), "{f:?}");
    assert!(has(Status::Warning, "not in time order"), "{f:?}");
    assert!(has(Status::Warning, "fades are longer than the subtitle"), "{f:?}");
    assert!(has(Status::Warning, "neither text nor image"), "{f:?}");
}

#[test]
fn missing_font_and_broken_documents_fail() {
    let d = dir("nofont");
    let xml = dc_subtitle(SUB, "Font1", TWO);
    package(&d, true, ("subs.xml", xml.as_bytes()), &[("sub2.png", PNG)], &interop_asset(240));
    let f = findings(&validate(&d));
    assert!(f.iter().any(|(s, m)| *s == Status::Fail && m.contains("the font 'Font1' (font.ttf) is not in the package")), "{f:?}");

    let r = interop("garbage", "<NotSubtitles/>", PNG, 240);
    let f = findings(&r);
    assert!(f.iter().any(|(s, m)| *s == Status::Fail && m.contains("neither a subtitle document nor a timed text track file")), "{f:?}");

    let bad_time = dc_subtitle(SUB, "Font1", r#"<Subtitle SpotNumber="1" TimeIn="soon" TimeOut="00:00:02:000"><Text>a</Text></Subtitle>"#);
    let f = findings(&interop("badtime", &bad_time, PNG, 240));
    assert!(f.iter().any(|(s, m)| *s == Status::Fail && m.contains("TimeIn 'soon' is not a time")), "{f:?}");
}

#[test]
fn interop_subtitle_in_a_smpte_composition_fails() {
    let d = dir("mixed");
    let xml = dc_subtitle(SUB, "Font1", TWO);
    let asset = format!("<MainSubtitle><Id>urn:uuid:{SUB}</Id><EditRate>24 1</EditRate><IntrinsicDuration>240</IntrinsicDuration></MainSubtitle>");
    package(&d, false, ("sub/subs.xml", xml.as_bytes()), &[("sub/font.ttf", b"\0\x01\0\0"), ("sub/sub2.png", PNG)], &asset);
    let f = findings(&validate(&d));
    assert!(f.iter().any(|(s, m)| *s == Status::Fail && m.contains("is an Interop subtitle document in a SMPTE composition")), "{f:?}");
}

fn subtitle_reel(id: &str, rate: &str, image: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST"><Id>urn:uuid:{id}</Id><ContentTitleText>T</ContentTitleText><ReelNumber>1</ReelNumber><Language>hu</Language><EditRate>{rate}</EditRate><TimeCodeRate>24</TimeCodeRate><StartTime>00:00:00:00</StartTime>
<LoadFont ID="Font1">urn:uuid:20202020-2020-2020-2020-202020202020</LoadFont>
<SubtitleList><Font ID="Font1">
<Subtitle SpotNumber="1" TimeIn="00:00:01:00" TimeOut="00:00:03:00" FadeUpTime="00:00:00:02" FadeDownTime="00:00:00:02"><Text Vposition="10">Szia</Text></Subtitle>
<Subtitle SpotNumber="2" TimeIn="00:00:04:00" TimeOut="00:00:06:12"><Image>urn:uuid:{image}</Image></Subtitle>
</Font></SubtitleList></SubtitleReel>"#
    )
}

fn smpte(name: &str, xml: &str, image: &[u8]) -> ValidationReport {
    let d = dir(name);
    let mxf = cfa4_mxf::testing::build_timed_text(xml.as_bytes(), &[("application/x-font-opentype", b"OTTO".as_slice()), ("image/png", image)]);
    let asset = format!("<MainSubtitle><Id>urn:uuid:{SUB}</Id><EditRate>24 1</EditRate><IntrinsicDuration>240</IntrinsicDuration></MainSubtitle>");
    package(&d, false, ("subs.mxf", &mxf), &[], &asset);
    validate(&d)
}

#[test]
fn a_good_smpte_timed_text_track_file_passes() {
    let r = smpte("smpte-good", &subtitle_reel(RESOURCE, "24 1", "21212121-2121-2121-2121-212121212121"), PNG);
    assert!(passed(&r), "{:?}", findings(&r));
    assert_eq!(findings(&r).len(), 1, "{:?}", findings(&r));
}

#[test]
fn smpte_resource_id_edit_rate_and_images() {
    let r = smpte("smpte-bad", &subtitle_reel("22222222-2222-4222-8222-222222222222", "25 1", "33333333-3333-4333-8333-333333333333"), b"JFIF");
    let f = findings(&r);
    let has = |m: &str| f.iter().any(|(s, msg)| *s == Status::Fail && msg.contains(m));
    assert!(has(&format!("is not the track file's timed text resource id urn:uuid:{RESOURCE}")), "{f:?}");
    assert!(has("has the EditRate 25 1, the reel asset 24 1"), "{f:?}");
    assert!(has("the image 'urn:uuid:33333333-3333-4333-8333-333333333333' is not in the package"), "{f:?}");

    let r = smpte("smpte-jpeg", &subtitle_reel(RESOURCE, "24 1", "21212121-2121-2121-2121-212121212121"), b"JFIF");
    let f = findings(&r);
    assert!(f.iter().any(|(s, m)| *s == Status::Fail && m.contains("is not PNG")), "{f:?}");
}

#[test]
fn profiles_without_the_rule_do_not_read_subtitles() {
    let d = dir("quick");
    let xml = "<NotSubtitles/>";
    package(&d, true, ("subs.xml", xml.as_bytes()), &[], &interop_asset(240));
    let req = ValidationRequest::new(&d, Profile::builtin("quick-scan").unwrap());
    let r = NativeValidator.validate(&req, &ValidationControl::new()).unwrap();
    assert!(findings(&r).is_empty());
}
