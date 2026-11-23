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

// ---------- X2: tiles and the CFX bridge ----------

mod x2 {
    use super::*;
    use crate::tiles::{LEAF_VOXELS, Leaf, TileSet};
    use cfa4_expr::{Ty, Value};

    fn leaf(origin: [i32; 3], ch: usize) -> Leaf {
        Leaf { origin, active: [u64::MAX, 0, 1, 0, 0, 0, 0, 0], values: (0..LEAF_VOXELS * ch).map(|i| i as f32 * 0.5).collect() }
    }

    #[test]
    fn tiles_round_trip() {
        for ch in [1, 3] {
            let t = TileSet { channels: ch, voxel_size: 0.25, background: [1.0, 2.0, 3.0], leaves: vec![leaf([0, 0, 0], ch), leaf([8, -16, 24], ch)] };
            assert_eq!(TileSet::parse(&t.to_bytes()).unwrap(), t);
            assert_eq!(t.active_voxels(), 2 * 65);
        }
    }

    #[test]
    fn hostile_tile_files_are_errors() {
        let t = TileSet { channels: 1, voxel_size: 1.0, background: [0.0; 3], leaves: vec![leaf([0, 0, 0], 1)] };
        let good = t.to_bytes();
        assert!(TileSet::parse(&[]).is_err());
        assert!(TileSet::parse(b"CFXT").is_err());
        assert!(TileSet::parse(&good[..good.len() - 1]).is_err());
        let mut extra = good.clone();
        extra.push(0);
        assert!(TileSet::parse(&extra).is_err());
        let mut huge = good.clone();
        huge[16..24].copy_from_slice(&u64::MAX.to_le_bytes()); // leaf count: the size check must not overflow or allocate
        assert!(TileSet::parse(&huge).is_err());
        let mut chans = good.clone();
        chans[8] = 7;
        assert!(TileSet::parse(&chans).is_err());
        let mut ver = good.clone();
        ver[4] = 9;
        assert!(TileSet::parse(&ver).is_err());
        let mut org = good;
        org[HEADER_ORIGIN] = 3; // not a multiple of 8
        assert!(TileSet::parse(&org).is_err());
        // every prefix and every single-byte mutation of a header answers with a value, never a panic
        let base = t.to_bytes();
        for n in 0..64 {
            let _ = TileSet::parse(&base[..n.min(base.len())]);
        }
        for i in 0..64 {
            for v in [0u8, 1, 0x7f, 0xff] {
                let mut m = base.clone();
                m[i] = v;
                let _ = TileSet::parse(&m);
            }
        }
    }
    const HEADER_ORIGIN: usize = 48;

    fn export(w: &Worker, vdb: &Path, grid: &str, dir: &Path, name: &str) -> TileSet {
        let f = dir.join(name);
        w.export_tiles(vdb, grid, &f, Duration::from_secs(60), None).unwrap();
        TileSet::read(&f).unwrap()
    }

    fn at(ts: &TileSet, ijk: [i32; 3]) -> f32 {
        let o = [ijk[0] & !7, ijk[1] & !7, ijk[2] & !7];
        let l = ts.leaves.iter().find(|l| l.origin == o).unwrap();
        l.values[(((ijk[0] & 7) << 6) | ((ijk[1] & 7) << 3) | (ijk[2] & 7)) as usize]
    }

    #[test]
    fn real_worker_expression_matches_hand_computed_values() {
        let Some(w) = real() else { return };
        let d = tmp("x2-basic");
        let a = d.join("a.vdb");
        w.write_selftest(&a).unwrap();
        let out = d.join("out.vdb");
        let mut req = ExprRequest::new("reads @density; writes @out; param float gain = 2.0; @out = @density * gain + P.x;", "density");
        req.outputs = vec![("out".into(), Ty::Float)];
        let rep = w.apply_expression(&a, &out, &d.join("work"), &req, None).unwrap();
        assert_eq!(rep.voxels, 8000);
        assert_eq!(rep.written, vec!["out".to_string()]);
        assert!(!d.join("work").exists(), "temporary tiles are removed");
        let ts = export(&w, &out, "out", &d, "o.tiles");
        // density(i,j,k) = i+j+k, voxel size 0.5, so P.x = 0.5 i
        assert_eq!(at(&ts, [3, 4, 5]), 12.0 * 2.0 + 1.5);
        assert_eq!(at(&ts, [0, 0, 0]), 0.0);
        assert_eq!(at(&ts, [19, 19, 19]), 57.0 * 2.0 + 9.5);
        // the original grids are still there and unchanged
        assert_eq!(w.inspect(&out).unwrap().grids.len(), 3);
        assert_eq!(at(&export(&w, &out, "density", &d, "d.tiles"), [3, 4, 5]), 12.0);
        // a parameter value overrides the default
        let mut req2 = req.clone();
        req2.params = vec![("gain".into(), Value::Float(10.0))];
        w.apply_expression(&a, &out, &d.join("work"), &req2, None).unwrap();
        assert_eq!(at(&export(&w, &out, "out", &d, "o2.tiles"), [3, 4, 5]), 120.0 + 1.5);
    }

