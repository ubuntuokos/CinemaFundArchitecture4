use super::*;

const REEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- authored by hand -->
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <Id>urn:uuid:7c5bd5b8-3b0f-4a3b-9b1e-1c2d3e4f5a6b</Id>
  <ContentTitleText>Test Reel</ContentTitleText>
  <IssueDate>2026-11-11T00:00:00Z</IssueDate>
  <Language>hu</Language>
  <EditRate>24 1</EditRate>
  <TimeCodeRate>24</TimeCodeRate>
  <StartTime>00:00:00:00</StartTime>
  <LoadFont ID="f1">urn:uuid:3dec6dc0-39d0-498d-97d0-928d2eb78391</LoadFont>
  <SubtitleList>
    <Font ID="f1" Size="42" Weight="normal" Color="FFFFFFFF">
      <Subtitle SpotNumber="1" TimeIn="00:00:05:00" TimeOut="00:00:07:12" FadeUpTime="00:00:00:00" FadeDownTime="00:00:00:00">
        <Text Valign="bottom" Vposition="8" Halign="center">második sor</Text>
        <Text Valign="bottom" Vposition="16" Halign="center">Első sor</Text>
      </Subtitle>
      <Subtitle SpotNumber="2" TimeIn="00:00:08:00" TimeOut="00:00:10:00">
        <Font Italic="yes"><Text Valign="top" Vposition="10">Dőlt <Font Weight="bold">és vastag</Font></Text></Font>
      </Subtitle>
      <Subtitle SpotNumber="3" TimeIn="00:00:11:00" TimeOut="00:00:12:00">
        <Text>漢<Ruby><Rb>字</Rb><Rt>じ</Rt></Ruby><Space Size="0.5"/>end</Text>
      </Subtitle>
      <Subtitle SpotNumber="4" TimeIn="00:00:13:00" TimeOut="00:00:14:00">
        <Image Valign="bottom" Vposition="10">urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Image>
      </Subtitle>
    </Font>
  </SubtitleList>
</SubtitleReel>
"#;

fn secs(s: i64, frames: i64) -> Tick {
    Tick::from_rational(s * 24 + frames, 1, 24)
}

#[test]
fn text_subtitles_become_cues() {
    assert!(sniff(REEL));
    let r = parse_reel(REEL).unwrap();
    assert_eq!(r.title.as_deref(), Some("Test Reel"));
    assert_eq!(r.language.as_deref(), Some("hu"));
    assert_eq!(r.id.as_deref(), Some("urn:uuid:7c5bd5b8-3b0f-4a3b-9b1e-1c2d3e4f5a6b"));
    let c = &r.document.cues;
    assert_eq!(c.len(), 3);
    // lines in screen order: the higher Vposition from the bottom comes first
    assert_eq!(c[0].text, "Első sor\nmásodik sor");
    assert_eq!((c[0].start, c[0].end), (secs(5, 0), secs(7, 12)));
    assert_eq!(c[0].id.as_deref(), Some("1"));
    assert_eq!(c[1].text, "<i>Dőlt <b>és vastag</b></i>");
    assert_eq!(c[2].text, "漢字 end");
    assert_eq!(r.images.len(), 1);
    assert!(r.document.warnings.is_empty(), "{:?}", r.document.warnings);
    // the cue-only parse still says what it left out
    assert_eq!(parse(REEL).unwrap().warnings.len(), 1);
}

