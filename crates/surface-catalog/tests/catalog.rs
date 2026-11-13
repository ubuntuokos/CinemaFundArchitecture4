//! Catalogue, report and status tests (plan CFA4-HW-CONTROL-SURFACES-001 §2–§4, AC-01, AC-02,
//! AC-04, AC-14).

use std::path::PathBuf;

use cfa4_surface_catalog::catalog::Catalog;
use cfa4_surface_catalog::report::{Report, load_reports};
use cfa4_surface_catalog::status::{Adapters, Gate, evaluate, evaluate_all, gate};
use cfa4_surface_catalog::{Error, FeatureStatus, ModelStatus};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn shipped() -> Catalog {
    match Catalog::load_dir(&crate_dir().join("catalog")) {
        Ok(c) => c,
        Err(e) => panic!("shipped catalogue: {e}"),
    }
}

fn adapters(list: &[(&str, u64)]) -> Adapters {
    list.iter().map(|(a, v)| ((*a).to_owned(), *v)).collect()
}

/// AC-01: every family of plan §2 has a catalogue entry.
#[test]
fn shipped_catalogue_covers_every_family_of_the_plan() {
    let c = shipped();
    let families: std::collections::BTreeSet<&str> = c.families.values().map(String::as_str).collect();
    for f in [
        "elgato",
        "loupedeck",
        "mirabox-family",
        "tourbox",
        "xencelabs",
        "mountain",
        "logitech-creative",
        "control-surface-001",
        "generic-hid",
        "xkeys",
        "spacenav",
        "network",
        "virtual",
    ] {
        assert!(families.contains(f), "family {f} missing from the catalogue");
    }
    for id in ["ajazz.akp153e", "elgato.stream-deck-plus-xl", "elgato.stream-deck-studio", "cfa4.virtual-deck"] {
        assert!(c.model(id).is_some(), "{id} missing");
    }
    assert_eq!(c.by_usb(0x0fd9, 0x00c6).map(|m| m.id.as_str()), Some("elgato.stream-deck-plus-xl"));
}

/// AC-04 and AC-14: the reports in `docs/audit/hw/` match the schema and the catalogue.
#[test]
fn repository_reports_are_valid() {
    let c = shipped();
    let dir = crate_dir().join("../../docs/audit/hw");
    if let Err(e) = load_reports(&dir, &c) {
        panic!("{e}");
    }
}

/// AC-02: without adapters every model is PLANNED, never VERIFIED.
#[test]
fn nothing_is_verified_without_adapters_and_reports() {
    let c = shipped();
    for s in evaluate_all(&c, &[], &Adapters::new()) {
        assert_eq!(s.status, ModelStatus::Planned, "{}", s.id);
        assert!(s.features.values().all(|f| f.status != FeatureStatus::Verified));
    }
    let states = evaluate_all(&c, &[], &adapters(&[("virtual", 1), ("mirabox-family", 0)]));
    let akp = states.iter().find(|s| s.id == "ajazz.akp153e");
    assert_eq!(akp.map(|s| s.status), Some(ModelStatus::Experimental));
}

fn report(model: &str, category: &str, adapter: &str, version: &str, date: &str, results: &str) -> String {
    format!(
        "format: cfa4-hw-report\nversion: 1\ncategory: {category}\n\
         device: {{ model: {model}, vid: 0x0300, pid: 0x1010, serial_hash: \"sha256:{}\", firmware: \"1.0\" }}\n\
         environment: {{ os: \"Ubuntu 26.04 LTS amd64\", kernel: \"6.18\", session: \"KDE Plasma, Wayland\", cfa4_version: \"v0.1\", adapter: {{ id: {adapter}, version: \"{version}\" }} }}\n\
         results:\n{results}tested_by: tester\ndate: {date}\n",
        "a".repeat(64)
    )
}

fn parse(text: &str) -> Report {
    match Report::parse("test.yaml", text) {
        Ok(r) => r,
        Err(e) => panic!("{e}\n{text}"),
    }
}

const ALL_KEYS_PASS: &str = "  - { feature: keys.press, result: pass }\n  - { feature: keys.release, result: pass }\n  - { feature: keys.image, result: pass }\n  - { feature: keys.brightness, result: pass }\n  - { feature: device.firmware_query, result: pass }\n  - { feature: device.sleep, result: pass }\n  - { feature: device.hotplug, result: pass }\n";

