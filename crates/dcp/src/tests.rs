use super::*;

const H: &str = "2jmj7l5rSw0yVb/vlWAYkK/YBwk="; // SHA-1 of the empty string

fn smpte_cpl() -> String {
    // Laid out like the CPL of `cfa4-export`'s DCP writer: picture, sound, ST 429-16 metadata and
    // an immersive-audio AuxData in the Dolby namespace with default-namespace children.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:11111111-2222-4333-8444-555555555555</Id>
  <AnnotationText>Test</AnnotationText>
  <IssueDate>2026-11-05T00:00:00+00:00</IssueDate>
  <Issuer>CFA4</Issuer>
  <Creator>CFA4</Creator>
  <ContentTitleText>Test_FTR_F_EN-XX_51_2K_20261105_SMPTE_OV</ContentTitleText>
  <ContentKind>feature</ContentKind>
  <ContentVersion>
    <Id>urn:uuid:21111111-2222-4333-8444-555555555555</Id>
    <LabelText>Test</LabelText>
  </ContentVersion>
  <RatingList/>
  <ReelList>
    <Reel>
      <Id>urn:uuid:31111111-2222-4333-8444-555555555555</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:41111111-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>48</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>48</Duration>
          <Hash>{H}</Hash>
          <FrameRate>24 1</FrameRate>
          <ScreenAspectRatio>1998 1080</ScreenAspectRatio>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:51111111-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>48</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>48</Duration>
          <Hash>{H}</Hash>
        </MainSound>
        <meta:CompositionMetadataAsset xmlns:meta="http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata">
          <Id>urn:uuid:61111111-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>48</IntrinsicDuration>
          <meta:FullContentTitleText>Test</meta:FullContentTitleText>
          <meta:MainSoundConfiguration>51/L,R,C,LFE,Ls,Rs</meta:MainSoundConfiguration>
        </meta:CompositionMetadataAsset>
        <axd:AuxData xmlns:axd="http://www.dolby.com/schemas/2012/AD">
          <Id>urn:uuid:71111111-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>48</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>48</Duration>
          <Hash>{H}</Hash>
          <axd:DataType>urn:smpte:ul:060e2b34.04010105.0e090604.00000000</axd:DataType>
        </axd:AuxData>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>
"#
    )
}

const INTEROP_CPL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#" xmlns:msp="http://www.digicine.com/schemas/437-Y/2007/Main-Stereo-Picture-CPL">
  <Id>urn:uuid:aaaaaaaa-2222-4333-8444-555555555555</Id>
  <IssueDate>2010-01-01T00:00:00+00:00</IssueDate>
  <ContentTitleText>Interop</ContentTitleText>
  <ContentKind>trailer</ContentKind>
  <ReelList>
    <Reel>
      <Id>urn:uuid:bbbbbbbb-2222-4333-8444-555555555555</Id>
      <AssetList>
        <msp:MainStereoscopicPicture>
          <Id>urn:uuid:cccccccc-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>240</IntrinsicDuration>
          <EntryPoint>24</EntryPoint>
          <Duration>200</Duration>
          <FrameRate>48 1</FrameRate>
          <ScreenAspectRatio>1.85</ScreenAspectRatio>
        </msp:MainStereoscopicPicture>
        <MainSubtitle>
          <Id>urn:uuid:dddddddd-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>240</IntrinsicDuration>
          <Language>en</Language>
        </MainSubtitle>
        <MainMarkers>
          <Id>urn:uuid:eeeeeeee-2222-4333-8444-555555555555</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>240</IntrinsicDuration>
          <MarkerList>
            <Marker><Label>FFOC</Label><Offset>1</Offset></Marker>
            <Marker><Label>LFOC</Label><Offset>239</Offset></Marker>
          </MarkerList>
        </MainMarkers>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>
"#;

const SMPTE_PKL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:11111111-2222-4333-8444-555555555555</Id>
  <IssueDate>2026-11-05T00:00:00+00:00</IssueDate>
  <Issuer>CFA4</Issuer>
  <Creator>CFA4</Creator>
  <AssetList>
    <Asset>
      <Id>urn:uuid:41111111-2222-4333-8444-555555555555</Id>
      <Hash>2jmj7l5rSw0yVb/vlWAYkK/YBwk=</Hash>
      <Size>1024</Size>
      <Type>application/mxf</Type>
      <OriginalFileName>picture.mxf</OriginalFileName>
    </Asset>
    <Asset>
      <Id>41111111-2222-4333-8444-555555555556</Id>
      <Hash>not base64!</Hash>
      <Size>-1</Size>
      <Type>text/xml</Type>
      <Colour>red</Colour>
    </Asset>
  </AssetList>
