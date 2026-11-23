//! The shared AU-1 test vectors (`tests/vectors/audit/v1/*.json`), the same files the Python side reads.

use std::fs;
use std::path::PathBuf;

use cfa4_audit::{AuditError, canonical_bytes, event_hash, to_hex};
use serde_json::Value;

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/vectors/audit/v1")
}

#[test]
fn every_shared_vector_matches() {
    let mut count = 0;
    let mut names: Vec<_> = fs::read_dir(vectors_dir()).expect("vector directory").map(|e| e.expect("entry").path()).collect();
    names.sort();
    for path in names.into_iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        let vec: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read vector")).expect("vector is JSON");
        let name = vec["name"].as_str().expect("name");
        let input: Value = serde_json::from_str(vec["input"].as_str().expect("input text")).expect("input is JSON");
        let is_event = vec["event"].as_bool().expect("event flag");
        let result = if is_event { event_canonical(&input).map(|c| (c, Some(event_hash(&input)))) } else { canonical_bytes(&input).map(|c| (c, None)) };
        match vec.get("error").and_then(Value::as_str) {
            Some(code) => {
                let err = result.expect_err(name);
                assert_eq!(err.code(), code, "{name}");
            }
            None => {
                let (canonical, hash) = result.unwrap_or_else(|e: AuditError| panic!("{name}: {e}"));
                assert_eq!(String::from_utf8(canonical).expect("utf-8"), vec["canonical"].as_str().expect("canonical"), "{name}");
                if let Some(hash) = hash {
                    assert_eq!(to_hex(&hash.expect(name)), vec["hash"].as_str().expect("hash"), "{name}");
                }
            }
        }
        count += 1;
    }
    assert!(count >= 10, "expected the shared vectors, found {count}");
}

/// Canonical bytes of the event as `event_hash` sees them: without the `hash` field.
fn event_canonical(event: &Value) -> Result<Vec<u8>, AuditError> {
    let mut e = event.clone();
    e.as_object_mut().ok_or(AuditError::NotObject)?.remove("hash");
    canonical_bytes(&e)
}
