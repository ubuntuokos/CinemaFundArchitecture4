//! Credits templates (`.cfa4credtpl`): a look for credits – the styling (page and content styles,
//! backgrounds, transitions, entrances, inline styles), an optional fitting length, and a sample
//! sheet that shows the page structure the styles are made for.
//!
//! Versioned JSON (`format: "cfa4.credits-template"`, `version: 1`; newer versions are refused,
//! unknown fields are ignored). A template names itself (`id`, `name`, `description`), where it
//! came from (`source`) and under what licence it may be used (`license`). Templates come in three
//! kinds: **built in** (original CFA4 designs in code, `MIT OR Apache-2.0` by the owner's
//! decision), **user** (files in the data directory) and **project** (kept with a project).

use serde::{Deserialize, Serialize};

use crate::style::{Background, BackgroundKind, Behaviour, Entrance, PageStyle, Styling};
use crate::{ContentStyle, CreditsError};

pub const FORMAT: &str = "cfa4.credits-template";
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "cfa4credtpl";
/// Largest template accepted.
pub const MAX_TEMPLATE_BYTES: usize = 8 << 20;
/// The licence of the built-in templates.
pub const BUILTIN_LICENSE: &str = "MIT OR Apache-2.0";

/// Where a template is kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Builtin,
    #[default]
    User,
    Project,
}

/// A credits template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreditsTemplate {
    pub format: String,
    pub version: u32,
    /// Stable identifier (lower case, `-`): built-in ids start with `cfa4-`.
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub category: Category,
    /// Who made it, or where it came from.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// The licence it may be used under (SPDX expression or free text).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub license: String,
    pub styling: Styling,
    /// A sample sheet (rows, header first) showing the pages and blocks the styles are made for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<Vec<String>>,
    /// Optional total length (ticks) the credits are fitted to when made from the template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<crate::fit::Fit>,
}

impl Default for CreditsTemplate {
    fn default() -> Self {
        CreditsTemplate {
            format: FORMAT.into(),
            version: VERSION,
            id: String::new(),
            name: String::new(),
            description: String::new(),
            category: Category::User,
            source: String::new(),
            license: String::new(),
            styling: Styling::default(),
            sample: Vec::new(),
            fit: None,
        }
    }
}

/// An identifier from a name: lower case letters and digits, `-` between words.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() > 64 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "template".into() } else { out }
}

impl CreditsTemplate {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<CreditsTemplate, CreditsError> {
        if bytes.len() > MAX_TEMPLATE_BYTES {
            return Err(CreditsError::TooLarge(format!("the credits template is larger than {} MiB", MAX_TEMPLATE_BYTES >> 20)));
        }
        let t: CreditsTemplate = serde_json::from_slice(bytes).map_err(|e| CreditsError::Format(format!("damaged credits template: {e}")))?;
        if t.format != FORMAT {
            return Err(CreditsError::Format(format!("not a credits template (format `{}`)", t.format)));
        }
        if t.version > VERSION {
            return Err(CreditsError::Format(format!("the credits template is version {}; this CFA4 reads up to {VERSION}", t.version)));
        }
        if t.name.trim().is_empty() {
            return Err(CreditsError::Format("the credits template has no name".into()));
        }
        let limits = crate::table::Limits::default();
        if t.sample.len() > limits.max_rows || t.sample.iter().any(|r| r.len() > limits.max_cols) {
            return Err(CreditsError::TooLarge("the template's sample exceeds the row or column limit".into()));
        }
        Ok(t)
    }

    /// The sample sheet as CSV text (for a new spreadsheet to start from).
    pub fn sample_csv(&self) -> String {
        let quote = |c: &str| if c.contains([',', '"', '\n', '\r']) { format!("\"{}\"", c.replace('"', "\"\"")) } else { c.to_string() };
        let mut s = String::new();
        for r in &self.sample {
            s.push_str(&r.iter().map(|c| quote(c)).collect::<Vec<_>>().join(","));
            s.push('\n');
        }
        s
    }
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_string()).collect()
}

fn builtin(id: &str, name: &str, description: &str, styling: Styling, sample: &[&[&str]]) -> CreditsTemplate {
    CreditsTemplate {
        id: format!("cfa4-{id}"),
        name: name.into(),
        description: description.into(),
        category: Category::Builtin,
        source: "CFA4 (original design)".into(),
        license: BUILTIN_LICENSE.into(),
        styling,
        sample: sample.iter().map(|r| row(r)).collect(),
        ..Default::default()
    }
}