#[test]
fn hil_report_verifies_and_status_follows_section_3_2() {
    let c = shipped();
    let Some(m) = c.model("ajazz.akp153e") else { panic!("model") };
    let a = adapters(&[("mirabox-family", 0)]);

    let partial = parse(&report("ajazz.akp153e", "cfa4-hil", "mirabox-family", "0.1.0", "2026-11-20", "  - { feature: keys.press, result: pass }\n"));
    assert!(partial.check(&c).is_empty(), "{:?}", partial.check(&c));
    let s = evaluate(m, &[("r1".into(), partial.clone())], &a);
    assert_eq!(s.status, ModelStatus::Partial);
    assert_eq!(s.features.get("keys.press").map(|f| f.status), Some(FeatureStatus::Verified));
    assert_eq!(s.features.get("keys.image").map(|f| f.status), Some(FeatureStatus::Untested));

    let full = parse(&report("ajazz.akp153e", "cfa4-hil", "mirabox-family", "0.2.0", "2026-11-21", ALL_KEYS_PASS));
    let s = evaluate(m, &[("r1".into(), partial), ("r2".into(), full.clone())], &a);
    assert_eq!(s.status, ModelStatus::Verified, "{s:?}");
    // identify is UNSUPPORTED and haptic NOT_APPLICABLE: neither keeps the model from VERIFIED
    assert_eq!(s.features.get("device.identify").map(|f| f.status), Some(FeatureStatus::Unsupported));

    // a newer failing report takes the verification back
    let fail =
        parse(&report("ajazz.akp153e", "cfa4-hil", "mirabox-family", "0.2.1", "2026-11-22", "  - { feature: keys.image, result: fail, note: rotated }\n"));
    let s = evaluate(m, &[("r2".into(), full.clone()), ("r3".into(), fail)], &a);
    assert_eq!(s.features.get("keys.image").map(|f| f.status), Some(FeatureStatus::Untested));
    assert_eq!(s.status, ModelStatus::Partial);

    // §4.3: a new adapter major version needs re-verification
    let s = evaluate(m, &[("r2".into(), full)], &adapters(&[("mirabox-family", 1)]));
    assert_eq!(s.status, ModelStatus::Experimental);
    assert!(s.features.get("keys.press").is_some_and(|f| f.reverify));
}

#[test]
fn ci_and_external_reports_verify_only_what_they_may() {
    let c = shipped();
    let a = adapters(&[("mirabox-family", 0), ("virtual", 0)]);
    let Some(akp) = c.model("ajazz.akp153e") else { panic!("model") };
    let ci = parse(&report("ajazz.akp153e", "cfa4-ci", "mirabox-family", "0.1.0", "2026-11-20", ALL_KEYS_PASS));
    assert_eq!(evaluate(akp, &[("ci".into(), ci)], &a).status, ModelStatus::Experimental);
    let ext = parse(&report("ajazz.akp153e", "external", "mirabox-family", "0.1.0", "2026-11-20", ALL_KEYS_PASS));
    assert_eq!(evaluate(akp, &[("ext".into(), ext)], &a).status, ModelStatus::Experimental);

    // the virtual deck's key features are protocol-level: a CI report verifies them
    let Some(v) = c.model("cfa4.virtual-deck") else { panic!("model") };
    let text = report(
        "cfa4.virtual-deck",
        "cfa4-ci",
        "virtual",
        "0.1.0",
        "2026-11-20",
        "  - { feature: keys.press, result: pass }\n  - { feature: keys.release, result: pass }\n  - { feature: keys.image, result: pass }\n  - { feature: keys.brightness, result: pass }\n",
    );
    let s = evaluate(v, &[("ci".into(), parse(&text))], &a);
    assert_eq!(s.status, ModelStatus::Verified);
}

#[test]
fn blockers_and_unavailable() {
    let text = r#"
format = "cfa4-surface-catalog"
version = 1
family = "t"

[[model]]
id = "t.blocked"
vendor = "T"
name = "Blocked"
kind = "lcd_keypad"
goal = "FULL"
adapter = "t"
phase = "M5"
blockers = [{ scope = "model", reason = "firmware 0.2.26 breaks input" }]

[[model]]
id = "t.none"
vendor = "T"
name = "No route"
kind = "lcd_keypad"
goal = "FULL"
phase = "PLANNED"
unavailable = true

[[model]]
id = "t.wheel"
vendor = "T"
name = "Wheel"
kind = "console"
goal = "FULL"
adapter = "t"
phase = "M5"
blockers = [{ scope = "wheel.turn", reason = "not decoded yet" }]

[[model.controls]]
id = "wheel"
type = "jog_wheel"
features = { turn = "UNTESTED", touch = "UNTESTED", display = "NOT_APPLICABLE" }
"#;
    let file = match Catalog::parse_file("t.toml", text) {
        Ok(f) => f,
        Err(e) => panic!("{e}"),
    };
    let c = match Catalog::from_files(vec![("t.toml".into(), file)]) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    let s = evaluate_all(&c, &[], &adapters(&[("t", 1)]));
    let by = |id: &str| s.iter().find(|x| x.id == id).cloned();
    assert_eq!(by("t.blocked").map(|x| x.status), Some(ModelStatus::Blocked));
    assert_eq!(by("t.none").map(|x| x.status), Some(ModelStatus::Unavailable));
    let wheel = by("t.wheel");
    assert_eq!(wheel.as_ref().and_then(|w| w.features.get("wheel.turn")).map(|f| f.status), Some(FeatureStatus::Blocked));
    assert_eq!(wheel.as_ref().and_then(|w| w.features.get("wheel.touch")).map(|f| f.status), Some(FeatureStatus::Untested));
}

