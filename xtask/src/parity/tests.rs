use super::*;
use serde_json::json;

fn paths(v: &[&[&str]]) -> Vec<Vec<String>> {
    v.iter().map(|p| p.iter().map(|s| s.to_string()).collect()).collect()
}

fn items(v: &[&[&str]]) -> Vec<OurItem> {
    paths(v).into_iter().map(|path| OurItem { path, ..OurItem::default() }).collect()
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
    assert_eq!(ours, vec![OurItem { path: paths(&[&["File", "Save"]]).remove(0), id: "file.save".into(), ..OurItem::default() }]);
    assert!(our_items(&json!({})).is_err());
    assert!(parse_args(&["--min", "101"]).is_err());
    assert!(parse_args(&["--min", "x"]).is_err());
    assert!(parse_args(&["--out"]).is_err());
    assert!(parse_args(&["--bogus"]).is_err());
    assert!(parse_args(&["--min", "90", "--reference", "r.json"]).is_ok_and(|a| a.min == Some(90.0) && a.reference.is_some() && a.area == Area::Menus));
    assert!(parse_args(&["--area", "transitions"]).is_ok_and(|a| a.area == Area::Transitions));
    assert!(parse_args(&["--area", "windows"]).is_err());
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
        OurItem { path: paths(&[&["File", "Save"]]).remove(0), id: "file.save".into(), ..OurItem::default() },
        OurItem { path: paths(&[&["File", "Export", "Export Frame"]]).remove(0), id: "export.frame".into(), ..OurItem::default() },
    ];
    let r = compare(Area::Menus, &paths(&[&["File", "Save"], &["File", "Export Frame"], &["Edit", "Undo"]]), &ours);
    let ids: Vec<Option<&str>> = r.items.iter().map(|(_, pl)| r.found(pl).map(|it| it.id.as_str())).collect();
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
    let reference = reference_paths(&json!({"commands": ["Sequence > Razor", "Sequence > Nest", "Speed/Duration", "Match Frame"]}), Area::Commands).unwrap();
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

fn panel_dump() -> Value {
    json!([
        {"title": "Program", "status": "central"},
        {"title": "Timeline", "status": "docked"},
        {"title": "Effect Controls", "status": "docked"},
        {"title": "Libraries", "status": "placeholder"},
        {"title": "Reference Monitor", "status": "listed"},
        {"title": "  ", "status": "docked"},
        {"status": "docked"}
    ])
}

#[test]
fn panels_status_maps_to_stub() {
    let ours = our_panels(&panel_dump()).unwrap();
    let stubs: Vec<(&str, bool)> = ours.iter().map(|it| (it.id.as_str(), it.tier == Tier::Stub)).collect();
    assert_eq!(stubs, vec![("Program", false), ("Timeline", false), ("Effect Controls", false), ("Libraries", true), ("Reference Monitor", true)]);
    assert_eq!(ours.get(3).map(|it| it.note.as_str()), Some("placeholder dock: no content yet"));
    assert!(our_panels(&json!({"panels": []})).is_err());
    let inv = inventory(Area::Panels, &ours);
    assert!(inv.starts_with("CFA4 Qt 6 GUI panels: 5 items"), "{inv}");
    assert!(inv.contains("stub: Libraries (placeholder dock: no content yet)"), "{inv}");
}

#[test]
fn panels_reference_bare_names() {
    let reference = reference_paths(&json!({"panels": ["Program", "Timeline", "Libraries", "Reference Monitor", "Essential Graphics"]}), Area::Panels).unwrap();
    assert_eq!(reference.len(), 5);
    let r = compare(Area::Panels, &reference, &our_panels(&panel_dump()).unwrap());
    // bare names have no place to compare: all found are "same"; stubs still count as present
    assert_eq!(r.counts(), (4, 0, 1), "{}", r.summary());
    assert!(
        r.summary()
            .starts_with("panels: presence 4/5 (80.0%), same place 4/5 (80.0%), elsewhere 0, missing 1, fidelity exact 0 / approximate 0 / stub 2 / unrated 2"),
        "{}",
        r.summary()
    );
    assert_eq!(r.ours_only, paths(&[&["Effect Controls"]]));
    let md = r.markdown("panels.json", 5);
    assert!(md.starts_with("# Panel parity checklist"), "{md}");
    assert!(md.contains("| Panels | 5 | 4 | 0 | 1 | 80.0% |"), "{md}");
    assert!(md.contains("### Stub (2)"), "{md}");
    assert!(md.contains("- Reference Monitor → Reference Monitor: not yet in the Qt GUI"), "{md}");
    assert!(md.contains("- [ ] Essential Graphics"), "{md}");
    assert!(parse_args(&["--area", "panels"]).is_ok_and(|a| a.area == Area::Panels));
}

