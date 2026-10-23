//! Page styles, content styles and letter styles of a credits sequence.
//!
//! Sizes are in pixels of a 1080-pixel-high frame and scale with the sequence height. A
//! [`Styling`] is plain serde JSON (`camelCase`), so the UI, the CLI and agents can pass one as a
//! command parameter or keep it next to the spreadsheet. Missing fields take their defaults and
//! the built-in styles are always available unless a style of the same name replaces them.

use serde::{Deserialize, Serialize};

/// How a page is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Behaviour {
    /// A still card: fades in, holds, fades out.
    Card,
    /// Rolls up from below the frame to above it at a constant speed.
    #[default]
    Scroll,
}

/// A page style (`Page Style` column).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PageStyle {
    pub name: String,
    pub behaviour: Behaviour,
    /// Card: frames on screen, fades included.
    pub card_frames: u32,
    /// Card: fade-in and fade-out length in frames (0 = cut).
    pub fade_in_frames: u32,
    pub fade_out_frames: u32,
    /// Scroll: pixels per frame (1080-high frame). Whole numbers avoid shimmer.
    pub scroll_px_per_frame: f64,
    /// Frames of empty screen after the page (negative: the next page starts earlier).
    pub gap_after_frames: i32,
}

impl Default for PageStyle {
    fn default() -> Self {
        PageStyle {
            name: "Scroll".into(),
            behaviour: Behaviour::Scroll,
            card_frames: 120,
            fade_in_frames: 12,
            fade_out_frames: 12,
            scroll_px_per_frame: 3.0,
            gap_after_frames: 0,
        }
    }
}

/// Head and tail beside the body, or above and below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    /// Head left of the spine, body right of it, tail right of the body.
    Horizontal,
    /// Head above the body, tail below it.
    #[default]
    Vertical,
}

/// How several body cells are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BodyLayout {
    /// Cells in `columns` columns, row by row.
    #[default]
    Grid,
    /// Cells run on in one wrapped paragraph, joined by `flowSeparator`.
    Flow,
    /// Each cell is a wrapped paragraph.
    Paragraphs,
}

/// Horizontal alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

impl Align {
    /// The text layer's `align` choice index (Left, Center, Right, Justify).
    pub fn choice(self) -> u32 {
        match self {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
        }
    }
}

/// The look of a piece of text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LetterStyle {
    pub font: String,
    pub font_style: String,
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
    /// 1/1000 em.
    pub tracking: f64,
    pub all_caps: bool,
    /// Pixels added to the natural line height.
    pub leading: f64,
}

impl Default for LetterStyle {
    fn default() -> Self {
        LetterStyle { font: "Inter".into(), font_style: "Regular".into(), size: 40.0, color: "#ffffff".into(), tracking: 0.0, all_caps: false, leading: 6.0 }
    }
}

impl LetterStyle {
    fn with(size: f64, style: &str) -> Self {
        LetterStyle { size, font_style: style.into(), ..Default::default() }
    }
}

/// A content style (`Content Style` column).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentStyle {
    pub name: String,
    pub orientation: Orientation,
    pub layout: BodyLayout,
    /// Grid columns (1–16).
    pub columns: u32,
    /// Alignment of the block on the spine (vertical orientation) and of its text.
    pub align: Align,
    pub body: LetterStyle,
    pub head: LetterStyle,
    pub tail: LetterStyle,
    /// Gap between head and body (and body and tail).
    pub head_gap: f64,
    pub tail_gap: f64,
    /// Gap between grid columns.
    pub column_gap: f64,
    /// Gap between paragraphs.
    pub paragraph_gap: f64,
    /// Wrap width of flow and paragraphs layouts.
    pub wrap_width: f64,
    pub flow_separator: String,
    /// Minimum space above and below the block.
    pub margin: f64,
}

impl Default for ContentStyle {
    fn default() -> Self {
        ContentStyle {
            name: "Centered".into(),
            orientation: Orientation::Vertical,
            layout: BodyLayout::Grid,
            columns: 1,
            align: Align::Center,
            body: LetterStyle::default(),
            head: LetterStyle { size: 30.0, color: "#c8c8c8".into(), tracking: 80.0, all_caps: true, ..Default::default() },
            tail: LetterStyle { size: 30.0, color: "#c8c8c8".into(), ..Default::default() },
            head_gap: 12.0,
            tail_gap: 12.0,
            column_gap: 64.0,
            paragraph_gap: 24.0,
            wrap_width: 1200.0,
            flow_separator: "  •  ".into(),
            margin: 0.0,
        }
    }
}

