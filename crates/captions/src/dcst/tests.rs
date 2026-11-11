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
    assert_eq!(r.image_subtitles, 1);
    assert_eq!(r.document.warnings.len(), 1, "{:?}", r.document.warnings);
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