</PackingList>
"#;

const INTEROP_AM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.digicine.com/PROTO-ASDCP-AM-20040311#">
  <Id>urn:uuid:11111111-2222-4333-8444-555555555555</Id>
  <Creator>x</Creator>
  <VolumeCount>1</VolumeCount>
  <IssueDate>2010-01-01T00:00:00+00:00</IssueDate>
  <Issuer>x</Issuer>
  <AssetList>
    <Asset>
      <Id>urn:uuid:21111111-2222-4333-8444-555555555555</Id>
      <PackingList/>
      <ChunkList>
        <Chunk><Path>pkl.xml</Path><VolumeIndex>1</VolumeIndex><Offset>0</Offset><Length>100</Length></Chunk>
      </ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:31111111-2222-4333-8444-555555555555</Id>
      <ChunkList>
        <Chunk><Path>picture.mxf</Path></Chunk>
      </ChunkList>
    </Asset>
  </AssetList>
</AssetMap>
"#;

fn composition(text: &str) -> Composition {
    match parse(text, Limits::default()) {
        Ok(Document::Composition(c)) => c,
        other => panic!("not a composition: {other:?}"),
    }
}

#[test]
fn smpte_cpl_in_the_export_layout() {
    let c = composition(&smpte_cpl());
    assert_eq!(c.standard, Standard::Smpte);
    assert!(c.issues.is_empty(), "{:#?}", c.issues);
    assert_eq!(c.content_kind.as_deref(), Some("feature"));
    assert_eq!(c.content_version.as_ref().and_then(|v| v.label.as_deref()), Some("Test"));
    assert_eq!(c.reels.len(), 1);
    let kinds: Vec<AssetKind> = c.reels[0].assets.iter().map(|a| a.kind).collect();
    assert_eq!(kinds, [AssetKind::MainPicture, AssetKind::MainSound, AssetKind::CompositionMetadata, AssetKind::AuxData]);
    let pic = &c.reels[0].assets[0];
    assert_eq!(pic.edit_rate, Some(Rational { num: 24, den: 1 }));
    assert_eq!(pic.duration, Some(48));
    assert_eq!(pic.screen_aspect_ratio, Some(ScreenAspectRatio::Ratio(Rational { num: 1998, den: 1080 })));
    assert_eq!(pic.sha1.map(|d| d[0]), Some(0xda));
    let meta = &c.reels[0].assets[2];
    assert_eq!(meta.element, format!("{{{}}}CompositionMetadataAsset", ns::SMPTE_CPL_METADATA));
    assert_eq!(meta.extensions.len(), 2);
    let aux = &c.reels[0].assets[3];
    assert_eq!(aux.element, "{http://www.dolby.com/schemas/2012/AD}AuxData");
    assert_eq!(aux.data_type.as_deref(), Some("urn:smpte:ul:060e2b34.04010105.0e090604.00000000"));
    assert!(aux.extensions.is_empty());
}

#[test]
fn interop_cpl_with_stereo_subtitle_and_markers() {
    let c = composition(INTEROP_CPL);
    assert_eq!(c.standard, Standard::Interop);
    assert!(c.issues.is_empty(), "{:#?}", c.issues);
    let a = &c.reels[0].assets;
    assert_eq!(a[0].kind, AssetKind::MainStereoscopicPicture);
    assert_eq!(a[0].screen_aspect_ratio, Some(ScreenAspectRatio::Decimal("1.85".into())));
    assert_eq!(a[0].entry_point, Some(24));
    assert_eq!(a[1].kind, AssetKind::MainSubtitle);
    assert_eq!(a[1].language.as_deref(), Some("en"));
    assert_eq!(a[2].markers.len(), 2);
    assert_eq!(a[2].markers[1].label.as_deref(), Some("LFOC"));
    assert_eq!(a[2].markers[1].offset, Some(239));
}