#[test]
fn bad_catalogue_files_are_errors_not_panics() {
    let base = "format = \"cfa4-surface-catalog\"\nversion = 1\nfamily = \"t\"\n";
    let model = "[[model]]\nid = \"t.a\"\nvendor = \"T\"\nname = \"A\"\nkind = \"k\"\ngoal = \"FULL\"\nphase = \"M1\"\n";
    let declared_verified = format!("{base}{model}[[model.controls]]\nid = \"keys\"\ntype = \"key\"\nfeatures = {{ press = \"VERIFIED\" }}\n");
    let wrong_feature = format!("{base}{model}[[model.controls]]\nid = \"keys\"\ntype = \"key\"\nfeatures = {{ image = \"UNTESTED\" }}\n");
    let bad_baseline = format!("{base}{model}baseline = [\"keys.press\"]\n");
    for text in [declared_verified, wrong_feature, bad_baseline] {
        let file = match Catalog::parse_file("t.toml", &text) {
            Ok(f) => f,
            Err(e) => panic!("{e}"),
        };
        assert!(matches!(Catalog::from_files(vec![("t.toml".into(), file)]), Err(Error::Invalid(_))), "{text}");
    }
    let dup = format!("{base}{model}{model}");
    let Ok(f) = Catalog::parse_file("t.toml", &dup) else { panic!("two models in one file must parse") };
    assert!(Catalog::from_files(vec![("t.toml".into(), f)]).is_err(), "a duplicate model id must be refused");
    for text in
        ["", "format = \"other\"\nversion = 1\nfamily = \"t\"\n", "format = \"cfa4-surface-catalog\"\nversion = 99\nfamily = \"t\"\n", "\u{0}\u{ff}garbage[[["]
    {
        assert!(Catalog::parse_file("t.toml", text).is_err());
    }
    assert!(Catalog::load_dir(&crate_dir().join("does-not-exist")).is_err());
}

#[test]
fn bad_reports_are_errors_not_panics() {
    let c = shipped();
    // raw serial numbers are refused by the schema (the repository is public)
    let raw_serial =
        report("ajazz.akp153e", "cfa4-hil", "mirabox-family", "0.1.0", "2026-11-20", ALL_KEYS_PASS).replace("serial_hash: \"sha256:", "serial: \"");
    assert!(Report::parse("r.yaml", &raw_serial).is_err());
    let r =
        parse(&report("ajazz.akp153e", "cfa4-hil", "elgato", "x", "20261120", "  - { feature: keys.nope, result: pass }\n").replace(&"a".repeat(64), "short"));
    let problems = r.check(&c);
    assert!(problems.len() >= 5, "{problems:?}");
    let unknown = parse(&report("nobody.nothing", "cfa4-hil", "x", "1.0.0", "2026-11-20", ALL_KEYS_PASS));
    assert_eq!(unknown.check(&c).len(), 1);
    for text in ["", "format: cfa4-hw-report", ": : :", "format: other\nversion: 1"] {
        assert!(Report::parse("r.yaml", text).is_err());
    }
}

#[test]
fn editor_gate_follows_section_3_4() {
    assert_eq!(gate(FeatureStatus::Verified, false), Gate::Assignable);
    assert_eq!(gate(FeatureStatus::Partial, false), Gate::AssignableWithWarning);
    assert_eq!(gate(FeatureStatus::Untested, false), Gate::Disabled);
    assert_eq!(gate(FeatureStatus::Untested, true), Gate::Experimental);
    assert_eq!(gate(FeatureStatus::Blocked, true), Gate::Disabled);
    assert_eq!(gate(FeatureStatus::Unsupported, true), Gate::Disabled);
    assert_eq!(gate(FeatureStatus::NotApplicable, true), Gate::Hidden);
}