/// The built-in templates: original CFA4 designs.
pub fn builtin_templates() -> Vec<CreditsTemplate> {
    let page = |name: &str, behaviour: Behaviour| PageStyle { name: name.into(), behaviour, ..PageStyle::default() };
    let dark = Background { kind: BackgroundKind::Gradient, color: "#1b2340".into(), color2: "#05060b".into(), angle: 90.0, ..Background::default() };
    let header = ["Head", "Body", "Tail", "Content Style", "Page Style", "Vertical Gap"];
    vec![
        builtin(
            "classic-roll",
            "Classic Roll",
            "A title card, then one roll: roles left of the centre, names right of it, at 2 px per frame.",
            Styling {
                default_page_style: "Roll".into(),
                page_styles: vec![PageStyle { scroll_px_per_frame: 2.0, ..page("Roll", Behaviour::Scroll) }, page("Title", Behaviour::Card)],
                ..Styling::default()
            },
            &[
                &header,
                &["", "THE TITLE", "", "Title", "Title", ""],
                &["Directed by", "Name Surname", "", "Centered", "Roll", ""],
                &["", "", "", "", "", ""],
                &["Cast", "First Actor", "as the Hero", "Gutter", "", ""],
                &["", "Second Actor", "as the Friend", "", "", ""],
                &["", "", "", "", "", "2"],
                &["Music", "Composer Name", "", "", "", ""],
            ],
        ),
        builtin(
            "cards-and-roll",
            "Cards and Roll",
            "Title and end cards that dissolve, a roll between them, on a dark gradient.",
            Styling {
                default_page_style: "Roll".into(),
                page_styles: vec![
                    PageStyle { transition: "dissolve".into(), transition_frames: 18, background: dark.clone(), ..page("Card", Behaviour::Card) },
                    PageStyle { transition: "dissolve".into(), transition_frames: 18, background: dark.clone(), ..page("Roll", Behaviour::Scroll) },
                    PageStyle { background: dark.clone(), card_frames: 150, ..page("End", Behaviour::Card) },
                ],
                ..Styling::default()
            },
            &[
                &header,
                &["", "THE TITLE", "", "Title", "Card", ""],
                &["Directed by", "Name Surname", "", "Centered", "Roll", ""],
                &["", "", "", "", "", ""],
                &["Cast", "First Actor", "", "Leaders", "", ""],
                &["", "Second Actor", "", "", "", ""],
                &["", "THE END", "", "Heading", "End", ""],
            ],
        ),
        builtin(
            "news-crawl",
            "News Crawl",
            "One line running along the bottom on a dark band, for thanks and sponsors.",
            Styling {
                default_page_style: "Crawl".into(),
                page_styles: vec![PageStyle {
                    scroll_px_per_frame: 4.0,
                    crawl_line: 0.9,
                    background: Background { kind: BackgroundKind::Color, color: "#101418".into(), opacity: 85.0, ..Background::default() },
                    ..page("Crawl", Behaviour::Crawl)
                }],
                ..Styling::default()
            },
            &[&header, &["Thanks to", "Everyone who helped", "", "Gutter", "Crawl", ""], &["Supported by", "The Film Fund", "", "", "", ""]],
        ),
        builtin(
            "cast-portraits",
            "Cast Portraits",
            "Faces above the names, four to a row, sliding up one block after another.",
            Styling {
                default_page_style: "Cast".into(),
                default_content_style: "Portraits".into(),
                page_styles: vec![PageStyle {
                    card_frames: 175,
                    entrance: Entrance::SlideUp,
                    stagger_frames: 8,
                    background: dark,
                    ..page("Cast", Behaviour::Card)
                }],
                content_styles: vec![ContentStyle {
                    name: "Portraits".into(),
                    columns: 4,
                    column_gap: 48.0,
                    paragraph_gap: 40.0,
                    ..crate::style::builtin_content_styles().into_iter().find(|c| c.name == "Portraits").unwrap_or_default()
                }],
                ..Styling::default()
            },
            &[
                &header,
                &["CAST", "{{Pic first.jpg}} First Actor", "", "Portraits", "Cast", ""],
                &["", "{{Pic second.jpg}} Second Actor", "", "", "", ""],
                &["", "{{Pic third.jpg}} Third Actor", "", "", "", ""],
                &["", "{{Pic fourth.jpg}} Fourth Actor", "", "", "", ""],
            ],
        ),
    ]
}
