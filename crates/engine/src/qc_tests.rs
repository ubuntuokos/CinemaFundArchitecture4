//! Media QC commands: a queued export is checked automatically, the report and contact sheet land
//! next to it, the queue keeps the QC state apart from the render state, and the human review is
//! saved into the report (docs/media-qc.md).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::Session;

#[derive(Default)]
struct MemoryFiles(Mutex<BTreeMap<String, Vec<u8>>>);

impl crate::Services for MemoryFiles {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.0.lock().unwrap().get(path).cloned().ok_or_else(|| std::io::ErrorKind::NotFound.into())
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.0.lock().unwrap().insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn export_in_memory(&self) -> bool {
        true
    }
}

fn session() -> (Session, Arc<MemoryFiles>) {
    let files = Arc::new(MemoryFiles::default());
    let mut s = Session::new(files.clone());
    s.execute("file.newSequence", json!({"name":"QC", "fps":25, "width":64, "height":36})).unwrap();
    s.path = Some("/work/film.fcproj".into());
    (s, files)
}

fn result(v: Value) -> Value {
    v.get("result").cloned().unwrap_or(v)
}

#[test]
fn queued_export_is_checked_and_reviewed() {
    let (mut s, files) = session();
    let added = result(
        s.execute(
            "export.queue.add",
            json!({"format":"h264", "path":"/work/out.mp4", "range":"custom", "startTime":0, "endTime":cfa4_time::TICKS_PER_SECOND, "qc":"delivery-web", "start":true, "wait":true}),
        )
        .unwrap(),
    );
    let item = added["items"][0].clone();
    assert_eq!(item["status"], "done", "{item}");
    let list = result(s.execute("export.queue.list", json!({})).unwrap());
    let qc = &list["items"][0]["qc"];
    // an empty (black, silent) sequence: the render succeeded, QC fails it; the states stay apart
    assert_eq!(list["items"][0]["status"], "done");
    assert_eq!(qc["status"], "failed", "{qc}");
    assert_eq!(qc["report"], "/work/out.mp4.cfa4qc.json");
    assert!(files.0.lock().unwrap().contains_key("/work/out.mp4.cfa4qc.png"), "contact sheet stored next to the export");

    let r = result(s.execute("qc.report.get", json!({"path":"/work/out.mp4"})).unwrap());
    assert_eq!(r["verdict"], "FAIL");
    assert_eq!(r["contactSheet"], "/work/out.mp4.cfa4qc.png");
    let full = &r["full"];
    assert_eq!(full["render"]["status"], "done");
    assert_eq!(full["links"]["project"], "/work/film.fcproj");
    let get = |id: &str| full["checks"].as_array().unwrap().iter().find(|c| c["id"] == id).cloned().unwrap();
    // the render plan came from the queue item: 25 frames at 25 fps, 64x36, stereo
    assert_eq!(get("integrity.frameCount")["status"], "PASS", "{}", get("integrity.frameCount"));
    assert_eq!(get("integrity.streams")["status"], "PASS", "{}", get("integrity.streams"));
    assert_eq!(get("audio.loudness")["status"], "FAIL", "a silent mix has no programme loudness");
    assert_eq!(get("video.contactSheet")["detail"]["file"], "/work/out.mp4.cfa4qc.png");

    // the project index lists the report; the project file is not touched
    let idx = result(s.execute("qc.index", json!({})).unwrap());
    assert_eq!(idx["index"], "/work/film.cfa4qc/index.json");
    assert_eq!(idx["entries"][0]["media"], "/work/out.mp4");
    assert!(!files.0.lock().unwrap().contains_key("/work/film.fcproj"));

    // a time-coded human finding is saved into the report
    let review = json!({"state":"done","reviewer":"QC","answers":[{"question":"picture.defect","answer":"yes","note":"black"}],
        "findings":[{"description":"black picture","severity":"major","at":{"frame":12,"timecode":"00:00:00:12"},"note":""}]});
    let saved = result(s.execute("qc.review.save", json!({"path":"/work/out.mp4.cfa4qc.json", "review": review})).unwrap());
    assert_eq!(saved["verdict"], "FAIL");
    let r = result(s.execute("qc.report.get", json!({"path":"/work/out.mp4"})).unwrap());
    assert_eq!(r["full"]["human"]["findings"][0]["at"]["timecode"], "00:00:00:12");
    assert_eq!(r["full"]["human"]["state"], "done");
}

