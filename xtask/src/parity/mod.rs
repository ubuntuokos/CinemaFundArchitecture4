//! `cargo xtask parity [--area menus|commands|panels|effects|transitions|menus-egui|all]
//! [--reference FILE | --committed] [--ours FILE] [--out FILE] [--min PERCENT] [--min-exact COUNT]
//! [--fidelity FILE]`: measured parity (docs/gaps.md G7);
//! `cargo xtask parity --check-reference` checks the committed references (see [`schema`]).
//!
//! Compares one area of what CFA4 ships with a reference list, and reports presence and
//! placement separately: a reference item is *same place* when we have its label at the same path,
//! *elsewhere* when we have the label under another path, and *missing* otherwise. A label match is
//! presence only, not behaviour. The areas:
//!
//! - `menus` (default): the menu bar of the Qt 6 GUI, read from the live window by
//!   `scripts/dev/qt_ui_inventory.py --menus` (run with `.venv/bin/python`; needs a built
//!   `cfa4-cli`); paths are menu paths. App, Apple and Help menus, recent-file lists, account /
//!   cloud items and macOS system items are counted out of scope.
//! - `menus-egui`: the same measurement of the frozen egui menu bar
//!   (`crates/ui-egui/examples/menu_tree.rs`), against the same reference (`menus.json`).
//! - `panels`: the panels of the Qt 6 GUI (`qt_ui_inventory.py --panels`): the docks, the central
//!   Program monitor and the panels the Window menu lists. A panel is matched by its title. A
//!   panel that is only a placeholder dock, or only listed in the Window menu with no dock, is an
//!   automatic *stub*: present, but not the real panel; the report lists the stubs.
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
//! `--ours` reads a saved dump of our side instead of running the example or the Qt dump. The reference defaults
//! to the maintainers' local snapshot `plan/premiere/<area>.json` (gitignored, never committed).
//! Without one the tool prints our own inventory and measures nothing. The reference may be any
//! JSON of paths: an array of paths (`["File", "New", "Sequence…"]`) or of
//! `"File > New > Sequence…"` strings, a tree of nodes (`{"name": …, "children" | "items" |
//! "submenu": […]}`), or nested objects keyed by label.
//!
//! The reference is `--reference FILE`, else the local `plan/premiere/<area>.json`, else the committed
//! `xtask/parity/reference/<area>.json` (schema 1, see [`schema`]); `--committed` uses only the
//! committed one. The report names which it used. A report made from a reference that is not
//! committed can only be written under `target/` (or outside the repository): it holds a local
//! snapshot's contents.
//!
//! `--area all` measures menus, commands, panels, effects and transitions in one report
//! (`target/parity/parity-all.md`, a summary table first); it runs every area and lists every
//! failure at the end. Each present item also has a fidelity tier ([`fidelity`]): exact,
//! approximate, stub or unrated (the default; exact needs evidence). The tier is not a behaviour test.
//!
//! The report goes to `target/parity/parity-checklist.md` for menus and
//! `target/parity/parity-<area>.md` for the other areas (or `--out`); `--min` fails when presence
//! is below the given percentage, `--min-exact` when fewer than COUNT items are exact.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

mod fidelity;
mod reference;
mod report;
mod schema;
#[cfg(test)]
mod tests;

use fidelity::Tier;
use reference::*;
use report::*;
use schema::*;

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
const PANEL_WRAPPER_KEYS: [&str; 2] = ["panels", "items"];
/// The Qt 6 GUI dump script, from the repository root.
const QT_DUMP: &str = "scripts/dev/qt_ui_inventory.py";

/// What a parity run measures.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Area {
    Menus,
    MenusEgui,
    Commands,
    Panels,
    Effects,
    Transitions,
}

/// One item on our side: its full path (menu path or folders, then the label), its id, and its
/// fidelity tier with the evidence or note behind it (unrated unless annotated; a panel with no
/// real content yet is an automatic stub).
#[derive(Clone, Debug, Default, PartialEq)]
struct OurItem {
    path: Vec<String>,
    id: String,
    tier: Tier,
    evidence: String,
    note: String,
}

