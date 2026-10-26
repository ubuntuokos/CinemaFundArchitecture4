//! Checks on laid-out credits: problems a viewer or a broadcaster would notice, found before the
//! credits are placed or exported.
//!
//! Every [`Issue`] has a stable `code`, a severity, where it is (page, spreadsheet row) and its
//! values, so a user interface can word it in the user's language; `message` is the English text.
//!
//! | Code | Severity | What |
//! |---|---|---|
//! | `duplicate-name` | warning | the same name twice in one block |
//! | `name-spelling` | warning | one person written two ways (differs only in accents, case, spacing or punctuation) |
//! | `missing-font` | error | the font is not installed; another is used |
//! | `missing-glyphs` | warning | the font lacks some characters (e.g. ő, ű); they come from another font |
//! | `outside-title-safe` | warning | text outside the title-safe area (5 % from each edge; for rolls, the sides only) |
//! | `low-contrast` | warning | text colour too dark against black (contrast below 4.5:1, WCAG 2.1) |
//! | `small-text` | warning | text smaller than 24 px on a 1080-line frame |
//! | `uneven-scroll` | warning | the roll does not move a whole number of pixels per frame, which shimmers |
//! | `fast-scroll` | warning | a line crosses the frame in under 6 seconds: hard to read |
//! | `short-card` | warning | a card is on screen for less than 2 seconds after its fades |
//! | `overlap` | info | a page starts before the previous one has ended (negative page gap) |
//! | `upscaled-picture` | warning | a picture is shown larger than its pixels |
//! | `more-issues` | info | more than 50 issues of one code: the rest are counted (`code`, `more`) |

use std::collections::BTreeMap;

use cfa4_project::{EffectInstance, ParamValue};
use cfa4_time::Tick;
use serde::Serialize;

use crate::layout::{Frame, Laid, Measure};
use crate::{Behaviour, Credits};

/// Title-safe margin on each side, as a fraction of the frame (SMPTE RP 2046-1: 90 % title safe).
pub const TITLE_SAFE_MARGIN: f64 = 0.05;
/// Minimum WCAG contrast ratio for text.
pub const MIN_CONTRAST: f64 = 4.5;
/// Smallest comfortable text on a 1080-line frame.
pub const MIN_TEXT_PX: f64 = 24.0;
/// A line should take at least this long to cross the frame.
pub const MIN_CROSSING_SECONDS: f64 = 6.0;
/// A card should hold at least this long between its fades.
pub const MIN_CARD_HOLD_SECONDS: f64 = 2.0;
/// At most this many issues of one code are listed (the count is reported).
const MAX_PER_CODE: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub code: &'static str,
    pub severity: Severity,
    /// 0-based page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    /// 1-based spreadsheet row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    pub values: BTreeMap<&'static str, String>,
    pub message: String,
}

/// What the font database says about a font for a text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontCheck {
    pub missing: bool,
    /// Characters of the text the font does not have.
    pub uncovered: Vec<char>,
}

/// Fonts are the caller's (the engine asks the text engine); by default every font is fine.
pub trait FontInfo {
    fn check(&self, _family: &str, _style: &str, _text: &str) -> FontCheck {
        FontCheck::default()
    }
}

struct Out {
    issues: Vec<Issue>,
    counts: BTreeMap<&'static str, usize>,
}

impl Out {
    fn add(&mut self, code: &'static str, severity: Severity, page: Option<usize>, row: Option<usize>, values: &[(&'static str, String)], message: String) {
        let n = self.counts.entry(code).or_insert(0);
        *n += 1;
        if *n <= MAX_PER_CODE {
            self.issues.push(Issue { code, severity, page, row, values: values.iter().cloned().collect(), message });
        }
    }
}