#[test]
fn menus_area_uses_qt_dump() {
    assert_eq!(dump_of(Area::Menus), Dump::Qt { flag: "--menus" });
    assert_eq!(dump_of(Area::Panels), Dump::Qt { flag: "--panels" });
    assert_eq!(dump_of(Area::MenusEgui), Dump::Example { package: "cfa4-ui-egui", example: "menu_tree" });
    // the Qt dump has the menu_tree shape, so it parses the same way
    let ours = our_items(&json!([{"id": "", "label": "Timeline", "path": ["Window"]}, {"id": "app.quit", "label": "Quit", "path": ["File"]}])).unwrap();
    assert_eq!(item_paths(&ours), paths(&[&["Window", "Timeline"], &["File", "Quit"]]));
    let r = compare(Area::Menus, &paths(&[&["Window", "Timeline"], &["Help", "About"]]), &ours);
    assert_eq!(r.counts(), (1, 0, 0), "Help is out of scope: {}", r.summary());
    assert!(r.markdown("menus.json", 2).contains("CFA4's Qt 6 GUI menu bar"));
    // a missing Python environment is an error that names the fix, not an empty list
    let err = dump_ours(Path::new("/nonexistent-cfa4-root"), Area::Menus).unwrap_err();
    assert!(err.contains("docs/development/linux.md") && err.contains("--ours FILE"), "{err}");
}

#[test]
fn menus_egui_still_parses() {
    let a = parse_args(&["--area", "menus-egui"]).unwrap();
    assert_eq!(a.area, Area::MenusEgui);
    assert_eq!(a.area.reference_name(), "menus");
    assert_eq!(a.area.out_file(), "parity-menus-egui.md");
    let ours = our_items(&json!([{"id": "file.save", "label": "Save", "path": ["File"]}])).unwrap();
    let reference = reference_paths(&json!(["File > Save", "Help > About", "Export Frame"]), Area::MenusEgui).unwrap();
    // menu items need a menu: the bare name is dropped; Help is out of scope
    assert_eq!(reference.len(), 2);
    let r = compare(Area::MenusEgui, &reference, &ours);
    assert_eq!(r.counts(), (1, 0, 0));
    assert!(r.summary().starts_with("menus-egui: presence 1/1"), "{}", r.summary());
    assert!(r.markdown("menus.json", 1).contains("frozen egui menu bar"));
}

// ---- schema 1 (G7.5) ----

fn v1_file(area: &str, oos: Value, items: Value) -> Value {
    json!({"schema": 1, "area": area, "outOfScope": oos, "items": items})
}

fn v1_items(rows: &[(&str, &str)]) -> Value {
    json!(rows.iter().map(|(p, s)| json!({"path": p, "source": s})).collect::<Vec<_>>())
}

/// Runs the item checks against in-memory documents; `docs` are (path, text), all tracked.
fn check(area: Area, v: &Value, docs: &[(&str, &str)]) -> Vec<String> {
    let r = parse_v1(v, area).unwrap();
    let tracked = |p: &str| docs.iter().any(|(d, _)| *d == p);
    let mut read = |p: &str| docs.iter().find(|(d, _)| *d == p).map(|(_, t)| t.to_string()).ok_or_else(|| "missing".to_string());
    check_items("menus.json", &r, &tracked, &mut read)
}

