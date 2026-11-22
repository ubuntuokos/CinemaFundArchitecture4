//! `vdb.inspect`: parameters, the "no worker" message, and (when a built worker is found through
//! `CFA4_VDB_WORKER`) the real answer on a synthetic file.

use serde_json::json;

use super::*;

#[test]
fn inspect_requires_a_path() {
    let mut s = Session::default();
    assert!(matches!(s.execute("vdb.inspect", json!({})), Err(EngineError::BadParams { .. })));
}

#[test]
fn inspect_without_or_with_worker() {
    let mut s = Session::default();
    match cfa4_vdb::Worker::discover() {
        Err(_) => {
            let e = s.execute("vdb.inspect", json!({"path": "/x.vdb"})).unwrap_err();
            assert!(e.to_string().contains("not available"), "{e}");
        }
        Ok(w) => {
            let f = std::env::temp_dir().join(format!("cfa4-engine-vdb-{}.vdb", std::process::id()));
            w.write_selftest(&f).unwrap();
            let r = s.execute("vdb.inspect", json!({"path": f.to_str().unwrap()})).unwrap();
            assert_eq!(r["inspection"]["grids"].as_array().unwrap().len(), 2);
            // a worker error is an error, not a panic
            assert!(s.execute("vdb.inspect", json!({"path": "/nonexistent.vdb"})).is_err());
            let _ = std::fs::remove_file(f);
        }
    }
}