/// #936: image subtitles keep their resource, times, fades and position.
#[test]
fn image_subtitles_are_listed_with_position_and_fades() {
    let x = r#"<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2014/DCST" IntrinsicPictureResolution="2K">
      <EditRate>24 1</EditRate><TimeCodeRate>24</TimeCodeRate><StartTime>00:00:00:00</StartTime><SubtitleList>
      <Subtitle SpotNumber="7" TimeIn="00:00:01:00" TimeOut="00:00:03:00" FadeUpTime="00:00:00:06" FadeDownTime="00:00:00:00">
        <Image Valign="bottom" Vposition="10" Halign="left" Hposition="5">urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Image>
        <Image>urn:uuid:00112233-4455-6677-8899-aabbccddeeff</Image>
        <Image>urn:uuid:not-a-uuid</Image>
      </Subtitle></SubtitleList></SubtitleReel>"#;
    let r = parse_reel(x).unwrap();
    assert_eq!(r.intrinsic_resolution.as_deref(), Some("2K"));
    assert_eq!(r.images.len(), 2);
    assert_eq!(r.document.warnings.len(), 1, "{:?}", r.document.warnings);
    let a = &r.images[0];
    assert_eq!((a.start, a.end), (secs(1, 0), secs(3, 0)));
    assert_eq!((a.fade_in, a.fade_out), (secs(0, 6), Tick::ZERO));
    assert_eq!(a.resource, [0xaa, 0xaa, 0xaa, 0xaa, 0xbb, 0xbb, 0xcc, 0xcc, 0xdd, 0xdd, 0xee, 0xee, 0xee, 0xee, 0xee, 0xee]);
    assert_eq!(a.spot.as_deref(), Some("7"));
    // left edge 5% in, bottom edge 10% up: a 200 x 100 image over 2000 x 1000
    assert_eq!(a.centre((2000.0, 1000.0), (200.0, 100.0)), (200.0, 850.0));
    // defaults: centred; the fades are the subtitle's
    let b = &r.images[1];
    assert_eq!((b.h_align, b.v_align), (Align::Center, Align::Center));
    assert_eq!((b.fade_in, b.fade_out), (a.fade_in, a.fade_out));
    // a subtitle without fade times fades over two units each way
    let d = &parse_reel(REEL).unwrap().images[0];
    assert_eq!((d.fade_in, d.fade_out), (secs(0, 2), secs(0, 2)));
    assert_eq!(b.centre((2000.0, 1000.0), (200.0, 100.0)), (1000.0, 500.0));
    assert_eq!(parse_uuid("00112233-4455-6677-8899-AABBCCDDEEFF"), Some(b.resource));
    assert_eq!(parse_uuid("urn:uuid:0011223344556677-8899-aabbccddeeff00"), None);
}

#[test]
fn times_count_from_the_default_start_time_at_the_edit_rate() {
    let x = r#"<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2007/DCST"><EditRate>24000 1001</EditRate><TimeCodeRate>24</TimeCodeRate>
      <SubtitleList><Subtitle TimeIn="01:00:01:00" TimeOut="01:00:02:12"><Text>a</Text></Subtitle>
      <Subtitle TimeIn="00:59:00:00" TimeOut="01:00:00:10"><Text>before start</Text></Subtitle></SubtitleList></SubtitleReel>"#;
    let d = parse(x).unwrap();
    assert_eq!(d.cues.len(), 1);
    // 24 units at 24000/1001 per second = 1.001 s
    assert_eq!(d.cues[0].start, Tick::from_rational(24, 1001, 24000));
    assert_eq!(d.cues[0].end, Tick::from_rational(60, 1001, 24000));
    assert_eq!(d.warnings.len(), 1);
}

#[test]
fn three_digit_edit_units_and_derived_time_code_rate() {
    let x = r#"<SubtitleReel><EditRate>250 1</EditRate><StartTime>10:00:00:000</StartTime>
      <SubtitleList><Subtitle TimeIn="10:00:00:125" TimeOut="10:00:01:000"><Text>x</Text></Subtitle></SubtitleList></SubtitleReel>"#;
    let d = parse(x).unwrap();
    assert_eq!((d.cues[0].start, d.cues[0].end), (Tick::from_rational(1, 1, 2), Tick::from_rational(1, 1, 1)));
}

#[test]
fn not_dcst() {
    assert!(!sniff("<tt xmlns=\"http://www.w3.org/ns/ttml\"/>"));
    assert!(!sniff(""));
    assert!(matches!(parse("<tt/>"), Err(Error::NotFormat(_))));
    assert!(parse("<SubtitleReel><EditRate>0 1</EditRate></SubtitleReel>").is_err());
    assert!(parse("not xml").is_err());
}

#[test]
fn hostile_documents_never_panic() {
    let deep = format!("<SubtitleReel>{}{}</SubtitleReel>", "<Font>".repeat(5000), "</Font>".repeat(5000));
    assert!(parse(&deep).is_err());
    for bad in [
        "<SubtitleReel><EditRate>1000000 1</EditRate><StartTime>99:59:59:999999</StartTime><SubtitleList><Subtitle TimeIn=\"99:59:59:999999\" TimeOut=\"00:00:00:00\"><Text>x</Text></Subtitle></SubtitleList></SubtitleReel>",
        "<SubtitleReel><SubtitleList><Subtitle TimeIn=\"a:b:c:d\" TimeOut=\"::::\"><Text>x</Text></Subtitle></SubtitleList></SubtitleReel>",
        "<SubtitleReel><SubtitleList><Subtitle TimeIn=\"01:00:00:00\" TimeOut=\"01:00:01:00\"><Text Vposition=\"NaN\" Valign=\"bottom\">x</Text><Text Vposition=\"inf\">y</Text></Subtitle></SubtitleList></SubtitleReel>",
    ] {
        let _ = parse(bad);
    }
    let mut s = REEL.to_string();
    for cut in (0..s.len()).rev().step_by(7) {
        if s.is_char_boundary(cut) {
            s.truncate(cut);
            let _ = parse(&s);
        }
    }
}

