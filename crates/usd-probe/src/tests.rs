use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfa4-usd-probe-test-{}-{name}", std::process::id()));
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
    let w = Worker::at("/nonexistent/cfa4-usd-probe");
    assert!(matches!(w.inspect(Path::new("x.usda")), Err(Error::Unavailable)));
}

#[test]
fn parses_a_good_answer() {
    let d = tmp("good");
    let w = script(
        &d,
        r#"echo '{"upAxis":"Y","metersPerUnit":0.01,"startTimeCode":0,"endTimeCode":24,"timeCodesPerSecond":24,"primCount":3,"maxDepth":2,"truncated":false,"rootCount":1,"rootPrims":["World"],"layerCount":2,"peakRssKb":1000}'"#,
    );
    let i = w.inspect(Path::new("x.usda")).unwrap();
    assert_eq!(i.prim_count, 3);
    assert_eq!(i.root_prims, ["World"]);
    assert!(!i.truncated);
}

#[test]
fn reported_error_is_an_error() {
    let d = tmp("rep");
    let w = script(&d, r#"echo '{"error":"IoError:   bad   file"}'; exit 2"#);
    match w.inspect(Path::new("x.usda")) {
        Err(Error::Reported(m)) => assert_eq!(m, "IoError: bad file"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn crash_is_an_error_not_a_panic() {
    let d = tmp("crash");
    let w = script(&d, "kill -SEGV $$");
    assert!(matches!(w.inspect(Path::new("x.usda")), Err(Error::WorkerDied(_))));
}

#[test]
fn garbage_is_bad_answer() {
    let d = tmp("garbage");
    let w = script(&d, "printf '\\377\\376 not json'");
    assert!(matches!(w.inspect(Path::new("x.usda")), Err(Error::BadAnswer(_))));
}

#[test]
fn hang_times_out() {
    let d = tmp("hang");
    let w = script(&d, "sleep 30");
    assert!(matches!(w.inspect(Path::new("x.usda")), Err(Error::Timeout)));
}

#[test]
fn endless_output_is_bounded() {
    let d = tmp("flood");
    let w = script(&d, "yes aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").with_timeout(Duration::from_secs(20));
    assert!(matches!(w.inspect(Path::new("x.usda")), Err(Error::OutputTooLarge | Error::WorkerDied(_))));
}

/// Real worker tests: run only when `CFA4_USD_PROBE` points at a built probe (OpenUSD is optional).
fn real() -> Option<Worker> {
    Worker::discover().ok()
}

fn assert_scene(i: &Inspection) {
    assert_eq!(i.up_axis, "Y");
    assert!((i.meters_per_unit - 0.01).abs() < 1e-9);
    assert_eq!(i.prim_count, 3);
    assert_eq!(i.max_depth, 2);
    assert_eq!(i.root_prims, ["World"]);
}

#[test]
fn real_probe_reads_usda_usdc_and_usdz() {
    let Some(w) = real() else { return };
    let d = tmp("real");
    for name in ["a.usda", "a.usdc", "a.usdz"] {
        let f = d.join(name);
        w.write_selftest(&f).unwrap();
        assert_scene(&w.inspect(&f).unwrap());
    }
}

#[test]
fn real_probe_limits_depth_and_cycles() {
    let Some(w) = real() else { return };
    let d = tmp("limits");
    let deep = d.join("deep.usda");
    w.write_deep(&deep, 300).unwrap();
    let i = w.inspect(&deep).unwrap();
    assert_eq!((i.prim_count, i.max_depth, i.truncated), (300, 300, false));
    // Past the worker's depth limit it stops and says so (AGENTS.md §0.5).
    w.write_deep(&deep, 700).unwrap();
    let i = w.inspect(&deep).unwrap();
    assert!(i.truncated && i.max_depth <= 512, "{i:?}");
    // A sublayer cycle: USD cuts it; the answer is a value, not a hang.
    let cyc = d.join("cyc.usda");
    w.write_cycle(&cyc).unwrap();
    let i = w.inspect(&cyc).unwrap();
    assert_eq!(i.root_prims, ["A"]);
}

#[test]
fn real_probe_memory_and_time_limits_are_errors() {
    let Some(w) = real() else { return };
    let d = tmp("mem");
    let f = d.join("a.usda");
    w.write_selftest(&f).unwrap();
    assert!(w.clone().with_memory_limit_mb(16).inspect(&f).is_err());
    assert!(matches!(w.clone().with_timeout(Duration::from_millis(1)).inspect(&f), Err(Error::Timeout)));
    assert!(w.inspect(&f).is_ok());
}

/// SB-G6 (1): truncations, byte flips and wrong sizes of USDA/USDC/USDZ seeds. Always `Ok` or `Err`, never a
/// host panic, never a hang. `CFA4_USD_FUZZ_ITERS=7500` for the long run of the gate. A finding becomes a test.
#[test]
fn real_probe_survives_hostile_files() {
    let Some(w) = real() else { return };
    let d = tmp("hostile");
    assert!(w.inspect(&d.join("missing.usda")).is_err());
    let iters: u32 = std::env::var("CFA4_USD_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let mut seed = 0x9e37_79b9_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    let mut seeds = Vec::new();
    for name in ["s.usda", "s.usdc", "s.usdz"] {
        let f = d.join(name);
        w.write_selftest(&f).unwrap();
        seeds.push((name, fs::read(&f).unwrap()));
    }
    for i in 0..iters {
        let (name, good) = &seeds[(i as usize) % seeds.len()];
        let mut bytes = good.clone();
        match i % 3 {
            0 => bytes.truncate((next() as usize) % bytes.len()),
            1 => {
                for _ in 0..8 {
                    let at = (next() as usize) % bytes.len();
                    bytes[at] ^= (next() >> 24) as u8 | 1;
                }
            }
            // Wrong sizes: a run of 0xFF where a length or offset may sit.
            _ => {
                let at = (next() as usize) % bytes.len();
                let end = (at + 8).min(bytes.len());
                bytes[at..end].fill(0xff);
            }
        }
        let f = d.join(format!("m-{name}"));
        fs::write(&f, &bytes).unwrap();
        let _ = w.inspect(&f);
    }
}