impl Area {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "menus" => Ok(Area::Menus),
            "menus-egui" => Ok(Area::MenusEgui),
            "commands" => Ok(Area::Commands),
            "panels" => Ok(Area::Panels),
            "effects" => Ok(Area::Effects),
            "transitions" => Ok(Area::Transitions),
            other => Err(format!("--area {other}: expected menus, commands, panels, effects, transitions, menus-egui or all")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Area::Menus => "menus",
            Area::MenusEgui => "menus-egui",
            Area::Commands => "commands",
            Area::Panels => "panels",
            Area::Effects => "effects",
            Area::Transitions => "transitions",
        }
    }

    /// The reference file name in `plan/premiere/`: the egui menu bar is measured against the
    /// same menu reference as the Qt one.
    fn reference_name(self) -> &'static str {
        match self {
            Area::MenusEgui => Area::Menus.name(),
            other => other.name(),
        }
    }

    /// Our side, as the report names it.
    fn ours(self) -> &'static str {
        match self {
            Area::Menus => "Qt 6 GUI menu bar",
            Area::MenusEgui => "frozen egui menu bar",
            Area::Commands => "engine commands",
            Area::Panels => "Qt 6 GUI panels",
            Area::Effects => "Effects panel effects",
            Area::Transitions => "Effects panel transitions",
        }
    }

    /// What a path is made of: "menu path" or "folder".
    fn place(self) -> &'static str {
        match self {
            Area::Menus | Area::MenusEgui | Area::Commands => "menu path",
            Area::Panels => "group",
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
            Area::Menus | Area::MenusEgui => 2,
            Area::Commands | Area::Panels | Area::Effects | Area::Transitions => 1,
        }
    }

    fn wrapper_keys(self) -> &'static [&'static str] {
        match self {
            Area::Menus | Area::MenusEgui => &MENU_WRAPPER_KEYS,
            Area::Commands => &COMMAND_WRAPPER_KEYS,
            Area::Panels => &PANEL_WRAPPER_KEYS,
            Area::Effects | Area::Transitions => &EFFECT_WRAPPER_KEYS,
        }
    }

    /// The effect kinds (`EffectKind` names in the catalog dump) this area counts.
    fn kinds(self) -> &'static [&'static str] {
        match self {
            Area::Menus | Area::MenusEgui | Area::Commands | Area::Panels => &[],
            Area::Effects => &["Video", "Audio"],
            Area::Transitions => &["VideoTransition", "AudioTransition"],
        }
    }

    /// The group a path is tallied under in the report: the top-level menu, or the folder path.
    fn group(self, path: &[String]) -> String {
        match self {
            Area::Menus | Area::MenusEgui => path.first().cloned().unwrap_or_default(),
            Area::Panels => match path.split_last() {
                Some((_, groups)) if !groups.is_empty() => show(groups),
                _ => "Panels".into(),
            },
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

#[derive(Debug)]
struct Args {
    check_reference: bool,
    /// `--area all`: every area of [`ALL_AREAS`] in one report.
    all: bool,
    /// `--committed`: only the committed references count.
    committed: bool,
    area: Area,
    reference: Option<PathBuf>,
    ours: Option<PathBuf>,
    out: Option<PathBuf>,
    min: Option<f64>,
    min_exact: Option<usize>,
    fidelity: Option<PathBuf>,
}

fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut a = Args {
        check_reference: false,
        all: false,
        committed: false,
        area: Area::Menus,
        reference: None,
        ours: None,
        out: None,
        min: None,
        min_exact: None,
        fidelity: None,
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().map(|v| v.to_string()).ok_or_else(|| format!("{flag} needs a value"));
        match *flag {
            "--check-reference" => a.check_reference = true,
            "--committed" => a.committed = true,
            "--area" => match value()?.as_str() {
                "all" => a.all = true,
                other => {
                    a.all = false;
                    a.area = Area::parse(other)?;
                }
            },
            "--reference" => a.reference = Some(value()?.into()),
            "--ours" => a.ours = Some(value()?.into()),
            "--out" => a.out = Some(value()?.into()),
            "--fidelity" => a.fidelity = Some(value()?.into()),
            "--min-exact" => {
                let v = value()?;
                a.min_exact = Some(v.parse().map_err(|_| format!("--min-exact {v}: not a count"))?);
            }
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
                    "parity: unknown argument {other} (usage: cargo xtask parity [--area menus|commands|panels|effects|transitions|menus-egui|all] [--reference FILE | --committed] [--ours FILE] [--out FILE] [--min PERCENT] [--min-exact COUNT] [--fidelity FILE] | --check-reference)"
                ));
            }
        }
    }
    Ok(a)
}