fn options() -> ReelOptions {
    ReelOptions {
        id: "urn:uuid:7c5bd5b8-3b0f-4a3b-9b1e-1c2d3e4f5a6b".into(),
        title: "Próba & <film>".into(),
        issue_date: "2026-11-14T10:00:00+00:00".into(),
        reel_number: 1,
        language: "hu-HU".into(),
        edit_rate: (24, 1),
        font: Some(("font1".into(), "urn:uuid:3dec6dc0-39d0-498d-97d0-928d2eb78391".into())),
        size: DEFAULT_SIZE,
    }
}

/// #938: cues written as a SubtitleReel and read back: times, line order, formatting, escaping.
#[test]
fn written_reels_read_back() {
    let cue = |s: Tick, e: Tick, t: &str| Cue { start: s, end: e, text: t.into(), ..Cue::default() };
    let doc = Document {
        cues: vec![
            cue(secs(5, 0), secs(7, 12), "Első sor\nmásodik <b>sor</b>"),
            cue(secs(8, 0), secs(10, 0), "<i>Dőlt</i> & <u>aláhúzott</u> <font color=\"red\">piros</font> a < b"),
            cue(secs(10, 0), secs(10, 0), "egy egység"),
            cue(secs(11, 0), secs(12, 0), "  \n<i></i>"),
            cue(secs(3601, 3), secs(3602, 0), "egy óra után"),
        ],
        ..Document::default()
    };
    let x = write_reel(&doc, &options());
    assert!(x.contains("<LoadFont ID=\"font1\">urn:uuid:3dec6dc0-39d0-498d-97d0-928d2eb78391</LoadFont>"), "{x}");
    assert!(x.contains("<StartTime>00:00:00:00</StartTime>") && x.contains("<TimeCodeRate>24</TimeCodeRate>"));
    assert!(x.contains("TimeIn=\"00:00:05:00\" TimeOut=\"00:00:07:12\""), "{x}");
    assert!(x.contains("TimeIn=\"01:00:01:03\""), "{x}");
    assert!(roxmltree::Document::parse(&x).is_ok(), "{x}");
    let r = parse_reel(&x).unwrap();
    assert_eq!(r.title.as_deref(), Some("Próba & <film>"));
    assert_eq!(r.language.as_deref(), Some("hu-HU"));
    assert_eq!(r.id.as_deref(), Some("urn:uuid:7c5bd5b8-3b0f-4a3b-9b1e-1c2d3e4f5a6b"));
    let c = &r.document.cues;
    assert_eq!(c.len(), 4, "{c:?}");
    assert_eq!(c[0].text, "Első sor\nmásodik <b>sor</b>");
    assert_eq!((c[0].start, c[0].end), (secs(5, 0), secs(7, 12)));
    assert_eq!(c[1].text, "<i>Dőlt</i> & <u>aláhúzott</u> piros a < b", "other tags dropped");
    // a zero-length cue keeps one edit unit
    assert_eq!((c[2].start, c[2].end), (secs(10, 0), secs(10, 1)));
    assert_eq!(c[3].start, secs(3601, 3));
    assert!(r.document.warnings.is_empty(), "{:?}", r.document.warnings);
    // spot numbers count the written subtitles
    assert_eq!(c.iter().map(|c| c.id.clone().unwrap()).collect::<Vec<_>>(), ["1", "2", "3", "4"]);
    // two lines: the first above the second
    assert!(x.contains("Vposition=\"14.5\"") && x.contains("Vposition=\"8.0\""));
    // no font: no LoadFont, no Font ID
    let x = write_reel(&doc, &ReelOptions { font: None, ..options() });
    assert!(!x.contains("LoadFont") && !x.contains(" ID=\""));
    // 25 fps and a non-integer edit rate
    let x = write_reel(&doc, &ReelOptions { edit_rate: (25, 1), ..options() });
    assert!(x.contains("TimeIn=\"00:00:05:00\" TimeOut=\"00:00:07:13\""), "{x}");
    let x = write_reel(&doc, &ReelOptions { edit_rate: (24000, 1001), ..options() });
    let r = parse_reel(&x).unwrap();
    assert!((r.document.cues[0].start.0 - secs(5, 0).0).abs() <= Tick::from_rational(1, 1001, 24000).0);
}
