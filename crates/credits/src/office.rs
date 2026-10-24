//! Office spreadsheet files: Office Open XML (`.xlsx`, ECMA-376 SpreadsheetML) and
//! OpenDocument (`.ods`, OASIS ODF 1.2+), read from their published specifications into the
//! same rows of cells as a CSV file.
//!
//! - **xlsx:** `xl/workbook.xml` lists the sheets, `xl/_rels/workbook.xml.rels` maps them to
//!   parts, `xl/sharedStrings.xml` holds shared text (rich-text runs are joined, phonetic runs
//!   skipped); cells are placed by their `r` reference (`B7`). Inline strings, formulas' cached
//!   values, booleans and numbers are read as text.
//! - **ods:** `content.xml`; repeated rows and cells (`number-rows-repeated`,
//!   `number-columns-repeated`) are expanded only up to the next non-empty content, so the
//!   million empty rows a sheet ends with cost nothing; `text:p` paragraphs become lines,
//!   `text:s` spaces, `text:tab` tabs, `text:line-break` breaks.
//!
//! The values shown are the stored values, not the displayed formatting (a number formatted as
//! a date reads as its serial number); write durations as text (`12s`, `00:00:12:00`).

use roxmltree::{Document, Node, ParsingOptions};

use crate::CreditsError;
use crate::table::Limits;
use crate::zip::Zip;

/// Which file format a byte string holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Xlsx,
    Ods,
}

/// The rows of the chosen sheet and the names of every sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub rows: Vec<Vec<String>>,
    pub sheets: Vec<String>,
    pub chosen: String,
}

fn parse_xml(bytes: &[u8]) -> Result<String, CreditsError> {
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn doc(text: &str) -> Result<Document<'_>, CreditsError> {
    let opt = ParsingOptions { allow_dtd: false, nodes_limit: 8_000_000, ..Default::default() };
    Document::parse_with_options(text, opt).map_err(|e| CreditsError::Format(format!("damaged spreadsheet XML: {e}")))
}

fn fmt_err(m: impl Into<String>) -> CreditsError {
    CreditsError::Format(m.into())
}

/// Detect the format of a ZIP-packaged spreadsheet.
pub fn kind(zip: &Zip) -> Option<Kind> {
    if zip.find("xl/workbook.xml").is_some() {
        return Some(Kind::Xlsx);
    }
    if zip.find("content.xml").is_some() {
        let mime = zip.find("mimetype").and_then(|e| zip.read(e).ok()).unwrap_or_default();
        let mime = String::from_utf8_lossy(&mime);
        if mime.is_empty() || mime.contains("opendocument.spreadsheet") {
            return Some(Kind::Ods);
        }
    }
    None
}

/// Pick a sheet: by name (case-insensitive) or 1-based number; the first sheet by default.
fn choose(names: &[String], want: Option<&str>) -> Result<usize, CreditsError> {
    let Some(w) = want.map(str::trim).filter(|w| !w.is_empty()) else {
        return if names.is_empty() { Err(fmt_err("the spreadsheet has no sheets")) } else { Ok(0) };
    };
    if let Some(i) = names.iter().position(|n| n.trim().eq_ignore_ascii_case(w)) {
        return Ok(i);
    }
    if let Ok(n) = w.parse::<usize>()
        && n >= 1
        && n <= names.len()
    {
        return Ok(n - 1);
    }
    Err(fmt_err(format!("there is no sheet `{w}` (sheets: {})", names.join(", "))))
}

/// Read an office spreadsheet package.
pub fn read_office(bytes: &[u8], sheet: Option<&str>, limits: Limits) -> Result<Sheet, CreditsError> {
    if bytes.len() > limits.max_bytes {
        return Err(CreditsError::TooLarge(format!("the file is {} bytes; the limit is {}", bytes.len(), limits.max_bytes)));
    }
    let zip = Zip::open(bytes)?;
    match kind(&zip) {
        Some(Kind::Xlsx) => xlsx(&zip, sheet, limits),
        Some(Kind::Ods) => ods(&zip, sheet, limits),
        None => Err(fmt_err("this ZIP file is not an .xlsx or .ods spreadsheet (Apple Numbers files: export as CSV or .xlsx)")),
    }
}

// ------------------------------------------------------------------------------------------------
// Grid builder with limits
// ------------------------------------------------------------------------------------------------

struct Grid {
    rows: Vec<Vec<String>>,
    limits: Limits,
}