#[test]
fn cpl_problems_are_issues_with_positions() {
    let text = smpte_cpl()
        .replace("<Duration>48</Duration>\n          <Hash>", "<Duration>x</Duration>\n          <Hash>")
        .replace("<ContentKind>feature</ContentKind>", "")
        .replace(
            "<EditRate>24 1</EditRate>\n          <IntrinsicDuration>48</IntrinsicDuration>\n          <EntryPoint>",
            "<EditRate>24 0</EditRate>\n          <IntrinsicDuration>48</IntrinsicDuration>\n          <EntryPoint>",
        )
        .replace("<ScreenAspectRatio>1998 1080</ScreenAspectRatio>", "<ScreenAspectRatio>wide</ScreenAspectRatio><Foo/>");
    let c = composition(&text);
    let has = |kind: IssueKind, path: &str| c.issues.iter().any(|i| i.kind == kind && i.path == path);
    assert!(has(IssueKind::Missing, "CompositionPlaylist/ContentKind"), "{:#?}", c.issues);
    assert!(has(IssueKind::Invalid, "CompositionPlaylist/ReelList/Reel[1]/AssetList/MainPicture[1]/Duration"));
    assert!(has(IssueKind::Invalid, "CompositionPlaylist/ReelList/Reel[1]/AssetList/MainPicture[1]/EditRate"));
    assert!(has(IssueKind::Invalid, "CompositionPlaylist/ReelList/Reel[1]/AssetList/MainPicture[1]/ScreenAspectRatio"));
    assert!(has(IssueKind::Unknown, "CompositionPlaylist/ReelList/Reel[1]/AssetList/MainPicture[1]/Foo"));
    let dur = c.issues.iter().find(|i| i.path.ends_with("MainPicture[1]/Duration")).map(|i| i.pos);
    assert_eq!(dur.map(|p| p.line), Some(24));
    assert_eq!(c.reels[0].assets[0].duration, None);
}

#[test]
fn smpte_pkl() {
    let Ok(Document::PackingList(p)) = parse(SMPTE_PKL, Limits::default()) else { panic!("not a PKL") };
    assert_eq!(p.standard, Standard::Smpte);
    assert_eq!(p.assets.len(), 2);
    assert_eq!(p.assets[0].size, Some(1024));
    assert_eq!(p.assets[0].sha1.map(|d| d[19]), Some(0x09));
    assert_eq!(p.assets[1].id, Uuid::parse("urn:uuid:41111111-2222-4333-8444-555555555556"));
    let kinds: Vec<(IssueKind, &str)> = p.issues.iter().map(|i| (i.kind, i.path.as_str())).collect();
    assert_eq!(
        kinds,
        [
            (IssueKind::Invalid, "PackingList/AssetList/Asset[2]/Id"),
            (IssueKind::Invalid, "PackingList/AssetList/Asset[2]/Hash"),
            (IssueKind::Invalid, "PackingList/AssetList/Asset[2]/Size"),
            (IssueKind::Unknown, "PackingList/AssetList/Asset[2]/Colour"),
        ]
    );
    assert!(p.issues[0].message.contains("urn:uuid: prefix"));
}

#[test]
fn interop_asset_map() {
    let Ok(Document::AssetMap(m)) = parse(INTEROP_AM, Limits::default()) else { panic!("not an asset map") };
    assert_eq!(m.standard, Standard::Interop);
    assert!(m.issues.is_empty(), "{:#?}", m.issues);
    assert!(m.assets[0].packing_list);
    assert!(!m.assets[1].packing_list);
    assert_eq!(m.assets[0].chunks[0].length, Some(100));
    assert_eq!(m.assets[1].chunks[0].path.as_deref(), Some("picture.mxf"));
}

#[test]
fn smpte_volume_index() {
    let t = r#"<VolumeIndex xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Index>1</Index></VolumeIndex>"#;
    let Ok(Document::VolumeIndex(v)) = parse(t, Limits::default()) else { panic!("not a volume index") };
    assert_eq!((v.standard, v.index), (Standard::Smpte, Some(1)));
}

#[test]
fn repeated_element_is_reported_once() {
    let t = SMPTE_PKL.replace("<Issuer>CFA4</Issuer>", "<Issuer>a</Issuer><Issuer>b</Issuer>");
    let Ok(Document::PackingList(p)) = parse(&t, Limits::default()) else { panic!("not a PKL") };
    assert_eq!(p.issuer.as_deref(), Some("a"));
    assert_eq!(p.issues.iter().filter(|i| i.kind == IssueKind::Repeated).count(), 1);
}

