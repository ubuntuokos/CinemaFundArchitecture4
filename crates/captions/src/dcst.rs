//! D-Cinema subtitles: the SMPTE ST 428-7 DCDM subtitle XML (`SubtitleReel`, namespaces
//! `http://www.smpte-ra.org/schemas/428-7/{2007,2010,2014}/DCST`), as carried in DCP timed text
//! track files (ST 429-5).
//!
//! Text subtitles become cues: each `Text` element is one line, ordered top to bottom by its
//! `Valign`/`Vposition` (§6.3.3-6.3.4); `Font Italic`/`Weight="bold"`/`Underline` become
//! `<i>`/`<b>`/`<u>` tags; ruby keeps its base text (`Rb`); `Space` is one space; control codes are
//! dropped (§6.3). Times are subtitle time codes `HH:MM:SS:E+` counted in `TimeCodeRate` units
//! and measured from `StartTime` (default `01:00:00:00`, §5.10), at the `EditRate` (§4.2.5).
//! Image subtitles (`Image`, PNG) have no text: they are listed in [`Reel::images`] with their
//! times, fades (§6.1.4-6.1.5, default two units) and position (§6.2.1-6.2.4) for the caller to
//! place the PNG resources they name.

use cfa4_time::Tick;

use crate::{Cue, Document, Error, Result};

/// Image subtitles kept per reel (a feature reel has a few thousand at most).
pub const MAX_IMAGES: usize = 20_000;

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
    /// Subtitles given as PNG images (not converted to cues), in document order.
    pub images: Vec<ImageSubtitle>,
    /// `IntrinsicPictureResolution` (§5.1.1): the picture size the PNGs were made for, e.g. `4K`.
    pub intrinsic_resolution: Option<String>,
}

/// Which edge (or the centre) of an image a position is measured from (`Halign`/`Valign`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// `left` / `top`.
    Start,
    #[default]
    Center,
    /// `right` / `bottom`.
    End,
}

/// An `Image` subtitle (ST 428-7 §6.2): a PNG resource shown over the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageSubtitle {
    pub start: Tick,
    pub end: Tick,
    /// `FadeUpTime` / `FadeDownTime`.
    pub fade_in: Tick,
    pub fade_out: Tick,
    /// UUID of the PNG resource (the element's `urn:uuid:` body).
    pub resource: [u8; 16],
    pub h_align: Align,
    /// `Hposition`, percent of the picture width.
    pub h_position: f64,
    pub v_align: Align,
    /// `Vposition`, percent of the picture height.
    pub v_position: f64,
    /// `SpotNumber` of the subtitle.
    pub spot: Option<String>,
}

impl ImageSubtitle {
    /// Centre of an `image`-sized picture over a `frame`-sized one, in pixels from the top left
    /// (Tables 4 and 5).
    pub fn centre(&self, frame: (f64, f64), image: (f64, f64)) -> (f64, f64) {
        let place = |align: Align, pos: f64, f: f64, i: f64| match align {
            Align::Start => f * pos / 100.0 + i / 2.0,
            Align::Center => f / 2.0 + f * pos / 100.0,
            Align::End => f - f * pos / 100.0 - i / 2.0,
        };
        (place(self.h_align, self.h_position, frame.0, image.0), place(self.v_align, self.v_position, frame.1, image.1))
    }
}

/// The 16 bytes of a `urn:uuid:` (or bare) UUID.
pub fn parse_uuid(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    let s = s.strip_prefix("urn:uuid:").unwrap_or(s);
    let hex: Vec<u8> = s.bytes().filter(|b| *b != b'-').collect();
    if hex.len() != 32 || s.len() != 36 {
        return None;
    }
    let mut out = [0u8; 16];
    for (o, pair) in out.iter_mut().zip(hex.chunks(2)) {
        *o = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

fn align(v: Option<&str>, start: &str, end: &str) -> Align {
    match v.map(str::trim) {
        Some(x) if x == start => Align::Start,
        Some(x) if x == end => Align::End,
        _ => Align::Center,
    }
}

fn percent(v: Option<&str>) -> f64 {
    // a position outside the picture is clamped to a picture width/height away
    v.and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-100.0, 100.0)
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

/// Parse a DCDM subtitle document into cues (image subtitles are reported in the warnings).
pub fn parse(text: &str) -> Result<Document> {
    parse_reel(text).map(|r| {
        let mut doc = r.document;
        if !r.images.is_empty() {
            doc.warnings.push(format!("{} image subtitle(s) (PNG) not imported as text", r.images.len()));
        }
        doc
    })
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
    let mut reel = Reel {
        title: child_text(root, "ContentTitleText"),
        language: child_text(root, "Language"),
        id: child_text(root, "Id"),
        intrinsic_resolution: root.attribute("IntrinsicPictureResolution").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
        ..Reel::default()
    };
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
        // fades: two units unless given (§6.1.4-6.1.5), shared by the subtitle's images
        let fade = |name: &str| match sub.attribute(name) {
            Some(v) => time_code(v, tc_rate).unwrap_or(2),
            None => 2,
        };
        let (fade_in, fade_out) = (fade("FadeUpTime"), fade("FadeDownTime"));
        for img in sub.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Image") {
            let Some(resource) = img.text().and_then(parse_uuid) else {
                doc.warnings.push(format!("line {}: image subtitle without a valid urn:uuid skipped", xml.text_pos_at(img.range().start).row));
                continue;
            };
            if reel.images.len() >= MAX_IMAGES {
                doc.warnings.push(format!("more than {MAX_IMAGES} image subtitles; the rest skipped"));
                break;
            }
            reel.images.push(ImageSubtitle {
                start: at(t_in),
                end: at(t_out),
                fade_in: Tick::from_rational(fade_in, den, num),
                fade_out: Tick::from_rational(fade_out, den, num),
                resource,
                h_align: align(img.attribute("Halign"), "left", "right"),
                h_position: percent(img.attribute("Hposition")),
                v_align: align(img.attribute("Valign"), "top", "bottom"),
                v_position: percent(img.attribute("Vposition")),
                spot: sub.attribute("SpotNumber").map(str::to_string),
            });
        }
        if lines.is_empty() {
            continue;
        }
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        let text = lines.into_iter().map(|l| l.1).collect::<Vec<_>>().join("\n");
        doc.cues.push(Cue { start: at(t_in), end: at(t_out), text, id: sub.attribute("SpotNumber").map(str::to_string), ..Cue::default() });
    }
    reel.images.retain(|i| i.end > i.start);
    doc.cues.retain(|c| c.end > c.start);
    doc.cues.sort_by_key(|c| (c.start, c.end));
    Ok(reel)
}

#[cfg(test)]
mod tests;