#[test]
fn schema_v1_path_source_items() {
    let v = v1_file("menus", json!(["Window ▸ Workspaces"]), v1_items(&[("File ▸ Export ▸ Media", "docs/a.md"), ("Edit ▸ Undo", "docs/a.md")]));
    assert!(is_v1(&v));
    let r = parse_v1(&v, Area::Menus).unwrap();
    assert_eq!(r.paths(), paths(&[&["File", "Export", "Media"], &["Edit", "Undo"]]));
    assert_eq!(r.items.first().map(|i| i.source.as_str()), Some("docs/a.md"));
    assert_eq!(r.out_of_scope, paths(&[&["Window", "Workspaces"]]));
    // egui menus share the menus reference; an effect may be a bare name, a menu item may not
    assert!(parse_v1(&v1_file("menus", json!([]), v1_items(&[("Undo", "docs/a.md")])), Area::MenusEgui).is_err());
    assert!(parse_v1(&v1_file("effects", json!([]), v1_items(&[("Gaussian Blur", "docs/a.md")])), Area::Effects).is_ok());
    assert!(!is_v1(&json!(["File > Save"])));
    assert!(!is_v1(&json!({"menus": []})));
}

#[test]
fn schema_v1_out_of_scope_prefix_is_counted_out() {
    let r = parse_v1(
        &v1_file("menus", json!(["Window ▸ Workspaces"]), v1_items(&[("Window ▸ Workspaces ▸ Editing", "docs/a.md"), ("Window ▸ Timeline", "docs/a.md")])),
        Area::Menus,
    )
    .unwrap();
    let ours = items(&[&["Window", "Timeline"]]);
    let rep = compare_scoped(Area::Menus, &r.paths(), &ours, &r.out_of_scope);
    assert_eq!(rep.counts(), (1, 0, 0));
    assert_eq!(rep.out_of_scope.get("listed out of scope by the reference"), Some(&1));
}

#[test]
fn check_reference_accepts_a_matching_source() {
    let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Export ▸ Media", "docs/a.md")]));
    let ok = check(Area::Menus, &v, &[("docs/a.md", "The File menu has Export, and Export has Media.")]);
    assert!(ok.is_empty(), "{ok:?}");
    // case, ellipsis, "&" and spacing are normalised like the labels
    let v = v1_file("menus", json!([]), v1_items(&[("Edit ▸ Find & Replace…", "docs/a.md")]));
    assert!(check(Area::Menus, &v, &[("docs/a.md", "EDIT: find   and\nreplace")]).is_empty());
}

#[test]
fn check_reference_rejects_name_not_in_source() {
    let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Teleport", "docs/a.md")]));
    let e = check(Area::Menus, &v, &[("docs/a.md", "File only")]);
    assert_eq!(e.len(), 1);
    assert!(e.first().is_some_and(|m| m.contains("\"Teleport\" does not appear in docs/a.md")), "{e:?}");
}

#[test]
fn check_reference_rejects_source_under_plan() {
    let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", "plan/premiere/menus.md")]));
    let e = check(Area::Menus, &v, &[("plan/premiere/menus.md", "File Save")]);
    assert!(e.first().is_some_and(|m| m.contains("under plan/")), "{e:?}");
    for bad in ["docs/../plan/x.md", "/docs/a.md", "docs\\a.md", "docs/a.txt", "other/a.md", "docs//a.md"] {
        let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", bad)]));
        assert_eq!(check(Area::Menus, &v, &[(bad, "File Save")]).len(), 1, "{bad}");
    }
}

#[test]
fn check_reference_rejects_untracked_source() {
    let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", "docs/untracked.md")]));
    let r = parse_v1(&v, Area::Menus).unwrap();
    let e = check_items("menus.json", &r, &|_| false, &mut |_| Ok("File Save".into()));
    assert!(e.first().is_some_and(|m| m.contains("not tracked by git")), "{e:?}");
}

#[test]
fn check_reference_reports_unreadable_source_once() {
    let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", "docs/gone.md"), ("File ▸ Open", "docs/gone.md")]));
    let r = parse_v1(&v, Area::Menus).unwrap();
    let e = check_items("menus.json", &r, &|_| true, &mut |_| Err("gone".into()));
    assert_eq!(e.len(), 1, "{e:?}");
}