#[test]
fn signature_presence() {
    let t = SMPTE_PKL.replace("</PackingList>", "<Signer/><ds:Signature xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"/></PackingList>");
    let Ok(Document::PackingList(p)) = parse(&t, Limits::default()) else { panic!("not a PKL") };
    assert!(p.has_signer && p.has_signature);
}

#[test]
fn errors() {
    assert!(matches!(parse("<x/>", Limits::default()), Err(Error::UnknownDocument(n)) if n == "x"));
    assert!(matches!(parse("<AssetMap xmlns=\"urn:other\"/>", Limits::default()), Err(Error::UnknownDocument(_))));
    assert!(matches!(parse("<a>", Limits::default()), Err(Error::Xml(_))));
    let dtd = "<!DOCTYPE x [<!ENTITY a \"aaaa\">]><AssetMap xmlns=\"http://www.smpte-ra.org/schemas/429-9/2007/AM\">&a;</AssetMap>";
    assert!(matches!(parse(dtd, Limits::default()), Err(Error::Xml(_))));
    let small = Limits { max_bytes: 10, ..Limits::default() };
    assert!(matches!(parse(SMPTE_PKL, small), Err(Error::TooLarge { .. })));
    let few = Limits { max_nodes: 20, ..Limits::default() };
    assert!(matches!(parse(SMPTE_PKL, few), Err(Error::Xml(_))));
    assert!(matches!(parse_bytes(&[0xff, 0xfe], Limits::default()), Err(Error::NotUtf8)));
    let bom = [&[0xEF, 0xBB, 0xBF][..], INTEROP_AM.as_bytes()].concat();
    assert!(matches!(parse_bytes(&bom, Limits::default()), Ok(Document::AssetMap(_))));
}

#[test]
fn uuid_text() {
    let u = Uuid::parse("URN:UUID:AAAAAAAA-2222-4333-8444-555555555555").expect("uuid");
    assert_eq!(u.to_string(), "urn:uuid:aaaaaaaa-2222-4333-8444-555555555555");
    assert_eq!(Uuid::parse("aaaaaaaa-2222-4333-8444-555555555555"), None);
    assert_eq!(Uuid::parse_lenient("aaaaaaaa-2222-4333-8444-555555555555"), Some(u));
    for bad in ["", "urn:uuid:", "urn:uuid:aaaaaaaa-2222-4333-8444-55555555555g", "urn:uuid:aaaaaaaa22222-4333-8444-555555555555", "urn:uuid:é"] {
        assert_eq!(Uuid::parse_lenient(bad), None, "{bad}");
    }
    let json = serde_json::to_string(&u).expect("json");
    assert_eq!(serde_json::from_str::<Uuid>(&json).ok(), Some(u));
}

#[test]
fn rational_text() {
    assert_eq!(Rational::parse("24000 1001"), Some(Rational { num: 24000, den: 1001 }));
    assert!(Rational::parse("48 2").is_some_and(|r| r.same_value(Rational { num: 24, den: 1 })));
    for bad in ["", "24", "24 0", "24 -1", "24 1 1", "x 1", "99999999999999999999 1"] {
        assert_eq!(Rational::parse(bad), None, "{bad}");
    }
}

#[test]
fn base64_is_strict() {
    assert_eq!(decode_base64(""), Some(vec![]));
    assert_eq!(decode_base64("TWE="), Some(b"Ma".to_vec()));
    assert_eq!(decode_base64("TQ=="), Some(b"M".to_vec()));
    for bad in ["TQ=", "T===", "TQ==TWFu", "TW E", "TWF\n"] {
        assert_eq!(decode_base64(bad), None, "{bad}");
    }
    assert_eq!(decode_sha1(H).map(|d| d.len()), Some(20));
    assert_eq!(decode_sha1("TWFu"), None);
}

#[test]
fn model_round_trips_through_json() {
    let d = parse(&smpte_cpl(), Limits::default()).expect("cpl");
    let json = serde_json::to_string(&d).expect("json");
    assert_eq!(serde_json::from_str::<Document>(&json).ok(), Some(d));
}

/// Every prefix and many single-byte corruptions of each sample: an error or a model, never a panic.
#[test]
fn hostile_input_never_panics() {
    let samples = [smpte_cpl(), INTEROP_CPL.to_string(), SMPTE_PKL.to_string(), INTEROP_AM.to_string()];
    for s in &samples {
        let b = s.as_bytes();
        for cut in (0..b.len()).step_by(7) {
            let r = std::panic::catch_unwind(|| parse_bytes(&b[..cut], Limits::default()).map(|d| d.issues().len()));
            assert!(r.is_ok(), "panic at prefix {cut}");
        }
        let mut seed = 0x9e37_79b9_u32;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let mut m = b.to_vec();
            let i = seed as usize % m.len();
            m[i] = (seed >> 8) as u8;
            let r = std::panic::catch_unwind(|| parse_bytes(&m, Limits::default()).map(|d| d.issues().len()));
            assert!(r.is_ok(), "panic with byte {i} set");
        }
    }
}

