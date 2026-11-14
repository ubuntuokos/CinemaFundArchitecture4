//! D-Cinema subtitle documents, read for their structure: SMPTE ST 428-7 `SubtitleReel`
//! (namespaces `http://www.smpte-ra.org/schemas/428-7/{2007,2010,2014}/DCST`) and the Interop
//! `DCSubtitle` (no namespace). The reader keeps what a validator compares: the ids, the time
//! base, the fonts loaded, and per subtitle its spot number, times, fades, the fonts it names and
//! its text lines and images. Text content and styling are not kept (`cfa4-captions` turns
//! subtitles into cues).
//!
//! Times are counted in units of [`SubtitleDocument::time_code_rate`] a second: SMPTE
//! `HH:MM:SS:EE` in `TimeCodeRate` units (§5.10; `StartTime` is kept separately), Interop
//! `HH:MM:SS:TTT` in ticks of 4 ms (250 a second); Interop also takes `HH:MM:SS.sss` and fade times
//! as a bare tick count.

use roxmltree::Node;
use serde::{Deserialize, Serialize};

use crate::xml::Reader;
use crate::{Error, IssueKind, Limits, Pos, Rational, Result, Standard, Uuid};

/// The SMPTE ST 428-7 namespaces.
pub const SMPTE_NAMESPACES: &[&str] =
    &["http://www.smpte-ra.org/schemas/428-7/2007/DCST", "http://www.smpte-ra.org/schemas/428-7/2010/DCST", "http://www.smpte-ra.org/schemas/428-7/2014/DCST"];

/// Interop time units a second.
pub const INTEROP_TICKS: u32 = 250;

/// A `LoadFont`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadFont {
    pub id: String,
    /// SMPTE: the font resource (`urn:uuid:…`); Interop: the font file (`URI`), relative to the
    /// subtitle file.
    pub resource: String,
    pub pos: Pos,
}

/// One `Subtitle`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subtitle {
    pub spot_number: Option<u64>,
    pub time_in: Option<u64>,
    pub time_out: Option<u64>,
    pub fade_up: Option<u64>,
    pub fade_down: Option<u64>,
    /// The `Font` ids that apply to it (enclosing or inside it), in document order.
    pub fonts: Vec<String>,
    /// `Text` elements.
    pub texts: usize,
    /// `Image` contents: SMPTE `urn:uuid:…` resources, Interop PNG files.
    pub images: Vec<String>,
    /// `SubtitleList/…/Subtitle[n]`.
    pub path: String,
    pub pos: Pos,
}

/// A subtitle document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubtitleDocument {
    pub standard: Standard,
    /// SMPTE `Id`, Interop `SubtitleID`.
    pub id: Option<Uuid>,
    pub language: Option<String>,
    /// Interop `ReelNumber`, SMPTE `ReelNumber`.
    pub reel_number: Option<u64>,
    /// SMPTE `EditRate`.
    pub edit_rate: Option<Rational>,
    /// Time units a second (SMPTE `TimeCodeRate`, Interop 250).
    pub time_code_rate: u32,
    /// SMPTE `StartTime` in time units (0 when absent and for Interop).
    pub start_time: u64,
    pub fonts: Vec<LoadFont>,
    pub subtitles: Vec<Subtitle>,
    pub issues: Vec<crate::Issue>,
}

/// Whether `text` holds a subtitle document (by the root element), without reading it.
pub fn sniff(text: &str) -> bool {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        // the XML declaration, processing instructions and comments before the root element
        if let Some((open, close)) = [("<!--", "-->"), ("<?", "?>")].into_iter().find(|(o, _)| rest.starts_with(o)) {
            let Some(end) = rest[open.len()..].find(close) else { return false };
            rest = &rest[open.len() + end + close.len()..];
            continue;
        }
        break;
    }
    let Some(r) = rest.strip_prefix('<') else { return false };
    let name: String = r.chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
    matches!(name.rsplit(':').next(), Some("SubtitleReel" | "DCSubtitle"))
}

/// Reads a subtitle document from bytes (UTF-8, an optional byte order mark).
pub fn parse_bytes(bytes: &[u8], limits: Limits) -> Result<SubtitleDocument> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > limits.max_bytes {
        return Err(Error::TooLarge { size, limit: limits.max_bytes });
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| Error::NotUtf8)?;
    parse(text, limits)
}

