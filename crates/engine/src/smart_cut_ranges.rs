//! Several cut ranges of one source in one command (CFA4-LOSSLESS-MEDIA-TOOLS-001 1.1, K1):
//! `export.plan` and `export.start` with source scope and a `"ranges"` list.
//!
//! - Every range is planned exactly as a single-range Smart Cut ([`crate::smart_cut`]); the
//!   parameters outside `ranges` (`path` / `item`, `mode`, `guarantee`, `crf`, `audio`) apply to
//!   all of them, each range brings its own `in`, `out` and, optionally, `output` and `name`.
//! - One output file per range. An `output` on the range wins; otherwise the command's `output`
//!   is a pattern that must contain `{n}` (the range's 1-based number) and may contain `{name}`.
//! - `export.start` plans **every** range before it writes anything: a range that cannot run, a
//!   re-encode without `"confirm": true` (one confirmation for the whole batch) or a boundary
//!   re-encoder that refuses stops the batch with nothing written, and so does an output file
//!   that already exists, unless the batch runs with `"overwrite": true` (the plan lists them
//!   under `existing`). Each written file goes through
//!   the single-range path, so it is verified and written atomically; if a later range still fails
//!   while writing, the error names the files already written (they are complete and verified).
//! - `"join": true` writes the ranges one after the other into the one `output` file instead
//!   (MP4 / MOV sources): every range is cut and verified in memory, the parts are joined without
//!   re-encoding ([`cfa4_isobmff::concat`], which reads the file back and compares every sample),
//!   and the file is written once. `"segmentChapters": true` makes the ranges' names (or their
//!   numbers) the joined file's chapters instead of the source's chapters inside the ranges.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::commands::{bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

/// More ranges than this in one command is refused (an input-derived count).
pub(crate) const MAX_RANGES: usize = 1000;

type Single = fn(&mut Session, &Value) -> Result<Value>;
type Cut = fn(&mut Session, &Value) -> Result<(Vec<u8>, Value)>;

/// The single-range steps the batch is made of.
pub(crate) struct Steps {
    pub plan: Single,
    pub start: Single,
    /// One range cut and verified in memory (for `join`).
    pub cut: Cut,
}

fn join_p(p: &Value, cmd: &str) -> Result<bool> {
    match p.get("join") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(cmd, "`join` must be true or false")),
    }
}

/// Whether `p` asks for several ranges.
pub(crate) fn has_ranges(p: &Value) -> bool {
    p.get("ranges").is_some_and(|r| !r.is_null())
}

