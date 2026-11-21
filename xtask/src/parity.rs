//! `cargo xtask parity [--area menus|commands|effects|transitions] [--reference FILE] [--ours FILE]
//! [--out FILE] [--min PERCENT]`: measured parity (docs/gaps.md G7).
//!
//! Compares one area of what CFA4 ships with a reference list, and reports presence and
//! placement separately: a reference item is *same place* when we have its label at the same path,
//! *elsewhere* when we have the label under another path, and *missing* otherwise. A label match is
//! presence only, not behaviour. The areas:
//!
//! - `menus` (default): the shipped menu bar (`crates/ui-egui/examples/menu_tree.rs`); paths are
//!   menu paths. App, Apple and Help menus, recent-file lists, account / cloud items and macOS
//!   system items are counted out of scope.
//! - `commands`: the engine's command registry (`crates/engine/examples/command_list.rs`), queries
//!   left out; a command's path is its menu path and label, or the label alone when it is in no
//!   menu. A reference entry may be a menu path or a bare name.
//! - `effects` / `transitions`: the effects (video and audio) or transitions (video and audio) the
//!   Effects panel lists, from the engine's effect definitions
//!   (`crates/project/examples/effect_catalog.rs`); paths are Effects-panel folders
//!   (`Video Effects > Blur & Sharpen > Gaussian Blur`). A reference entry may also be a bare name,
//!   which has no folder and so is matched by name alone. Presets and Lumetri Presets are out of
//!   scope, and so are the other area's items (a folder named `… Transitions`), so one reference
//!   of the whole Effects panel serves both areas.
//!
//! `--ours` reads a saved dump of our side instead of running the example. The reference defaults
//! to the maintainers' local snapshot `plan/premiere/<area>.json` (gitignored, never committed).
//! Without one the tool prints our own inventory and measures nothing. The reference may be any
//! JSON of paths: an array of paths (`["File", "New", "Sequence…"]`) or of
//! `"File > New > Sequence…"` strings, a tree of nodes (`{"name": …, "children" | "items" |
//! "submenu": […]}`), or nested objects keyed by label.
//!
//! The report goes to `target/parity/parity-checklist.md` for menus and
//! `target/parity/parity-<area>.md` for the other areas (or `--out`); `--min` fails when presence
//! is below the given percentage.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Deepest menu nesting accepted from a reference file.
const MAX_DEPTH: usize = 16;
/// Most menu items accepted from a reference file.
const MAX_ITEMS: usize = 20_000;
/// Most UTF-8 bytes retained in one expanded leaf path.
const MAX_PATH_BYTES: usize = 4 * 1024;
/// Keys holding a node's label, and its children.
const LABEL_KEYS: [&str; 3] = ["name", "title", "label"];
const CHILD_KEYS: [&str; 6] = ["children", "items", "submenu", "menu", "menuItems", "menu_items"];
/// Keys of a wrapper object around the whole menu bar, or the whole effect list.
const MENU_WRAPPER_KEYS: [&str; 5] = ["menus", "menuBar", "menu_bar", "menubar", "items"];
const EFFECT_WRAPPER_KEYS: [&str; 4] = ["effects", "transitions", "folders", "items"];
const COMMAND_WRAPPER_KEYS: [&str; 2] = ["commands", "items"];

/// What a parity run measures.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Area {
    Menus,
    Commands,
    Effects,
    Transitions,
}

/// One item on our side: its full path (menu path or folders, then the label) and its id.
#[derive(Clone, Debug, PartialEq)]
struct OurItem {
    path: Vec<String>,
    id: String,
}

impl Area {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "menus" => Ok(Area::Menus),
            "commands" => Ok(Area::Commands),
            "effects" => Ok(Area::Effects),
            "transitions" => Ok(Area::Transitions),
            other => Err(format!("--area {other}: expected menus, commands, effects or transitions")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Area::Menus => "menus",
            Area::Commands => "commands",
            Area::Effects => "effects",
            Area::Transitions => "transitions",
        }
    }

    /// Our side, as the report names it.
    fn ours(self) -> &'static str {
        match self {
            Area::Menus => "shipped menu bar",
            Area::Commands => "engine commands",
            Area::Effects => "Effects panel effects",
            Area::Transitions => "Effects panel transitions",
        }
    }

    /// What a path is made of: "menu path" or "folder".
    fn place(self) -> &'static str {
        match self {
            Area::Menus | Area::Commands => "menu path",
            Area::Effects | Area::Transitions => "folder",
        }
    }

    fn out_file(self) -> String {
        match self {
            Area::Menus => "parity-checklist.md".into(),
            other => format!("parity-{}.md", other.name()),
        }
    }

    /// Fewest path parts a reference item has: a menu item needs its menu, an effect may be a
    /// bare name.
    fn min_len(self) -> usize {
        match self {
            Area::Menus => 2,
            Area::Commands | Area::Effects | Area::Transitions => 1,
        }
    }

    fn wrapper_keys(self) -> &'static [&'static str] {
        match self {
            Area::Menus => &MENU_WRAPPER_KEYS,
            Area::Commands => &COMMAND_WRAPPER_KEYS,
            Area::Effects | Area::Transitions => &EFFECT_WRAPPER_KEYS,
        }
    }

    /// The effect kinds (`EffectKind` names in the catalog dump) this area counts.
    fn kinds(self) -> &'static [&'static str] {
        match self {
            Area::Menus | Area::Commands => &[],
            Area::Effects => &["Video", "Audio"],
            Area::Transitions => &["VideoTransition", "AudioTransition"],
        }
    }

    /// The group a path is tallied under in the report: the top-level menu, or the folder path.
    fn group(self, path: &[String]) -> String {
        match self {
            Area::Menus => path.first().cloned().unwrap_or_default(),
            Area::Commands => match path.split_first() {
                Some((menu, rest)) if !rest.is_empty() => menu.clone(),
                _ => "(no menu)".into(),
            },
            Area::Effects | Area::Transitions => match path.split_last() {
                Some((_, folders)) if !folders.is_empty() => show(folders),
                _ => "(no folder)".into(),
            },
        }
    }
}

