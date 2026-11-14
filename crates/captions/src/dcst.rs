//! D-Cinema subtitles: the SMPTE ST 428-7 DCDM subtitle XML (`SubtitleReel`, namespaces
//! `http://www.smpte-ra.org/schemas/428-7/{2007,2010,2014}/DCST`), as carried in DCP timed text
//! track files (ST 429-5).
//!
//! Text subtitles become cues: each `Text` element is one line, ordered top to bottom by its
//! `Valign`/`Vposition` (§6.3.3-6.3.4); `Font Italic`/`Weight="bold"`/`Underline` become
//! `<i>`/`<b>`/`<u>` tags; ruby keeps its base text (`Rb`); `Space` is one space; control codes are
//! dropped (§6.3). Times are subtitle time codes `HH:MM:SS:E+` counted in `TimeCodeRate` units
//! and measured from `StartTime` (default `01:00:00:00`, §5.10), at the `EditRate` (§4.2.5).
//! Image subtitles (`Image`, PNG) have no text: they are counted and reported.

use cfa4_time::Tick;

use crate::{Cue, Document, Error, Result};

/// A parsed subtitle reel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reel {
    pub document: Document,
    /// `ContentTitleText`.
    pub title: Option<String>,
    /// `Language` (an xs:language code).
    pub language: Option<String>,
    /// `Id` (`urn:uuid:…`).
    pub id: Option<String>,
    /// Subtitles given as PNG images (not converted to cues).
    pub image_subtitles: usize,
}

/// Whether `text` is a DCDM subtitle document (root element `SubtitleReel`).
pub fn sniff(text: &str) -> bool {
    // the root element follows the XML declaration and comments
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("<?").or_else(|| rest.strip_prefix("<!--")) {
            let Some(end) = r.find('>') else { return false };
            rest = &r[end + 1..];
            continue;
        }
        break;
    }
    let Some(r) = rest.strip_prefix('<') else { return false };
    let name: String = r.chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
    name.rsplit(':').next() == Some("SubtitleReel")
}