#[test]
fn check_reference_rejects_extra_fields() {
    let mut v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", "docs/a.md")]));
    v["note"] = json!("x");
    assert!(parse_v1(&v, Area::Menus).unwrap_err().contains("unknown field"));
    let v = v1_file("menus", json!([]), json!([{"path": "File ▸ Save", "source": "docs/a.md", "shortcut": "Cmd+S"}]));
    assert!(parse_v1(&v, Area::Menus).unwrap_err().contains("unknown field"));
}

#[test]
fn check_reference_rejects_data_uri() {
    for bad in
        ["data:text/plain;base64,QQ==", "see https://example.com/a", "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0NTY3ODk="]
    {
        let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", bad)]));
        assert!(parse_v1(&v, Area::Menus).is_err(), "{bad}");
        let v = v1_file("menus", json!([bad]), v1_items(&[("File ▸ Save", "docs/a.md")]));
        assert!(parse_v1(&v, Area::Menus).is_err(), "{bad}");
        let v = v1_file("effects", json!([]), v1_items(&[(bad, "docs/a.md")]));
        assert!(parse_v1(&v, Area::Effects).is_err(), "{bad}");
    }
}

#[test]
fn check_reference_rejects_non_string() {
    for items in [
        json!([{"path": 5, "source": "docs/a.md"}]),
        json!([{"path": "File ▸ Save", "source": null}]),
        json!(["File ▸ Save"]),
        json!({"path": "x"}),
        json!([{"path": "File ▸ Save"}]),
    ] {
        assert!(parse_v1(&v1_file("menus", json!([]), items.clone()), Area::Menus).is_err(), "{items}");
    }
    assert!(parse_v1(&v1_file("menus", json!([1]), json!([])), Area::Menus).is_err());
    assert!(parse_v1(&v1_file("menus", json!("x"), json!([])), Area::Menus).is_err());
    assert!(parse_v1(&json!({"schema": "1", "area": "menus", "outOfScope": [], "items": []}), Area::Menus).is_err());
    assert!(parse_v1(&json!({"schema": 2, "area": "menus", "outOfScope": [], "items": []}), Area::Menus).is_err());
    assert!(parse_v1(&json!({"schema": 1, "area": 7, "outOfScope": [], "items": []}), Area::Menus).is_err());
    assert!(parse_v1(&json!({"schema": 1, "area": "menus", "items": []}), Area::Menus).is_err());
    assert!(parse_v1(&json!([]), Area::Menus).is_err());
    assert!(parse_v1(&v1_file("panels", json!([]), json!([])), Area::Menus).unwrap_err().contains("expected \"menus\""));
}

#[test]
fn check_reference_rejects_duplicate() {
    let v = v1_file("menus", json!([]), v1_items(&[("File ▸ Save", "docs/a.md"), ("file ▸ save…", "docs/b.md")]));
    assert!(parse_v1(&v, Area::Menus).unwrap_err().contains("listed twice"));
}

#[test]
fn hostile_v1_is_refused_not_overflowed() {
    let many: Vec<(String, String)> = (0..MAX_ITEMS + 1).map(|i| (format!("File ▸ Item {i}"), "docs/a.md".to_string())).collect();
    let rows: Vec<(&str, &str)> = many.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
    assert!(parse_v1(&v1_file("menus", json!([]), v1_items(&rows)), Area::Menus).unwrap_err().contains("more than"));
    let deep = vec!["x"; MAX_DEPTH + 1].join(" ▸ ");
    assert!(parse_v1(&v1_file("menus", json!([]), v1_items(&[(&deep, "docs/a.md")])), Area::Menus).unwrap_err().contains("deeper"));
    let long = format!("File ▸ {}", "x".repeat(MAX_PATH_BYTES));
    assert!(parse_v1(&v1_file("menus", json!([]), v1_items(&[(&long, "docs/a.md")])), Area::Menus).unwrap_err().contains("bytes"));
    for empty in ["File ▸ ", " ▸ Save", "File ▸ … ▸ Save", ""] {
        assert!(parse_v1(&v1_file("menus", json!([]), v1_items(&[(empty, "docs/a.md")])), Area::Menus).is_err(), "{empty:?}");
    }
}

