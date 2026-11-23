//! `expr.check/save/list/get/delete/approve/revoke/bake/export/import` (plan CFA4-AX-INTERPRETER-001, X3: N3-5′, N3-7′).

use serde_json::json;

use super::*;

const SRC: &str = "reads @density; writes @hot; param float gain = 2.0; @hot = @density * gain;";

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-engine-expr-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn save(s: &mut Session) -> u64 {
    let r = s
        .execute("expr.save", json!({"name": "Hot spots", "source": SRC, "domain": "density", "outputs": {"hot": "float"}, "params": {"gain": 3}, "fuel": 500}))
        .unwrap();
    r["expression"]["id"].as_u64().unwrap()
}

#[test]
fn check_reports_what_the_source_reads_writes_and_declares_and_where_it_fails() {
    let mut s = Session::default();
    let r = s.execute("expr.check", json!({"source": SRC, "attrs": {"density": "float"}, "outputs": {"hot": "float"}})).unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["reads"], json!(["density"]));
    assert_eq!(r["writes"], json!(["hot"]));
    assert_eq!(r["params"][0]["name"], "gain");
    assert_eq!(r["params"][0]["default"], 2.0);
    assert_eq!(r["hash"].as_str().unwrap().len(), 64);
    // a compile error is an answer with the stable code and the place, not a command failure
    let r = s
        .execute("expr.check", json!({"source": "reads @density;\nwrites @hot;\n@hot = @missing;", "attrs": {"density": "float"}, "outputs": {"hot": "float"}}))
        .unwrap();
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"]["code"], "E-ATTR-1");
    assert_eq!(r["error"]["line"], 3);
    // hostile input is a diagnostic, never a panic
    for src in ["", "\u{0}", "((((((((((((((((", "writes @a; @a = 1/0;", &"x".repeat(100_000)] {
        let r = s.execute("expr.check", json!({"source": src, "outputs": {"a": "float"}})).unwrap();
        assert!(r["ok"].is_boolean());
    }
    assert!(matches!(s.execute("expr.check", json!({"source": "x", "attrs": {"d": "int"}})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("expr.check", json!({})), Err(EngineError::BadParams { .. })));
    // nothing was added to the project or the journal as a change
    assert!(s.project.expressions.is_empty());
}