/// Parse a DCDM subtitle document into cues.
pub fn parse(text: &str) -> Result<Document> {
    parse_reel(text).map(|r| r.document)
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn child_text(n: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `EditRate`: two positive integers.
fn edit_rate(s: &str) -> Option<(i64, i64)> {
    let mut it = s.split_whitespace().map(|p| p.parse::<i64>().ok().filter(|v| (1..=1_000_000).contains(v)));
    let r = (it.next()??, it.next()??);
    it.next().is_none().then_some(r)
}

/// A subtitle time code as a count of editable units.
fn time_code(s: &str, rate: i64) -> Option<i64> {
    let p: Vec<&str> = s.trim().split(':').collect();
    let [h, m, sec, e] = p.as_slice() else { return None };
    let num = |x: &str, max: i64| x.parse::<i64>().ok().filter(|v| (0..=max).contains(v) && !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()));
    let (h, m, sec, e) = (num(h, 99)?, num(m, 59)?, num(sec, 59)?, num(e, 999_999)?);
    // tolerate E+ ≥ TimeCodeRate (some authoring tools): it still counts units
    (((h * 60 + m) * 60 + sec) * rate).checked_add(e)
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Fmt {
    italic: bool,
    bold: bool,
    underline: bool,
}

fn font_fmt(n: roxmltree::Node<'_, '_>, parent: Fmt) -> Fmt {
    let mut f = parent;
    if let Some(v) = n.attribute("Italic") {
        f.italic = v.trim() != "no";
    }
    if let Some(v) = n.attribute("Weight") {
        f.bold = v.trim() == "bold";
    }
    if let Some(v) = n.attribute("Underline") {
        f.underline = v.trim() == "yes";
    }
    f
}

/// Text of a `Text` element and its children, with format tags.
fn collect(n: roxmltree::Node<'_, '_>, fmt: Fmt, out: &mut String) {
    for c in n.children() {
        if c.is_text() {
            out.extend(c.text().unwrap_or("").chars().filter(|ch| !ch.is_control()));
        } else if c.is_element() {
            match c.tag_name().name() {
                "Font" => {
                    let f = font_fmt(c, fmt);
                    let tags: Vec<&str> = [(f.italic && !fmt.italic, "i"), (f.bold && !fmt.bold, "b"), (f.underline && !fmt.underline, "u")]
                        .into_iter()
                        .filter(|x| x.0)
                        .map(|x| x.1)
                        .collect();
                    for t in &tags {
                        out.push_str(&format!("<{t}>"));
                    }
                    collect(c, f, out);
                    for t in tags.iter().rev() {
                        out.push_str(&format!("</{t}>"));
                    }
                }
                "Ruby" => {
                    if let Some(rb) = child(c, "Rb") {
                        collect(rb, fmt, out);
                    }
                }
                "Space" => out.push(' '),
                "Rt" | "LoadVariableZ" | "Image" => {}
                _ => collect(c, fmt, out),
            }
        }
    }
}

/// Screen height position of a line (0 top, 100 bottom) from `Valign`/`Vposition`.
fn line_y(n: roxmltree::Node<'_, '_>) -> f64 {
    let v = n.attribute("Vposition").and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
    match n.attribute("Valign").map(str::trim) {
        Some("top") => v,
        Some("bottom") => 100.0 - v,
        _ => 50.0 + v,
    }
}

/// Parse a DCDM subtitle document with its reel metadata.
pub fn parse_reel(text: &str) -> Result<Reel> {
    crate::ttml::check_xml_depth(text)?;
    let xml = roxmltree::Document::parse(text).map_err(|_| Error::NotFormat("D-Cinema subtitle"))?;
    let root = xml.root_element();
    if root.tag_name().name() != "SubtitleReel" {
        return Err(Error::NotFormat("D-Cinema subtitle"));
    }
    let mut reel = Reel { title: child_text(root, "ContentTitleText"), language: child_text(root, "Language"), id: child_text(root, "Id"), ..Reel::default() };
    let doc = &mut reel.document;
    let (num, den) = match child_text(root, "EditRate") {
        Some(s) => edit_rate(&s).ok_or_else(|| Error::Syntax { line: 1, msg: format!("bad EditRate \"{s}\"") })?,
        None => {
            doc.warnings.push("no EditRate; assuming 24 1".into());
            (24, 1)
        }
    };
    // TimeCodeRate: EditRate rounded to the nearest integer, halves up (§5.9)
    let rounded = (2 * num + den) / (2 * den);
    let tc_rate = child_text(root, "TimeCodeRate").and_then(|s| s.parse::<i64>().ok()).filter(|v| (1..=1_000_000).contains(v)).unwrap_or(rounded.max(1));
    let start = match child_text(root, "StartTime") {
        Some(s) => time_code(&s, tc_rate).ok_or_else(|| Error::Syntax { line: 1, msg: format!("bad StartTime \"{s}\"") })?,
        None => 3600 * tc_rate,
    };
    let at = |units: i64| Tick::from_rational(units.saturating_sub(start), den, num);
    let Some(list) = child(root, "SubtitleList") else { return Ok(reel) };
    for sub in list.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Subtitle") {
        let pos = xml.text_pos_at(sub.range().start);
        let (Some(t_in), Some(t_out)) =
            (sub.attribute("TimeIn").and_then(|s| time_code(s, tc_rate)), sub.attribute("TimeOut").and_then(|s| time_code(s, tc_rate)))
        else {
            doc.warnings.push(format!("line {}: subtitle without a valid TimeIn/TimeOut skipped", pos.row));
            continue;
        };
        if t_in < start {
            doc.warnings.push(format!("line {}: subtitle before StartTime skipped", pos.row));
            continue;
        }
        let mut lines: Vec<(f64, String)> = Vec::new();
        for t in sub.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Text") {
            // formatting from every enclosing Font element (they may wrap whole subtitle lists)
            let mut fonts: Vec<roxmltree::Node<'_, '_>> = t.ancestors().skip(1).filter(|a| a.is_element() && a.tag_name().name() == "Font").collect();
            fonts.reverse();
            let fmt = fonts.into_iter().fold(Fmt::default(), |f, n| font_fmt(n, f));
            let mut s = String::new();
            for (on, tag) in [(fmt.italic, "i"), (fmt.bold, "b"), (fmt.underline, "u")] {
                if on {
                    s.push_str(&format!("<{tag}>"));
                }
            }
            collect(t, fmt, &mut s);
            for (on, tag) in [(fmt.underline, "u"), (fmt.bold, "b"), (fmt.italic, "i")] {
                if on {
                    s.push_str(&format!("</{tag}>"));
                }
            }
            let s = s.trim().to_string();
            if !s.is_empty() {
                lines.push((line_y(t), s));
            }
        }
        if lines.is_empty() {
            if sub.descendants().any(|n| n.is_element() && n.tag_name().name() == "Image") {
                reel.image_subtitles += 1;
            }
            continue;
        }
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        let text = lines.into_iter().map(|l| l.1).collect::<Vec<_>>().join("\n");
        doc.cues.push(Cue { start: at(t_in), end: at(t_out), text, id: sub.attribute("SpotNumber").map(str::to_string), ..Cue::default() });
    }
    if reel.image_subtitles > 0 {
        doc.warnings.push(format!("{} image subtitle(s) (PNG) not imported as text", reel.image_subtitles));
    }
    doc.cues.retain(|c| c.end > c.start);
    doc.cues.sort_by_key(|c| (c.start, c.end));
    Ok(reel)
}

