//! N1-2: a file with at least 1 GiB of active data is inspected within a bounded memory. Heavy (about 2 s and
//! 120 MB of disk): `cargo test -p cfa4-vdb --release --test big -- --ignored`, with `CFA4_VDB_WORKER` set.
//! The measure is the worker's own peak resident set (`VmHWM`), reported in the answer; a cgroup limit on top of it
//! is a run-time choice of the machine, not available inside every container.

use std::path::PathBuf;
use std::process::Command;

use cfa4_vdb::Worker;

const LAUNCHER: &str = env!("CARGO_BIN_EXE_cfa4-vdb-launch");

#[test]
#[ignore = "heavy: writes a 650^3 dense VDB (about 1 GiB of active data)"]
fn a_gib_of_active_data_is_read_in_bounded_memory() {
    let Ok(w) = Worker::discover() else { return };
    let w = w.with_launcher(LAUNCHER);
    let dir: PathBuf = std::env::temp_dir().join(format!("cfa4-vdb-big-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("big.vdb");
    // `selftest-write-big` is a worker command the typed API does not expose: run the worker directly, unconfined,
    // only to make the fixture (the repository holds no binary fixtures).
    let exe = std::env::var_os(cfa4_vdb::WORKER_ENV).unwrap();
    let ok = Command::new(exe).arg("selftest-write-big").arg(&f).arg("650").output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stdout));
    let i = w.inspect(&f).unwrap();
    let g = &i.grids[0];
    assert!(g.active_voxels >= 1 << 28, "{} active voxels", g.active_voxels); // 2^28 float voxels = 1 GiB
    assert!(i.peak_rss_kb > 0 && i.peak_rss_kb < 1024 * 1024, "peak RSS {} KiB", i.peak_rss_kb);
    let _ = std::fs::remove_dir_all(dir);
}