/// Everything that styles a credits sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Styling {
    /// Pixels of one gap unit (an empty row).
    pub unit_px: f64,
    /// Used when the spreadsheet names no page style.
    pub default_page_style: String,
    /// Used when the spreadsheet names no content style.
    pub default_content_style: String,
    pub page_styles: Vec<PageStyle>,
    pub content_styles: Vec<ContentStyle>,
}

impl Default for Styling {
    fn default() -> Self {
        Styling { unit_px: 32.0, default_page_style: "Scroll".into(), default_content_style: "Gutter".into(), page_styles: vec![], content_styles: vec![] }
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The built-in page styles: `Scroll` and `Card`.
pub fn builtin_page_styles() -> Vec<PageStyle> {
    vec![PageStyle::default(), PageStyle { name: "Card".into(), behaviour: Behaviour::Card, ..PageStyle::default() }]
}

/// The built-in content styles (original CFA4 designs).
pub fn builtin_content_styles() -> Vec<ContentStyle> {
    let base = ContentStyle::default();
    vec![
        // Role left of the spine, names right of it.
        ContentStyle {
            name: "Gutter".into(),
            orientation: Orientation::Horizontal,
            head: LetterStyle { size: 34.0, color: "#c8c8c8".into(), ..Default::default() },
            head_gap: 40.0,
            tail_gap: 40.0,
            ..base.clone()
        },
        ContentStyle { name: "Centered".into(), ..base.clone() },
        ContentStyle {
            name: "Heading".into(),
            body: LetterStyle { tracking: 120.0, all_caps: true, ..LetterStyle::with(64.0, "Bold") },
            margin: 24.0,
            ..base.clone()
        },
        ContentStyle { name: "Title".into(), body: LetterStyle::with(96.0, "SemiBold"), head: LetterStyle::with(36.0, "Regular"), ..base.clone() },
        ContentStyle { name: "Grid".into(), columns: 3, body: LetterStyle::with(36.0, "Regular"), ..base.clone() },
        ContentStyle { name: "Bullets".into(), layout: BodyLayout::Flow, body: LetterStyle::with(36.0, "Regular"), ..base.clone() },
        ContentStyle {
            name: "Paragraphs".into(),
            layout: BodyLayout::Paragraphs,
            body: LetterStyle { leading: 10.0, ..LetterStyle::with(34.0, "Regular") },
            wrap_width: 1100.0,
            ..base
        },
    ]
}

impl Styling {
    /// Parse a styling from JSON.
    pub fn from_json(s: &str) -> Result<Styling, crate::CreditsError> {
        if s.len() > 4 << 20 {
            return Err(crate::CreditsError::TooLarge("the styling is larger than 4 MiB".into()));
        }
        serde_json::from_str(s).map_err(|e| crate::CreditsError::Styling(e.to_string()))
    }

    /// The page style called `name` (case-insensitive): the styling's own first, then a built-in.
    pub fn page_style(&self, name: &str) -> Option<PageStyle> {
        let n = norm(name);
        self.page_styles.iter().find(|p| norm(&p.name) == n).cloned().or_else(|| builtin_page_styles().into_iter().find(|p| norm(&p.name) == n))
    }

    /// The content style called `name` (case-insensitive): the styling's own first, then a built-in.
    pub fn content_style(&self, name: &str) -> Option<ContentStyle> {
        let n = norm(name);
        self.content_styles.iter().find(|c| norm(&c.name) == n).cloned().or_else(|| builtin_content_styles().into_iter().find(|c| norm(&c.name) == n))
    }

    /// Every style name a spreadsheet can use.
    pub fn names(&self) -> (Vec<String>, Vec<String>) {
        let mut pages: Vec<String> = self.page_styles.iter().map(|p| p.name.clone()).collect();
        for p in builtin_page_styles() {
            if !pages.iter().any(|n| norm(n) == norm(&p.name)) {
                pages.push(p.name);
            }
        }
        let mut contents: Vec<String> = self.content_styles.iter().map(|c| c.name.clone()).collect();
        for c in builtin_content_styles() {
            if !contents.iter().any(|n| norm(n) == norm(&c.name)) {
                contents.push(c.name);
            }
        }
        (pages, contents)
    }
}

/// Parse `#rrggbb` (or `rrggbb`, `#rgb`) into linear-agnostic sRGB floats; `None` if malformed.
pub fn parse_color(s: &str) -> Option<[f32; 4]> {
    let h = s.trim().trim_start_matches('#');
    let digits: Vec<u32> = h.chars().map(|c| c.to_digit(16)).collect::<Option<Vec<_>>>()?;
    let (r, g, b) = match digits.as_slice() {
        [r, g, b] => (r * 17, g * 17, b * 17),
        [r1, r2, g1, g2, b1, b2] => (r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2),
        _ => return None,
    };
    Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
}