#[test]
fn hash_whitespace_is_collapsed() {
    assert_eq!(decode_sha1("2jmj7l5rSw0yVb/v\n  lWAYkK/YBwk="), decode_sha1(H));
    let t = SMPTE_PKL.replace("<Hash>2jmj7l5rSw0yVb/vlWAYkK/YBwk=</Hash>", "<Hash>2jmj7l5rSw0y\n Vb/vlWAYkK/YBwk=</Hash>");
    let Ok(Document::PackingList(p)) = parse(&t, Limits::default()) else { panic!("not a PKL") };
    assert!(p.assets[0].sha1.is_some());
    assert!(!p.issues.iter().any(|i| i.path == "PackingList/AssetList/Asset[1]/Hash"));
}

#[test]
fn schema_elements_must_be_in_the_document_namespace() {
    let t = SMPTE_PKL.replace("<Issuer>CFA4</Issuer>", "<x:Issuer xmlns:x=\"urn:other\">CFA4</x:Issuer>");
    let Ok(Document::PackingList(p)) = parse(&t, Limits::default()) else { panic!("not a PKL") };
    assert_eq!(p.issuer, None);
    assert!(p.issues.iter().any(|i| i.kind == IssueKind::Missing && i.path == "PackingList/Issuer"));
    assert!(p.extensions.contains(&"{urn:other}Issuer".to_string()));
    // a foreign reel asset keeps its default-namespace children (the export's AuxData)
    let c = composition(&smpte_cpl());
    assert_eq!(c.reels[0].assets[3].id.map(|u| u.to_string()).as_deref(), Some("urn:uuid:71111111-2222-4333-8444-555555555555"));
}

#[test]
fn smpte_packing_list_flag_is_a_boolean() {
    let smpte = INTEROP_AM.replace(ns::INTEROP_AM, ns::SMPTE_AM);
    let Ok(Document::AssetMap(m)) = parse(&smpte, Limits::default()) else { panic!("not an asset map") };
    assert!(!m.assets[0].packing_list);
    assert!(m.issues.iter().any(|i| i.kind == IssueKind::Invalid && i.path == "AssetMap/AssetList/Asset[1]/PackingList"));
}

#[test]
fn pkl_size_is_positive_and_issue_date_a_date_time() {
    let t = SMPTE_PKL
        .replace("<Size>1024</Size>", "<Size>0</Size>")
        .replace("<IssueDate>2026-11-05T00:00:00+00:00</IssueDate>", "<IssueDate>not-a-date</IssueDate>");
    let Ok(Document::PackingList(p)) = parse(&t, Limits::default()) else { panic!("not a PKL") };
    assert_eq!(p.assets[0].size, None);
    assert!(p.issues.iter().any(|i| i.kind == IssueKind::Invalid && i.path == "PackingList/AssetList/Asset[1]/Size"));
    assert_eq!(p.issue_date, None);
    assert!(p.issues.iter().any(|i| i.kind == IssueKind::Invalid && i.path == "PackingList/IssueDate"));
    for ok in ["2026-11-09T21:00:00Z", "2026-11-09T21:00:00+01:00", "2026-11-09T21:00:00.25-05:30", "2026-11-09T21:00:00", "2024-02-29T00:00:00Z"] {
        assert!(xml::is_date_time(ok), "{ok}");
    }
    for bad in ["", "2026-11-09", "2026-11-09 21:00:00Z", "2026-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "2026-11-09T25:00:00Z", "2026-11-09t21:00:00z"] {
        assert!(!xml::is_date_time(bad), "{bad}");
    }
}

#[test]
fn smpte_picture_needs_rational_frame_rate_and_aspect_ratio() {
    let t = smpte_cpl()
        .replace("<FrameRate>24 1</FrameRate>", "")
        .replace("<ScreenAspectRatio>1998 1080</ScreenAspectRatio>", "<ScreenAspectRatio>1.85</ScreenAspectRatio>");
    let c = composition(&t);
    let has = |kind: IssueKind, path: &str| c.issues.iter().any(|i| i.kind == kind && i.path == path);
    assert!(has(IssueKind::Missing, "CompositionPlaylist/ReelList/Reel[1]/AssetList/MainPicture[1]/FrameRate"), "{:#?}", c.issues);
    assert!(has(IssueKind::Invalid, "CompositionPlaylist/ReelList/Reel[1]/AssetList/MainPicture[1]/ScreenAspectRatio"));
    // Interop keeps its decimal form without an issue (`interop_cpl_with_stereo_subtitle_and_markers`)
}

