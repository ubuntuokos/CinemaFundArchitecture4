//! The credits document (`.cfa4credits`): everything needed to rebuild a credits sequence on any
//! machine, and to refresh it from its spreadsheet.
//!
//! Versioned JSON (`format: "cfa4.credits"`, `version: 1`; newer versions are refused, unknown
//! fields are ignored):
//!
//! | Field | Meaning |
//! |---|---|
//! | `source` | the spreadsheet it came from: `path` (absolute when saved), `name` (file name, tried next to the document when the path is gone), `sheet` (worksheet) |
//! | `pictureDir` | folder of `{{Pic …}}` pictures (default: the spreadsheet's or document's folder) |
//! | `styling` | the [`Styling`], embedded |
//! | `table` | the accepted rows of the sheet, embedded: the credits rebuild without the spreadsheet |
//!
//! The table is the source of truth for the credits as accepted; [`crate::diff`] compares it with
//! the spreadsheet as it is now.

use serde::{Deserialize, Serialize};

use crate::{Credits, CreditsError, Styling, read_sheet};

pub const FORMAT: &str = "cfa4.credits";
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "cfa4credits";
/// Largest document accepted.
pub const MAX_DOC_BYTES: usize = 32 << 20;

/// Where the credits came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SourceRef {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
}

/// A credits document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreditsDoc {
    pub format: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture_dir: Option<String>,
    pub styling: Styling,
    pub table: Vec<Vec<String>>,
}

impl Default for CreditsDoc {
    fn default() -> Self {
        CreditsDoc { format: FORMAT.into(), version: VERSION, source: None, picture_dir: None, styling: Styling::default(), table: Vec::new() }
    }
}

/// Does `bytes` look like a credits document (JSON with our format tag)?
pub fn is_doc(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(4096)).unwrap_or(&[]);
    let text = String::from_utf8_lossy(head);
    text.trim_start().starts_with('{') && text.contains(FORMAT)
}

impl CreditsDoc {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The text of a `.cfa4credits` file: readable JSON, or compact JSON when that is too large;
    /// empty cells at the ends of rows are left out. An error if even that is larger than the
    /// reader accepts, so a saved file always opens again.
    pub fn to_file_text(&self) -> Result<String, CreditsError> {
        let mut d = self.clone();
        for r in &mut d.table {
            while r.last().is_some_and(|c| c.is_empty()) {
                r.pop();
            }
        }
        let pretty = d.to_json();
        let text = if pretty.len() <= MAX_DOC_BYTES { pretty } else { d.to_compact() };
        if text.len() > MAX_DOC_BYTES {
            return Err(CreditsError::TooLarge(format!("the credits document would be larger than {} MiB", MAX_DOC_BYTES >> 20)));
        }
        Ok(text)
    }

    /// Compact JSON, as stored with the clips in a project.
    pub fn to_compact(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<CreditsDoc, CreditsError> {
        if bytes.len() > MAX_DOC_BYTES {
            return Err(CreditsError::TooLarge(format!("the credits document is larger than {} MiB", MAX_DOC_BYTES >> 20)));
        }
        let d: CreditsDoc = serde_json::from_slice(bytes).map_err(|e| CreditsError::Format(format!("damaged credits document: {e}")))?;
        if d.format != FORMAT {
            return Err(CreditsError::Format(format!("not a credits document (format `{}`)", d.format)));
        }
        if d.version > VERSION {
            return Err(CreditsError::Format(format!("the credits document is version {}; this CFA4 reads up to {VERSION}", d.version)));
        }
        let limits = crate::table::Limits::default();
        if d.table.len() > limits.max_rows || d.table.iter().any(|r| r.len() > limits.max_cols) {
            return Err(CreditsError::TooLarge("the credits document's table exceeds the row or column limit".into()));
        }
        Ok(d)
    }

    /// The credits the embedded table stands for.
    pub fn credits(&self) -> Result<Credits, CreditsError> {
        read_sheet(&self.table, &self.styling.default_page_style, &self.styling.default_content_style)
    }
}

/// 64-bit FNV-1a: stable across builds and platforms (the std hasher is not guaranteed to be).
pub fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        for b in p.iter() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        // separator, so ("ab","c") and ("a","bc") differ
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Fingerprint of a layer's parameters and per-character styles: changes when the layer is
/// edited by hand.
pub fn layer_fingerprint(e: &cfa4_project::EffectInstance) -> u64 {
    let params = serde_json::to_string(&e.params).unwrap_or_default();
    let runs = e.layer.as_ref().map(|l| serde_json::to_string(&l.runs).unwrap_or_default()).unwrap_or_default();
    fnv1a(&[e.effect.as_bytes(), params.as_bytes(), runs.as_bytes(), &[u8::from(e.enabled)]])
}
