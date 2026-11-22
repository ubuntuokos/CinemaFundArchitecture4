//! OpenVDB commands (`vdb.*`), plan CFA4-DNEG-TECHNOLOGY-ADAPTATION-001, N1.
//!
//! | command | does |
//! |---|---|
//! | `vdb.inspect` | grids, value types, voxel size, active voxels, bounding box and metadata of a `.vdb` file, read by the separate `cfa4-vdb-worker` process |
//!
//! A missing or failing worker is an ordinary error message; the engine never depends on it to cut, play or save.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Result, Session};

fn inspect(_: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("vdb.inspect", "`path` is required"))?;
    let worker = cfa4_vdb::Worker::discover().map_err(|e| EngineError::Other(e.to_string()))?;
    let info = worker.inspect(std::path::Path::new(path)).map_err(|e| EngineError::Other(e.to_string()))?;
    serde_json::to_value(&info).map(|v| json!({"path": path, "inspection": v})).map_err(|e| EngineError::Other(e.to_string()))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "vdb.inspect",
        label: "Inspect VDB File",
        menu: &[],
        shortcut: None,
        params: r#"{"path":"file.vdb"}"#,
        enabled: always,
        run: inspect,
        journal: false,
    }]
}