/// Reads a subtitle document; any other root element is [`Error::UnknownDocument`].
pub fn parse(text: &str, limits: Limits) -> Result<SubtitleDocument> {
    let doc = crate::xml::parse(text, limits)?;
    let root = doc.root_element();
    let (ns, name) = (root.tag_name().namespace(), root.tag_name().name());
    let standard = match (ns, name) {
        (Some(ns), "SubtitleReel") if SMPTE_NAMESPACES.contains(&ns) => Standard::Smpte,
        (None, "DCSubtitle") => Standard::Interop,
        (ns, local) => {
            return Err(Error::UnknownDocument(match ns {
                Some(ns) => format!("{{{ns}}}{local}"),
                None => local.to_string(),
            }));
        }
    };
    let mut r = Reader::new(&doc);
    let smpte = standard == Standard::Smpte;
    let root_path = name;
    let id = if smpte { r.uuid(root, root_path, "Id", true) } else { subtitle_id(&mut r, root) };
    let language = r.text(root, root_path, "Language");
    let reel_number = r.u64(root, root_path, "ReelNumber", !smpte);
    let edit_rate = if smpte { r.rational(root, root_path, "EditRate", true) } else { None };
    let time_code_rate = if smpte {
        match r.u64(root, root_path, "TimeCodeRate", true) {
            Some(t) if (1..=1000).contains(&t) => t as u32,
            Some(t) => {
                r.issue(IssueKind::Invalid, &format!("{root_path}/TimeCodeRate"), root, format!("TimeCodeRate {t} is outside 1-1000"));
                24
            }
            None => 24,
        }
    } else {
        INTEROP_TICKS
    };
    let start_time = if smpte {
        match r.value(root, root_path, "StartTime", false) {
            Some((t, n)) => match time(&t, time_code_rate, false) {
                Some(v) => v,
                None => {
                    r.issue(IssueKind::Invalid, &format!("{root_path}/StartTime"), n, format!("'{t}' is not a time code HH:MM:SS:EE"));
                    0
                }
            },
            None => 0,
        }
    } else {
        0
    };
    let mut d = SubtitleDocument {
        standard,
        id,
        language,
        reel_number,
        edit_rate,
        time_code_rate,
        start_time,
        fonts: Vec::new(),
        subtitles: Vec::new(),
        issues: Vec::new(),
    };
    for n in root.children().filter(|c| c.is_element() && c.tag_name().name() == "LoadFont") {
        let path = format!("{root_path}/LoadFont");
        let id = if smpte { n.attribute("ID") } else { n.attribute("Id").or_else(|| n.attribute("ID")) };
        let resource = if smpte { n.text().map(str::trim).map(str::to_string) } else { n.attribute("URI").map(str::to_string) };
        match (id, resource) {
            (Some(id), Some(res)) if !res.is_empty() => d.fonts.push(LoadFont { id: id.to_string(), resource: res, pos: r.pos(n) }),
            _ => r.issue(IssueKind::Missing, &path, n, if smpte { "LoadFont needs an ID and a resource" } else { "LoadFont needs an Id and a URI" }),
        }
    }
    // SMPTE: the subtitles are under SubtitleList; Interop: under the root (inside Font)
    let list = if smpte { r.child(root, root_path, "SubtitleList") } else { Some(root) };
    match list {
        Some(list) => {
            let base = if smpte { format!("{root_path}/SubtitleList") } else { root_path.to_string() };
            walk(&mut r, list, &base, smpte, time_code_rate, &mut Vec::new(), &mut d.subtitles);
        }
        None => r.issue(IssueKind::Missing, &format!("{root_path}/SubtitleList"), root, "SubtitleList is missing"),
    }
    d.issues = r.issues;
    Ok(d)
}

/// Interop `SubtitleID`: a bare UUID (the Interop schema has no `urn:uuid:` prefix).
fn subtitle_id<'a, 'i>(r: &mut Reader<'a, 'i>, root: Node<'a, 'i>) -> Option<Uuid> {
    let (t, n) = r.value(root, "DCSubtitle", "SubtitleID", true)?;
    let id = Uuid::parse_lenient(&t);
    if id.is_none() {
        r.issue(IssueKind::Invalid, "DCSubtitle/SubtitleID", n, format!("'{t}' is not a UUID"));
    }
    id
}

fn font_id(n: Node<'_, '_>) -> Option<String> {
    n.attribute("ID").or_else(|| n.attribute("Id")).map(str::to_string)
}

/// Collects the `Subtitle` elements under `node`, `fonts` holding the enclosing `Font` ids.
fn walk(r: &mut Reader<'_, '_>, node: Node<'_, '_>, path: &str, smpte: bool, rate: u32, fonts: &mut Vec<String>, out: &mut Vec<Subtitle>) {
    for c in node.children().filter(Node::is_element) {
        match c.tag_name().name() {
            "Font" => {
                let id = font_id(c);
                if let Some(id) = &id {
                    fonts.push(id.clone());
                }
                walk(r, c, &format!("{path}/Font"), smpte, rate, fonts, out);
                if id.is_some() {
                    fonts.pop();
                }
            }
            "Subtitle" => {
                let p = format!("{path}/Subtitle[{}]", out.len() + 1);
                out.push(subtitle(r, c, &p, smpte, rate, fonts));
            }
            _ => {}
        }
    }
}

