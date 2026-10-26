//! File ▸ Export ▸ Batch Variants from Spreadsheet…: one copy of the active sequence per
//! spreadsheet row, with the row's values put into the graphics' text, each queued for export
//! (A/B ad versions, one video per customer quote, localised titles).
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `export.batchVariants` | File ▸ Export ▸ Batch Variants from Spreadsheet… | reads a CSV / TSV / `.xlsx` / `.ods` table and makes (and queues) one sequence per row |
//!
//! The first row names the columns. In every text layer of every graphic clip, `{{Column}}`
//! is replaced by that row's cell (case-insensitive; static text and every Source Text keyframe),
//! and a text layer whose Layer Name equals a column name gets the cell as its whole text.
//! Per-character styles follow the edited text, and a changed instance of a source graphic gets
//! its own project item (so variants never sync back to the template). A `Name` (or `File`)
//! column names the sequence and the exported file (repeated names get ` (2)`, ` (3)`…);
//! otherwise the variants are numbered. Placeholders without a column are left as they are and
//! reported. At most [`MAX_ROWS`] rows. `path` is the spreadsheet; the export folder is
//! `exportPath`. The whole batch is one undo step.

use cfa4_project::graphic::TEXT_LAYER;
use cfa4_project::graphic_design::adjust_runs;
use cfa4_project::{ItemId, ParamValue, Project};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, str_p};
use crate::{Result, Session};

/// Most rows (variants) one run makes.
pub const MAX_ROWS: usize = 500;
/// Largest spreadsheet read from disk.
const MAX_FILE: u64 = 16 << 20;

/// Replace `{{key}}` (case-insensitive, spaces around the key allowed) by its value; unknown
/// keys stay and are collected in `missing`.
pub fn fill(text: &str, row: &[(String, String)], missing: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(a) = rest.find("{{") {
        let Some(b) = rest[a + 2..].find("}}") else { break };
        let key = rest[a + 2..a + 2 + b].trim();
        out.push_str(&rest[..a]);
        match row.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some((_, v)) => out.push_str(v),
            None => {
                if !missing.iter().any(|m| m.eq_ignore_ascii_case(key)) {
                    missing.push(key.to_string());
                }
                out.push_str(&rest[a..a + 2 + b + 2]);
            }
        }
        rest = &rest[a + 2 + b + 2..];
    }
    out.push_str(rest);
    out
}

fn file_safe(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() { "Variant".into() } else { t.chars().take(80).collect() }
}

/// Put one row into the text layers of a sequence; returns how many layers changed. A changed
/// instance of a source graphic gets its own project item, so the variant's text is not synced
/// back to the template and the other variants.
fn apply_row(p: &mut Project, q: &mut cfa4_project::Sequence, row: &[(String, String)], missing: &mut Vec<String>) -> usize {
    let mut n = 0;
    for t in &mut q.video_tracks {
        for it in &mut t.items {
            let before = n;
            for e in it.effects.iter_mut().filter(|e| e.effect == TEXT_LAYER) {
                let layer_name = match e.params.get("name").map(|p| &p.value) {
                    Some(ParamValue::Text(s)) => s.trim().to_string(),
                    _ => String::new(),
                };
                let whole = (!layer_name.is_empty()).then(|| row.iter().find(|(k, _)| k.eq_ignore_ascii_case(&layer_name)).map(|(_, v)| v.clone())).flatten();
                let Some(param) = e.params.get_mut("text") else { continue };
                let old = match &param.value {
                    ParamValue::Text(s) => s.clone(),
                    _ => continue,
                };
                let mut miss = Vec::new();
                param.map_values(|v| match v {
                    ParamValue::Text(s) => ParamValue::Text(match &whole {
                        Some(w) => w.clone(),
                        None => fill(&s, row, &mut Vec::new()),
                    }),
                    v => v,
                });
                if whole.is_none() {
                    fill(&old, row, &mut miss);
                }
                for m in miss {
                    if !missing.iter().any(|x| x.eq_ignore_ascii_case(&m)) {
                        missing.push(m);
                    }
                }
                let new = match &param.value {
                    ParamValue::Text(s) => s.clone(),
                    _ => continue,
                };
                if new != old {
                    n += 1;
                    if let Some(x) = e.layer.as_mut() {
                        x.runs = adjust_runs(&old, &new, &x.runs);
                    }
                }
            }
            if n > before
                && p.source_graphics.contains_key(&it.item)
                && let Some(src) = p.item(it.item).cloned()
            {
                it.item = p.add_item(&src.name, src.label, src.kind, None);
            }
        }
    }
    n
}

