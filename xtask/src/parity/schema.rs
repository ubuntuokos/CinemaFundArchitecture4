//! The committed reference format (schema 1) and its checks (docs/plans/CFA4-PARITY-G7, R7, R8).
//!
//! ```json
//! {"schema": 1, "area": "menus", "outOfScope": ["Window ▸ Workspaces"],
//!  "items": [{"path": "File ▸ Export ▸ Media", "source": "docs/target-app-parity.md"}]}
//! ```
//!
//! Text only: every item names the committed document it comes from, and `--check-reference` (part
//! of `cargo xtask ci`) verifies that the document is tracked by git, is not under `plan/`, and
//! contains every path segment of the item as whole words. The file is read as hostile input: a fixed schema, only
//! strings, bounded size, no `data:` / URL / base64 values and no duplicates.

use super::*;

/// Largest reference, fidelity or source file read (bytes).
pub(super) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Where the committed references live, from the repository root.
pub(super) const REFERENCE_DIR: &str = "xtask/parity/reference";
/// The path separator of a schema-1 path.
const SEP: char = '▸';

/// One reference item: its path and the committed document it comes from.
#[derive(Debug, PartialEq)]
pub(super) struct RefItem {
    pub(super) path: Vec<String>,
    pub(super) source: String,
}

/// A parsed schema-1 reference.
#[derive(Debug, PartialEq)]
pub(super) struct RefV1 {
    /// Path prefixes the reference counts out of scope (on top of the built-in rules).
    pub(super) out_of_scope: Vec<Vec<String>>,
    pub(super) items: Vec<RefItem>,
}

impl RefV1 {
    pub(super) fn paths(&self) -> Vec<Vec<String>> {
        self.items.iter().map(|i| i.path.clone()).collect()
    }
}

/// Whether a reference JSON declares a schema (and so must be a valid schema-1 file).
pub(super) fn is_v1(v: &Value) -> bool {
    v.as_object().is_some_and(|o| o.contains_key("schema"))
}

/// Refuses values that carry data rather than text: `data:` URIs, URLs and base64 blobs.
pub(super) fn plain_text(what: &str, s: &str) -> Result<(), String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("data:") || lower.contains("://") {
        return Err(format!("{what}: a data: URI or URL is not allowed"));
    }
    let base64 = t.len() >= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    if base64 {
        return Err(format!("{what}: a base64 value is not allowed"));
    }
    Ok(())
}

/// Splits a schema-1 path (`File ▸ Export ▸ Media`) into its parts, bounded and non-empty.
pub(super) fn parse_path(what: &str, s: &str, min: usize) -> Result<Vec<String>, String> {
    plain_text(what, s)?;
    if s.len() > MAX_PATH_BYTES {
        return Err(format!("{what}: more than {MAX_PATH_BYTES} bytes"));
    }
    let parts: Vec<String> = s.split(SEP).map(|p| p.trim().to_string()).collect();
    if parts.iter().any(|p| norm(p).is_empty()) {
        return Err(format!("{what}: {s:?} has an empty path part"));
    }
    if parts.len() > MAX_DEPTH {
        return Err(format!("{what}: nested deeper than {MAX_DEPTH} levels"));
    }
    if parts.len() < min.max(1) {
        return Err(format!("{what}: {s:?} needs at least {min} path parts separated by {SEP}"));
    }
    Ok(parts)
}

/// The string value of `key` in `o`; anything else is an error.
fn string_field<'a>(what: &str, o: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str, String> {
    match o.get(key) {
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(format!("{what}: {key} is not a string")),
        None => Err(format!("{what}: {key} is missing")),
    }
}

/// Refuses an object with a key outside `allowed`.
fn only_keys(what: &str, o: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{what}: unknown field {k:?}")),
        None => Ok(()),
    }
}

/// Parses a schema-1 reference for `area`; any deviation from the fixed schema is an error.
pub(super) fn parse_v1(v: &Value, area: Area) -> Result<RefV1, String> {
    let o = v.as_object().ok_or("reference: not an object")?;
    only_keys("reference", o, &["schema", "area", "outOfScope", "items"])?;
    match o.get("schema").and_then(Value::as_u64) {
        Some(1) => {}
        _ => return Err("reference: schema must be the number 1".into()),
    }
    let declared = string_field("reference", o, "area")?;
    if declared != area.reference_name() {
        return Err(format!("reference: area is {declared:?}, expected {:?}", area.reference_name()));
    }
    let mut out_of_scope = Vec::new();
    match o.get("outOfScope") {
        Some(Value::Array(a)) => {
            if a.len() > MAX_ITEMS {
                return Err(format!("reference: more than {MAX_ITEMS} outOfScope entries"));
            }
            for (i, e) in a.iter().enumerate() {
                let s = e.as_str().ok_or_else(|| format!("reference: outOfScope[{i}] is not a string"))?;
                out_of_scope.push(parse_path(&format!("outOfScope[{i}]"), s, 1)?);
            }
        }
        Some(_) => return Err("reference: outOfScope is not an array".into()),
        None => return Err("reference: outOfScope is missing".into()),
    }
    let list = match o.get("items") {
        Some(Value::Array(a)) => a,
        Some(_) => return Err("reference: items is not an array".into()),
        None => return Err("reference: items is missing".into()),
    };
    if list.len() > MAX_ITEMS {
        return Err(format!("reference: more than {MAX_ITEMS} items"));
    }
    let mut seen = HashSet::new();
    let mut items = Vec::with_capacity(list.len());
    for (i, e) in list.iter().enumerate() {
        let what = format!("items[{i}]");
        let it = e.as_object().ok_or_else(|| format!("{what}: not an object"))?;
        only_keys(&what, it, &["path", "source"])?;
        let path = parse_path(&what, string_field(&what, it, "path")?, area.min_len())?;
        let source = string_field(&what, it, "source")?.trim().to_string();
        plain_text(&what, &source)?;
        if !seen.insert(path.iter().map(|p| norm(p)).collect::<Vec<_>>()) {
            return Err(format!("{what}: {} is listed twice", show(&path)));
        }
        items.push(RefItem { path, source });
    }
    Ok(RefV1 { out_of_scope, items })
}