#[test]
fn qc_run_on_a_path_and_bad_input() {
    let (mut s, files) = session();
    s.execute(
        "file.exportMedia",
        json!({"format":"h264", "path":"/work/a.mp4", "range":"custom", "startTime":0, "endTime":cfa4_time::TICKS_PER_SECOND, "wait":true}),
    )
    .unwrap();
    let r = result(s.execute("qc.run", json!({"path":"/work/a.mp4", "wait":true, "expect":{"frames":25,"frameRate":{"num":25,"den":1}}})).unwrap());
    assert_eq!(r["report"], "/work/a.mp4.cfa4qc.json");
    assert!(r["summary"]["reasons"].as_array().is_some());
    // not waiting: the job is followed through jobs.list
    let r = result(s.execute("qc.run", json!({"path":"/work/a.mp4"})).unwrap());
    assert_eq!(r["poll"], "jobs.list");
    for _ in 0..2000 {
        s.poll_persistence();
        if s.qc.pending.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(s.qc.pending.is_empty());

    // errors, never panics
    assert!(s.execute("qc.run", json!({})).is_err());
    assert!(s.execute("qc.run", json!({"path":"/work/missing.mp4","wait":true})).is_err());
    assert!(s.execute("qc.run", json!({"path":"/work/a.mp4","profile":"nope"})).is_err());
    assert!(s.execute("qc.run", json!({"path":"/work/a.mp4","expect":{"frame":3}})).is_err());
    assert!(s.execute("qc.run", json!({"queueItem":99})).is_err());
    assert!(s.execute("qc.report.get", json!({"path":"/work/missing.mp4"})).is_err());
    assert!(s.execute("qc.review.save", json!({"path":"/work/a.mp4","review":{"state":"done","answers":[{"question":"x","answer":"yes"}]}})).is_err());
    // a hand-edited report is refused
    files.0.lock().unwrap().insert("/work/a.mp4.cfa4qc.json".into(), b"{\"schema\":\"cfa4-qc-report/1\"}".to_vec());
    assert!(s.execute("qc.report.get", json!({"path":"/work/a.mp4"})).is_err());
    assert!(s.execute("export.queue.add", json!({"format":"h264","path":"/work/b.mp4","qc":"nope"})).is_err());

    let p = result(s.execute("qc.profiles.list", json!({})).unwrap());
    assert_eq!(p["profiles"][0]["id"], "delivery-web");
}

fn settle(s: &mut Session) {
    for _ in 0..4000 {
        s.poll_persistence();
        if s.qc.pending.is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("QC did not finish");
}

/// Review of #317: a check finishing after another project was opened is filed in the index of
/// the project it was started in; a second check of the same file is refused while the first
/// runs (both would write one report).
#[test]
fn a_running_check_keeps_its_project_and_its_file() {
    let (mut s, files) = session();
    s.execute(
        "file.exportMedia",
        json!({"format":"h264", "path":"/work/a.mp4", "range":"custom", "startTime":0, "endTime":cfa4_time::TICKS_PER_SECOND, "wait":true}),
    )
    .unwrap();
    let first = result(s.execute("qc.run", json!({"path":"/work/a.mp4"})).unwrap());
    let err = s.execute("qc.run", json!({"path":"/work/a.mp4.cfa4qc.json", "profile":"delivery-web"})).unwrap_err().to_string();
    assert!(err.contains("already being checked"), "{err}");
    s.path = Some("/work/other.fcproj".into());
    settle(&mut s);
    let fs = files.0.lock().unwrap();
    assert!(fs.contains_key("/work/film.cfa4qc/index.json"), "filed where the check started");
    assert!(!fs.contains_key("/work/other.cfa4qc/index.json"));
    drop(fs);

    // a cancelled check no longer blocks a new one
    let job = first["job"].as_u64().unwrap();
    assert!(s.qc.pending.is_empty() && s.jobs.iter().any(|j| j.id == job));
    let again = result(s.execute("qc.run", json!({"path":"/work/a.mp4"})).unwrap());
    let j = again["job"].as_u64().unwrap();
    if let Some(job) = s.jobs.iter().find(|x| x.id == j) {
        job.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    assert!(s.execute("qc.run", json!({"path":"/work/a.mp4", "wait":true})).is_ok());
    settle(&mut s);
}

/// Review of #317: retrying a checked export clears its QC result (the profile stays) so the
/// new render is checked again instead of showing the old file's verdict.
#[test]
fn retrying_an_export_checks_the_new_render() {
    let (mut s, _files) = session();
    let added = result(
        s.execute(
            "export.queue.add",
            json!({"format":"h264", "path":"/work/r.mp4", "range":"custom", "startTime":0, "endTime":cfa4_time::TICKS_PER_SECOND, "qc":"delivery-web", "start":true, "wait":true}),
        )
        .unwrap(),
    );
    let id = added["items"][0]["id"].as_u64().unwrap();
    let qc = |s: &mut Session| result(s.execute("export.queue.list", json!({})).unwrap())["items"][0]["qc"].clone();
    assert_eq!(qc(&mut s)["status"], "failed");
    s.execute("export.queue.retry", json!({"id": id})).unwrap();
    let q = qc(&mut s);
    assert_eq!(q["status"], "notChecked", "{q}");
    assert_eq!(q["profile"], "delivery-web");
    assert!(q["report"].is_null());
    s.execute("export.queue.start", json!({"wait": true})).unwrap();
    settle(&mut s);
    let q = qc(&mut s);
    assert_eq!(q["status"], "failed", "{q}");
    assert_eq!(q["report"], "/work/r.mp4.cfa4qc.json");
}