/// The areas `--area all` measures (the frozen egui menu bar is not one of them).
const ALL_AREAS: [Area; 5] = [Area::Menus, Area::Commands, Area::Panels, Area::Effects, Area::Transitions];

/// Where a reference came from. Only a committed one may be written into the repository.
#[derive(Clone, Debug, PartialEq)]
enum RefSource {
    /// `--reference FILE`.
    Explicit(PathBuf),
    /// The maintainers' local snapshot `plan/premiere/<area>.json` (gitignored).
    Local(PathBuf),
    /// `xtask/parity/reference/<area>.json`.
    Committed(PathBuf),
}

impl RefSource {
    fn path(&self) -> &Path {
        match self {
            RefSource::Explicit(p) | RefSource::Local(p) | RefSource::Committed(p) => p,
        }
    }

    fn is_committed(&self) -> bool {
        matches!(self, RefSource::Committed(_))
    }

    /// How the report names the reference.
    fn label(&self, root: &Path) -> String {
        let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
        match self {
            RefSource::Explicit(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            RefSource::Local(p) => format!("{} (local snapshot, not committed)", rel(p)),
            RefSource::Committed(p) => format!("{} (committed)", rel(p)),
        }
    }
}

/// The reference for `area`: `--reference FILE`, else the local `plan/premiere/<area>.json`, else the
/// committed `xtask/parity/reference/<area>.json`; with `committed_only` just the committed one.
fn resolve_reference(root: &Path, area: Area, explicit: Option<&Path>, committed_only: bool) -> Option<RefSource> {
    if let Some(p) = explicit {
        return Some(RefSource::Explicit(p.to_path_buf()));
    }
    let name = format!("{}.json", area.reference_name());
    let local = root.join("plan/premiere").join(&name);
    if !committed_only && local.exists() {
        return Some(RefSource::Local(local));
    }
    let committed = root.join(REFERENCE_DIR).join(&name);
    committed.exists().then_some(RefSource::Committed(committed))
}

/// `p` made absolute against the working directory, with `.` and `..` resolved lexically.
fn lexical_absolute(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A report made from a reference that is not committed may only be written under `target/`: it
/// holds the contents of a local snapshot, which must never reach a commit (AGENTS.md §1.5).
fn check_out(root: &Path, out: &Path, source: &RefSource) -> Result<(), String> {
    if source.is_committed() {
        return Ok(());
    }
    let (abs, repo, target) = (lexical_absolute(out), lexical_absolute(root), lexical_absolute(&crate::target_dir()));
    if abs.starts_with(&repo) && !abs.starts_with(&target) {
        return Err(format!(
            "{}: a report made from a reference that is not committed ({}) can only be written under {} (or outside the repository)",
            out.display(),
            source.path().display(),
            crate::target_dir().display()
        ));
    }
    Ok(())
}

/// What one area's measurement produced.
struct Outcome {
    area: Area,
    ours: usize,
    inventory: String,
    /// The report and the reference it was made from, if the area has one.
    measured: Option<(Report, RefSource)>,
}

/// Dumps our side of `area`, applies the fidelity annotations and compares it with its reference.
fn measure(root: &Path, args: &Args, area: Area, fidelity: &fidelity::Fidelity, explicit_ours: Option<&Path>) -> Result<Outcome, String> {
    let ours_json = match explicit_ours {
        Some(p) => read_bounded(p)?,
        None => dump_ours(root, area)?,
    };
    let ours_value: Value = serde_json::from_str(&ours_json).map_err(|e| format!("our {} dump: {e}", area.name()))?;
    let mut ours = match area {
        Area::Menus | Area::MenusEgui => our_items(&ours_value)?,
        Area::Commands => our_commands(&ours_value)?,
        Area::Panels => our_panels(&ours_value)?,
        Area::Effects | Area::Transitions => our_catalog(&ours_value, area)?,
    };
    fidelity.apply(area, &mut ours)?;
    let inventory = inventory(area, &ours);
    let explicit = if args.all { None } else { args.reference.as_deref() };
    let Some(source) = resolve_reference(root, area, explicit, args.committed) else {
        return Ok(Outcome { area, ours: ours.len(), inventory, measured: None });
    };
    let text = read_bounded(source.path())?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", source.path().display()))?;
    let (paths, extra_out) = if is_v1(&value) {
        let v1 = parse_v1(&value, area).map_err(|e| format!("{}: {e}", source.path().display()))?;
        (v1.paths(), v1.out_of_scope)
    } else {
        (reference_paths(&value, area)?, Vec::new())
    };
    let report = compare_scoped(area, &paths, &ours, &extra_out);
    Ok(Outcome { area, ours: ours.len(), inventory, measured: Some((report, source)) })
}

/// The `--min` / `--min-exact` failures of one outcome.
fn threshold_failures(args: &Args, o: &Outcome) -> Vec<String> {
    let name = o.area.name();
    let Some((report, _)) = &o.measured else {
        return match (args.min, args.min_exact) {
            (None, None) => Vec::new(),
            _ => vec![format!("{name}: --min and --min-exact need a reference list")],
        };
    };
    let mut failures = Vec::new();
    if let Some(min) = args.min.filter(|m| report.presence() < *m) {
        failures.push(format!("{name} presence {:.1}% is below --min {min}%", report.presence()));
    }
    let exact = report.tier_counts()[Tier::Exact.index()];
    if let Some(min) = args.min_exact.filter(|m| exact < *m) {
        failures.push(format!("{name} has {exact} exact items, below --min-exact {min}"));
    }
    failures
}

fn write_report(root: &Path, out: &Path, source: &RefSource, text: &str) -> Result<(), String> {
    check_out(root, out, source)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let args = parse_args(args)?;
    if args.check_reference {
        println!("{}", check_references(root)?);
        return Ok(());
    }
    if args.all && (args.ours.is_some() || args.reference.is_some()) {
        return Err("--area all measures five areas: --ours and --reference name one file, so they need a single --area".into());
    }
    if args.committed && args.reference.is_some() {
        return Err("--committed and --reference contradict each other".into());
    }
    let fidelity = fidelity::load(root, args.fidelity.as_deref())?;
    let areas: Vec<Area> = if args.all { ALL_AREAS.to_vec() } else { vec![args.area] };
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();
    for area in areas {
        match measure(root, &args, area, &fidelity, args.ours.as_deref()) {
            Ok(o) => outcomes.push(o),
            // a single area stops at its first error; `all` runs the other areas and lists every error at the end
            Err(e) if !args.all => return Err(e),
            Err(e) => failures.push(format!("{}: {e}", area.name())),
        }
    }
    for o in &outcomes {
        match &o.measured {
            Some((report, _)) => println!("{}", report.summary()),
            None => {
                print!("{}", o.inventory);
                println!("\nno reference list for {} ({}): parity not measured", o.area.name(), reference_hint(o.area, args.committed));
            }
        }
        failures.extend(threshold_failures(&args, o));
    }
    if let Some(first) = outcomes.first().filter(|_| !args.all)
        && let Some((report, source)) = &first.measured
    {
        let out = args.out.clone().unwrap_or_else(|| crate::target_dir().join("parity").join(first.area.out_file()));
        write_report(root, &out, source, &report.markdown(&source.label(root), first.ours))?;
        println!("report: {}", out.display());
    } else if args.all && outcomes.iter().any(|o| o.measured.is_some()) {
        let out = args.out.clone().unwrap_or_else(|| crate::target_dir().join("parity").join("parity-all.md"));
        // the strictest source decides: one local or explicit reference keeps the whole report under target/
        let source = outcomes
            .iter()
            .filter_map(|o| o.measured.as_ref().map(|(_, s)| s))
            .find(|s| !s.is_committed())
            .or_else(|| outcomes.iter().find_map(|o| o.measured.as_ref().map(|(_, s)| s)));
        if let Some(source) = source {
            write_report(root, &out, source, &all_markdown(root, &outcomes))?;
            println!("report: {}", out.display());
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
}

/// Where a missing reference would be looked for, for the "not measured" message.
fn reference_hint(area: Area, committed_only: bool) -> String {
    let name = area.reference_name();
    if committed_only { format!("{REFERENCE_DIR}/{name}.json") } else { format!("plan/premiere/{name}.json, {REFERENCE_DIR}/{name}.json, or --reference FILE") }
}

/// How our side is dumped: a cargo example, or the Qt 6 GUI dump script.
#[derive(Debug, PartialEq)]
enum Dump {
    Example { package: &'static str, example: &'static str },
    Qt { flag: &'static str },
}

fn dump_of(area: Area) -> Dump {
    match area {
        Area::Menus => Dump::Qt { flag: "--menus" },
        Area::Panels => Dump::Qt { flag: "--panels" },
        Area::MenusEgui => Dump::Example { package: "cfa4-ui-egui", example: "menu_tree" },
        Area::Commands => Dump::Example { package: "cfa4-engine", example: "command_list" },
        Area::Effects | Area::Transitions => Dump::Example { package: "cfa4-project", example: "effect_catalog" },
    }
}

/// Our side as JSON: the Qt 6 GUI menu bar or panels from the live window (`qt_ui_inventory.py`),
/// the egui menu bar from the `menu_tree` example, the command registry from the `command_list`
/// example, or the Effects panel catalog from the `effect_catalog` example.
fn dump_ours(root: &Path, area: Area) -> Result<String, String> {
    let (mut cmd, what) = match dump_of(area) {
        Dump::Example { package, example } => {
            let mut cmd = Command::new(env!("CARGO"));
            cmd.args(["run", "-q", "-p", package, "--example", example]);
            (cmd, example.to_string())
        }
        Dump::Qt { flag } => {
            let python = root.join(".venv/bin/python");
            if !python.exists() {
                return Err(format!(
                    "{}: missing; set up the Python environment (docs/development/linux.md), or pass a saved dump with --ours FILE",
                    python.display()
                ));
            }
            let mut cmd = Command::new(python);
            cmd.arg(root.join(QT_DUMP)).arg(flag).current_dir(root);
            let engine = crate::target_dir().join("release").join("cfa4-cli");
            if engine.exists() {
                cmd.arg("--engine").arg(engine);
            }
            (cmd, format!("{QT_DUMP} {flag}"))
        }
    };
    cmd.stderr(Stdio::inherit());
    let hint = match dump_of(area) {
        Dump::Qt { .. } => " (needs a built engine: cargo build --release -p cfa4-cli; or pass a saved dump with --ours FILE)",
        Dump::Example { .. } => " (or pass a saved dump with --ours FILE)",
    };
    let out = cmd.output().map_err(|e| format!("{what}: {e}{hint}"))?;
    if !out.status.success() {
        return Err(format!("{what} failed: {}{hint}", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{what}: {e}"))
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
            (path.len() >= 2).then(|| OurItem { path, id: id_of(it), ..OurItem::default() })
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
            Some(OurItem { path, id: id_of(it), ..OurItem::default() })
        })
        .collect())
}

/// Why a panel of the given status (`qt_ui_inventory.py --panels`) is an automatic stub, if it is.
fn panel_stub(status: &str) -> Option<&'static str> {
    match status {
        "placeholder" => Some("placeholder dock: no content yet"),
        "listed" => Some("not yet in the Qt GUI: listed in the Window menu, no dock"),
        _ => None,
    }
}

/// Our panels (by title), from the `qt_ui_inventory.py --panels` dump; placeholder and
/// listed-only panels are automatic stubs.
fn our_panels(v: &Value) -> Result<Vec<OurItem>, String> {
    let items = v.as_array().ok_or("our panel dump is not an array")?;
    Ok(items
        .iter()
        .filter_map(|it| {
            let title = it.get("title")?.as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            let status = it.get("status").and_then(Value::as_str).unwrap_or_default();
            let stub = panel_stub(status);
            Some(OurItem {
                path: vec![title.to_string()],
                id: title.to_string(),
                tier: if stub.is_some() { Tier::Stub } else { Tier::Unrated },
                note: stub.unwrap_or_default().to_string(),
                ..OurItem::default()
            })
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
            Some(OurItem { path, id: id_of(it), ..OurItem::default() })
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
    let stubs: Vec<&OurItem> = ours.iter().filter(|it| it.tier == Tier::Stub).collect();
    let width = per.keys().map(|g| g.chars().count()).max().unwrap_or(0).max(22);
    for (group, n) in per {
        s.push_str(&format!("  {group:<width$} {n}\n"));
    }
    for it in stubs {
        s.push_str(&format!("  stub: {} ({})\n", show(&it.path), it.note));
    }
    s
}
