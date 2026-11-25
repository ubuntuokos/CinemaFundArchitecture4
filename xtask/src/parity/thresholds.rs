//! Parity thresholds, the ratchet and the committed checklist (docs/plans/CFA4-PARITY-G7, R9–R12).
//!
//! `xtask/parity/thresholds.json` records, per area, the least number of reference items that must
//! be present (`present`) and rated exact (`exact`):
//!
//! ```json
//! {"schema": 1, "areas": {"panels": {"present": 24, "exact": 0}}}
//! ```
//!
//! Counts, not percentages (owner decision D1). `cargo xtask parity --ci` measures every area against
//! the committed references and fails below a threshold; above one it prints a hint to raise it.
//! `--check-ratchet --base REF` fails when the file lowers a count, or drops an area or a key,
//! compared with `REF`: thresholds only go up. `--check-checklist` keeps `docs/parity-checklist.md`
//! (the committed references, one list per area, no date or hash) in step with the references.
//! The file is read as hostile input, like the references.

use super::*;

/// The committed thresholds, from the repository root.
pub(super) const THRESHOLDS_FILE: &str = "xtask/parity/thresholds.json";
/// The committed checklist, from the repository root.
pub(super) const CHECKLIST_FILE: &str = "docs/parity-checklist.md";
/// The counts an area may carry.
const KEYS: [&str; 2] = ["present", "exact"];

/// Least counts per area name, then key.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Thresholds {
    pub(super) areas: BTreeMap<String, BTreeMap<String, usize>>,
}

/// Parses a thresholds file; any deviation from the fixed schema is an error.
pub(super) fn parse(v: &Value) -> Result<Thresholds, String> {
    let o = v.as_object().ok_or("thresholds: not an object")?;
    if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "schema" | "areas")) {
        return Err(format!("thresholds: unknown field {k:?}"));
    }
    if o.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err("thresholds: schema must be the number 1".into());
    }
    let areas = o.get("areas").and_then(Value::as_object).ok_or("thresholds: areas is missing or not an object")?;
    let mut out = Thresholds::default();
    for (name, counts) in areas {
        if !ALL_AREAS.iter().any(|a| a.name() == name) {
            return Err(format!("thresholds: unknown area {name:?}"));
        }
        let counts = counts.as_object().ok_or_else(|| format!("thresholds: {name} is not an object"))?;
        let mut map = BTreeMap::new();
        for (key, n) in counts {
            if !KEYS.contains(&key.as_str()) {
                return Err(format!("thresholds: {name}: unknown key {key:?} (present or exact)"));
            }
            let n = n.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n <= MAX_ITEMS);
            let n = n.ok_or_else(|| format!("thresholds: {name}.{key} must be a whole number from 0 to {MAX_ITEMS}"))?;
            map.insert(key.clone(), n);
        }
        out.areas.insert(name.clone(), map);
    }
    Ok(out)
}

fn parse_text(what: &str, text: &str) -> Result<Thresholds, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{what}: {e}"))?;
    parse(&v).map_err(|e| format!("{what}: {e}"))
}

/// The committed thresholds; a missing file is an error (`--ci` needs it).
pub(super) fn load(root: &Path) -> Result<Thresholds, String> {
    let path = root.join(THRESHOLDS_FILE);
    if !path.exists() {
        return Err(format!("{THRESHOLDS_FILE}: missing (--ci needs the committed thresholds)"));
    }
    parse_text(THRESHOLDS_FILE, &read_bounded(&path)?)
}

/// Compares the measured counts with the thresholds. Returns the failures and the hints (a count
/// above its threshold: the threshold can be raised).
pub(super) fn check(t: &Thresholds, outcomes: &[Outcome]) -> (Vec<String>, Vec<String>) {
    let (mut failures, mut hints) = (Vec::new(), Vec::new());
    for (name, counts) in &t.areas {
        let Some(o) = outcomes.iter().find(|o| o.area.name() == name) else {
            failures.push(format!("{name}: has a threshold but was not measured"));
            continue;
        };
        let Some((report, _)) = &o.measured else {
            failures.push(format!("{name}: has a threshold but no committed reference ({REFERENCE_DIR}/{name}.json)"));
            continue;
        };
        let (same, elsewhere, _) = report.counts();
        let actual = |key: &str| if key == "exact" { report.tier_counts()[Tier::Exact.index()] } else { same + elsewhere };
        for (key, min) in counts {
            let now = actual(key);
            if now < *min {
                failures.push(format!("{name}: {key} {now} is below the threshold {min} ({THRESHOLDS_FILE})"));
            } else if now > *min {
                hints.push(format!("{name}: {key} is {now}, above the threshold {min}: raise it in {THRESHOLDS_FILE}"));
            }
        }
    }
    (failures, hints)
}