/// Why a `source` is not an acceptable document path, if it isn't: a relative `docs/**/*.md`.
fn source_path_error(source: &str) -> Option<&'static str> {
    if source.starts_with("plan/") || source.starts_with("/plan/") {
        return Some("is under plan/ (maintainer-local, never committed)");
    }
    if source.starts_with('/') || source.contains('\\') || source.split('/').any(|p| p == ".." || p == "." || p.is_empty()) {
        return Some("is not a clean relative path");
    }
    if !source.starts_with("docs/") || !source.ends_with(".md") {
        return Some("is not a docs/**/*.md file");
    }
    None
}

/// Checks every item of a schema-1 reference against its source document. `tracked` says whether
/// git tracks a path; `read` returns a source's text (size-bounded by the caller). All problems
/// are returned together.
pub(super) fn check_items(file: &str, r: &RefV1, tracked: &dyn Fn(&str) -> bool, read: &mut dyn FnMut(&str) -> Result<String, String>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut texts: HashMap<String, Option<String>> = HashMap::new();
    for it in &r.items {
        let at = format!("{file}: {}", show(&it.path));
        if let Some(why) = source_path_error(&it.source) {
            errors.push(format!("{at}: source {:?} {why}", it.source));
            continue;
        }
        if !tracked(&it.source) {
            errors.push(format!("{at}: source {:?} is not tracked by git", it.source));
            continue;
        }
        let text = texts.entry(it.source.clone()).or_insert_with(|| match read(&it.source) {
            Ok(t) => Some(norm(&t)),
            Err(e) => {
                errors.push(format!("{file}: source {:?}: {e}", it.source));
                None
            }
        });
        let Some(text) = text else { continue };
        if let Some(part) = it.path.iter().find(|p| !contains_phrase(text, &norm(p))) {
            errors.push(format!("{at}: {part:?} does not appear in {}", it.source));
        }
    }
    errors
}

/// Whether `phrase` occurs in the normalised `text` as whole words: not inside a longer word, so
/// "info" is not found in "information" and "push" not in "pushed". An empty phrase never occurs.
pub(super) fn contains_phrase(text: &str, phrase: &str) -> bool {
    if phrase.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(rel) = text.get(from..).and_then(|t| t.find(phrase)) {
        let start = from + rel;
        let end = start + phrase.len();
        let before = text.get(..start).and_then(|t| t.chars().next_back());
        let after = text.get(end..).and_then(|t| t.chars().next());
        if !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric) {
            return true;
        }
        // step past the first character of this hit (on a character boundary)
        from = start + text.get(start..).and_then(|t| t.chars().next()).map_or(1, char::len_utf8);
    }
    false
}

/// The area a committed reference file name stands for (`menus.json` → menus).
fn area_of_file(name: &str) -> Option<Area> {
    [Area::Menus, Area::Commands, Area::Panels, Area::Effects, Area::Transitions].into_iter().find(|a| format!("{}.json", a.name()) == name)
}

/// Reads a regular file of at most [`MAX_FILE_BYTES`] bytes as UTF-8 text.
pub(super) fn read_bounded(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{}: not a regular file", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{}: larger than {MAX_FILE_BYTES} bytes", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The paths git tracks under `docs/`.
fn tracked_docs(root: &Path) -> Result<HashSet<String>, String> {
    let out = Command::new("git").args(["ls-files", "--cached", "--", "docs"]).current_dir(root).output().map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-files: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect())
}

/// `cargo xtask parity --check-reference`: every committed reference parses as schema 1 and agrees
/// with its source documents, and the fidelity file, if there is one, is well formed. No committed
/// reference is fine (the areas are then measured only against a local reference).
pub(super) fn check_references(root: &Path) -> Result<String, String> {
    let dir = root.join(REFERENCE_DIR);
    let mut errors = Vec::new();
    let mut checked = 0usize;
    let mut items = 0usize;
    if dir.exists() {
        let tracked = tracked_docs(root)?;
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
        names.sort();
        for name in names {
            let file = format!("{REFERENCE_DIR}/{name}");
            let Some(area) = area_of_file(&name) else {
                errors.push(format!("{file}: not a reference of a known area (menus, commands, panels, effects, transitions)"));
                continue;
            };
            let parsed = read_bounded(&dir.join(&name))
                .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|e| format!("{file}: {e}")))
                .and_then(|v| parse_v1(&v, area).map_err(|e| format!("{file}: {e}")));
            let r = match parsed {
                Ok(r) => r,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };
            let mut read = |source: &str| read_bounded(&root.join(source));
            errors.extend(check_items(&file, &r, &|s| tracked.contains(s), &mut read));
            checked += 1;
            items += r.items.len();
        }
    }
    let fidelity_file = root.join(fidelity::FIDELITY_FILE);
    if fidelity_file.exists() {
        let res = read_bounded(&fidelity_file)
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
            .and_then(|v| fidelity::parse(&v).map(|_| ()));
        if let Err(e) = res {
            errors.push(format!("{}: {e}", fidelity::FIDELITY_FILE));
        }
    }
    if errors.is_empty() {
        Ok(format!("parity references: ok ({checked} files, {items} items)"))
    } else {
        Err(format!("parity references: {} problem(s)\n  {}", errors.len(), errors.join("\n  ")))
    }
}