#[test]
fn committed_references_dir_is_checked_on_disk() {
    let dir = std::env::temp_dir().join(format!("cfa4-parity-ref-{}", std::process::id()));
    let refs = dir.join(REFERENCE_DIR);
    std::fs::create_dir_all(&refs).unwrap();
    // no committed reference at all is fine
    let none = std::env::temp_dir().join(format!("cfa4-parity-none-{}", std::process::id()));
    std::fs::create_dir_all(&none).unwrap();
    assert!(check_references(&none).unwrap().contains("0 files"));
    // an unknown file name, bad JSON and an oversized file are errors, not panics
    std::fs::write(refs.join("other.json"), "{}").unwrap();
    std::fs::write(refs.join("menus.json"), "not json").unwrap();
    let big = std::fs::File::create(refs.join("panels.json")).unwrap();
    big.set_len(MAX_FILE_BYTES + 1).unwrap();
    let init = Command::new("git").args(["init", "-q"]).current_dir(&dir).status().is_ok_and(|s| s.success());
    if init {
        let e = check_references(&dir).unwrap_err();
        assert!(e.contains("not a reference of a known area") && e.contains("menus.json") && e.contains("larger than"), "{e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&none);
}

// ---- fidelity tiers (G7.6) ----

fn fid(entries: Value) -> Result<fidelity::Fidelity, String> {
    fidelity::parse(&json!({"schema": 1, "items": entries}))
}

fn cmd_items(ids: &[(&str, &str)]) -> Vec<OurItem> {
    ids.iter().map(|(id, label)| OurItem { path: vec![label.to_string()], id: id.to_string(), ..OurItem::default() }).collect()
}

#[test]
fn unrated_is_not_exact() {
    let ours = cmd_items(&[("edit.undo", "Undo"), ("edit.redo", "Redo")]);
    let r = compare(Area::Commands, &paths(&[&["Undo"], &["Redo"], &["Nope"]]), &ours);
    // no annotation at all: present items are unrated, none is exact, the missing one counts nowhere
    assert_eq!(r.tier_counts(), [0, 0, 0, 2]);
    assert!(r.summary().contains("fidelity exact 0 / approximate 0 / stub 0 / unrated 2"), "{}", r.summary());
    assert_eq!(Tier::default(), Tier::Unrated);
    assert!(fidelity::Fidelity::default().entries.is_empty());
}

#[test]
fn exact_needs_evidence() {
    let e = fid(json!([{"area": "commands", "id": "edit.undo", "tier": "exact"}])).unwrap_err();
    assert!(e.contains("exact but has no evidence"), "{e}");
    assert!(fid(json!([{"area": "commands", "id": "edit.undo", "tier": "exact", "evidence": "  "}])).is_err());
    let ok = fid(json!([{"area": "commands", "id": "edit.undo", "tier": "exact", "evidence": "engine tests::undo_roundtrip"}])).unwrap();
    assert_eq!(ok.entries.first().map(|e| e.tier), Some(Tier::Exact));
    // a claim of `unrated` is not an annotation: leave the entry out
    assert!(fid(json!([{"area": "commands", "id": "x", "tier": "unrated", "note": "n"}])).unwrap_err().contains("unrated"));
}

#[test]
fn note_required_for_non_exact() {
    for tier in ["approximate", "stub"] {
        let e = fid(json!([{"area": "effects", "id": "warp", "tier": tier}])).unwrap_err();
        assert!(e.contains("has no note"), "{tier}: {e}");
        assert!(fid(json!([{"area": "effects", "id": "warp", "tier": tier, "note": "2-D only"}])).is_ok());
    }
}

#[test]
fn unknown_fidelity_id_is_error() {
    let f = fid(json!([{"area": "commands", "id": "gone.command", "tier": "stub", "note": "n"}])).unwrap();
    let mut ours = cmd_items(&[("edit.undo", "Undo")]);
    let e = f.apply(Area::Commands, &mut ours).unwrap_err();
    assert!(e.contains("gone.command") && e.contains("stale"), "{e}");
    // an entry of another area is not checked against this one
    assert!(f.apply(Area::Effects, &mut ours).is_ok());
    // an annotation that contradicts an automatic stub is refused
    let f = fid(json!([{"area": "panels", "id": "Libraries", "tier": "exact", "evidence": "x"}])).unwrap();
    let mut panels = our_panels(&json!([{"title": "Libraries", "status": "placeholder"}])).unwrap();
    assert!(f.apply(Area::Panels, &mut panels).unwrap_err().contains("automatic stub"));
}

#[test]
fn approximate_and_stub_listed_with_note() {
    let f = fid(json!([
        {"area": "commands", "id": "edit.undo", "tier": "exact", "evidence": "engine tests::undo_roundtrip"},
        {"area": "commands", "id": "fx.warp", "tier": "approximate", "note": "2-D only"},
        {"area": "commands", "id": "fx.morph", "tier": "stub", "note": "dissolve instead of morph"}
    ]))
    .unwrap();
    let mut ours = cmd_items(&[("edit.undo", "Undo"), ("fx.warp", "Warp Stabilizer"), ("fx.morph", "Morph Cut"), ("fx.plain", "Plain")]);
    f.apply(Area::Commands, &mut ours).unwrap();
    let r = compare(Area::Commands, &paths(&[&["Undo"], &["Warp Stabilizer"], &["Morph Cut"], &["Plain"], &["Missing One"]]), &ours);
    assert_eq!(r.tier_counts(), [1, 1, 1, 1]);
    let md = r.markdown("commands.json", 4);
    assert!(md.contains("### Approximate (1)") && md.contains("- Warp Stabilizer → Warp Stabilizer: 2-D only"), "{md}");
    assert!(md.contains("### Stub (1)") && md.contains("- Morph Cut → Morph Cut: dissolve instead of morph"), "{md}");
    assert!(md.contains("| **Total** | **4** | **1** | **1** | **1** | **1** |"), "{md}");
    assert!(md.contains("not a behaviour test") && md.contains("*unrated*, not exact"), "{md}");
}

#[test]
fn min_exact_fails_below() {
    let dir = std::env::temp_dir().join(format!("cfa4-parity-minexact-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, text: &str| {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    };
    let ours = write("ours.json", r#"[{"id":"edit.undo","label":"Undo","menu":[],"journal":true},{"id":"edit.redo","label":"Redo","menu":[],"journal":true}]"#);
    let reference = write("ref.json", r#"["Undo","Redo"]"#);
    let fidelity_file =
        write("fid.json", r#"{"schema":1,"items":[{"area":"commands","id":"edit.undo","tier":"exact","evidence":"engine tests::undo_roundtrip"}]}"#);
    let out = dir.join("report.md");
    let go = |min_exact: &str| {
        let argv = [
            "--area",
            "commands",
            "--ours",
            ours.to_str().unwrap_or(""),
            "--reference",
            reference.to_str().unwrap_or(""),
            "--fidelity",
            fidelity_file.to_str().unwrap_or(""),
            "--out",
            out.to_str().unwrap_or(""),
            "--min-exact",
            min_exact,
        ];
        // the root only names default paths; every file is explicit, and the report goes outside it
        run(Path::new("/nonexistent-cfa4-root"), &argv)
    };
    assert!(go("1").is_ok());
    let e = go("2").unwrap_err();
    assert!(e.contains("1 exact items, below --min-exact 2"), "{e}");
    assert!(go("x").unwrap_err().contains("not a count"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_fidelity_is_refused() {
    let note = "x".repeat(1025);
    for bad in [
        json!([{"area": "commands", "id": "a", "tier": "stub", "note": note}]),
        json!([{"area": "commands", "id": "a", "tier": "exact", "evidence": "data:text/plain,x"}]),
        json!([{"area": "commands", "id": "a", "tier": "stub", "note": 5}]),
        json!([{"area": "commands", "id": "a", "tier": "stub", "note": "n", "extra": 1}]),
        json!([{"area": "nowhere", "id": "a", "tier": "stub", "note": "n"}]),
        json!([{"area": "commands", "id": "", "tier": "stub", "note": "n"}]),
        json!([{"area": "commands", "id": 7, "tier": "stub", "note": "n"}]),
        json!([{"area": "commands", "id": "a", "tier": "great", "note": "n"}]),
        json!([{"area": "commands", "id": "a", "tier": "stub", "note": "n"}, {"area": "commands", "id": "a", "tier": "stub", "note": "m"}]),
        json!(["a"]),
        json!({"a": 1}),
    ] {
        assert!(fid(bad.clone()).is_err(), "{bad}");
    }
    let many: Vec<Value> = (0..MAX_ITEMS + 1).map(|i| json!({"area": "commands", "id": format!("c{i}"), "tier": "stub", "note": "n"})).collect();
    assert!(fid(json!(many)).unwrap_err().contains("more than"));
    assert!(fidelity::parse(&json!({"schema": 2, "items": []})).is_err());
    assert!(fidelity::parse(&json!({"schema": 1})).is_err());
    assert!(fidelity::parse(&json!({"schema": 1, "items": [], "x": 1})).is_err());
    assert!(fidelity::parse(&json!([])).is_err());
    // an oversized or non-regular file is an error, not a read
    let missing = std::env::temp_dir().join(format!("cfa4-no-such-fidelity-{}.json", std::process::id()));
    assert!(fidelity::load(Path::new("."), Some(&missing)).is_err());
    assert!(fidelity::load(Path::new("/nonexistent-cfa4-root"), None).unwrap().entries.is_empty());
}

// ---- --area all, reference resolution, --out guard (G7.7) ----

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cfa4-parity-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(root: &Path, rel: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "[]").unwrap();
    p
}

#[test]
fn resolution_order() {
    let root = temp_root("resolve");
    let explicit = Path::new("/somewhere/mine.json");
    assert_eq!(resolve_reference(&root, Area::Menus, None, false), None);
    let committed = touch(&root, "xtask/parity/reference/menus.json");
    assert_eq!(resolve_reference(&root, Area::Menus, None, false), Some(RefSource::Committed(committed.clone())));
    let local = touch(&root, "plan/premiere/menus.json");
    // local snapshot beats the committed file; --reference beats both; --committed ignores the other two
    assert_eq!(resolve_reference(&root, Area::Menus, None, false), Some(RefSource::Local(local)));
    assert_eq!(resolve_reference(&root, Area::Menus, Some(explicit), false), Some(RefSource::Explicit(explicit.to_path_buf())));
    assert_eq!(resolve_reference(&root, Area::Menus, None, true), Some(RefSource::Committed(committed.clone())));
    // the egui menu bar uses the menus reference; other areas have their own
    assert_eq!(resolve_reference(&root, Area::MenusEgui, None, true), Some(RefSource::Committed(committed)));
    assert_eq!(resolve_reference(&root, Area::Panels, None, false), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn report_names_reference_source() {
    let root = Path::new("/repo");
    assert_eq!(RefSource::Committed("/repo/xtask/parity/reference/menus.json".into()).label(root), "xtask/parity/reference/menus.json (committed)");
    assert_eq!(RefSource::Local("/repo/plan/premiere/menus.json".into()).label(root), "plan/premiere/menus.json (local snapshot, not committed)");
    assert_eq!(RefSource::Explicit("/tmp/x/mine.json".into()).label(root), "mine.json");
    let r = compare(Area::Commands, &paths(&[&["Undo"]]), &cmd_items(&[("edit.undo", "Undo")]));
    let src = RefSource::Local("/repo/plan/premiere/commands.json".into());
    assert!(r.markdown(&src.label(root), 1).contains("from `plan/premiere/commands.json (local snapshot, not committed)`"));
}

#[test]
fn local_reference_cannot_write_into_repo() {
    let root = temp_root("guard");
    let local = RefSource::Local(root.join("plan/premiere/menus.json"));
    let committed = RefSource::Committed(root.join("xtask/parity/reference/menus.json"));
    let target = crate::target_dir();
    // the repository's docs/ is refused for a local or explicit reference, even through `..`
    for bad in [root.join("docs/parity-checklist.md"), root.join("target/../docs/x.md"), root.join("./docs/../README.md")] {
        let e = check_out(&root, &bad, &local).unwrap_err();
        assert!(e.contains("not committed") && e.contains("under"), "{e}");
        assert!(check_out(&root, &bad, &RefSource::Explicit("/tmp/r.json".into())).is_err());
        // a committed reference may be written anywhere
        assert!(check_out(&root, &bad, &committed).is_ok());
    }
    // under target/, or outside the repository altogether, is fine
    assert!(check_out(&root, &target.join("parity/parity-menus.md"), &local).is_ok());
    assert!(check_out(&root, Path::new("/tmp/anywhere/report.md"), &local).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn all_writes_five_sections() {
    let root = Path::new("/repo");
    let ours = cmd_items(&[("edit.undo", "Undo")]);
    let src = RefSource::Committed("/repo/xtask/parity/reference/commands.json".into());
    let outcome = |area: Area, measured: bool| Outcome {
        area,
        ours: 1,
        inventory: String::new(),
        measured: measured.then(|| (compare(area, &paths(&[&["Undo"], &["Redo"]]), &ours), src.clone())),
    };
    let outcomes: Vec<Outcome> = ALL_AREAS.iter().map(|a| outcome(*a, *a != Area::Transitions)).collect();
    let md = all_markdown(root, &outcomes);
    assert!(md.starts_with("# Parity report"), "{md}");
    for a in ALL_AREAS {
        assert!(md.contains(&format!("| {} |", a.name())), "{} row\n{md}", a.name());
    }
    assert!(md.contains("| transitions | not measured (no reference) |"), "{md}");
    assert!(md.contains("| commands | xtask/parity/reference/commands.json (committed) | 2 | 1 | 0 | 0 | 0 | 1 | 1 | 50.0% |"), "{md}");
    // each measured area's own report sits one level down; the unmeasured one says so
    assert_eq!(md.matches("\n## Command parity checklist").count() + md.matches("\n## Menu parity checklist").count(), 2, "{md}");
    assert!(md.contains("## transitions parity\n\nNo reference: not measured."), "{md}");
    assert!(!md.lines().any(|l| l.starts_with("# ") && !l.starts_with("# Parity report")), "{md}");
    assert!(ALL_AREAS.len() == 5 && !ALL_AREAS.contains(&Area::MenusEgui));
}

#[test]
fn area_all_and_committed_arguments() {
    let a = parse_args(&["--area", "all", "--committed"]).unwrap();
    assert!(a.all && a.committed);
    // a later --area names a single area again
    let a = parse_args(&["--area", "all", "--area", "panels"]).unwrap();
    assert!(!a.all && a.area == Area::Panels);
    let root = Path::new("/nonexistent-cfa4-root");
    assert!(run(root, &["--area", "all", "--ours", "x.json"]).unwrap_err().contains("single --area"));
    assert!(run(root, &["--area", "all", "--reference", "x.json"]).unwrap_err().contains("single --area"));
    assert!(run(root, &["--committed", "--reference", "x.json"]).unwrap_err().contains("contradict"));
    assert!(parse_args(&["--area", "everything"]).unwrap_err().contains("or all"));
}

#[test]
fn threshold_failures_are_listed_together() {
    let args = parse_args(&["--min", "100", "--min-exact", "1"]).unwrap();
    let ours = cmd_items(&[("edit.undo", "Undo")]);
    let r = compare(Area::Commands, &paths(&[&["Undo"], &["Redo"]]), &ours);
    let src = RefSource::Explicit("/tmp/c.json".into());
    let measured = Outcome { area: Area::Commands, ours: 1, inventory: String::new(), measured: Some((r, src)) };
    let f = threshold_failures(&args, &measured);
    assert_eq!(f.len(), 2, "{f:?}");
    let unmeasured = Outcome { area: Area::Panels, ours: 1, inventory: String::new(), measured: None };
    assert!(threshold_failures(&args, &unmeasured).first().is_some_and(|m| m.contains("need a reference")));
    assert!(threshold_failures(&parse_args(&[]).unwrap(), &unmeasured).is_empty());
}
