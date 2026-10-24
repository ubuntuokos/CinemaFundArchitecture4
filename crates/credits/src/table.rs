//! A forgiving reader for delimited text tables (CSV, TSV, semicolon-separated), RFC 4180 style.
//!
//! - UTF-8 with or without a byte-order mark; invalid bytes become U+FFFD.
//! - The delimiter is detected from the first non-empty line: tab, `;` or `,` (whichever occurs
//!   most outside quotes; `,` on a tie).
//! - Quoted fields may hold delimiters, `""` (a quote) and line breaks. CRLF, LF and CR all end
//!   a record. A missing closing quote ends the field at the end of the input.
//! - Hostile input is bounded: see [`Limits`]. Exceeding a limit is an error, never a crash.

use crate::CreditsError;

/// Upper bounds for a table read from untrusted input.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_rows: usize,
    pub max_cols: usize,
    pub max_cell_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 16 << 20, max_rows: 100_000, max_cols: 256, max_cell_chars: 20_000 }
    }
}

/// Detect the delimiter from the first non-empty line.
pub fn detect_delimiter(text: &str) -> char {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut counts = [0usize; 3]; // tab, ';', ','
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            '\t' if !quoted => counts[0] += 1,
            ';' if !quoted => counts[1] += 1,
            ',' if !quoted => counts[2] += 1,
            _ => {}
        }
    }
    let [tab, semi, comma] = counts;
    if tab > semi && tab > comma {
        '\t'
    } else if semi > comma {
        ';'
    } else {
        ','
    }
}

/// Read a table from raw bytes. Rows keep their cells as written (no trimming).
pub fn read_table(bytes: &[u8], limits: Limits) -> Result<Vec<Vec<String>>, CreditsError> {
    if bytes.len() > limits.max_bytes {
        return Err(CreditsError::TooLarge(format!("the file is {} bytes; the limit is {}", bytes.len(), limits.max_bytes)));
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let delim = detect_delimiter(text);
    parse(text, delim, limits)
}

/// Parse `text` with an explicit delimiter.
pub fn parse(text: &str, delim: char, limits: Limits) -> Result<Vec<Vec<String>>, CreditsError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut cell_chars = 0usize;
    let mut quoted = false;
    let mut at_field_start = true;
    let mut chars = text.chars().peekable();

    let push_char = |cell: &mut String, n: &mut usize, c: char| -> Result<(), CreditsError> {
        *n = n.saturating_add(1);
        if *n > limits.max_cell_chars {
            return Err(CreditsError::TooLarge(format!("a cell is longer than {} characters", limits.max_cell_chars)));
        }
        cell.push(c);
        Ok(())
    };
    let end_cell = |row: &mut Vec<String>, cell: &mut String, n: &mut usize| -> Result<(), CreditsError> {
        if row.len() >= limits.max_cols {
            return Err(CreditsError::TooLarge(format!("a row has more than {} columns", limits.max_cols)));
        }
        row.push(std::mem::take(cell));
        *n = 0;
        Ok(())
    };

    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    push_char(&mut cell, &mut cell_chars, '"')?;
                } else {
                    quoted = false;
                }
            } else {
                push_char(&mut cell, &mut cell_chars, c)?;
            }
            continue;
        }
        match c {
            '"' if at_field_start => {
                quoted = true;
                at_field_start = false;
            }
            c if c == delim => {
                end_cell(&mut row, &mut cell, &mut cell_chars)?;
                at_field_start = true;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                end_cell(&mut row, &mut cell, &mut cell_chars)?;
                if rows.len() >= limits.max_rows {
                    return Err(CreditsError::TooLarge(format!("the table has more than {} rows", limits.max_rows)));
                }
                rows.push(std::mem::take(&mut row));
                at_field_start = true;
            }
            c => {
                push_char(&mut cell, &mut cell_chars, c)?;
                at_field_start = false;
            }
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        end_cell(&mut row, &mut cell, &mut cell_chars)?;
        if rows.len() >= limits.max_rows {
            return Err(CreditsError::TooLarge(format!("the table has more than {} rows", limits.max_rows)));
        }
        rows.push(row);
    }
    Ok(rows)
}