struct Args {
    area: Area,
    reference: Option<PathBuf>,
    ours: Option<PathBuf>,
    out: Option<PathBuf>,
    min: Option<f64>,
}

fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut a = Args { area: Area::Menus, reference: None, ours: None, out: None, min: None };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().map(|v| v.to_string()).ok_or_else(|| format!("{flag} needs a value"));
        match *flag {
            "--area" => a.area = Area::parse(&value()?)?,
            "--reference" => a.reference = Some(value()?.into()),
            "--ours" => a.ours = Some(value()?.into()),
            "--out" => a.out = Some(value()?.into()),
            "--min" => {
                let v = value()?;
                let pct: f64 = v.parse().map_err(|_| format!("--min {v}: not a number"))?;
                if !(0.0..=100.0).contains(&pct) {
                    return Err(format!("--min {v}: must be 0–100"));
                }
                a.min = Some(pct);
            }
            other => {
                return Err(format!(
                    "parity: unknown argument {other} (usage: cargo xtask parity [--area menus|commands|effects|transitions] [--reference FILE] [--ours FILE] [--out FILE] [--min PERCENT])"
                ));
            }
        }
    }
    Ok(a)
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let args = parse_args(args)?;
    let area = args.area;
    let ours_json = match &args.ours {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
        None => dump_ours(area)?,
    };
    let ours_value: Value = serde_json::from_str(&ours_json).map_err(|e| format!("our {} dump: {e}", area.name()))?;
    let ours = match area {
        Area::Menus => our_items(&ours_value)?,
        Area::Commands => our_commands(&ours_value)?,
        Area::Effects | Area::Transitions => our_catalog(&ours_value, area)?,
    };
    let default_ref = root.join(format!("plan/premiere/{}.json", area.name()));
    let reference = match &args.reference {
        Some(p) => Some(p.clone()),
        None => default_ref.exists().then_some(default_ref),
    };
    let Some(reference) = reference else {
        print!("{}", inventory(area, &ours));
        let a = area.name();
        println!("\nno reference list (plan/premiere/{a}.json, or --reference FILE): {a} parity not measured");
        return match args.min {
            Some(_) => Err("--min needs a reference list".into()),
            None => Ok(()),
        };
    };
    let text = std::fs::read_to_string(&reference).map_err(|e| format!("{}: {e}", reference.display()))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", reference.display()))?;
    let paths = reference_paths(&value, area)?;
    let name = reference.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let report = compare(area, &paths, &ours);
    let out = args.out.unwrap_or_else(|| crate::target_dir().join("parity").join(area.out_file()));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&out, report.markdown(&name, ours.len())).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{}", report.summary());
    println!("report: {}", out.display());
    match args.min {
        Some(min) if report.presence() < min => Err(format!("{} presence {:.1}% is below --min {min}%", area.name(), report.presence())),
        _ => Ok(()),
    }
}

