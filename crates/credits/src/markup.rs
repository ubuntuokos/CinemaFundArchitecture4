//! Inline letter styles inside a cell: `{{Style Name}}` switches the following text to the inline
//! style `Name` until the next `{{Style …}}` tag or the end of the line; `{{Style}}` alone switches
//! back. `{{Stílus Név}}` works too. An inline style changes only what it names (font, weight,
//! size, colour, …); the rest comes from the block's letter style.
//!
//! Built-in inline styles: `Bold` (`Félkövér`), `Italic` (`Dőlt`), `Caps` (`Nagybetűs`),
//! `Underline` (`Aláhúzott`). Your own are `inlineStyles` in the styling.

use cfa4_project::graphic_design::{CharStyle, StyleRun};
use serde::{Deserialize, Serialize};

/// A named inline style: only the fields it sets change.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InlineStyle {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_style: Option<String>,
    /// Pixels of a 1080-high frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// `#rrggbb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// 1/1000 em.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    /// Pixels of a 1080-high frame, up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_shift: Option<f64>,
}

/// The built-in inline styles (with their Hungarian names).
pub fn builtin_inline_styles() -> Vec<(InlineStyle, &'static str)> {
    let fs = |name: &str, style: &str| InlineStyle { name: name.into(), font_style: Some(style.into()), ..Default::default() };
    vec![
        (fs("Bold", "Bold"), "félkövér"),
        (fs("Italic", "Italic"), "dőlt"),
        (InlineStyle { name: "Caps".into(), all_caps: Some(true), ..Default::default() }, "nagybetűs"),
        (InlineStyle { name: "Underline".into(), underline: Some(true), ..Default::default() }, "aláhúzott"),
    ]
}

impl InlineStyle {
    /// The character style for a frame scaled by `s` (frame height / 1080); `None` and a message
    /// for a bad colour.
    pub fn char_style(&self, s: f64) -> (CharStyle, Option<String>) {
        let fin = |v: f64, lo: f64, hi: f64| if v.is_finite() { Some(v.clamp(lo, hi)) } else { None };
        let mut bad = None;
        let fill = match &self.color {
            Some(c) => match crate::style::parse_color(c) {
                Some(rgba) => Some(rgba),
                None => {
                    bad = Some(c.clone());
                    None
                }
            },
            None => None,
        };
        let cs = CharStyle {
            font: self.font.clone().filter(|f| !f.trim().is_empty()),
            font_style: self.font_style.clone().filter(|f| !f.trim().is_empty()),
            size: self.size.and_then(|v| fin(v, 1.0, 2_000.0)).map(|v| (v * s) as f32),
            fill,
            tracking: self.tracking.and_then(|v| fin(v, -1_000.0, 10_000.0)).map(|v| v as f32),
            underline: self.underline,
            caps: self.all_caps.map(u32::from),
            baseline_shift: self.baseline_shift.and_then(|v| fin(v, -10_000.0, 10_000.0)).map(|v| (v * s) as f32),
            ..Default::default()
        };
        (cs, bad)
    }
}

/// If `tag` (the inside of `{{…}}`) is a style tag, the style it names (`""` = back to normal).
pub fn style_tag(tag: &str) -> Option<&str> {
    let t = tag.trim();
    if let Some(rest) = crate::cinecred::multiword_style(t) {
        return Some(rest);
    }
    let (word, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    let w = word.to_lowercase();
    (w == "style" || w == "stílus" || crate::cinecred::is_style_word(&w)).then_some(rest.trim())
}

/// `text` without its style tags.
pub fn plain(text: &str) -> String {
    styled(text, |_| None).text
}

/// A text with its style tags turned into character runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Styled {
    pub text: String,
    /// Character ranges (not bytes) with their style.
    pub runs: Vec<StyleRun>,
    /// Style names that do not exist (their text keeps the block's style).
    pub unknown: Vec<String>,
}

/// Turn style tags into runs; `lookup` gives the style of a name. Other `{{…}}` tags stay as
/// they are.
pub fn styled(text: &str, lookup: impl Fn(&str) -> Option<CharStyle>) -> Styled {
    let mut out = Styled::default();
    let mut current: Option<CharStyle> = None;
    let mut run_start = 0usize;
    let mut n = 0usize; // characters written
    let mut rest = text;
    let close = |out: &mut Styled, current: &Option<CharStyle>, start: usize, end: usize| {
        if let Some(st) = current
            && end > start
            && !st.is_empty()
        {
            out.runs.push(StyleRun { start, end, style: st.clone() });
        }
    };
    loop {
        // the next tag or line break, whichever comes first
        let tag_at = rest.find("{{");
        let nl_at = rest.find('\n');
        match (tag_at, nl_at) {
            (_, Some(nl)) if tag_at.is_none_or(|t| nl < t) => {
                let part = rest.get(..nl).unwrap_or("");
                out.text.push_str(part);
                n += part.chars().count();
                // a style ends with its line
                close(&mut out, &current, run_start, n);
                current = None;
                out.text.push('\n');
                n += 1;
                run_start = n;
                rest = rest.get(nl + 1..).unwrap_or("");
            }
            (Some(t), _) => {
                let part = rest.get(..t).unwrap_or("");
                out.text.push_str(part);
                n += part.chars().count();
                let after = rest.get(t + 2..).unwrap_or("");
                let Some(j) = after.find("}}") else {
                    let tail = rest.get(t..).unwrap_or("");
                    out.text.push_str(tail);
                    n += tail.chars().count();
                    rest = "";
                    continue;
                };
                let inner = after.get(..j).unwrap_or("");
                match style_tag(inner) {
                    Some(name) => {
                        close(&mut out, &current, run_start, n);
                        run_start = n;
                        current = if name.is_empty() {
                            None
                        } else {
                            let found = lookup(name);
                            if found.is_none() && !out.unknown.iter().any(|u| u == name) {
                                out.unknown.push(name.to_string());
                            }
                            found
                        };
                    }
                    None => {
                        let raw = rest.get(t..t + 2 + j + 2).unwrap_or("");
                        out.text.push_str(raw);
                        n += raw.chars().count();
                    }
                }
                rest = after.get(j + 2..).unwrap_or("");
            }
            _ => {
                out.text.push_str(rest);
                n += rest.chars().count();
                close(&mut out, &current, run_start, n);
                break;
            }
        }
        if out.runs.len() > 10_000 {
            // hostile input: keep the text, drop further styling
            out.text.push_str(&plain_rest(rest));
            break;
        }
    }
    out
}

fn plain_rest(rest: &str) -> String {
    let mut s = String::new();
    let mut r = rest;
    while let Some(i) = r.find("{{") {
        s.push_str(r.get(..i).unwrap_or(""));
        let after = r.get(i + 2..).unwrap_or("");
        match after.find("}}") {
            Some(j) if style_tag(after.get(..j).unwrap_or("")).is_some() => r = after.get(j + 2..).unwrap_or(""),
            Some(j) => {
                s.push_str(r.get(i..i + 2 + j + 2).unwrap_or(""));
                r = after.get(j + 2..).unwrap_or("");
            }
            None => {
                s.push_str(r.get(i..).unwrap_or(""));
                r = "";
            }
        }
    }
    s.push_str(r);
    s
}