/// What the ratchet forbids: a lower count, or an area or key that disappeared.
pub(super) fn ratchet(base: &Thresholds, head: &Thresholds) -> Vec<String> {
    let mut failures = Vec::new();
    for (name, counts) in &base.areas {
        let Some(now) = head.areas.get(name) else {
            failures.push(format!("{name}: area removed from {THRESHOLDS_FILE}"));
            continue;
        };
        for (key, was) in counts {
            match now.get(key) {
                None => failures.push(format!("{name}.{key}: removed from {THRESHOLDS_FILE} (was {was})")),
                Some(n) if n < was => failures.push(format!("{name}.{key}: lowered from {was} to {n}")),
                Some(_) => {}
            }
        }
    }
    failures
}

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git").args(args).current_dir(root).output().map_err(|e| format!("git {}: {e}", args.join(" ")))
}

/// `--check-ratchet --base REF`: the thresholds at `REF` against the working tree. No file at
/// `REF` (the first version) is fine.
pub(super) fn check_ratchet(root: &Path, base: &str) -> Result<String, String> {
    if base.starts_with('-') {
        return Err(format!("--base {base}: not a git ref"));
    }
    let rev = format!("{base}^{{commit}}");
    let verify = git(root, &["rev-parse", "--verify", "--quiet", &rev])?;
    if !verify.status.success() {
        return Err(format!("--base {base}: not a commit"));
    }
    let tree = git(root, &["ls-tree", "--name-only", &rev, "--", THRESHOLDS_FILE])?;
    if !tree.status.success() {
        return Err(format!("git ls-tree {base}: {}", String::from_utf8_lossy(&tree.stderr).trim()));
    }
    if tree.stdout.is_empty() {
        return Ok(format!("parity ratchet: ok (no {THRESHOLDS_FILE} at {base})"));
    }
    let shown = git(root, &["show", &format!("{rev}:{THRESHOLDS_FILE}")])?;
    if !shown.status.success() {
        return Err(format!("git show {base}:{THRESHOLDS_FILE}: {}", String::from_utf8_lossy(&shown.stderr).trim()));
    }
    if shown.stdout.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("{THRESHOLDS_FILE} at {base}: larger than {MAX_FILE_BYTES} bytes"));
    }
    let before = parse_text(&format!("{THRESHOLDS_FILE} at {base}"), &String::from_utf8_lossy(&shown.stdout))?;
    let after = load(root)?;
    let failures = ratchet(&before, &after);
    if failures.is_empty() {
        Ok(format!("parity ratchet: ok (no threshold lowered since {base})"))
    } else {
        Err(format!("parity ratchet: {} problem(s)\n  {}", failures.len(), failures.join("\n  ")))
    }
}

/// The checklist text: the thresholds, then the committed references' items per area, in a fixed
/// order. No date, no hash: the same inputs give the same bytes.
pub(super) fn checklist(root: &Path) -> Result<String, String> {
    let mut s = String::from(
        "# Parity checklist\n\nGenerated by `cargo xtask parity --write-checklist` from the committed references in \
         `xtask/parity/reference/`; `cargo xtask parity --check-checklist` (part of `cargo xtask ci`) fails when this file is out of date. \
         It lists what the reference names, with the document each item comes from. Presence and fidelity are measured, not listed here: \
         `cargo xtask parity --ci`.\n\n",
    );
    let thresholds = root.join(THRESHOLDS_FILE);
    if thresholds.exists() {
        let t = parse_text(THRESHOLDS_FILE, &read_bounded(&thresholds)?)?;
        s.push_str("## Thresholds\n\n| Area | Present at least | Exact at least |\n|---|---|---|\n");
        for (name, counts) in &t.areas {
            let cell = |k: &str| counts.get(k).map_or("–".to_string(), |n| n.to_string());
            s.push_str(&format!("| {name} | {} | {} |\n", cell("present"), cell("exact")));
        }
        s.push('\n');
    }
    for area in ALL_AREAS {
        let file = root.join(REFERENCE_DIR).join(format!("{}.json", area.name()));
        if !file.exists() {
            continue;
        }
        let name = area.name();
        let v: Value = serde_json::from_str(&read_bounded(&file)?).map_err(|e| format!("{REFERENCE_DIR}/{name}.json: {e}"))?;
        let r = parse_v1(&v, area).map_err(|e| format!("{REFERENCE_DIR}/{name}.json: {e}"))?;
        let mut items: Vec<(String, &str)> = r.items.iter().map(|i| (show(&i.path), i.source.as_str())).collect();
        items.sort();
        s.push_str(&format!("## {name} ({})\n\n", items.len()));
        for (path, source) in items {
            s.push_str(&format!("- {path} (`{source}`)\n"));
        }
        s.push('\n');
    }
    Ok(s.trim_end().to_string() + "\n")
}

/// `--check-checklist` (or `--write-checklist`): the committed checklist is the generated one.
pub(super) fn check_checklist(root: &Path, write: bool) -> Result<String, String> {
    let want = checklist(root)?;
    let path = root.join(CHECKLIST_FILE);
    if write {
        std::fs::write(&path, &want).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(format!("parity checklist: wrote {CHECKLIST_FILE}"));
    }
    let have = read_bounded(&path).map_err(|e| format!("{e} (run cargo xtask parity --write-checklist)"))?;
    if have == want {
        Ok(format!("parity checklist: ok ({CHECKLIST_FILE})"))
    } else {
        Err(format!("{CHECKLIST_FILE} is out of date: run cargo xtask parity --write-checklist and commit it"))
    }
}
