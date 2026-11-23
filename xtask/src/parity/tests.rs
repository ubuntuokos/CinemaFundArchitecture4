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
    assert_eq!(ours, vec![OurItem { path: paths(&[&["File", "Save"]]).remove(0), id: "file.save".into(), stub: None }]);
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
        OurItem { path: paths(&[&["File", "Save"]]).remove(0), id: "file.save".into(), stub: None },
        OurItem { path: paths(&[&["File", "Export", "Export Frame"]]).remove(0), id: "export.frame".into(), stub: None },
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
    let stubs: Vec<(&str, bool)> = ours.iter().map(|it| (it.id.as_str(), it.stub.is_some())).collect();
    assert_eq!(stubs, vec![("Program", false), ("Timeline", false), ("Effect Controls", false), ("Libraries", true), ("Reference Monitor", true)]);
    assert_eq!(ours.get(3).and_then(|it| it.stub), Some("placeholder dock: no content yet"));
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
    assert!(r.summary().starts_with("panels: presence 4/5 (80.0%) (of which stub 2)"), "{}", r.summary());
    assert_eq!(r.ours_only, paths(&[&["Effect Controls"]]));
    let md = r.markdown("panels.json", 5);
    assert!(md.starts_with("# Panel parity checklist"), "{md}");
    assert!(md.contains("| Panels | 5 | 4 | 0 | 1 | 80.0% |"), "{md}");
    assert!(md.contains("## Automatic stubs (2)"), "{md}");
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