/// Our side as JSON: the menu bar from the `menu_tree` example, the command registry from the
/// `command_list` example, or the Effects panel catalog from the `effect_catalog` example.
fn dump_ours(area: Area) -> Result<String, String> {
    let (package, example) = match area {
        Area::Menus => ("cfa4-ui-egui", "menu_tree"),
        Area::Commands => ("cfa4-engine", "command_list"),
        Area::Effects | Area::Transitions => ("cfa4-project", "effect_catalog"),
    };
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["run", "-q", "-p", package, "--example", example]).stderr(Stdio::inherit());
    let out = cmd.output().map_err(|e| format!("{example}: {e} (or pass a saved dump with --ours FILE)"))?;
    if !out.status.success() {
        return Err(format!("{example} failed: {} (or pass a saved dump with --ours FILE)", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{example}: {e}"))
}

/// The string elements of a JSON array field (`path`, `menu`); other elements are skipped.
fn strings(it: &Value, key: &str) -> Option<Vec<String>> {
    Some(it.get(key)?.as_array()?.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
}

fn id_of(it: &Value) -> String {
    it.get("id").and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Our menu items (menu path + label), from the `menu_tree` dump.
fn our_items(v: &Value) -> Result<Vec<OurItem>, String> {
    let items = v.as_array().ok_or("our menu dump is not an array")?;
    Ok(items
        .iter()
        .filter_map(|it| {
            let label = it.get("label")?.as_str()?;
            let mut path = strings(it, "path")?;
            path.push(label.to_string());
            (path.len() >= 2).then(|| OurItem { path, id: id_of(it) })
        })
        .collect())
}

/// Our commands (menu path + label, or the label alone), queries left out, from the
/// `command_list` dump.
fn our_commands(v: &Value) -> Result<Vec<OurItem>, String> {
    let items = v.as_array().ok_or("our command dump is not an array")?;
    Ok(items
        .iter()
        .filter(|it| it.get("journal").and_then(Value::as_bool) == Some(true))
        .filter_map(|it| {
            let label = it.get("label")?.as_str()?.trim();
            if label.is_empty() {
                return None;
            }
            let mut path = strings(it, "menu")?;
            path.push(label.to_string());
            Some(OurItem { path, id: id_of(it) })
        })
        .collect())
}

/// Full paths (Effects-panel folders + name) of the catalog items of `area`, from the
/// `effect_catalog` dump.
fn our_catalog(v: &Value, area: Area) -> Result<Vec<OurItem>, String> {
    let items = v.as_array().ok_or("our effect catalog dump is not an array")?;
    Ok(items
        .iter()
        .filter(|it| it.get("kind").and_then(Value::as_str).is_some_and(|k| area.kinds().contains(&k)))
        .filter_map(|it| {
            let name = it.get("name")?.as_str()?;
            let mut path = strings(it, "path")?;
            path.push(name.to_string());
            Some(OurItem { path, id: id_of(it) })
        })
        .collect())
}

/// Item counts per group (top-level menu, or folder), for runs without a reference.
fn inventory(area: Area, ours: &[OurItem]) -> String {
    let mut per: BTreeMap<String, usize> = BTreeMap::new();
    for it in ours {
        *per.entry(area.group(&it.path)).or_default() += 1;
    }
    let mut s = format!("CFA4 {}: {} items\n", area.ours(), ours.len());
    let width = per.keys().map(|g| g.chars().count()).max().unwrap_or(0).max(22);
    for (group, n) in per {
        s.push_str(&format!("  {group:<width$} {n}\n"));
    }
    s
}

/// Leaf paths (top-level menu or folder first, item label last) of a reference file.
fn reference_paths(v: &Value, area: Area) -> Result<Vec<Vec<String>>, String> {
    let mut root = v;
    if let Value::Object(o) = v
        && label_of(o).is_none()
        && let Some(inner) = area.wrapper_keys().iter().find_map(|k| o.get(*k).filter(|c| c.is_array() || c.is_object()))
    {
        root = inner;
    }
    let mut out = Vec::new();
    walk(root, &[], 0, area.min_len(), &mut out)?;
    Ok(out)
}

fn label_of(o: &serde_json::Map<String, Value>) -> Option<&str> {
    LABEL_KEYS.iter().find_map(|k| o.get(*k)?.as_str())
}

/// A separator or placeholder rather than a menu item.
fn is_separator(label: &str) -> bool {
    let l = label.trim();
    l.is_empty() || l.chars().all(|c| matches!(c, '-' | '—' | '–' | '_')) || l.eq_ignore_ascii_case("missing value") || l.eq_ignore_ascii_case("separator")
}

fn push_leaf(prefix: &[String], label: &str, min: usize, out: &mut Vec<Vec<String>>) -> Result<(), String> {
    if is_separator(label) || prefix.iter().any(|p| is_separator(p)) {
        return Ok(());
    }
    if out.len() >= MAX_ITEMS {
        return Err(format!("reference has more than {MAX_ITEMS} items"));
    }
    let bytes = prefix.iter().try_fold(label.trim().len(), |n, part| n.checked_add(part.len())).ok_or("reference path is too large")?;
    if bytes > MAX_PATH_BYTES {
        return Err(format!("reference path has more than {MAX_PATH_BYTES} bytes"));
    }
    let mut path = prefix.to_vec();
    path.push(label.trim().to_string());
    if path.len() >= min.max(1) {
        out.push(path);
    }
    Ok(())
}

fn split_path(s: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = s.replace('\u{a0}', " ").split(['>', '▸']).map(|p| p.trim().to_string()).collect();
    (parts.len() >= 2 && parts.iter().all(|p| !p.is_empty())).then_some(parts)
}

fn walk(v: &Value, prefix: &[String], depth: usize, min: usize, out: &mut Vec<Vec<String>>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("reference is nested deeper than {MAX_DEPTH} levels"));
    }
    let with = |label: &str| {
        let mut p = prefix.to_vec();
        p.push(label.trim().to_string());
        p
    };
    match v {
        Value::String(s) => match split_path(s).filter(|_| prefix.is_empty()) {
            Some(parts) => match parts.split_last() {
                Some((label, menus)) => push_leaf(menus, label, min, out),
                None => Ok(()),
            },
            None => push_leaf(prefix, s, min, out),
        },
        Value::Array(a) => {
            for el in a {
                match el {
                    Value::Array(path) if !path.is_empty() && path.iter().all(Value::is_string) => {
                        let parts: Vec<String> = prefix.iter().cloned().chain(path.iter().filter_map(|p| p.as_str().map(str::to_string))).collect();
                        if let Some((label, menus)) = parts.split_last() {
                            push_leaf(menus, label, min, out)?;
                        }
                    }
                    _ => walk(el, prefix, depth + 1, min, out)?,
                }
            }
            Ok(())
        }
        Value::Object(o) => match label_of(o) {
            Some(label) => {
                let children = CHILD_KEYS.iter().find_map(|k| o.get(*k).filter(|c| has_entries(c)));
                match children {
                    Some(c) if !is_separator(label) => walk(c, &with(label), depth + 1, min, out),
                    Some(_) => Ok(()),
                    None => push_leaf(prefix, label, min, out),
                }
            }
            None => {
                for (k, val) in o {
                    if has_entries(val) {
                        walk(val, &with(k), depth + 1, min, out)?;
                    } else {
                        push_leaf(prefix, k, min, out)?;
                    }
                }
                Ok(())
            }
        },
        _ => Ok(()),
    }
}

fn has_entries(v: &Value) -> bool {
    match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => false,
    }
}

/// A label compared case-insensitively, without ellipsis, with `&` as "and" and one space.
fn norm(label: &str) -> String {
    let l = label.to_lowercase().replace('…', "").replace("...", "").replace(['’', '‘'], "'").replace('&', " and ");
    l.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Why a reference item is not counted, if it isn't.
fn out_of_scope(area: Area, path: &[String]) -> Option<&'static str> {
    match area {
        Area::Menus => menu_out_of_scope(path),
        Area::Commands => None,
        Area::Effects | Area::Transitions => {
            let top = norm(path.first()?);
            if path.len() >= 2 && (top == "presets" || top == "lumetri presets") {
                return Some("presets");
            }
            let folders = path.get(..path.len().saturating_sub(1)).unwrap_or_default();
            let transition = folders.iter().any(|f| norm(f).ends_with("transitions"));
            match area {
                Area::Effects if transition => Some("transitions (--area transitions)"),
                Area::Transitions if !folders.is_empty() && !transition => Some("effects (--area effects)"),
                _ => None,
            }
        }
    }
}

fn menu_out_of_scope(path: &[String]) -> Option<&'static str> {
    let (top, label) = (norm(path.first()?), norm(path.last()?));
    if top == "apple" || top == "help" || top.contains("premiere") {
        return Some("app, Apple and Help menus");
    }
    let parents = path.get(..path.len().saturating_sub(1)).unwrap_or_default();
    if parents.iter().skip(1).any(|p| norm(p).starts_with("open recent") || norm(p).starts_with("recent")) {
        return Some("recent-file lists");
    }
    if ["adobe", "creative cloud", "sign in", "sign out", "log in", "log out"].iter().any(|w| label.contains(w)) {
        return Some("account and cloud items");
    }
    if ["autofill", "start dictation", "emoji and symbols", "writing tools", "services"].iter().any(|w| label.starts_with(w)) {
        return Some("macOS system items");
    }
    None
}

