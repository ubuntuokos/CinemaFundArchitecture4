//! CFA4 credits sequences: film credits from a spreadsheet.
//!
//! A clean-room implementation of the *capabilities* of dedicated credits tools (credits kept in
//! a spreadsheet, page and content styles, rolling and card pages), written for CFA4 from public
//! user-level descriptions only. No third-party code was read or copied.
//!
//! - [`table`]: a forgiving CSV / TSV / semicolon reader with hard limits.
//! - [`sheet`]: the spreadsheet's meaning (head / body / tail blocks, gaps, content and page
//!   styles, spine offsets, page runtimes and gaps) → [`Credits`].
//! - [`style`]: [`Styling`] (page, content and letter styles, JSON) with built-in styles.
//! - [`layout`]: [`Credits`] → one graphic clip per page ([`layout::PageOut`]): text layers, Roll
//!   for scroll pages, opacity fades for cards, exact [`filmcraft_time::Tick`] durations.
//!
//! The crate does no file I/O and draws nothing; the engine (`graphics.credits.*` commands)
//! reads files, measures text with the real text engine and places the clips.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod layout;
pub mod sheet;
pub mod style;
pub mod table;

pub use layout::{Frame, Laid, Measure, PageOut, layout};
pub use sheet::{Block, Credits, Gap, Page, TimeSpec, Warning, read_sheet};
pub use style::{Align, Behaviour, BodyLayout, ContentStyle, LetterStyle, Orientation, PageStyle, Styling};

/// Why credits could not be read or laid out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreditsError {
    /// A limit on untrusted input was exceeded.
    TooLarge(String),
    /// No header row with a Body column in the first 50 rows.
    NoHeader,
    /// The sheet has no credits.
    Empty,
    /// The styling JSON is malformed.
    Styling(String),
    /// Layout failed.
    Layout(String),
}

impl std::fmt::Display for CreditsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreditsError::TooLarge(m) => write!(f, "credits spreadsheet too large: {m}"),
            CreditsError::NoHeader => f.write_str("no header row with a `Body` column was found in the first 50 rows"),
            CreditsError::Empty => f.write_str("the spreadsheet holds no credits"),
            CreditsError::Styling(m) => write!(f, "credits styling: {m}"),
            CreditsError::Layout(m) => write!(f, "credits layout: {m}"),
        }
    }
}

impl std::error::Error for CreditsError {}

/// Read a credits spreadsheet (CSV / TSV bytes) with `styling`'s default styles.
pub fn read(bytes: &[u8], styling: &Styling) -> Result<Credits, CreditsError> {
    let rows = table::read_table(bytes, table::Limits::default())?;
    read_sheet(&rows, &styling.default_page_style, &styling.default_content_style)
}

#[cfg(test)]
mod tests;