/// `label`, or `label (2)`, `label (3)`… when an earlier variant already took it (compared as
/// file names, case-insensitively), so no two variants export to the same file.
fn unique(label: String, taken: &mut std::collections::HashSet<String>) -> String {
    let mut out = label.clone();
    let mut k = 2u32;
    while !taken.insert(out.to_lowercase()) {
        out = format!("{label} ({k})");
        k = k.saturating_add(1);
    }
    out
}

fn read_rows(cmd: &str, p: &Value) -> Result<Vec<Vec<String>>> {
    let bytes = match (str_p(p, "csv"), str_p(p, "path")) {
        (Some(text), _) => text.as_bytes().to_vec(),
        (None, Some(path)) => {
            let meta = std::fs::metadata(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
            if meta.len() > MAX_FILE {
                return Err(bad(cmd, format!("{path} is larger than {} MiB", MAX_FILE >> 20)));
            }
            std::fs::read(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?
        }
        (None, None) => return Err(bad(cmd, "need `path` (a CSV / TSV / .xlsx / .ods file) or `csv` text")),
    };
    let sheet = cfa4_credits::read_rows(&bytes, str_p(p, "sheet")).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(sheet.rows)
}

fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "export.batchVariants";
    let seq = crate::export_tools::sequence_param(s, p, CMD)?;
    let rows = read_rows(CMD, p)?;
    let mut it = rows.into_iter().filter(|r| r.iter().any(|c| !c.trim().is_empty()));
    let header: Vec<String> = it.next().ok_or_else(|| bad(CMD, "the table is empty"))?.into_iter().map(|h| h.trim().to_string()).collect();
    if header.iter().all(String::is_empty) {
        return Err(bad(CMD, "the first row must name the columns"));
    }
    let data: Vec<Vec<String>> = it.collect();
    if data.is_empty() {
        return Err(bad(CMD, "the table has column names but no rows"));
    }
    if data.len() > MAX_ROWS {
        return Err(bad(CMD, format!("{} rows; at most {MAX_ROWS} variants per run", data.len())));
    }
    let name_col = header.iter().position(|h| h.eq_ignore_ascii_case("name")).or_else(|| header.iter().position(|h| h.eq_ignore_ascii_case("file")));
    let base = s.project.item(seq).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into());
    let export = bool_p(p, "export").unwrap_or(true);
    let mut made = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut unchanged = 0usize;
    let mut taken = std::collections::HashSet::new();
    // one undo step for the whole batch
    s.edit_group("Batch Variants", |s| {
        for (i, cells) in data.iter().enumerate() {
            let row: Vec<(String, String)> =
                header.iter().enumerate().filter(|(_, h)| !h.is_empty()).map(|(k, h)| (h.clone(), cells.get(k).cloned().unwrap_or_default())).collect();
            let label =
                name_col.and_then(|k| cells.get(k)).map(|v| v.trim()).filter(|v| !v.is_empty()).map(file_safe).unwrap_or_else(|| format!("Variant {}", i + 1));
            let label = unique(label, &mut taken);
            let name = format!("{base} – {label}");
            let mut changed = 0;
            let mut miss = Vec::new();
            let id = crate::social::copy_sequence(s, seq, &name, "Batch Variant", |p, q| {
                changed = apply_row(p, q, &row, &mut miss);
                Ok(())
            })?;
            if changed == 0 {
                unchanged += 1;
            }
            for m in miss {
                if !missing.iter().any(|x| x.eq_ignore_ascii_case(&m)) {
                    missing.push(m);
                }
            }
            made.push((id, label));
        }
        Ok(())
    })?;
    let mut queued = Value::Array(Vec::new());
    if export {
        let mut q = json!({"sequences": made.iter().map(|(i, _)| i.0).collect::<Vec<_>>()});
        for k in ["preset", "settings", "start", "wait", "range", "loudnessLufs"] {
            if let Some(v) = p.get(k) {
                q[k] = v.clone();
            }
        }
        if let Some(v) = p.get("exportPath") {
            q["path"] = v.clone();
        }
        queued = s.execute("export.queue.add", q)?["added"].clone();
    }
    let ids: Vec<ItemId> = made.iter().map(|(i, _)| *i).collect();
    s.state.project_selection = ids;
    Ok(json!({
        "variants": made.iter().map(|(i, n)| json!({"sequence": i.0, "name": n})).collect::<Vec<_>>(),
        "columns": header,
        "missing": missing,
        "unchanged": unchanged,
        "queued": queued,
    }))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "export.batchVariants",
        label: "Batch Variants from Spreadsheet…",
        menu: &["File", "Export"],
        shortcut: None,
        params: r#"{"path":str|"csv":str,"sheet":str?,"sequence":id?,"export":bool=true,"preset":str?,"exportPath":folder?,"start":bool=false,"wait":bool?}"#,
        enabled: has_seq,
        run: batch,
        journal: true,
    }]
}