/// Where a reference item is on our side; `Same` and `Elsewhere` carry the index of our item.
enum Placement {
    Same(#[cfg_attr(not(test), expect(dead_code, reason = "read by the fidelity tiers (G7.6); the tests read it already"))] usize),
    Elsewhere(usize),
    Missing,
}

struct Report {
    area: Area,
    /// Our items, which `Placement` points into.
    ours: Vec<OurItem>,
    /// (reference path, where it is in CFA4).
    items: Vec<(Vec<String>, Placement)>,
    out_of_scope: BTreeMap<&'static str, usize>,
    ours_only: Vec<Vec<String>>,
}

fn compare(area: Area, reference: &[Vec<String>], ours: &[OurItem]) -> Report {
    let key = |p: &[String]| p.iter().map(|s| norm(s)).collect::<Vec<_>>();
    let mut by_path: HashMap<Vec<String>, usize> = HashMap::new();
    let mut by_label: HashMap<String, usize> = HashMap::new();
    for (i, it) in ours.iter().enumerate() {
        by_path.entry(key(&it.path)).or_insert(i);
        if let Some(l) = it.path.last() {
            by_label.entry(norm(l)).or_insert(i);
        }
    }
    let mut report = Report { area, ours: ours.to_vec(), items: Vec::new(), out_of_scope: BTreeMap::new(), ours_only: Vec::new() };
    let mut seen = HashSet::new();
    for p in reference {
        if !seen.insert(key(p)) {
            continue;
        }
        if let Some(why) = out_of_scope(area, p) {
            *report.out_of_scope.entry(why).or_default() += 1;
            continue;
        }
        let label = p.last().map(|l| norm(l)).unwrap_or_default();
        // a bare name (no folder) has no place to compare: found is all it can be
        let placement = match (by_path.get(&key(p)), by_label.get(&label)) {
            (Some(&i), _) => Placement::Same(i),
            (None, Some(&i)) if p.len() == 1 => Placement::Same(i),
            (None, Some(&i)) => Placement::Elsewhere(i),
            (None, None) => Placement::Missing,
        };
        report.items.push((p.clone(), placement));
    }
    let ref_labels: HashSet<String> = reference.iter().filter_map(|p| p.last().map(|l| norm(l))).collect();
    report.ours_only = ours.iter().filter(|it| it.path.last().is_some_and(|l| !ref_labels.contains(&norm(l)))).map(|it| it.path.clone()).collect();
    report
}

fn pct(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { n as f64 * 100.0 / of as f64 }
}

fn show(p: &[String]) -> String {
    p.join(" ▸ ")
}

impl Report {
    fn counts(&self) -> (usize, usize, usize) {
        let same = self.items.iter().filter(|(_, pl)| matches!(pl, Placement::Same(_))).count();
        let elsewhere = self.items.iter().filter(|(_, pl)| matches!(pl, Placement::Elsewhere(_))).count();
        (same, elsewhere, self.items.len().saturating_sub(same + elsewhere))
    }