/// The ST 428-7:2010 namespace (also of the 2014 edition).
pub const NAMESPACE_2010: &str = "http://www.smpte-ra.org/schemas/428-7/2010/DCST";

/// Font size of [`write_reel`] (points, ST 428-7 6.2.2: as DCP authoring tools default).
pub const DEFAULT_SIZE: u32 = 42;
/// Distance of the bottom line from the bottom edge and between lines (percent of the screen
/// height) for the default size.
const BOTTOM: f64 = 8.0;
const LINE_STEP: f64 = 6.5;

/// What [`write_reel`] writes besides the cues.
#[derive(Clone, Debug, PartialEq)]
pub struct ReelOptions {
    /// The reel's `urn:uuid:`.
    pub id: String,
    pub title: String,
    /// `IssueDate` (xs:dateTime).
    pub issue_date: String,
    pub reel_number: u32,
    /// `Language` (an RFC 5646 / xs:language tag).
    pub language: String,
    /// `EditRate` (numerator, denominator; positive).
    pub edit_rate: (i64, i64),
    /// `LoadFont`: the font's `ID` in the document and its resource `urn:uuid:`.
    pub font: Option<(String, String)>,
    /// Font size in points.
    pub size: u32,
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o
}

/// A cue line as runs of text with their `<i>`/`<b>`/`<u>` formatting; other tags are dropped.
fn runs(line: &str) -> Vec<(Fmt, String)> {
    let mut out: Vec<(Fmt, String)> = Vec::new();
    let mut f = Fmt::default();
    let mut rest = line;
    while !rest.is_empty() {
        let (text, after) = match rest.find('<') {
            Some(i) => (rest.get(..i).unwrap_or(""), rest.get(i..).unwrap_or("")),
            None => (rest, ""),
        };
        if !text.is_empty() {
            match out.last_mut() {
                Some((lf, t)) if *lf == f => t.push_str(text),
                _ => out.push((f, text.to_string())),
            }
        }
        rest = after;
        if rest.is_empty() {
            break;
        }
        let Some(end) = rest.find('>') else {
            // a lone '<' is text
            match out.last_mut() {
                Some((lf, t)) if *lf == f => t.push_str(rest),
                _ => out.push((f, rest.to_string())),
            }
            break;
        };
        let tag = rest.get(1..end).unwrap_or("").trim().to_ascii_lowercase();
        let (close, name) = match tag.strip_prefix('/') {
            Some(n) => (true, n.trim().to_string()),
            None => (false, tag.split(|c: char| c.is_whitespace() || c == '.').next().unwrap_or("").to_string()),
        };
        match name.as_str() {
            "i" => f.italic = !close,
            "b" => f.bold = !close,
            "u" => f.underline = !close,
            _ => {}
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out
}

/// Write cues as an ST 428-7 `SubtitleReel` (2010 namespace): times from `StartTime`
/// `00:00:00:00` at the edit rate, each cue line a `Text` centred at the bottom of the screen
/// (the last line [`BOTTOM`] percent up), `<i>`/`<b>`/`<u>` as `Font` attributes, cues snapped to
/// edit units and at least one unit long. Cues that start before zero are cut at zero; empty
/// cues are left out.
pub fn write_reel(doc: &Document, o: &ReelOptions) -> String {
    let (num, den) = (o.edit_rate.0.max(1), o.edit_rate.1.max(1));
    let tc_rate = ((2 * num + den) / (2 * den)).max(1);
    let units = |t: Tick| t.to_rational_round(den, num).max(0);
    let tc = |u: i64| {
        let (s, e) = (u / tc_rate, u % tc_rate);
        format!("{:02}:{:02}:{:02}:{:02}", (s / 3600).min(99), s / 60 % 60, s % 60, e)
    };
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str(&format!("<SubtitleReel xmlns=\"{NAMESPACE_2010}\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">\n"));
    x.push_str(&format!("  <Id>{}</Id>\n", escape(&o.id)));
    x.push_str(&format!("  <ContentTitleText>{}</ContentTitleText>\n", escape(&o.title)));
    x.push_str(&format!("  <IssueDate>{}</IssueDate>\n", escape(&o.issue_date)));
    x.push_str(&format!("  <ReelNumber>{}</ReelNumber>\n", o.reel_number.max(1)));
    if !o.language.is_empty() {
        x.push_str(&format!("  <Language>{}</Language>\n", escape(&o.language)));
    }
    x.push_str(&format!("  <EditRate>{num} {den}</EditRate>\n  <TimeCodeRate>{tc_rate}</TimeCodeRate>\n  <StartTime>00:00:00:00</StartTime>\n"));
    let font_id = match &o.font {
        Some((id, urn)) => {
            x.push_str(&format!("  <LoadFont ID=\"{}\">{}</LoadFont>\n", escape(id), escape(urn)));
            format!(" ID=\"{}\"", escape(id))
        }
        None => String::new(),
    };
    let size = o.size.clamp(1, 1000);
    x.push_str("  <SubtitleList>\n");
    x.push_str(&format!(
        "    <Font{font_id} Size=\"{size}\" Color=\"FFFFFFFF\" Effect=\"border\" EffectColor=\"FF000000\" Weight=\"normal\" Italic=\"no\" Underline=\"no\" Script=\"normal\" AspectAdjust=\"1.0\">\n"
    ));
    // the line step grows with the font size
    let step = LINE_STEP * f64::from(size) / f64::from(DEFAULT_SIZE);
    let mut spot = 0u32;
    for c in &doc.cues {
        let lines: Vec<Vec<(Fmt, String)>> = c.text.lines().map(runs).filter(|r| r.iter().any(|(_, t)| !t.trim().is_empty())).collect();
        if lines.is_empty() {
            continue;
        }
        let t_in = units(c.start);
        let t_out = units(c.end).max(t_in + 1);
        spot += 1;
        x.push_str(&format!(
            "      <Subtitle SpotNumber=\"{spot}\" TimeIn=\"{}\" TimeOut=\"{}\" FadeUpTime=\"00:00:00:00\" FadeDownTime=\"00:00:00:00\">\n",
            tc(t_in),
            tc(t_out)
        ));
        let n = lines.len();
        for (i, line) in lines.iter().enumerate() {
            let v = BOTTOM + step * (n - 1 - i) as f64;
            x.push_str(&format!("        <Text Valign=\"bottom\" Vposition=\"{v:.1}\" Halign=\"center\" Hposition=\"0\" Direction=\"ltr\">"));
            for (f, t) in line {
                let attrs: String = [(f.italic, " Italic=\"yes\""), (f.bold, " Weight=\"bold\""), (f.underline, " Underline=\"yes\"")]
                    .into_iter()
                    .filter(|a| a.0)
                    .map(|a| a.1)
                    .collect();
                if attrs.is_empty() {
                    x.push_str(&escape(t));
                } else {
                    x.push_str(&format!("<Font{attrs}>{}</Font>", escape(t)));
                }
            }
            x.push_str("</Text>\n");
        }
        x.push_str("      </Subtitle>\n");
    }
    x.push_str("    </Font>\n  </SubtitleList>\n</SubtitleReel>\n");
    x
}

#[cfg(test)]
mod tests;
