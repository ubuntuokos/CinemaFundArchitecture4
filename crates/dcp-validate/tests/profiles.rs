//! DCP-11: versioned built-in profiles and the profile file format `cfa4-dcp-profile/1`.

use cfa4_dcp::Standard;
use cfa4_dcp_validate::profile::{BUILTIN, HISTORY, MAX_PROFILE_BYTES, PROFILE_FORMAT};
use cfa4_dcp_validate::rules::{AM_UNLISTED, AUD_LOUDNESS, MARK_MARKERS, NAME_ISDCF, PIC_FORMAT, PKL_HASH, SUB_DOCUMENTS};
use cfa4_dcp_validate::{Profile, ProfileError};

#[test]
fn every_builtin_profile_has_a_history_and_checks_out() {
    for id in BUILTIN {
        let latest = Profile::builtin(id).unwrap();
        let versions: Vec<u32> = HISTORY.iter().filter(|(p, _, _)| p == id).map(|(_, v, _)| *v).collect();
        assert_eq!(versions.first(), Some(&1), "{id}");
        assert_eq!(versions.last(), Some(&latest.version), "{id}");
        for v in versions {
            let p = Profile::builtin_version(id, v).unwrap();
            p.check().unwrap_or_else(|e| panic!("{id}@{v}: {e}"));
            // a built-in profile survives the file format unchanged
            assert_eq!(Profile::from_json(&p.to_json()).unwrap(), p, "{id}@{v}");
        }
    }
    for (id, _, _) in HISTORY {
        assert!(BUILTIN.contains(id), "{id}");
    }
    assert!(Profile::builtin_version("quick-scan", 2).is_none());
    assert!(Profile::builtin_version("nope", 1).is_none());
}

#[test]
fn cinema_profiles_gained_the_picture_format_in_version_2() {
    for id in ["smpte-cinema", "interop-cinema", "delivery-preflight"] {
        let v1 = Profile::builtin_version(id, 1).unwrap();
        let v2 = Profile::builtin_version(id, 2).unwrap();
        assert!(!v1.contains(PIC_FORMAT) && v2.contains(PIC_FORMAT), "{id}");
        assert_eq!(v1.rules.len() + 1, v2.rules.len(), "{id}");
        assert!(!v1.contains(AM_UNLISTED) && !v2.contains(AM_UNLISTED), "{id}: older versions stay as they were");
    }
}

#[test]
fn unlisted_documents_rule_came_with_the_next_profile_versions() {
    for (id, old, new) in [("full-integrity", 1, 2), ("smpte-cinema", 2, 3), ("interop-cinema", 2, 3), ("delivery-preflight", 2, 3)] {
        let before = Profile::builtin_version(id, old).unwrap();
        let after = Profile::builtin(id).unwrap();
        assert_eq!(after.version, new, "{id}");
        assert!(!before.contains(AM_UNLISTED) && after.contains(AM_UNLISTED), "{id}");
        assert_eq!(before.rules.len() + 1, after.rules.len(), "{id}");
    }
    for id in ["quick-scan", "security", "audio-quality", "subtitle-accessibility"] {
        assert!(!Profile::builtin(id).unwrap().contains(AM_UNLISTED), "{id}");
    }
}

#[test]
fn a_profile_file_extends_a_builtin_profile() {
    let text = r#"{
        "format": "cfa4-dcp-profile/1",
        "id": "festival-delivery",
        "version": 3,
        "title": "Festival Delivery",
        "standard": "smpte",
        "extends": "smpte-cinema@1",
        "remove": ["DCP.MARK.001"],
        "rules": [
            {"rule": "DCP.SUB.001", "required": true},
            {"rule": "DCP.AUD.002", "required": false}
        ]
    }"#;
    let p = Profile::from_json(text).unwrap();
    assert_eq!((p.id.as_str(), p.version, p.title.as_str(), p.standard), ("festival-delivery", 3, "Festival Delivery", Some(Standard::Smpte)));
    assert!(!p.contains(MARK_MARKERS) && !p.contains(PIC_FORMAT) && p.contains(PKL_HASH));
    let req = |id: &str| p.rules.iter().find(|r| r.rule == id).map(|r| r.required);
    assert_eq!(req(SUB_DOCUMENTS), Some(true), "an inherited optional rule made required");
    assert_eq!(req(AUD_LOUDNESS), Some(false));
    assert_eq!(req(NAME_ISDCF), Some(false));
    // written back out it stands on its own
    let again = Profile::from_json(&p.to_json()).unwrap();
    assert_eq!(again, p);
    assert!(!p.to_json().contains("extends"));
}

#[test]
fn bad_profile_files_are_errors() {
    let file = |extra: &str| format!(r#"{{"format":"{PROFILE_FORMAT}","id":"x","version":1,"title":"X",{extra}}}"#);
    let invalid = |text: &str, part: &str| match Profile::from_json(text) {
        Err(ProfileError::Invalid(m)) => assert!(m.contains(part), "{m}"),
        other => panic!("{text}: {other:?}"),
    };
    invalid(&file(r#""rules":[]"#), "no rules");
    invalid(&file(r#""rules":[{"rule":"DCP.NOPE.001","required":true}]"#), "DCP.NOPE.001 is not a rule");
    invalid(&file(r#""rules":[{"rule":"DCP.REF.001","required":true}]"#), "reference engine");
    invalid(&file(r#""rules":[{"rule":"DCP.AM.001","required":true},{"rule":"DCP.AM.001","required":false}]"#), "twice");
    invalid(&file(r#""extends":"nope""#), "not a built-in profile");
    invalid(&file(r#""extends":"quick-scan@x""#), "not a number");
    invalid(&file(r#""extends":"quick-scan","remove":["DCP.PKL.004"]"#), "removes DCP.PKL.004");
    invalid(&file(r#""standard":"dci","rules":[{"rule":"DCP.AM.001","required":true}]"#), "standard");
    invalid(r#"{"format":"cfa4-dcp-profile/1","id":"Bad Id","version":1,"title":"X","extends":"quick-scan"}"#, "id");
    invalid(r#"{"format":"cfa4-dcp-profile/1","id":"x","version":0,"title":"X","extends":"quick-scan"}"#, "version 0");
    invalid(r#"{"format":"cfa4-dcp-profile/1","id":"x","version":1,"title":" ","extends":"quick-scan"}"#, "no title");
    assert!(
        matches!(Profile::from_json(r#"{"format":"cfa4-dcp-profile/2","id":"x","version":1,"title":"X"}"#), Err(ProfileError::Format(f)) if f == "cfa4-dcp-profile/2")
    );
    assert!(matches!(Profile::from_json(&file(r#""extends":"quick-scan","colour":"red""#)), Err(ProfileError::Syntax(_))));
    assert!(matches!(Profile::from_json("not json"), Err(ProfileError::Syntax(_))));
}

#[test]
fn profile_files_are_read_with_a_size_limit() {
    let d = std::env::temp_dir().join(format!("cfa4-dcp-validate-profiles-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let good = d.join("p.json");
    std::fs::write(&good, Profile::builtin("security").unwrap().to_json()).unwrap();
    assert_eq!(Profile::load(&good).unwrap(), Profile::builtin("security").unwrap());
    let big = d.join("big.json");
    std::fs::write(&big, vec![b' '; MAX_PROFILE_BYTES as usize + 1]).unwrap();
    assert!(matches!(Profile::load(&big), Err(ProfileError::TooLarge(_))));
    assert!(matches!(Profile::load(&d.join("missing.json")), Err(ProfileError::Io(_))));
    let _ = std::fs::remove_dir_all(&d);
}
