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
//!   re-encoder that refuses stops the batch with nothing written. Each written file goes through
//!   the single-range path, so it is verified and written atomically; if a later range still fails
//!   while writing, the error names the files already written (they are complete and verified).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::commands::{bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

/// More ranges than this in one command is refused (an input-derived count).
pub(crate) const MAX_RANGES: usize = 1000;

type Single = fn(&mut Session, &Value) -> Result<Value>;

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
                if !outputs.insert(o.clone()) {
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

/// `export.plan` with `ranges`: the plan of every range and the totals.
pub(crate) fn plan(s: &mut Session, p: &Value, single: Single) -> Result<Value> {
    let cmd = "export.plan";
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
    Ok(json!({"ranges": plans, "summary": summary, "problems": problems}))
}

/// `export.start` with `ranges`: every range planned and checked first, then written one by one.
pub(crate) fn start(s: &mut Session, p: &Value, single_plan: Single, single_start: Single) -> Result<Value> {
    let cmd = "export.start";
    let params = range_params(p, cmd, true)?;
    let mut plans = Vec::with_capacity(params.len());
    for (i, q) in params.iter().enumerate() {
        plans.push(single_plan(s, q).map_err(|e| EngineError::Other(format!("range {}: {e}", i + 1)))?);
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
    let mut written: Vec<Value> = Vec::with_capacity(params.len());
    for (i, q) in params.iter().enumerate() {
        match single_start(s, q) {
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
