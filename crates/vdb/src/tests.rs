use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-vdb-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn script(dir: &Path, body: &str) -> Worker {
    let p = dir.join("w.sh");
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    Worker::at(p).with_timeout(Duration::from_millis(500))
}

#[test]
fn missing_worker_is_unavailable() {
    let w = Worker::at("/nonexistent/cfa4-vdb-worker");
    assert!(matches!(w.inspect(Path::new("x.vdb")), Err(Error::Unavailable)));
}

#[test]
fn parses_a_good_answer() {
    let d = tmp("good");
    let w = script(
        &d,
        r#"echo '{"grids":[{"name":"d","type":"float","gridClass":"unknown","voxelSize":[1,1,1],"activeVoxels":3,"bbox":{"min":[0,0,0],"max":[1,1,1]},"isPoints":false,"meta":{}}],"fileMetaCount":0}'"#,
    );
    let i = w.inspect(Path::new("x.vdb")).unwrap();
    assert_eq!(i.grids.len(), 1);
    assert_eq!(i.grids[0].active_voxels, 3);
}

#[test]
fn reported_error_is_an_error() {
    let d = tmp("rep");
    let w = script(&d, r#"echo '{"error":"IoError:   bad   file"}'; exit 2"#);
    match w.inspect(Path::new("x.vdb")) {
        Err(Error::Reported(m)) => assert_eq!(m, "IoError: bad file"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn crash_is_an_error_not_a_panic() {
    let d = tmp("crash");
    let w = script(&d, "kill -SEGV $$");
    assert!(matches!(w.inspect(Path::new("x.vdb")), Err(Error::WorkerDied(_))));
}

#[test]
fn garbage_is_bad_answer() {
    let d = tmp("garbage");
    let w = script(&d, "printf '\\377\\376 not json'");
    assert!(matches!(w.inspect(Path::new("x.vdb")), Err(Error::BadAnswer(_))));
}

#[test]
fn hang_times_out() {
    let d = tmp("hang");
    let w = script(&d, "sleep 30");
    assert!(matches!(w.inspect(Path::new("x.vdb")), Err(Error::Timeout)));
}

#[test]
fn endless_output_is_bounded() {
    let d = tmp("flood");
    let w = script(&d, "yes aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").with_timeout(Duration::from_secs(20));
    assert!(matches!(w.inspect(Path::new("x.vdb")), Err(Error::OutputTooLarge | Error::WorkerDied(_))));
}

/// Real worker tests: run only when `CFA4_VDB_WORKER` points at a built worker (OpenVDB is optional).
fn real() -> Option<Worker> {
    Worker::discover().ok()
}

#[test]
fn real_worker_inspect_and_round_trip() {
    let Some(w) = real() else { return };
    let d = tmp("real");
    let a = d.join("a.vdb");
    let b = d.join("b.vdb");
    w.write_selftest(&a).unwrap();
    let ia = w.inspect(&a).unwrap();
    assert_eq!(ia.grids.len(), 2);
    let density = ia.grids.iter().find(|g| g.name == "density").unwrap();
    assert_eq!(density.value_type, "float");
    assert_eq!(density.active_voxels, 8000);
    assert_eq!(density.voxel_size, [0.5, 0.5, 0.5]);
    assert_eq!(density.bbox, Bbox { min: [0, 0, 0], max: [19, 19, 19] });
    assert!(ia.grids.iter().any(|g| g.name == "vel" && g.value_type == "vec3s"));
    w.copy(&a, &b).unwrap();
    assert_eq!(ia.grids, w.inspect(&b).unwrap().grids);
}

#[test]
fn real_worker_survives_hostile_files() {
    let Some(w) = real() else { return };
    let d = tmp("hostile");
    let a = d.join("a.vdb");
    w.write_selftest(&a).unwrap();
    let good = fs::read(&a).unwrap();
    assert!(w.inspect(&d.join("missing.vdb")).is_err());
    // Truncations and byte flips: always Ok or Err, never a panic, never a hang.
    let mut seed = 0x9e37_79b9_u32;
    // `CFA4_VDB_FUZZ_ITERS=20000` for the long run of N1-3
    let iters: u32 = std::env::var("CFA4_VDB_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    for i in 0..iters {
        let mut bytes = good.clone();
        if i % 2 == 0 {
            bytes.truncate((seed as usize) % bytes.len());
        } else {
            for _ in 0..8 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let at = (seed as usize) % bytes.len();
                bytes[at] ^= (seed >> 24) as u8 | 1;
            }
        }
        let f = d.join("m.vdb");
        fs::write(&f, &bytes).unwrap();
        let _ = w.inspect(&f);
    }
}