    /// Reference items whose label we have anywhere, in percent of the items in scope.
    fn presence(&self) -> f64 {
        let (same, elsewhere, _) = self.counts();
        pct(same + elsewhere, self.items.len())
    }

    fn summary(&self) -> String {
        let (same, elsewhere, missing) = self.counts();
        let n = self.items.len();
        format!(
            "{}: presence {}/{n} ({:.1}%), same place {same}/{n} ({:.1}%), elsewhere {elsewhere}, missing {missing}, out of scope {}",
            self.area.name(),
            same + elsewhere,
            self.presence(),
            pct(same, n),
            self.out_of_scope.values().sum::<usize>()
        )
    }

    fn markdown(&self, reference: &str, ours: usize) -> String {
        let (same, elsewhere, missing) = self.counts();
        let n = self.items.len();
        let area = self.area;
        let (title, column) = match area {
            Area::Menus => ("Menu", "Menu"),
            Area::Commands => ("Command", "Menu"),
            Area::Effects => ("Effect", "Folder"),
            Area::Transitions => ("Transition", "Folder"),
        };
        let mut s = format!("# {title} parity checklist\n\n");
        s.push_str(&format!(
            "Generated by `cargo xtask parity --area {}` from `{reference}` ({n} items in scope, {} out of scope) against \
             CFA4's {} ({ours} items). *Same place*: we have the label at the same {place}; *elsewhere*: under \
             another {place}; *missing*: nowhere. A label match is presence only, not behaviour.{}\n\n",
            area.name(),
            self.out_of_scope.values().sum::<usize>(),
            area.ours(),
            match area {
                Area::Menus => "",
                Area::Commands => " A bare name with no menu in the reference is matched by label alone; queries are left out.",
                Area::Effects | Area::Transitions => " A bare name with no folder in the reference is matched by name alone.",
            },
            place = area.place(),
        ));
        s.push_str(&format!("| {column} | Items | Same place | Elsewhere | Missing | Presence |\n|---|---|---|---|---|---|\n"));
        let mut per: Vec<(String, [usize; 3])> = Vec::new();
        for (p, pl) in &self.items {
            let top = area.group(p);
            let i = match per.iter().position(|(t, _)| *t == top) {
                Some(i) => i,
                None => {
                    per.push((top, [0; 3]));
                    per.len() - 1
                }
            };
            let col = match pl {
                Placement::Same(_) => 0,
                Placement::Elsewhere(_) => 1,
                Placement::Missing => 2,
            };
            if let Some(c) = per.get_mut(i).and_then(|(_, c)| c.get_mut(col)) {
                *c += 1;
            }
        }
        for (top, [a, b, c]) in &per {
            let total = a + b + c;
            s.push_str(&format!("| {top} | {total} | {a} | {b} | {c} | {:.1}% |\n", pct(a + b, total)));
        }
        s.push_str(&format!("| **Total** | **{n}** | **{same}** | **{elsewhere}** | **{missing}** | **{:.1}%** |\n", self.presence()));
        s.push_str(&format!("\n## Missing ({missing})\n\n"));
        for (p, _) in self.items.iter().filter(|(_, pl)| matches!(pl, Placement::Missing)) {
            s.push_str(&format!("- [ ] {}\n", show(p)));
        }
        s.push_str(&format!("\n## Elsewhere in CFA4 ({elsewhere})\n\n"));
        for (p, pl) in &self.items {
            if let Placement::Elsewhere(i) = pl
                && let Some(at) = self.ours.get(*i)
            {
                s.push_str(&format!("- {} → {}\n", show(p), show(&at.path)));
            }
        }
        s.push_str("\n## Out of scope\n\n");
        for (why, k) in &self.out_of_scope {
            s.push_str(&format!("- {why}: {k}\n"));
        }
        s.push_str(&format!("\n## CFA4 only ({})\n\n", self.ours_only.len()));
        for p in &self.ours_only {
            s.push_str(&format!("- {}\n", show(p)));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paths(v: &[&[&str]]) -> Vec<Vec<String>> {
        v.iter().map(|p| p.iter().map(|s| s.to_string()).collect()).collect()
    }

    fn items(v: &[&[&str]]) -> Vec<OurItem> {
        paths(v).into_iter().map(|path| OurItem { path, id: String::new() }).collect()
    }

    fn item_paths(v: &[OurItem]) -> Vec<Vec<String>> {
        v.iter().map(|it| it.path.clone()).collect()
    }

    #[test]
    fn reference_formats_give_the_same_paths() {
        let want = paths(&[&["File", "New", "Sequence…"], &["File", "Save"], &["Edit", "Undo"]]);
        let forms = [
            json!([["File", "New", "Sequence…"], ["File", "Save"], ["Edit", "Undo"]]),
            json!(["File > New > Sequence…", "File ▸ Save", "Edit > Undo"]),
            json!({"menus": [
                {"name": "File", "children": [
                    {"name": "New", "items": [{"name": "Sequence…", "shortcut": "Cmd+N"}]},
                    {"name": "missing value"},
                    {"title": "Save"}
                ]},
                {"name": "Edit", "submenu": [{"label": "Undo"}]}
            ]}),
            json!({"Edit": {"Undo": "Cmd+Z"}, "File": {"New": ["Sequence…"], "-": null, "Save": null}}),
        ];
        for f in forms {
            let mut got = reference_paths(&f, Area::Menus).unwrap();
            got.sort();
            let mut w = want.clone();
            w.sort();
            assert_eq!(got, w, "{f}");
        }
    }

    #[test]
    fn hostile_reference_is_refused_not_overflowed() {
        let mut v = json!("leaf");
        for _ in 0..200 {
            v = json!({ "name": "m", "children": [v] });
        }
        for area in [Area::Menus, Area::Effects] {
            assert!(reference_paths(&v, area).unwrap_err().contains("nested deeper"));
            let many: Vec<String> = (0..MAX_ITEMS + 1).map(|i| format!("File > Item {i}")).collect();
            assert!(reference_paths(&json!(many), area).unwrap_err().contains("more than"));
            let bare: Vec<String> = (0..MAX_ITEMS + 1).map(|i| format!("Effect {i}")).collect();
            assert!(area == Area::Menus || reference_paths(&json!(bare), area).unwrap_err().contains("more than"));
            let long = format!("File > {}", "x".repeat(MAX_PATH_BYTES));
            assert!(reference_paths(&json!([long]), area).unwrap_err().contains("bytes"));
        }
        // odd values are skipped, not trusted
        assert_eq!(reference_paths(&json!([1, null, true, {"name": 5}, ["File"]]), Area::Menus).unwrap(), Vec::<Vec<String>>::new());
    }

    #[test]
    fn presence_and_placement_are_reported_separately() {
        let reference = paths(&[
            &["File", "New", "Sequence..."],
            &["File", "Export Frame"],
            &["Edit", "Find & Replace"],
            &["Edit", "Spelling"],
            &["Help", "Premiere Pro Help…"],
            &["File", "Open Recent", "clip.prproj"],
            &["Edit", "AutoFill"],
            &["Edit", "Spelling"],
        ]);
        let ours = items(&[&["File", "New", "Sequence…"], &["File", "Export", "Export Frame"], &["Edit", "Find and Replace…"], &["Clip", "Nest…"]]);
        let r = compare(Area::Menus, &reference, &ours);
        assert_eq!(r.counts(), (2, 1, 1), "duplicates count once; out-of-scope items are not counted");
        assert!((r.presence() - 75.0).abs() < 1e-9);
        assert_eq!(r.out_of_scope.values().sum::<usize>(), 3);
        assert_eq!(r.ours_only, paths(&[&["Clip", "Nest…"]]));
        let md = r.markdown("menus.json", ours.len());
        assert!(md.contains("| **Total** | **4** | **2** | **1** | **1** | **75.0%** |"), "{md}");
        assert!(md.contains("- [ ] Edit ▸ Spelling"));
        assert!(md.contains("- File ▸ Export Frame → File ▸ Export ▸ Export Frame"));
        assert!(r.summary().contains("presence 3/4 (75.0%), same place 2/4 (50.0%)"));
    }

    #[test]
    fn empty_reference_measures_zero_without_dividing_by_zero() {
        let r = compare(Area::Menus, &[], &items(&[&["File", "Save"]]));
        assert_eq!(r.presence(), 0.0);
        assert!(r.summary().contains("presence 0/0 (0.0%)"));
    }

    #[test]
    fn our_dump_and_arguments_are_checked() {
        let ours = our_items(&json!([{"id": "file.save", "label": "Save", "path": ["File"]}, {"id": "x", "label": "Orphan", "path": []}, {"bad": 1}])).unwrap();
        assert_eq!(ours, vec![OurItem { path: paths(&[&["File", "Save"]]).remove(0), id: "file.save".into() }]);
        assert!(our_items(&json!({})).is_err());
        assert!(parse_args(&["--min", "101"]).is_err());
        assert!(parse_args(&["--min", "x"]).is_err());
        assert!(parse_args(&["--out"]).is_err());
        assert!(parse_args(&["--bogus"]).is_err());
        assert!(parse_args(&["--min", "90", "--reference", "r.json"]).is_ok_and(|a| a.min == Some(90.0) && a.reference.is_some() && a.area == Area::Menus));
        assert!(parse_args(&["--area", "transitions"]).is_ok_and(|a| a.area == Area::Transitions));
        assert!(parse_args(&["--area", "panels"]).is_err());
        assert!(parse_args(&["--area"]).is_err());
    }

    fn catalog() -> Value {
        json!([
            {"id": "gaussian_blur", "name": "Gaussian Blur", "kind": "Video", "path": ["Video Effects", "Blur & Sharpen"]},
            {"id": "mosaic", "name": "Mosaic", "kind": "Video", "path": ["Video Effects", "Stylize"]},
            {"id": "volume_x", "name": "Hard Limiter", "kind": "Audio", "path": ["Audio Effects", "Amplitude and Compression"]},
            {"id": "cross_dissolve", "name": "Cross Dissolve", "kind": "VideoTransition", "path": ["Video Transitions", "Dissolve"]},
            {"id": "push_legacy", "name": "Push (Legacy)", "kind": "VideoTransition", "path": ["Legacy", "Video Transitions"]},
            {"id": "constant_power", "name": "Constant Power", "kind": "AudioTransition", "path": ["Audio Transitions", "Crossfade"]},
            {"bad": true},
            {"name": "No kind", "path": []}
        ])
    }

    #[test]
    fn effect_catalog_splits_into_effects_and_transitions() {
        let fx = our_catalog(&catalog(), Area::Effects).unwrap();
        assert_eq!(
            item_paths(&fx),
            paths(&[
                &["Video Effects", "Blur & Sharpen", "Gaussian Blur"],
                &["Video Effects", "Stylize", "Mosaic"],
                &["Audio Effects", "Amplitude and Compression", "Hard Limiter"]
            ])
        );
        let tr = our_catalog(&catalog(), Area::Transitions).unwrap();
        assert_eq!(tr.len(), 3);
        assert!(item_paths(&tr).contains(&paths(&[&["Legacy", "Video Transitions", "Push (Legacy)"]])[0]));
        assert!(our_catalog(&json!({"effects": []}), Area::Effects).is_err());
        let inv = inventory(Area::Effects, &fx);
        assert!(inv.starts_with("CFA4 Effects panel effects: 3 items"), "{inv}");
        assert!(inv.contains("Video Effects ▸ Stylize"), "{inv}");
    }

    #[test]
    fn effects_reference_of_bare_names_and_folders_is_measured() {
        // one reference of the whole Effects panel: folders, bare names, presets, both areas
        let reference = json!({"effects": {
            "Presets": {"Bevel Edges": ["Bevel Edges Thick"]},
            "Video Effects": {"Blur & Sharpen": ["Gaussian Blur", "Camera Blur"], "Distort": ["Mosaic"]},
            "Video Transitions": {"Dissolve": ["Cross Dissolve", "Film Dissolve"]},
            "Audio Transitions": {"Crossfade": ["Constant Power"]}
        }});
        let bare = json!(["Hard Limiter", "Echo"]);
        let mut fx_ref = reference_paths(&reference, Area::Effects).unwrap();
        assert_eq!(fx_ref.len(), 7);
        fx_ref.extend(reference_paths(&bare, Area::Effects).unwrap());
        // menus need a menu: bare names are not menu items
        assert!(reference_paths(&bare, Area::Menus).unwrap().is_empty());
        let ours = our_catalog(&catalog(), Area::Effects).unwrap();
        let r = compare(Area::Effects, &fx_ref, &ours);
        // Gaussian Blur same; Mosaic elsewhere (Stylize); Hard Limiter by bare name; Camera Blur, Echo missing
        assert_eq!(r.counts(), (2, 1, 2), "{:?}", r.summary());
        assert_eq!(r.out_of_scope.get("presets"), Some(&1));
        assert_eq!(r.out_of_scope.get("transitions (--area transitions)"), Some(&3));
        let md = r.markdown("effects.json", ours.len());
        assert!(md.starts_with("# Effect parity checklist"), "{md}");
        assert!(md.contains("| Folder | Items |"), "{md}");
        assert!(md.contains("| Video Effects ▸ Blur & Sharpen | 2 | 1 | 0 | 1 | 50.0% |"), "{md}");
        assert!(md.contains("| (no folder) | 2 | 1 | 0 | 1 | 50.0% |"), "{md}");
        assert!(md.contains("- Video Effects ▸ Distort ▸ Mosaic → Video Effects ▸ Stylize ▸ Mosaic"), "{md}");
        assert!(md.contains("- [ ] Echo"), "{md}");
        assert!(r.summary().starts_with("effects: presence 3/5 (60.0%)"), "{}", r.summary());

        let tr_ref = reference_paths(&reference, Area::Transitions).unwrap();
        let tr = compare(Area::Transitions, &tr_ref, &our_catalog(&catalog(), Area::Transitions).unwrap());
        assert_eq!(tr.counts(), (2, 0, 1), "{}", tr.summary());
        assert_eq!(tr.out_of_scope.get("effects (--area effects)"), Some(&3));
        assert!(tr.markdown("effects.json", 3).contains("- [ ] Video Transitions ▸ Dissolve ▸ Film Dissolve"));
    }

    #[test]
    fn compare_points_at_our_item_for_same_and_elsewhere() {
        let ours = vec![
            OurItem { path: paths(&[&["File", "Save"]]).remove(0), id: "file.save".into() },
            OurItem { path: paths(&[&["File", "Export", "Export Frame"]]).remove(0), id: "export.frame".into() },
        ];
        let r = compare(Area::Menus, &paths(&[&["File", "Save"], &["File", "Export Frame"], &["Edit", "Undo"]]), &ours);
        let ids: Vec<Option<&str>> = r
            .items
            .iter()
            .map(|(_, pl)| match pl {
                Placement::Same(i) | Placement::Elsewhere(i) => r.ours.get(*i).map(|it| it.id.as_str()),
                Placement::Missing => None,
            })
            .collect();
        assert_eq!(ids, vec![Some("file.save"), Some("export.frame"), None]);
    }

    #[test]
    fn command_dump_skips_queries_and_keeps_menuless_commands() {
        let dump = json!([
            {"id": "timeline.razor", "label": "Razor", "menu": ["Sequence"], "journal": true},
            {"id": "clip.speed", "label": "Speed/Duration…", "menu": [], "journal": true},
            {"id": "command.list", "label": "List commands", "menu": [], "journal": false},
            {"id": "x", "label": "  ", "menu": [], "journal": true},
            {"id": "y", "label": "No journal flag", "menu": []},
            {"bad": 1}
        ]);
        let ours = our_commands(&dump).unwrap();
        assert_eq!(item_paths(&ours), paths(&[&["Sequence", "Razor"], &["Speed/Duration…"]]));
        assert_eq!(ours.first().map(|it| it.id.as_str()), Some("timeline.razor"));
        assert!(our_commands(&json!({"commands": []})).is_err());
        let inv = inventory(Area::Commands, &ours);
        assert!(inv.starts_with("CFA4 engine commands: 2 items"), "{inv}");
        assert!(inv.contains("(no menu)"), "{inv}");
    }

    #[test]
    fn commands_reference_of_menu_paths_and_bare_names_is_measured() {
        let ours = our_commands(&json!([
            {"id": "timeline.razor", "label": "Razor", "menu": ["Sequence"], "journal": true},
            {"id": "clip.nest", "label": "Nest…", "menu": ["Clip"], "journal": true},
            {"id": "clip.speed", "label": "Speed/Duration…", "menu": [], "journal": true},
            {"id": "edit.ripple", "label": "Ripple Delete", "menu": [], "journal": true}
        ]))
        .unwrap();
        let reference =
            reference_paths(&json!({"commands": ["Sequence > Razor", "Sequence > Nest", "Speed/Duration", "Match Frame"]}), Area::Commands).unwrap();
        assert_eq!(reference.len(), 4);
        let r = compare(Area::Commands, &reference, &ours);
        // Razor same; Nest elsewhere (Clip); Speed/Duration by bare name; Match Frame missing
        assert_eq!(r.counts(), (2, 1, 1), "{}", r.summary());
        assert_eq!(r.ours_only, paths(&[&["Ripple Delete"]]));
        let md = r.markdown("commands.json", ours.len());
        assert!(md.starts_with("# Command parity checklist"), "{md}");
        assert!(md.contains("| Sequence | 2 | 1 | 1 | 0 | 100.0% |"), "{md}");
        assert!(md.contains("| (no menu) | 2 | 1 | 0 | 1 | 50.0% |"), "{md}");
        assert!(md.contains("- Sequence ▸ Nest → Clip ▸ Nest…"), "{md}");
        assert!(r.summary().starts_with("commands: presence 3/4 (75.0%)"), "{}", r.summary());
        assert!(parse_args(&["--area", "commands"]).is_ok_and(|a| a.area == Area::Commands));
    }
}