#[test]
fn schema_order_and_positive_volume_fields() {
    let t = INTEROP_AM
        .replace("<VolumeCount>1</VolumeCount>", "<VolumeCount>0</VolumeCount>")
        .replace("<Issuer>x</Issuer>", "")
        .replace("<Creator>x</Creator>", "<Issuer>x</Issuer><Creator>x</Creator>");
    let Ok(Document::AssetMap(m)) = parse(&t, Limits::default()) else { panic!("not an asset map") };
    assert_eq!(m.volume_count, None);
    assert!(m.issues.iter().any(|i| i.kind == IssueKind::Invalid && i.path == "AssetMap/VolumeCount"));
    let order: Vec<&Issue> = m.issues.iter().filter(|i| i.message.contains("schema order")).collect();
    assert_eq!(order.len(), 1, "{:#?}", m.issues);
    assert_eq!(order[0].path, "AssetMap/Creator");
    assert_eq!(m.issuer.as_deref(), Some("x"), "an out-of-order element is still read");
    let t = r#"<VolumeIndex xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Index>0</Index></VolumeIndex>"#;
    let Ok(Document::VolumeIndex(v)) = parse(t, Limits::default()) else { panic!("not a volume index") };
    assert_eq!(v.index, None);
    // KeyId comes before Hash in a track file asset
    let c = composition(&smpte_cpl().replacen("<Hash>", "<KeyId>urn:uuid:81111111-2222-4333-8444-555555555555</KeyId><Hash>", 1));
    assert!(c.issues.is_empty(), "{:#?}", c.issues);
    let c = composition(&smpte_cpl().replacen(
        "<FrameRate>24 1</FrameRate>",
        "<FrameRate>24 1</FrameRate><KeyId>urn:uuid:81111111-2222-4333-8444-555555555555</KeyId>",
        1,
    ));
    assert!(c.issues.iter().any(|i| i.path.ends_with("MainPicture[1]/KeyId") && i.message.contains("schema order")), "{:#?}", c.issues);
}

#[test]
fn content_kind_version_and_volume_count() {
    let c = composition(&smpte_cpl().replace("<ContentKind>feature</ContentKind>", "<ContentKind>movie</ContentKind>"));
    assert!(c.issues.iter().any(|i| i.kind == IssueKind::Invalid && i.path == "CompositionPlaylist/ContentKind"));
    assert_eq!(c.content_kind.as_deref(), Some("movie"), "the value is kept");
    let scoped =
        smpte_cpl().replace("<ContentKind>feature</ContentKind>", "<ContentKind scope=\"http://isdcf.com/ns/cplmd/content-kind\">episode</ContentKind>");
    assert!(composition(&scoped).issues.is_empty());

    let start = smpte_cpl().find("<ContentVersion>").unwrap();
    let end = smpte_cpl().find("</ContentVersion>").unwrap() + "</ContentVersion>".len();
    let mut no_version = smpte_cpl();
    no_version.replace_range(start..end, "");
    let c = composition(&no_version);
    assert!(c.issues.iter().any(|i| i.kind == IssueKind::Missing && i.path == "CompositionPlaylist/ContentVersion"));
    let c = composition(&smpte_cpl().replace("<LabelText>Test</LabelText>", ""));
    assert!(c.issues.iter().any(|i| i.kind == IssueKind::Missing && i.path == "CompositionPlaylist/ContentVersion/LabelText"));
    assert!(composition(INTEROP_CPL).issues.is_empty(), "Interop needs no ContentVersion");

    let two = INTEROP_AM
        .replace(ns::INTEROP_AM, ns::SMPTE_AM)
        .replace("<PackingList/>", "<PackingList>true</PackingList>")
        .replace("<VolumeCount>1</VolumeCount>", "<VolumeCount>2</VolumeCount>");
    let Ok(Document::AssetMap(m)) = parse(&two, Limits::default()) else { panic!("not an asset map") };
    assert!(m.issues.iter().any(|i| i.kind == IssueKind::Invalid && i.path == "AssetMap/VolumeCount"));
    let Ok(Document::AssetMap(m)) = parse(&INTEROP_AM.replace("<VolumeCount>1</VolumeCount>", "<VolumeCount>2</VolumeCount>"), Limits::default()) else {
        panic!("not an asset map")
    };
    assert!(m.issues.is_empty(), "{:#?}", m.issues);
}