fn subtitle(r: &mut Reader<'_, '_>, n: Node<'_, '_>, path: &str, smpte: bool, rate: u32, fonts: &[String]) -> Subtitle {
    let mut t = |attr: &str, required: bool, fade: bool| -> Option<u64> {
        let Some(v) = n.attribute(attr) else {
            if required {
                r.issue(IssueKind::Missing, &format!("{path}/@{attr}"), n, format!("{attr} is missing"));
            }
            return None;
        };
        let parsed = if fade && !smpte { v.trim().parse::<u64>().ok().or_else(|| time(v, rate, true)) } else { time(v, rate, !smpte) };
        if parsed.is_none() {
            let form = if smpte { "HH:MM:SS:EE" } else { "HH:MM:SS:TTT" };
            r.issue(IssueKind::Invalid, &format!("{path}/@{attr}"), n, format!("{attr} '{v}' is not a time {form}"));
        }
        parsed
    };
    let time_in = t("TimeIn", true, false);
    let time_out = t("TimeOut", true, false);
    let fade_up = t("FadeUpTime", false, true);
    let fade_down = t("FadeDownTime", false, true);
    let spot_number = match n.attribute("SpotNumber") {
        Some(v) => {
            let s = v.trim().parse::<u64>().ok();
            if s.is_none() {
                r.issue(IssueKind::Invalid, &format!("{path}/@SpotNumber"), n, format!("SpotNumber '{v}' is not a number"));
            }
            s
        }
        None => {
            r.issue(IssueKind::Missing, &format!("{path}/@SpotNumber"), n, "SpotNumber is missing");
            None
        }
    };
    let mut all_fonts = fonts.to_vec();
    let (mut texts, mut images) = (0, Vec::new());
    for d in n.descendants().filter(Node::is_element) {
        match d.tag_name().name() {
            "Font" => all_fonts.extend(font_id(d)),
            "Text" => texts += 1,
            "Image" => images.push(d.text().unwrap_or("").trim().to_string()),
            _ => {}
        }
    }
    Subtitle { spot_number, time_in, time_out, fade_up, fade_down, fonts: all_fonts, texts, images, path: path.to_string(), pos: r.pos(n) }
}