/// The parameters of each range: the command's own, minus `ranges`, plus the range's fields.
fn range_params(p: &Value, cmd: &str, with_output: bool) -> Result<Vec<Value>> {
    if p.get("scope").and_then(Value::as_str) == Some("timeline") {
        return Err(bad(cmd, "`ranges` applies to a source (scope \"source\"), not to the timeline"));
    }
    let ranges = p.get("ranges").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`ranges` must be a list of {\"in\", \"out\", \"output\"?, \"name\"?}"))?;
    if ranges.is_empty() {
        return Err(bad(cmd, "`ranges` is empty"));
    }
    if ranges.len() > MAX_RANGES {
        return Err(bad(cmd, format!("`ranges` has {} entries; at most {MAX_RANGES} per command", ranges.len())));
    }
    let Some(base) = p.as_object() else {
        return Err(bad(cmd, "the parameters must be an object"));
    };
    let mut common: Map<String, Value> = base.clone();
    common.remove("ranges");
    let pattern = str_p(p, "output").filter(|o| !o.is_empty()).map(str::to_string);
    let join = join_p(p, cmd)?;
    if join && pattern.as_deref().is_some_and(|o| o.contains("{n}") || o.contains("{name}")) {
        return Err(bad(cmd, "`join` writes one file: its `output` is a file name, not a pattern with {n} or {name}"));
    }
    if join && with_output && pattern.is_none() {
        return Err(bad(cmd, "`join` needs the `output` file the ranges are joined into"));
    }
    common.remove("join");
    common.remove("segmentChapters");
    let mut out = Vec::with_capacity(ranges.len());
    let mut outputs = BTreeSet::new();
    for (i, r) in ranges.iter().enumerate() {
        let n = i + 1;
        let Some(r) = r.as_object() else {
            return Err(bad(cmd, format!("range {n}: must be an object {{\"in\", \"out\", \"output\"?, \"name\"?}}")));
        };
        let mut q = common.clone();
        for key in ["in", "out"] {
            match r.get(key) {
                Some(v) if !v.is_null() => {
                    q.insert(key.into(), v.clone());
                }
                _ => return Err(bad(cmd, format!("range {n}: `{key}` is required"))),
            }
        }
        let name = match r.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(_) => return Err(bad(cmd, format!("range {n}: `name` must be a string"))),
        };
        if join && r.get("output").is_some_and(|o| !o.is_null()) {
            return Err(bad(cmd, format!("range {n}: with `join` the ranges have no `output` of their own")));
        }
        if join {
            out.push(Value::Object(q));
            continue;
        }
        let output = match r.get("output") {
            Some(Value::String(o)) if !o.is_empty() => Some(o.clone()),
            None | Some(Value::Null) => match &pattern {
                Some(pat) if pat.contains("{n}") => Some(expand(pat, n, name)),
                Some(_) if ranges.len() > 1 => {
                    return Err(bad(cmd, format!("range {n}: no `output`; the command's `output` must contain {{n}} to name each range's file")));
                }
                Some(pat) => Some(pat.clone()),
                None if with_output => return Err(bad(cmd, format!("range {n}: no `output`, and the command has no `output` pattern with {{n}}"))),
                None => None,
            },
            Some(_) => return Err(bad(cmd, format!("range {n}: `output` must be a non-empty string"))),
        };
        match output {
            Some(o) => {
                if !outputs.insert(crate::smart_cut::lexical_path(&o)) {
                    return Err(bad(cmd, format!("range {n}: `{o}` is the output of another range too")));
                }
                q.insert("output".into(), Value::String(o));
            }
            None => {
                q.remove("output");
            }
        }
        out.push(Value::Object(q));
    }
    Ok(out)
}

/// The pattern with `{n}` and `{name}` filled in. A name is reduced to characters that are safe
/// in one path component; an empty result falls back to the number.
fn expand(pattern: &str, n: usize, name: Option<&str>) -> String {
    let safe: String = name
        .unwrap_or("")
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == '\0' || c.is_control() { '_' } else { c })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    let safe = if safe.is_empty() { n.to_string() } else { safe };
    pattern.replace("{n}", &n.to_string()).replace("{name}", &safe)
}

fn totals(plans: &[Value]) -> Value {
    let sum = |k: &str| plans.iter().filter_map(|r| r["summary"][k].as_u64()).sum::<u64>();
    let all = |k: &str| plans.iter().all(|r| r["summary"][k] == true);
    let any = |k: &str| plans.iter().any(|r| r["summary"][k] == true);
    json!({
        "ranges": plans.len(),
        "lossless": all("lossless"),
        "requiresConfirmation": any("requiresConfirmation"),
        "copiedFrames": sum("copiedFrames"),
        "reencodedFrames": sum("reencodedFrames"),
    })
}