#[test]
fn save_list_get_replace_delete_and_undo() {
    let mut s = Session::default();
    let id = save(&mut s);
    let l = s.execute("expr.list", json!({})).unwrap();
    assert_eq!(l["expressions"].as_array().unwrap().len(), 1);
    assert!(l["expressions"][0].get("source").is_none(), "the list is light");
    let g = s.execute("expr.get", json!({"id": id})).unwrap();
    assert_eq!(g["expression"]["source"], SRC);
    assert_eq!(g["expression"]["outputs"], json!({"hot": "float"}));
    assert_eq!(g["expression"]["params"], json!({"gain": 3}));
    assert_eq!(g["expression"]["fuel"], 500);
    assert_eq!(g["expression"]["hash"], cfa4_project::expression::source_hash(SRC));
    // replacing by id keeps name, grid and settings it is not given; the new text has a new hash
    let r = s.execute("expr.save", json!({"id": id, "source": "reads @density; writes @hot; @hot = @density;"})).unwrap();
    assert_ne!(r["expression"]["hash"], g["expression"]["hash"]);
    assert_eq!(r["expression"]["name"], "Hot spots");
    assert_eq!(s.project.expressions.len(), 1);
    // one undo step each
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.expressions[&cfa4_project::expression::ExprId(id)].source, SRC);
    s.execute("expr.delete", json!({"id": id})).unwrap();
    assert!(s.project.expressions.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.expressions.len(), 1);
    // the id does not collide with another item
    let other = s.execute("expr.save", json!({"name": "b", "source": "writes @a; @a = 1.0;", "domain": "d"})).unwrap()["expression"]["id"].as_u64().unwrap();
    assert_ne!(other, id);
    // refusals
    assert!(s.execute("expr.get", json!({"id": 999_999})).is_err());
    assert!(matches!(s.execute("expr.save", json!({"name": "x", "source": SRC})), Err(EngineError::BadParams { .. })), "domain is required");
    assert!(matches!(s.execute("expr.save", json!({"name": "", "source": SRC, "domain": "d"})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("expr.save", json!({"name": "x", "source": SRC, "domain": "d", "outputs": {"a": "int"}})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("expr.save", json!({"name": "x", "source": SRC, "domain": "d", "params": {"p": "text"}})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute("expr.save", json!({"name": "x", "source": SRC, "domain": "d", "fuel": 0})), Err(EngineError::BadParams { .. })));
    assert!(s.execute("expr.save", json!({"name": "x", "source": "x".repeat(70_000), "domain": "d"})).is_err());
    // with `attrs` the text is compiled first and a failure keeps its code
    let before = s.project.expressions.len();
    let e = s.execute("expr.save", json!({"name": "x", "source": "writes @a; @a = @nope;", "domain": "d", "attrs": {"a": "float"}})).unwrap_err().to_string();
    assert!(e.contains("E-ATTR"), "{e}");
    assert_eq!(s.project.expressions.len(), before);
}

/// N3-5′ and N3-7′: the project round-trips source + hash; opening it runs nothing and trusts nothing; the file never
/// says "approved"; another machine (a fresh session) must approve before a bake.
#[test]
fn a_project_from_elsewhere_runs_nothing_until_approved() {
    let d = dir("load");
    let mut a = Session::default();
    let id = save(&mut a);
    assert_eq!(a.execute("expr.list", json!({})).unwrap()["expressions"][0]["approved"], true, "the author's machine approved what it saved");
    let proj = d.join("p.fcproj").to_string_lossy().to_string();
    a.execute("file.saveAs", json!({"path": proj})).unwrap();
    let text = std::fs::read_to_string(&proj).unwrap().to_ascii_lowercase();
    assert!(!text.contains("approv") && !text.contains("trusted"), "no approval in the project file");
    assert!(text.contains(&cfa4_project::expression::source_hash(SRC)));
    // another machine
    let mut b = Session::default();
    b.execute("file.open", json!({"path": proj})).unwrap();
    assert_eq!(*b.project, *a.project, "source, hash and settings round-trip");
    let l = b.execute("expr.list", json!({})).unwrap();
    assert_eq!(l["expressions"][0]["approved"], false, "opening a project approves nothing");
    // the bake refuses before it looks at any volume or starts any worker
    let e = b.execute("expr.bake", json!({"id": id})).unwrap_err().to_string();
    assert!(e.contains("E-APPROVAL"), "{e}");
    // reading it is free; approving is the user's explicit step
    assert_eq!(b.execute("expr.get", json!({"id": id})).unwrap()["expression"]["source"], SRC);
    b.execute("expr.approve", json!({"id": id})).unwrap();
    assert_eq!(b.execute("expr.list", json!({})).unwrap()["expressions"][0]["approved"], true);
    // past the gate the next refusal is about the missing volume
    let e = b.execute("expr.bake", json!({"id": id})).unwrap_err().to_string();
    assert!(!e.contains("E-APPROVAL") && e.contains("volume"), "{e}");
    // revoking closes it again
    b.execute("expr.revoke", json!({"id": id})).unwrap();
    assert!(b.execute("expr.bake", json!({"id": id})).unwrap_err().to_string().contains("E-APPROVAL"));
    // editing the text is a different hash: saving it (by the user) approves that text only
    let r = b.execute("expr.save", json!({"id": id, "source": "reads @density; writes @hot; @hot = 0.0;"})).unwrap();
    assert_eq!(r["expression"]["approved"], true);
    assert_eq!(b.execute("expr.list", json!({})).unwrap()["expressions"][0]["hash"], r["expression"]["hash"]);
    // a project edited by hand so that the hash no longer matches the text is refused whole
    let broken = std::fs::read_to_string(&proj).unwrap().replace("@hot = @density * gain;", "@hot = 99.0;");
    let bp = d.join("broken.fcproj").to_string_lossy().to_string();
    std::fs::write(&bp, broken).unwrap();
    assert!(Session::default().execute("file.open", json!({"path": bp})).is_err());
}

#[test]
fn approvals_are_kept_per_machine_outside_the_project_and_a_bad_file_approves_nothing() {
    let d = dir("approvals");
    let mut a = Session::default();
    a.expr_approvals.set_dir(&d);
    let id = save(&mut a);
    let file = d.join("expression-approvals.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&cfa4_project::expression::source_hash(SRC)));
    // a later session on this machine finds it; the same project opened with another data dir does not
    let proj = d.join("p.fcproj").to_string_lossy().to_string();
    a.execute("file.saveAs", json!({"path": proj})).unwrap();
    let mut same = Session::default();
    same.expr_approvals.set_dir(&d);
    same.execute("file.open", json!({"path": proj})).unwrap();
    assert_eq!(same.execute("expr.get", json!({"id": id})).unwrap()["expression"]["approved"], true);
    let mut other = Session::default();
    other.expr_approvals.set_dir(&dir("approvals-other"));
    other.execute("file.open", json!({"path": proj})).unwrap();
    assert_eq!(other.execute("expr.get", json!({"id": id})).unwrap()["expression"]["approved"], false);
    // revoke is persisted too
    same.execute("expr.revoke", json!({"id": id})).unwrap();
    let mut again = Session::default();
    again.expr_approvals.set_dir(&d);
    assert!(!again.expr_approvals.is_approved(&cfa4_project::expression::source_hash(SRC)));
    // damaged, wrong-format and non-hash entries approve nothing
    for bad in [
        b"not json".to_vec(),
        br#"{"format":"other","hashes":["00"]}"#.to_vec(),
        json!({"format": "cfa4.expr-approvals", "hashes": ["xyz", 5, "A".repeat(64)]}).to_string().into_bytes(),
    ] {
        std::fs::write(&file, bad).unwrap();
        let mut t = Session::default();
        t.expr_approvals.set_dir(&d);
        assert!(!t.expr_approvals.is_approved(&cfa4_project::expression::source_hash(SRC)));
        assert!(!t.expr_approvals.is_approved(&"A".repeat(64)));
    }
}

#[test]
fn presets_round_trip_and_an_imported_expression_is_not_approved() {
    let d = dir("presets");
    let mut a = Session::default();
    let id = save(&mut a);
    let file = d.join("hot.cfa4expr.json").to_string_lossy().to_string();
    let r = a.execute("expr.export", json!({"path": file, "ids": [id]})).unwrap();
    assert_eq!(r["count"], 1);
    let mut b = Session::default();
    let r = b.execute("expr.import", json!({"path": file})).unwrap();
    assert_eq!(r["count"], 1);
    let new_id = r["expressions"][0]["id"].as_u64().unwrap();
    assert_eq!(r["expressions"][0]["approved"], false);
    let g = b.execute("expr.get", json!({"id": new_id})).unwrap();
    assert_eq!(g["expression"]["source"], SRC);
    assert_eq!(g["expression"]["params"], json!({"gain": 3}));
    assert!(b.execute("expr.bake", json!({"id": new_id})).unwrap_err().to_string().contains("E-APPROVAL"));
    b.execute("edit.undo", json!({})).unwrap();
    assert!(b.project.expressions.is_empty(), "one undo step");
    // hostile preset files are refused and change nothing
    let hostile = [
        ("big", "x".repeat(5 << 20)),
        ("json", "{".to_string()),
        ("format", json!({"format": "x", "version": 1, "expressions": []}).to_string()),
        ("many", json!({"format": "cfa4.expr-presets", "version": 1, "expressions": vec![json!({}); 300]}).to_string()),
        (
            "type",
            json!({"format": "cfa4.expr-presets", "version": 1, "expressions": [{"name": "n", "source": "s", "domain": "d", "outputs": {"a": "int"}}]})
                .to_string(),
        ),
        (
            "nan",
            json!({"format": "cfa4.expr-presets", "version": 1, "expressions": [{"name": "n", "source": "s", "domain": "d", "params": {"p": "1"}}]})
                .to_string(),
        ),
    ];
    for (n, text) in hostile {
        let f = d.join(format!("{n}.json"));
        std::fs::write(&f, text).unwrap();
        assert!(b.execute("expr.import", json!({"path": f.to_string_lossy()})).is_err(), "{n}");
    }
    assert!(b.project.expressions.is_empty());
    assert!(b.execute("expr.import", json!({"path": d.join("missing.json").to_string_lossy()})).is_err());
}

#[test]
fn open_never_compiles_or_runs() {
    // an expression that cannot compile anywhere is still only text until a user acts on it
    let d = dir("never");
    let mut a = Session::default();
    a.execute("expr.save", json!({"name": "hostile", "source": "open(\"/etc/passwd\"); while (true) {}", "domain": "d"})).unwrap();
    let proj = d.join("p.fcproj").to_string_lossy().to_string();
    a.execute("file.saveAs", json!({"path": proj})).unwrap();
    let mut b = Session::default();
    b.execute("file.open", json!({"path": proj})).unwrap();
    let id = *b.project.expressions.keys().next().unwrap();
    assert!(b.execute("expr.bake", json!({"id": id.0})).unwrap_err().to_string().contains("E-APPROVAL"));
    // the checker answers with a diagnostic for it
    let r = b.execute("expr.check", json!({"source": b.project.expressions[&id].source.clone()})).unwrap();
    assert_eq!(r["ok"], false);
}

#[test]
fn bake_writes_next_to_the_source_and_never_over_a_file() {
    // needs a built worker; returns early without one
    let Ok(w) = cfa4_vdb::Worker::discover() else { return };
    let d = dir("bake");
    let vdb = d.join("t.vdb");
    w.write_selftest(&vdb).unwrap();
    let mut s = Session::default();
    let vol = s.execute("vdb.import", json!({"paths": [vdb.to_string_lossy()]})).unwrap()["items"][0]["item"].as_u64().unwrap();
    let id = save(&mut s);
    let r = s.execute("expr.bake", json!({"id": id, "item": vol})).unwrap();
    let out = std::path::PathBuf::from(r["vdb"].as_str().unwrap());
    assert_eq!(out.parent().unwrap(), d.as_path(), "next to the source");
    assert!(out.file_name().unwrap().to_string_lossy().starts_with("t.hot-spots-"), "{out:?}");
    assert_eq!(r["report"]["written"], json!(["hot"]));
    assert_eq!(r["expression"]["id"], id);
    let new_item = r["item"].as_u64().unwrap();
    assert_ne!(new_item, vol);
    // a second bake takes the next free name; an explicit existing `out` is refused unless `overwrite`
    let r2 = s.execute("expr.bake", json!({"id": id, "item": vol})).unwrap();
    assert_ne!(r2["vdb"], r["vdb"]);
    assert!(s.execute("expr.bake", json!({"id": id, "item": vol, "out": out.to_string_lossy()})).is_err());
    s.execute("expr.bake", json!({"id": id, "item": vol, "out": out.to_string_lossy(), "overwrite": true})).unwrap();
    // the stored parameter (3) applies, a call can override it
    let a = w.inspect(&out).unwrap();
    assert!(a.grids.iter().any(|g| g.name == "hot"));
    // a limit or fuel error keeps its code and leaves the project as it was
    let n = s.project.items.len();
    let e = s.execute("expr.bake", json!({"id": id, "item": vol, "fuel": 1, "out": d.join("x.vdb").to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("R-FUEL"), "{e}");
    assert_eq!(s.project.items.len(), n);
}