    #[test]
    fn real_worker_vec3_domain_and_new_vec3_output() {
        let Some(w) = real() else { return };
        let d = tmp("x2-vec3");
        let a = d.join("a.vdb");
        w.write_selftest(&a).unwrap();
        let out = d.join("out.vdb");
        let mut req = ExprRequest::new("reads @vel; writes @col; @col = @vel * 2.0;", "vel");
        req.outputs = vec![("col".into(), Ty::Vec3)];
        w.apply_expression(&a, &out, &d.join("work"), &req, None).unwrap();
        let ts = export(&w, &out, "col", &d, "c.tiles");
        assert_eq!(ts.channels, 3);
        let l = ts.leaves.iter().find(|l| l.origin == [0, 0, 0]).unwrap();
        let i = (1 << 6) | (2 << 3) | 3;
        assert_eq!(&l.values[i * 3..i * 3 + 3], &[2.0, 4.0, 6.0]);
    }

    #[test]
    fn real_worker_threads_do_not_change_the_result() {
        let Some(w) = real() else { return };
        let d = tmp("x2-threads");
        let a = d.join("a.vdb");
        w.write_selftest(&a).unwrap();
        let mut req = ExprRequest::new(
            "reads @density; writes @n; vec3 q = P * 0.37; @n = noise(q) + sqrt(@density - 20.0) + float(hash(int(ijk.x), int(ijk.y))) * 0.0;",
            "density",
        );
        req.outputs = vec![("n".into(), Ty::Float)];
        let mut files = Vec::new();
        for threads in [1usize, 2, 7, 16] {
            req.threads = threads;
            let out = d.join(format!("o{threads}.vdb"));
            let rep = w.apply_expression(&a, &out, &d.join("work"), &req, None).unwrap();
            assert_eq!(rep.threads, threads);
            assert!(rep.wrote_nan, "sqrt of a negative is NaN and is reported");
            files.push(export(&w, &out, "n", &d, &format!("n{threads}.tiles")).to_bytes());
        }
        for f in &files[1..] {
            assert_eq!(&files[0], f, "bit-identical for any thread count");
        }
    }

    #[test]
    fn real_worker_errors_are_values() {
        let Some(w) = real() else { return };
        let d = tmp("x2-errors");
        let a = d.join("a.vdb");
        w.write_selftest(&a).unwrap();
        let out = d.join("out.vdb");
        let run = |src: &str, tweak: &dyn Fn(&mut ExprRequest)| {
            let mut r = ExprRequest::new(src, "density");
            r.outputs = vec![("o".into(), Ty::Float)];
            tweak(&mut r);
            w.apply_expression(&a, &out, &d.join("work"), &r, None)
        };
        // fuel: the budget interrupts and the error names the voxel
        match run("reads @density; writes @o; float a = 0.0; for i in 0..100 { a = a + @density; } @o = a;", &|r| r.fuel = 50) {
            Err(Error::Expression(m)) => assert!(m.contains("R-FUEL") && m.contains("voxel"), "{m}"),
            other => panic!("{other:?}"),
        }
        // compile errors and run-time errors keep their stable codes
        assert!(matches!(run("@o = 1.0;", &|_| {}), Err(Error::Expression(m)) if m.contains("E-ATTR")));
        assert!(matches!(run("writes @o; @o = float(1 / int(P.x));", &|_| {}), Err(Error::Expression(m)) if m.contains("R-INT")));
        assert!(matches!(run("writes @o; @o = 1.0;", &|r| r.domain = "nope".into()), Err(Error::Expression(_))));
        assert!(matches!(run("writes @o; @o = 1.0;", &|r| r.params = vec![("zz".into(), Value::Float(1.0))]), Err(Error::Expression(_))));
        // the size limit
        assert!(matches!(run("writes @o; @o = 1.0;", &|r| r.max_bytes = 100), Err(Error::Limit(_))));
        // the time limit and cancel
        assert!(matches!(run("writes @o; @o = 1.0;", &|r| r.timeout = Duration::from_nanos(1)), Err(Error::Timeout)));
        let cancel = AtomicBool::new(true);
        let mut r = ExprRequest::new("writes @o; @o = 1.0;", "density");
        r.outputs = vec![("o".into(), Ty::Float)];
        assert!(matches!(w.apply_expression(&a, &out, &d.join("work"), &r, Some(&cancel)), Err(Error::Cancelled)));
        assert!(!d.join("work").exists(), "temporary tiles are removed after a failure too");
    }

    /// The Q-X2 size test (1 GiB of active voxels, 30 s): `CFA4_VDB_BIG=650` runs it (n^3 voxels); not a default test.
    #[test]
    fn real_worker_big_volume() {
        let Some(w) = real() else { return };
        let Some(n) = std::env::var("CFA4_VDB_BIG").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
        let d = tmp("x2-big");
        let a = d.join("big.vdb");
        w.clone().with_timeout(Duration::from_secs(600)).run(&["selftest-write-big".as_ref(), a.as_os_str(), n.to_string().as_ref()], &[], &[&a]).unwrap();
        let out = d.join("out.vdb");
        let mut req = ExprRequest::new("reads @big; writes @o; @o = @big * 2.0 + P.x;", "big");
        req.outputs = vec![("o".into(), Ty::Float)];
        req.max_bytes = u64::MAX;
        req.timeout = Duration::from_secs(600);
        let t = Instant::now();
        let rep = w.apply_expression(&a, &out, &d.join("work"), &req, None).unwrap();
        eprintln!("X2 big volume: {} voxels in {:?}, {} threads, fuel {}", rep.voxels, t.elapsed(), rep.threads, rep.fuel_used);
        assert_eq!(rep.voxels, u64::from(n).pow(3));
    }
}