fn text_of(e: &EffectInstance, k: &str) -> String {
    match e.params.get(k).map(|p| &p.value) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

fn float_of(e: &EffectInstance, k: &str) -> f64 {
    match e.params.get(k).map(|p| &p.value) {
        Some(ParamValue::Float(v)) if v.is_finite() => *v,
        _ => 0.0,
    }
}

/// A name for comparing how people are written: no accents, case, punctuation or extra spaces.
pub fn name_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        let base = match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ő' | 'ø' | 'ō' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'ű' | 'ū' | 'ů' => 'u',
            'ý' | 'ÿ' => 'y',
            'ç' | 'č' | 'ć' => 'c',
            'š' | 'ś' | 'ș' | 'ş' => 's',
            'ž' | 'ź' | 'ż' => 'z',
            'ñ' | 'ń' | 'ň' => 'n',
            'ř' => 'r',
            'ł' => 'l',
            'ď' => 'd',
            'ť' | 'ț' | 'ţ' => 't',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        out.push(base);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// sRGB channel to linear, for WCAG relative luminance.
fn lin(c: f32) -> f64 {
    let c = f64::from(c.clamp(0.0, 1.0));
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// WCAG 2.1 contrast ratio of a colour against black.
pub fn contrast_on_black(rgb: [f32; 4]) -> f64 {
    let l = 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
    (l + 0.05) / 0.05
}

fn secs(t: Tick) -> f64 {
    t.0 as f64 / cfa4_time::TICKS_PER_SECOND as f64
}

/// Check laid-out `credits` for `frame`.
pub fn validate(credits: &Credits, laid: &Laid, frame: Frame, measure: &dyn Measure, fonts: &dyn FontInfo) -> Vec<Issue> {
    let mut out = Out { issues: Vec::new(), counts: BTreeMap::new() };
    let (w, h) = (f64::from(frame.width), f64::from(frame.height));
    let s = h / 1080.0;
    let fps = {
        let fd = frame.rate.frame_duration().0;
        if fd > 0 { cfa4_time::TICKS_PER_SECOND as f64 / fd as f64 } else { 25.0 }
    };

    // ----- names -----
    let mut seen: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for page in &credits.pages {
        for b in &page.blocks {
            let mut in_block: Vec<String> = Vec::new();
            for (ci, cell) in b.body.iter().enumerate() {
                // body cells are consecutive rows from the block's first row
                let cell_row = b.row.saturating_add(ci);
                for line in cell.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    let key = name_key(line);
                    if key.is_empty() {
                        continue;
                    }
                    if in_block.contains(&key) {
                        out.add(
                            "duplicate-name",
                            Severity::Warning,
                            None,
                            Some(cell_row),
                            &[("name", line.to_string())],
                            format!("`{line}` appears twice in the same block"),
                        );
                    } else {
                        in_block.push(key.clone());
                    }
                    match seen.get(&key) {
                        Some((first, row)) if first != line => out.add(
                            "name-spelling",
                            Severity::Warning,
                            None,
                            Some(cell_row),
                            &[("name", line.to_string()), ("other", first.clone()), ("otherRow", row.to_string())],
                            format!("`{line}` is written as `{first}` in row {row}"),
                        ),
                        Some(_) => {}
                        None => {
                            seen.insert(key, (line.to_string(), cell_row));
                        }
                    }
                }
            }
        }
    }

    // ----- pages -----
    let mut checked_fonts: Vec<(String, String)> = Vec::new();
    let mut prev_dur: Option<Tick> = None;
    for (pi, pg) in laid.pages.iter().enumerate() {
        let row = credits.pages.get(pi).map(|p| p.row);
        if let Some(prev) = prev_dur
            && let Some(prev_pg) = pi.checked_sub(1).and_then(|k| laid.pages.get(k))
            && prev_pg.gap_after.0 < 0
        {
            out.add(
                "overlap",
                Severity::Info,
                Some(pi),
                row,
                &[("seconds", format!("{:.1}", (-secs(prev_pg.gap_after)).min(secs(prev))))],
                format!("page {} starts {:.1} s before page {} ends", pi + 1, -secs(prev_pg.gap_after), pi),
            );
        }
        prev_dur = Some(pg.duration);
        match pg.behaviour {
            Behaviour::Scroll => {
                let travel = h + pg.content_height;
                let frames = secs(pg.duration) * fps;
                if frames > 0.0 {
                    let speed = travel / frames;
                    if (speed - speed.round()).abs() > 0.02 {
                        out.add(
                            "uneven-scroll",
                            Severity::Warning,
                            Some(pi),
                            row,
                            &[("speed", format!("{speed:.2}"))],
                            format!("page {} rolls {speed:.2} px per frame; a whole number does not shimmer", pi + 1),
                        );
                    }
                    let crossing = h / speed.max(1e-9) / fps;
                    if crossing < MIN_CROSSING_SECONDS {
                        out.add(
                            "fast-scroll",
                            Severity::Warning,
                            Some(pi),
                            row,
                            &[("seconds", format!("{crossing:.1}"))],
                            format!("on page {} a line crosses the frame in {crossing:.1} s; at least {MIN_CROSSING_SECONDS} s reads well", pi + 1),
                        );
                    }
                }
            }
            Behaviour::Card => {
                let hold = secs(pg.duration) - secs(pg.fade_in) - secs(pg.fade_out);
                if hold < MIN_CARD_HOLD_SECONDS {
                    out.add(
                        "short-card",
                        Severity::Warning,
                        Some(pi),
                        row,
                        &[("seconds", format!("{:.1}", hold.max(0.0)))],
                        format!("card {} holds for {:.1} s after its fades", pi + 1, hold.max(0.0)),
                    );
                }
            }
        }
        let (sx0, sx1) = (w * TITLE_SAFE_MARGIN, w * (1.0 - TITLE_SAFE_MARGIN));
        let (sy0, sy1) = (h * TITLE_SAFE_MARGIN, h * (1.0 - TITLE_SAFE_MARGIN));
        let mut unsafe_reported = false;
        for (li, e) in pg.layers.iter().enumerate().filter(|(_, e)| e.effect == "graphic_text") {
            // the row of the layer's own block, else the page's
            let row = pg.layer_rows.get(li).copied().filter(|r| *r > 0).or(row);
            let text = text_of(e, "text");
            if text.trim().is_empty() {
                continue;
            }
            let first_line = text.lines().next().unwrap_or("").chars().take(40).collect::<String>();
            let size = float_of(e, "size");
            if size > 0.0 && size < MIN_TEXT_PX * s {
                out.add(
                    "small-text",
                    Severity::Warning,
                    Some(pi),
                    row,
                    &[("text", first_line.clone()), ("px", format!("{:.0}", size / s))],
                    format!("`{first_line}` is {:.0} px high (on a 1080-line frame); {MIN_TEXT_PX} px or more reads well", size / s),
                );
            }
            if let Some(ParamValue::Color(c)) = e.params.get("fill_color").map(|p| &p.value) {
                let ratio = contrast_on_black(*c);
                if ratio < MIN_CONTRAST {
                    out.add(
                        "low-contrast",
                        Severity::Warning,
                        Some(pi),
                        row,
                        &[("text", first_line.clone()), ("ratio", format!("{ratio:.1}"))],
                        format!("`{first_line}` has a contrast of {ratio:.1}:1 against black; at least {MIN_CONTRAST}:1 is needed"),
                    );
                }
            }
            let (family, style) = (text_of(e, "font"), text_of(e, "font_style"));
            if !checked_fonts.iter().any(|(f, st)| *f == family && *st == style) {
                let fc = fonts.check(&family, &style, "");
                if fc.missing {
                    out.add(
                        "missing-font",
                        Severity::Error,
                        Some(pi),
                        row,
                        &[("font", format!("{family} {style}"))],
                        format!("the font `{family} {style}` is not installed; another font is used"),
                    );
                }
                checked_fonts.push((family.clone(), style.clone()));
            }
            let fc = fonts.check(&family, &style, &text);
            if !fc.missing && !fc.uncovered.is_empty() {
                let chars: String = fc.uncovered.iter().take(12).collect();
                out.add(
                    "missing-glyphs",
                    Severity::Warning,
                    Some(pi),
                    row,
                    &[("font", format!("{family} {style}")), ("chars", chars.clone())],
                    format!("the font `{family} {style}` has no `{chars}`; another font draws them"),
                );
            }
            if !unsafe_reported && let Some(b) = measure.bounds(e) {
                let (px, py) = match e.params.get("position").map(|p| &p.value) {
                    Some(ParamValue::Vec2(v)) => (v.x, v.y),
                    _ => (0.0, 0.0),
                };
                let (x0, x1, y0, y1) = (px + b[0], px + b[2], py + b[1], py + b[3]);
                let sideways = x0 < sx0 - 0.5 || x1 > sx1 + 0.5;
                let vertical = pg.behaviour == Behaviour::Card && (y0 < sy0 - 0.5 || y1 > sy1 + 0.5);
                if sideways || vertical {
                    unsafe_reported = true;
                    out.add(
                        "outside-title-safe",
                        Severity::Warning,
                        Some(pi),
                        row,
                        &[("text", first_line.clone())],
                        format!("`{first_line}` on page {} is outside the title-safe area", pi + 1),
                    );
                }
            }
        }
        for pic in &pg.pictures {
            if let Some((_pw, ph)) = measure.picture_size(&pic.file)
                && ph > 0.0
                && pic.size.1 > ph * 1.05
            {
                out.add(
                    "upscaled-picture",
                    Severity::Warning,
                    Some(pi),
                    Some(pic.row),
                    &[("file", pic.file.clone()), ("percent", format!("{:.0}", pic.size.1 / ph * 100.0))],
                    format!("`{}` is shown at {:.0} % of its pixels; it will look soft", pic.file, pic.size.1 / ph * 100.0),
                );
            }
        }
    }
    // more of one kind than listed
    for (code, n) in &out.counts {
        if *n > MAX_PER_CODE {
            let more = n - MAX_PER_CODE;
            out.issues.push(Issue {
                code: "more-issues",
                severity: Severity::Info,
                page: None,
                row: None,
                values: [("code", (*code).to_string()), ("more", more.to_string())].into_iter().collect(),
                message: format!("{more} more `{code}` issues"),
            });
        }
    }
    out.issues.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.page.cmp(&b.page)).then(a.row.cmp(&b.row)));
    out.issues
}