/// `HH:MM:SS:EE` in `rate` units a second (`EE` < `rate`); with `interop`, also `HH:MM:SS.sss`
/// (milliseconds, rounded to ticks of 1/`rate`).
fn time(s: &str, rate: u32, interop: bool) -> Option<u64> {
    let s = s.trim();
    let num = |p: &str| if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) { p.parse::<u64>().ok() } else { None };
    let parts: Vec<&str> = s.split(':').collect();
    let rate = u64::from(rate);
    let (h, m, sec, frac) = match parts[..] {
        [h, m, se, e] => (num(h)?, num(m)?, num(se)?, num(e).filter(|e| *e < rate)?),
        [h, m, se] if interop => {
            let (whole, ms) = se.split_once('.')?;
            let ms = num(ms).filter(|_| ms.len() <= 3)? * 10u64.pow(3 - ms.len() as u32);
            (num(h)?, num(m)?, num(whole)?, (ms * rate + 500) / 1000)
        }
        _ => return None,
    };
    if m >= 60 || sec >= 60 {
        return None;
    }
    ((h * 60 + m) * 60 + sec).checked_mul(rate)?.checked_add(frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMPTE: &str = r#"<?xml version="1.0"?>
<!-- a comment with > inside -->
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST"><Id>urn:uuid:11111111-1111-4111-8111-111111111111</Id><ContentTitleText>T</ContentTitleText><ReelNumber>2</ReelNumber><Language>hu</Language><EditRate>24 1</EditRate><TimeCodeRate>24</TimeCodeRate><StartTime>00:00:01:00</StartTime>
<LoadFont ID="F">urn:uuid:22222222-2222-4222-8222-222222222222</LoadFont>
<SubtitleList><Font ID="F"><Subtitle SpotNumber="1" TimeIn="00:00:02:12" TimeOut="00:00:03:00" FadeUpTime="00:00:00:02"><Font ID="G"><Text>a</Text></Font><Text>b</Text></Subtitle></Font>
<Subtitle SpotNumber="2" TimeIn="00:00:04:00" TimeOut="00:00:05:00"><Image>urn:uuid:33333333-3333-4333-8333-333333333333</Image></Subtitle></SubtitleList></SubtitleReel>"#;

    #[test]
    fn smpte_subtitle_reel() {
        assert!(sniff(SMPTE));
        let d = parse(SMPTE, Limits::default()).unwrap();
        assert_eq!(d.standard, Standard::Smpte);
        assert_eq!(d.id, Uuid::parse("urn:uuid:11111111-1111-4111-8111-111111111111"));
        assert_eq!((d.reel_number, d.language.as_deref(), d.time_code_rate, d.start_time), (Some(2), Some("hu"), 24, 24));
        assert_eq!(d.edit_rate, Some(Rational { num: 24, den: 1 }));
        assert_eq!(d.fonts.len(), 1);
        assert_eq!((d.fonts[0].id.as_str(), d.fonts[0].resource.as_str()), ("F", "urn:uuid:22222222-2222-4222-8222-222222222222"));
        let s = &d.subtitles[0];
        assert_eq!((s.spot_number, s.time_in, s.time_out, s.fade_up, s.fade_down), (Some(1), Some(60), Some(72), Some(2), None));
        assert_eq!((s.fonts.clone(), s.texts), (vec!["F".to_string(), "G".to_string()], 2));
        assert_eq!(d.subtitles[1].images, ["urn:uuid:33333333-3333-4333-8333-333333333333"]);
        assert!(d.subtitles[1].fonts.is_empty());
        assert!(d.issues.is_empty(), "{:?}", d.issues);
    }

    #[test]
    fn interop_dc_subtitle() {
        let x = r#"<DCSubtitle Version="1.0"><SubtitleID>44444444-4444-4444-8444-444444444444</SubtitleID><MovieTitle>T</MovieTitle><ReelNumber>1</ReelNumber><Language>en</Language>
<LoadFont Id="F" URI="font.ttf"/><Font Id="F"><Subtitle SpotNumber="1" TimeIn="00:00:01:125" TimeOut="00:00:02.500" FadeUpTime="20" FadeDownTime="00:00:00:010"><Text>x</Text></Subtitle></Font></DCSubtitle>"#;
        assert!(sniff(x));
        let d = parse_bytes(format!("\u{feff}{x}").as_bytes(), Limits::default()).unwrap();
        assert_eq!(d.standard, Standard::Interop);
        assert_eq!(d.id, Uuid::parse_lenient("44444444-4444-4444-8444-444444444444"));
        assert_eq!((d.time_code_rate, d.start_time, d.edit_rate), (INTEROP_TICKS, 0, None));
        assert_eq!(d.fonts[0].resource, "font.ttf");
        let s = &d.subtitles[0];
        assert_eq!((s.time_in, s.time_out, s.fade_up, s.fade_down), (Some(375), Some(625), Some(20), Some(10)));
        assert!(d.issues.is_empty(), "{:?}", d.issues);
    }

    #[test]
    fn bad_times_and_missing_attributes_are_issues() {
        let x = r#"<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2007/DCST"><Id>urn:uuid:11111111-1111-4111-8111-111111111111</Id><EditRate>24 1</EditRate><TimeCodeRate>24</TimeCodeRate><StartTime>bad</StartTime><SubtitleList>
<Subtitle TimeIn="00:00:01:24" TimeOut="00:61:00:00"><Text>a</Text></Subtitle></SubtitleList></SubtitleReel>"#;
        let d = parse(x, Limits::default()).unwrap();
        let messages: Vec<&str> = d.issues.iter().map(|i| i.message.as_str()).collect();
        for m in [
            "'bad' is not a time code HH:MM:SS:EE",
            "TimeIn '00:00:01:24' is not a time HH:MM:SS:EE",
            "TimeOut '00:61:00:00' is not a time",
            "SpotNumber is missing",
        ] {
            assert!(messages.iter().any(|x| x.contains(m)), "{m}: {messages:?}");
        }
        assert_eq!(d.start_time, 0);
    }

    #[test]
    fn other_documents_are_not_subtitles() {
        assert!(!sniff("<CompositionPlaylist/>"));
        assert!(!sniff("<!-- never closed"));
        assert!(!sniff("plain text"));
        assert!(matches!(parse("<CompositionPlaylist/>", Limits::default()), Err(Error::UnknownDocument(_))));
        assert!(matches!(parse(r#"<SubtitleReel xmlns="urn:other"/>"#, Limits::default()), Err(Error::UnknownDocument(_))));
    }

    #[test]
    fn times() {
        assert_eq!(time("01:00:00:00", 24, false), Some(86_400));
        assert_eq!(time("00:00:00.999", 250, true), Some(250));
        assert_eq!(time("00:00:00.5", 250, true), Some(125));
        assert_eq!(time("00:00:00.5", 250, false), None);
        assert_eq!(time("00:00:00:250", 250, true), None);
        assert_eq!(time("00:00:00:-1", 24, false), None);
    }
}