/// Why a planned range cannot run (its blockers, or the boundary re-encoder's refusal).
fn cannot_run(r: &Value) -> Option<String> {
    if r["plan"]["executable"] == false {
        let why: Vec<&str> = r["plan"]["blockers"].as_array().map(|b| b.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        return Some(if why.is_empty() { "the plan cannot run".into() } else { why.join("; ") });
    }
    if r["summary"]["requiresConfirmation"] == true && r["boundaryReencode"]["ready"] == false {
        return Some(r["boundaryReencode"]["problem"].as_str().unwrap_or("the boundary re-encoder cannot run this range").to_string());
    }
    None
}

/// The files the batch would write that already exist (in the order they would be written).
fn existing(s: &Session, p: &Value, cmd: &str) -> Result<Vec<String>> {
    let outputs: Vec<String> = if join_p(p, cmd)? {
        str_p(p, "output").filter(|o| !o.is_empty()).map(str::to_string).into_iter().collect()
    } else {
        range_params(p, cmd, false)?.iter().filter_map(|q| q.get("output").and_then(Value::as_str).map(str::to_string)).collect()
    };
    Ok(outputs.into_iter().filter(|o| s.services.file_size(o).is_ok()).collect())
}

/// `export.plan` with `ranges`: the plan of every range and the totals.
pub(crate) fn plan(s: &mut Session, p: &Value, single: Single) -> Result<Value> {
    let cmd = "export.plan";
    // the source is read and indexed once for every range
    let _batch = crate::source_cache::batch();
    let mut plans = Vec::new();
    for (i, q) in range_params(p, cmd, false)?.iter().enumerate() {
        let mut r = single(s, q).map_err(|e| EngineError::Other(format!("range {}: {e}", i + 1)))?;
        r["range"] = json!(i + 1);
        r["in"] = q["in"].clone();
        r["out"] = q["out"].clone();
        if let Some(o) = q.get("output") {
            r["output"] = o.clone();
        }
        plans.push(r);
    }
    let summary = totals(&plans);
    let problems: Vec<Value> = plans.iter().filter_map(|r| cannot_run(r).map(|why| json!({"range": r["range"], "problem": why}))).collect();
    let mut out = json!({"ranges": plans, "summary": summary, "problems": problems});
    if join_p(p, cmd)? {
        out["join"] = json!(true);
        out["output"] = p.get("output").cloned().unwrap_or(Value::Null);
    }
    out["existing"] = json!(existing(s, p, cmd)?);
    Ok(out)
}

/// `export.start` with `ranges`: every range planned and checked first, then written one by one.
pub(crate) fn start(s: &mut Session, p: &Value, steps: Steps) -> Result<Value> {
    let cmd = "export.start";
    // the source is read and indexed once for every range, plans and cuts alike
    let _batch = crate::source_cache::batch();
    let params = range_params(p, cmd, true)?;
    let mut plans = Vec::with_capacity(params.len());
    for (i, q) in params.iter().enumerate() {
        plans.push((steps.plan)(s, q).map_err(|e| EngineError::Other(format!("range {}: {e}", i + 1)))?);
    }
    for (i, r) in plans.iter().enumerate() {
        if let Some(why) = cannot_run(r) {
            return Err(EngineError::Other(format!("range {}: Smart Cut cannot run: {why}; nothing was written", i + 1)));
        }
    }
    let summary = totals(&plans);
    if summary["requiresConfirmation"] == true && !bool_p(p, "confirm").unwrap_or(false) {
        let which: Vec<String> =
            plans.iter().enumerate().filter(|(_, r)| r["summary"]["requiresConfirmation"] == true).map(|(i, _)| (i + 1).to_string()).collect();
        return Err(bad(
            cmd,
            format!(
                "ranges {} re-encode {} frames at their boundaries; run the batch with \"confirm\": true, or use \"mode\": \"gop-boundary\" to copy without re-encoding; nothing was written",
                which.join(", "),
                summary["reencodedFrames"]
            ),
        ));
    }
    // the source itself is never an output (plan 1.1 §7), whatever else the batch says
    // compared as the files they name, so another spelling of a path (`./`, `..`, a symbolic
    // link) is caught too
    let (source, _) = crate::smart_cut::source_p(s, &params[0], cmd)?;
    let key = |o: &str| crate::smart_cut::path_key(s, o);
    let source = key(&source);
    let mut seen = BTreeSet::new();
    for (i, o) in params.iter().enumerate().filter_map(|(i, q)| q.get("output").and_then(Value::as_str).map(|o| (i, o))) {
        let k = key(o);
        if k == source {
            return Err(bad(cmd, "the output must not be the source file; nothing was written"));
        }
        if !join_p(p, cmd)? && !seen.insert(k) {
            return Err(bad(cmd, format!("range {}: `{o}` is the same file as the output of another range; nothing was written", i + 1)));
        }
    }
    let taken = existing(s, p, cmd)?;
    if !taken.is_empty() && !bool_p(p, "overwrite").unwrap_or(false) {
        return Err(bad(
            cmd,
            format!(
                "{} already exist: {}; run the batch with \"overwrite\": true to replace them; nothing was written",
                if taken.len() == 1 { "this file" } else { "these files" },
                taken.join(", ")
            ),
        ));
    }
    if join_p(p, cmd)? {
        return join(s, p, &params, steps.cut, summary);
    }
    let mut written: Vec<Value> = Vec::with_capacity(params.len());
    for (i, q) in params.iter().enumerate() {
        match (steps.start)(s, q) {
            Ok(mut r) => {
                r["range"] = json!(i + 1);
                written.push(r);
            }
            Err(e) => {
                let done: Vec<&str> = written.iter().filter_map(|r| r["output"].as_str()).collect();
                let kept = if done.is_empty() { "nothing was written".to_string() } else { format!("already written and verified: {}", done.join(", ")) };
                return Err(EngineError::Other(format!("range {}: {e}; {kept}", i + 1)));
            }
        }
    }
    Ok(json!({"ranges": written, "summary": summary, "verified": true}))
}

/// Most bytes of cut ranges a join holds in memory (the joined file is built next to them).
const MAX_JOIN_BYTES: u64 = 2 << 30;

/// `export.start` with `ranges` and `"join": true`: the ranges cut in memory, joined, written once.
fn join(s: &mut Session, p: &Value, params: &[Value], cut: Cut, summary: Value) -> Result<Value> {
    let cmd = "export.start";
    let output = str_p(p, "output").unwrap_or_default().to_string();
    let segment_chapters = match p.get("segmentChapters") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`segmentChapters` must be true or false")),
    };
    let mut parts = Vec::with_capacity(params.len());
    let mut results = Vec::with_capacity(params.len());
    let mut held = 0u64;
    // the memory the parts may take: each cut stops as soon as it would pass what is left
    let limit = crate::smart_cut::max_bytes_p(p, cmd)?.min(MAX_JOIN_BYTES);
    for (i, q) in params.iter().enumerate() {
        let mut q = q.clone();
        q["maxBytes"] = json!(limit.saturating_sub(held).max(1));
        let (bytes, mut r) = cut(s, &q).map_err(|e| EngineError::Other(format!("range {}: {e}; nothing was written", i + 1)))?;
        if let Some(r) = r.as_object_mut() {
            r.remove("output");
        }
        r["range"] = json!(i + 1);
        held += bytes.len() as u64;
        parts.push(bytes);
        results.push(r);
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let joined = cfa4_isobmff::concat(&refs).map_err(|e| EngineError::Other(format!("{e}; nothing was written")))?;
    drop(parts);
    let at = |t: u64| -> i64 { i64::try_from(u128::from(t) * 10_000_000 / u128::from(joined.timescale.max(1))).unwrap_or(i64::MAX) };
    let mut bytes = joined.bytes;
    if segment_chapters {
        let ranges = p.get("ranges").and_then(Value::as_array).cloned().unwrap_or_default();
        let chapters: Vec<cfa4_isobmff::Chapter> = joined
            .starts
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                let name = ranges.get(i).and_then(|r| r.get("name")).and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
                cfa4_isobmff::Chapter { start_100ns: at(t).max(0) as u64, title: name.map_or_else(|| format!("{}", i + 1), str::to_string) }
            })
            .collect();
        let other = |e: cfa4_isobmff::Error| EngineError::Other(format!("joined chapters: {e}; nothing was written"));
        let before = cfa4_isobmff::open(bytes.as_slice()).map_err(other)?;
        let with = cfa4_isobmff::replace_chapters(&bytes, &chapters).map_err(other)?;
        let after = cfa4_isobmff::open(with.as_slice()).map_err(other)?;
        if after.tracks != before.tracks || after.metadata.chapters.len() != chapters.len().min(cfa4_isobmff::MAX_CHAPTERS) {
            return Err(EngineError::Other("joined chapters: the file read back is not as planned; nothing was written".into()));
        }
        bytes = with;
    }
    s.services.write_file(&output, &bytes).map_err(|e| EngineError::Other(format!("{output}: {e}")))?;
    let chapters: Vec<Value> = cfa4_isobmff::open(bytes.as_slice())
        .map(|f| f.metadata.chapters.iter().map(|c| json!({"start100ns": c.start_100ns, "title": c.title})).collect())
        .unwrap_or_default();
    for (r, &t) in results.iter_mut().zip(&joined.starts) {
        r["start100ns"] = json!(at(t));
    }
    Ok(json!({
        "output": output,
        "bytes": bytes.len(),
        "join": true,
        "ranges": results,
        "summary": summary,
        "samples": joined.samples,
        "duration100ns": at(joined.duration),
        "maxSyncError100ns": joined.max_sync_error_100ns,
        "chapters": chapters,
        "verified": true,
    }))
}
