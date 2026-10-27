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
    /// A one-line ticker: the blocks stand side by side and run from right to left.
    Crawl,
}

impl Behaviour {
    /// Rolls and crawls move at a constant speed; cards stand still.
    pub fn moves(self) -> bool {
        self != Behaviour::Card
    }
}

/// What is behind a page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundKind {
    /// Nothing: whatever is on the tracks below shows through (black on an empty sequence).
    #[default]
    None,
    Color,
    /// From `color` to `color2` along `angle`.
    Gradient,
    /// A still picture (`file`).
    Image,
    /// A video (`file`), from its start.
    Video,
}

/// How a picture or video background meets the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundFit {
    /// Covers the whole frame; the overhang is cut off.
    #[default]
    Fill,
    /// Fits inside the frame, whole; the rest stays empty.
    Fit,
}

/// A page's background. It becomes its own clip below the page (it does not roll).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Background {
    pub kind: BackgroundKind,
    /// `#rrggbb`: the colour, or where the gradient starts.
    pub color: String,
    /// `#rrggbb`: where the gradient ends.
    pub color2: String,
    /// Gradient direction in degrees (0 = left to right, 90 = top to bottom).
    pub angle: f64,
    /// Picture or video: relative to the picture folder, or absolute.
    pub file: String,
    pub fit: BackgroundFit,
    /// 0–100 %.
    pub opacity: f64,
    /// Blur radius in pixels of a 1080-high frame (0 = sharp).
    pub blur: f64,
}

impl Default for Background {
    fn default() -> Self {
        Background {
            kind: BackgroundKind::None,
            color: "#000000".into(),
            color2: "#202020".into(),
            angle: 90.0,
            file: String::new(),
            fit: BackgroundFit::Fill,
            opacity: 100.0,
            blur: 0.0,
        }
    }
}

impl Background {
    pub fn is_none(&self) -> bool {
        self.kind == BackgroundKind::None
    }
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
    /// Crawl: the line's centre, as a fraction of the frame height from the top (0–1).
    pub crawl_line: f64,
    /// Crawl: space between blocks (pixels), besides their own gaps.
    pub crawl_gap: f64,
    /// What is behind the page.
    #[serde(skip_serializing_if = "Background::is_none")]
    pub background: Background,
    /// Transition into the next page: a video transition id (`cross_dissolve`, `push`, …) or
    /// `dissolve` / `push` / `slide` / `wipe`; empty = a cut. Only where the pages touch
    /// (page gap 0); it replaces the cards' fades at that cut.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub transition: String,
    /// Transition length in frames.
    pub transition_frames: u32,
    /// Card: how the content comes in, over `entranceFrames`, from `entranceDistance` pixels away.
    pub entrance: Entrance,
    pub entrance_frames: u32,
    pub entrance_distance: f64,
    /// Card: each block comes in this many frames after the one before (0 = all together).
    pub stagger_frames: u32,
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
            crawl_line: 0.85,
            crawl_gap: 120.0,
            background: Background::default(),
            transition: String::new(),
            transition_frames: 12,
            entrance: Entrance::None,
            entrance_frames: 18,
            entrance_distance: 120.0,
            stagger_frames: 0,
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

/// Where a portrait stands next to its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PortraitSide {
    #[default]
    Above,
    Left,
}

/// How a card's content comes in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Entrance {
    /// Only the card's fade.
    #[default]
    None,
    /// From the right, moving left.
    SlideLeft,
    /// From the left, moving right.
    SlideRight,
    /// From below, moving up.
    SlideUp,
    /// From above, moving down.
    SlideDown,
}

impl Entrance {
    /// Where the content starts, relative to where it ends (unit vector).
    pub fn from(self) -> (f64, f64) {
        match self {
            Entrance::None => (0.0, 0.0),
            Entrance::SlideLeft => (1.0, 0.0),
            Entrance::SlideRight => (-1.0, 0.0),
            Entrance::SlideUp => (0.0, 1.0),
            Entrance::SlideDown => (0.0, -1.0),
        }
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
    /// Horizontal orientation: how heads stand in their column (`right`: against the body).
    pub head_align: Align,
    /// Horizontal orientation: every head of this style on a page gets the widest one's width,
    /// so with `headAlign: left` the heads form one even column.
    pub harmonize_heads: bool,
    /// Horizontal orientation: a leader (e.g. `.` or ` .`) repeated from the end of the head to
    /// the body; empty = none.
    pub leader: String,
    /// Space between the leader and the text on either side.
    pub leader_gap: f64,
    /// Portraits (`{{Pic face.jpg}} Name`): their height, side and gap to the name.
    pub portrait_height: f64,
    pub portrait_side: PortraitSide,
    pub portrait_gap: f64,
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
            head_align: Align::Right,
            harmonize_heads: false,
            leader: String::new(),
            leader_gap: 16.0,
            portrait_height: 220.0,
            portrait_side: PortraitSide::Above,
            portrait_gap: 16.0,
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
    /// Letter styles for parts of a cell (`{{Style Name}}`), see [`crate::markup`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inline_styles: Vec<crate::markup::InlineStyle>,
}

impl Default for Styling {
    fn default() -> Self {
        Styling {
            unit_px: 32.0,
            default_page_style: "Scroll".into(),
            default_content_style: "Gutter".into(),
            page_styles: vec![],
            content_styles: vec![],
            inline_styles: vec![],
        }
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The built-in page styles: `Scroll`, `Card` and `Crawl` (4 px per frame, near the bottom).
pub fn builtin_page_styles() -> Vec<PageStyle> {
    vec![
        PageStyle::default(),
        PageStyle { name: "Card".into(), behaviour: Behaviour::Card, ..PageStyle::default() },
        PageStyle { name: "Crawl".into(), behaviour: Behaviour::Crawl, scroll_px_per_frame: 4.0, ..PageStyle::default() },
    ]
}

/// The video transition a page style's `transition` names, or `None` for a cut.
pub fn transition_id(name: &str) -> Option<String> {
    let n = norm(name);
    let id = match n.as_str() {
        "" | "none" | "cut" => return None,
        "dissolve" | "crossdissolve" | "cross dissolve" | "áttűnés" => "cross_dissolve",
        "push" | "eltolás" => "push",
        "slide" => "slide",
        "wipe" => "wipe",
        "dip" | "dip to black" => "dip_to_black",
        _ => n.as_str(),
    };
    Some(id.to_string())
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
        // Faces above names, four to a row.
        ContentStyle { name: "Portraits".into(), columns: 4, body: LetterStyle::with(32.0, "Regular"), column_gap: 48.0, paragraph_gap: 40.0, ..base.clone() },
        // Roles in an even left column, dots to the names.
        ContentStyle {
            name: "Leaders".into(),
            orientation: Orientation::Horizontal,
            head: LetterStyle { size: 34.0, color: "#c8c8c8".into(), ..Default::default() },
            head_gap: 120.0,
            tail_gap: 40.0,
            head_align: Align::Left,
            harmonize_heads: true,
            leader: " .".into(),
            ..base.clone()
        },
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

    /// The inline style called `name` (case-insensitive; built-ins also by their Hungarian name):
    /// the styling's own first, then a built-in.
    pub fn inline_style(&self, name: &str) -> Option<crate::markup::InlineStyle> {
        let n = norm(name);
        self.inline_styles
            .iter()
            .find(|c| norm(&c.name) == n)
            .cloned()
            .or_else(|| crate::markup::builtin_inline_styles().into_iter().find(|(c, hu)| norm(&c.name) == n || *hu == n).map(|(c, _)| c))
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
