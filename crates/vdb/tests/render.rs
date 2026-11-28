//! N2: the density preview (`Worker::render`): quality levels, determinism, cancel, time and memory limits. Needs a
//! built worker (`CFA4_VDB_WORKER`); returns early without one.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cfa4_vdb::{Error, Quality, RenderRequest, Worker};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_cfa4-vdb-launch");

fn setup(name: &str) -> Option<(Worker, PathBuf, PathBuf)> {
    let w = Worker::discover().ok()?.with_launcher(LAUNCHER);
    let d = std::env::temp_dir().join(format!("cfa4-vdb-render-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    let vdb = d.join("t.vdb");
    match w.write_selftest(&vdb) {
        Err(Error::Reported(m)) if m.contains("cannot confine") => {
            eprintln!("SKIP: {m}");
            return None;
        }
        r => r.unwrap(),
    }
    Some((w, vdb, d))
}

#[test]
fn quality_levels_set_the_step_and_nothing_lowers_them_silently() {
    let Some((w, vdb, d)) = setup("quality") else { return };
    let mut steps = Vec::new();
    for q in [Quality::Preview, Quality::Standard, Quality::High] {
        let out = d.join(format!("{}.png", q.id()));
        let info = w.render(&vdb, &out, &RenderRequest::new(64, 48, q), None).unwrap();
        assert_eq!((info.width, info.height), (64, 48));
        assert_eq!(info.grid, "density");
        let png = fs::read(&out).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 64);
        steps.push(info.steps);
    }
    // half the step = about twice the samples; the level the user chose is the level that ran, twice in a row
    assert!(steps[0] < steps[1] && steps[1] < steps[2], "{steps:?}");
    let a = d.join("again1.png");
    let b = d.join("again2.png");
    let r = RenderRequest::new(64, 48, Quality::High);
    assert_eq!(w.render(&vdb, &a, &r, None).unwrap().steps, steps[2]);
    assert_eq!(w.render(&vdb, &b, &r, None).unwrap().steps, steps[2]);
}

#[test]
fn same_input_same_picture_for_any_thread_count() {
    let Some((w, vdb, d)) = setup("determinism") else { return };
    let r = RenderRequest::new(96, 72, Quality::Standard);
    let (a, b) = (d.join("a.png"), d.join("b.png"));
    w.render(&vdb, &a, &r, None).unwrap();
    w.render(&vdb, &b, &r, None).unwrap();
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap(), "two runs differ");
    // one thread against many: every pixel is independent, so the bytes are identical (no tolerance needed)
    let exe = std::env::var_os(cfa4_vdb::WORKER_ENV).unwrap();
    let one = d.join("one.png");
    let ok = Command::new(exe)
        .env("TBB_NUM_THREADS", "1")
        .args(["render".as_ref(), vdb.as_os_str(), "density".as_ref(), one.as_os_str(), "96".as_ref(), "72".as_ref(), "standard".as_ref()])
        .output()
        .unwrap();
    assert!(ok.status.success());
    assert_eq!(fs::read(&a).unwrap(), fs::read(&one).unwrap(), "thread count changed the picture");
}

#[test]
fn a_cancelled_render_stops_quickly() {
    let Some((w, vdb, d)) = setup("cancel") else { return };
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let r = std::thread::scope(|s| {
        s.spawn(|| {
            std::thread::sleep(Duration::from_millis(300));
            cancel.store(true, Ordering::Relaxed);
        });
        // large and slow: 4096 x 4096 at high quality
        w.render(&vdb, &d.join("big.png"), &RenderRequest::new(4096, 4096, Quality::High), Some(&cancel))
    });
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "cancel took {:?}", started.elapsed());
}

#[test]
fn the_time_limit_kills_the_worker() {
    let Some((w, vdb, d)) = setup("timeout") else { return };
    let mut r = RenderRequest::new(4096, 4096, Quality::High);
    r.timeout = Duration::from_millis(200);
    assert!(matches!(w.render(&vdb, &d.join("t.png"), &r, None), Err(Error::Timeout)));
}

#[test]
fn out_of_memory_is_an_error_not_a_crash() {
    let Some((_, vdb, d)) = setup("oom") else { return };
    // the worker caps its address space; ask for a picture that does not fit in 128 MiB
    let exe = std::env::var_os(cfa4_vdb::WORKER_ENV).unwrap();
    let out = d.join("huge.png");
    let r = Command::new(exe)
        .env("CFA4_VDB_MEM_LIMIT_MB", "128")
        .args(["render".as_ref(), vdb.as_os_str(), "-".as_ref(), out.as_os_str(), "8000".as_ref(), "8000".as_ref(), "preview".as_ref()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&r.stdout);
    assert_eq!(r.status.code(), Some(2), "{text}");
    assert!(text.contains("error"), "{text}");
}

#[test]
fn bad_requests_are_reported() {
    let Some((w, vdb, d)) = setup("bad") else { return };
    let mut r = RenderRequest::new(64, 48, Quality::Preview);
    r.grid = Some("vel".into());
    assert!(matches!(w.render(&vdb, &d.join("x.png"), &r, None), Err(Error::Reported(m)) if m.contains("not a float grid")));
    assert!(matches!(w.render(&vdb, &d.join("y.png"), &RenderRequest::new(0, 10, Quality::Preview), None), Err(Error::Reported(_))));
}