impl Grid {
    fn set(&mut self, r: usize, c: usize, v: String) -> Result<(), CreditsError> {
        if v.is_empty() {
            return Ok(());
        }
        if r >= self.limits.max_rows {
            return Err(CreditsError::TooLarge(format!("the sheet has more than {} rows", self.limits.max_rows)));
        }
        if c >= self.limits.max_cols {
            return Err(CreditsError::TooLarge(format!("a row has more than {} columns", self.limits.max_cols)));
        }
        if v.chars().count() > self.limits.max_cell_chars {
            return Err(CreditsError::TooLarge(format!("a cell is longer than {} characters", self.limits.max_cell_chars)));
        }
        if self.rows.len() <= r {
            self.rows.resize_with(r + 1, Vec::new);
        }
        if let Some(row) = self.rows.get_mut(r) {
            if row.len() <= c {
                row.resize_with(c + 1, String::new);
            }
            if let Some(cell) = row.get_mut(c) {
                *cell = v;
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------------------------------
// xlsx
// ------------------------------------------------------------------------------------------------

/// `"AB12"` → (row 11, column 27), zero-based.
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let split = r.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = (r.get(..split)?, r.get(split..)?);
    if letters.is_empty() || letters.len() > 4 {
        return None;
    }
    let mut col = 0usize;
    for b in letters.bytes() {
        let up = b.to_ascii_uppercase();
        if !up.is_ascii_uppercase() {
            return None;
        }
        col = col.checked_mul(26)?.checked_add(usize::from(up - b'A') + 1)?;
    }
    let row: usize = digits.parse().ok()?;
    Some((row.checked_sub(1)?, col.checked_sub(1)?))
}

fn local<'a>(n: &Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && local(c) == name)
}

/// The text of `t` elements under `n`, skipping phonetic runs.
fn run_text(n: Node) -> String {
    let mut s = String::new();
    for d in n.descendants() {
        if d.is_element() && local(&d) == "t" && !d.ancestors().any(|a| a.is_element() && local(&a) == "rPh") {
            s.push_str(d.text().unwrap_or(""));
        }
    }
    s
}

fn resolve_target(target: &str) -> String {
    let t = target.trim_start_matches('/');
    if t.starts_with("xl/") { t.to_string() } else { format!("xl/{t}") }
}

fn xlsx(zip: &Zip, sheet: Option<&str>, limits: Limits) -> Result<Sheet, CreditsError> {
    let wb_text = parse_xml(&zip.read_named("xl/workbook.xml")?)?;
    let wb = doc(&wb_text)?;
    let mut names = Vec::new();
    let mut rids = Vec::new();
    for s in wb.descendants().filter(|n| n.is_element() && local(n) == "sheet") {
        names.push(s.attribute("name").unwrap_or("").to_string());
        let rid = s.attributes().find(|a| a.name() == "id" && a.namespace().is_some_and(|ns| ns.ends_with("/relationships"))).map(|a| a.value().to_string());
        rids.push(rid.unwrap_or_default());
    }
    let idx = choose(&names, sheet)?;
    let rid = rids.get(idx).cloned().unwrap_or_default();

    let rels_text = parse_xml(&zip.read_named("xl/_rels/workbook.xml.rels")?)?;
    let rels = doc(&rels_text)?;
    let target = rels
        .descendants()
        .find(|n| n.is_element() && local(n) == "Relationship" && n.attribute("Id") == Some(rid.as_str()))
        .and_then(|n| n.attribute("Target"))
        .map(resolve_target)
        .ok_or_else(|| fmt_err("the workbook does not say where its sheet is"))?;

    let shared: Vec<String> = match zip.find("xl/sharedStrings.xml") {
        Some(e) => {
            let t = parse_xml(&zip.read(e)?)?;
            let d = doc(&t)?;
            d.root_element().children().filter(|n| n.is_element() && local(n) == "si").map(run_text).collect()
        }
        None => Vec::new(),
    };

    let sheet_text = parse_xml(&zip.read_named(&target)?)?;
    let sd = doc(&sheet_text)?;
    let mut grid = Grid { rows: Vec::new(), limits };
    let data = sd.descendants().find(|n| n.is_element() && local(n) == "sheetData").ok_or_else(|| fmt_err("the sheet has no data"))?;
    let mut next_row = 0usize;
    for row in data.children().filter(|n| n.is_element() && local(n) == "row") {
        let r = row.attribute("r").and_then(|v| v.parse::<usize>().ok()).and_then(|v| v.checked_sub(1)).unwrap_or(next_row);
        next_row = r.saturating_add(1);
        let mut next_col = 0usize;
        for c in row.children().filter(|n| n.is_element() && local(n) == "c") {
            let (rr, cc) = c.attribute("r").and_then(cell_ref).unwrap_or((r, next_col));
            next_col = cc.saturating_add(1);
            let v = child(c, "v").and_then(|v| v.text()).unwrap_or("");
            let text = match c.attribute("t").unwrap_or("n") {
                "s" => v.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default(),
                "inlineStr" => child(c, "is").map(run_text).unwrap_or_default(),
                "b" => (if v.trim() == "1" { "TRUE" } else { "FALSE" }).to_string(),
                _ => v.to_string(),
            };
            grid.set(rr, cc, text.replace("\r\n", "\n"))?;
        }
    }
    Ok(Sheet { rows: grid.rows, chosen: names.get(idx).cloned().unwrap_or_default(), sheets: names })
}

// ------------------------------------------------------------------------------------------------
// ods
// ------------------------------------------------------------------------------------------------

/// Most consecutive spaces one `text:s` may stand for.
const MAX_SPACES: usize = 1_000;

fn attr_local<'a>(n: &Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

fn repeat(n: &Node, name: &str) -> usize {
    attr_local(n, name).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1)
}

fn para_text(n: Node, out: &mut String) {
    for c in n.children() {
        if c.is_text() {
            out.push_str(c.text().unwrap_or(""));
        } else if c.is_element() {
            match local(&c) {
                "s" => {
                    let k = attr_local(&c, "c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(MAX_SPACES);
                    out.extend(std::iter::repeat_n(' ', k));
                }
                "tab" => out.push('\t'),
                "line-break" => out.push('\n'),
                "annotation" | "note" => {}
                _ => para_text(c, out),
            }
        }
    }
}

fn ods_cell(c: Node) -> String {
    let paras: Vec<String> = c
        .children()
        .filter(|p| p.is_element() && local(p) == "p")
        .map(|p| {
            let mut s = String::new();
            para_text(p, &mut s);
            s
        })
        .collect();
    if !paras.is_empty() {
        return paras.join("\n");
    }
    attr_local(&c, "value").or_else(|| attr_local(&c, "string-value")).unwrap_or("").to_string()
}

fn ods(zip: &Zip, sheet: Option<&str>, limits: Limits) -> Result<Sheet, CreditsError> {
    let text = parse_xml(&zip.read_named("content.xml")?)?;
    let d = doc(&text)?;
    let tables: Vec<Node> =
        d.descendants().filter(|n| n.is_element() && local(n) == "table" && n.tag_name().namespace().is_some_and(|ns| ns.contains(":table:"))).collect();
    let names: Vec<String> = tables.iter().map(|t| attr_local(t, "name").unwrap_or("").to_string()).collect();
    let idx = choose(&names, sheet)?;
    let table = tables.get(idx).copied().ok_or_else(|| fmt_err("the sheet is missing"))?;
    let mut grid = Grid { rows: Vec::new(), limits };
    let mut r = 0usize;
    // rows may sit inside header-rows / row-groups
    let rows = table.descendants().filter(|n| n.is_element() && local(n) == "table-row");
    for row in rows {
        let rep_r = repeat(&row, "number-rows-repeated");
        let mut cells: Vec<(usize, String)> = Vec::new();
        let mut c = 0usize;
        for cell in row.children().filter(|n| n.is_element() && matches!(local(n), "table-cell" | "covered-table-cell")) {
            let rep_c = repeat(&cell, "number-columns-repeated");
            let v = ods_cell(cell);
            if !v.is_empty() {
                for k in 0..rep_c.min(limits.max_cols) {
                    cells.push((c.saturating_add(k), v.clone()));
                }
            }
            c = c.saturating_add(rep_c);
        }
        if cells.is_empty() {
            r = r.saturating_add(rep_r);
            continue;
        }
        for k in 0..rep_r.min(limits.max_rows.saturating_add(1)) {
            for (cc, v) in &cells {
                grid.set(r.saturating_add(k), *cc, v.clone())?;
            }
        }
        r = r.saturating_add(rep_r);
    }
    Ok(Sheet { rows: grid.rows, chosen: names.get(idx).cloned().unwrap_or_default(), sheets: names })
}
