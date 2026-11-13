//! Media QC end to end (stage F, docs/plans/MEDIA-QC-2026-10-27.md): a real `cfa4-cli mcp`
//! process renders exports to disk, checks them, takes the human review and keeps the QC state
//! of each queued export, the way an agent or the Qt window drives it.
//!
//! What it proves: the render status and the QC status stay apart; only a PASS or WARN verdict
//! gives the "quality checked" state (`passed` / `warned` in `export.queue.list`); an automatic
//! FAIL is not cleared by a clean human review; a finished review that leaves a question open or
//! reports a problem without a finding is not a pass; and a report edited by hand is refused.
//!
//! No external tools: the media is CFA4's own render of a generated sequence, under
//! `target/qc-e2e-tests` (removed afterwards).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// A scratch directory under the workspace `target/`, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let d = cfa4_testkit::workspace_root().join("target").join("qc-e2e-tests").join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A minimal client for `cfa4-cli mcp` (newline-delimited JSON-RPC over stdio).
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(data_dir: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cfa4-cli"))
            .args(["--data-dir", data_dir, "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cfa4-cli mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp { child, stdin, stdout, next_id: 1 };
        let init = m.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "qc-e2e", "version": "1"}}));
        assert_eq!(init["result"]["serverInfo"]["name"], "cfa4", "{init}");
        m.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        m
    }

    fn send(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "the MCP server closed stdout while `{method}` was pending");
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC: {e}: {line}"));
            if v["id"] == id {
                return v;
            }
        }
    }

    /// `command_run`: the command's result, or `Err(message)` when the engine refused it.
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let r = self.request("tools/call", json!({"name": "command_run", "arguments": {"id": id, "params": params}}));
        assert!(r["error"].is_null(), "{id}: protocol error {r}");
        let text = r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
        if r["result"]["isError"] == true {
            return Err(text);
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.try_run(id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
    }

    /// The QC state of a queued export (`export.queue.list`), and its render status.
    fn states(&mut self, item: u64) -> (String, String) {
        let list = self.run("export.queue.list", json!({}));
        let it = list["items"].as_array().and_then(|a| a.iter().find(|i| i["id"] == item)).cloned().unwrap_or_else(|| panic!("no item {item}: {list}"));
        (it["status"].as_str().unwrap_or_default().to_string(), it["qc"]["status"].as_str().unwrap_or_default().to_string())
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A profile that checks what a black, silent render can pass (it opens, decodes, has the
/// planned frames), so the human review decides the verdict. The built-in `delivery-web` also
/// measures loudness, which a silent mix fails.
const STRUCTURE_PROFILE: &str = r#"
schema = "cfa4-qc-profile/1"
id = "e2e-structure"
version = 1
title = "E2E structure"
description = "Opens, decodes, has the planned frames; a person reviews it."
human_review = "mandatory"

[checks."probe.open"]
[checks."integrity.decode"]
[checks."integrity.frameCount"]
pass = { min = 0.0, max = 0.0 }
"#;

/// A finished review: every guided question answered, nothing wrong.
fn clean_review() -> Value {
    json!({"state": "done", "reviewer": "QC E2E", "answers": [
        {"question": "picture.defect", "answer": "no"},
        {"question": "picture.colorContrast", "answer": "yes"},
        {"question": "audio.distortion", "answer": "no"},
        {"question": "audio.noise", "answer": "no"},
    ]})
}

fn finding(severity: &str) -> Value {
    json!({"description": "flash frame", "severity": severity, "at": {"frame": 12, "timecode": "00:00:00:12"}})
}

/// One second of a 64×36, 25 fps sequence rendered to H.264 through the export queue.
fn export(m: &mut Mcp, path: &str, qc: Value) -> u64 {
    let added = m.run(
        "export.queue.add",
        json!({"format": "h264", "path": path, "range": "custom", "startTime": 0, "endTime": cfa4_time::TICKS_PER_SECOND, "qc": qc, "start": true, "wait": true}),
    );
    let id = added["added"][0].as_u64().unwrap_or_else(|| panic!("no item added: {added}"));
    let item = added["items"].as_array().and_then(|a| a.iter().find(|i| i["id"] == id)).cloned().unwrap_or_default();
    assert_eq!(item["status"], "done", "{added}");
    assert!(std::fs::metadata(path).is_ok_and(|f| f.len() > 0), "{path} was not written");
    id
}

#[test]
fn render_qc_review_gate_the_quality_checked_state() {
    let dir = Scratch::new("gate");
    let mut m = Mcp::start(&dir.path("data"));
    m.run("file.newSequence", json!({"name": "QC E2E", "fps": 25, "width": 64, "height": 36}));
    let profile = dir.path("structure.toml");
    std::fs::write(&profile, STRUCTURE_PROFILE).unwrap();

    // 1. delivery-web after export: the render is done, the automatic QC fails the silent mix,
    //    and a clean human review cannot clear an automatic FAIL
    let web = dir.path("web.mp4");
    let web_item = export(&mut m, &web, json!("delivery-web"));
    assert_eq!(m.states(web_item), ("done".into(), "failed".into()));
    let saved = m.run("qc.review.save", json!({"path": web, "review": clean_review()}));
    assert_eq!(saved["verdict"], "FAIL", "{saved}");
    assert_eq!(m.states(web_item), ("done".into(), "failed".into()), "a person cannot sign off an automatic FAIL");

    // 2. the structure profile on a second export: nothing is "quality checked" before a review
    let cut = dir.path("cut.mp4");
    let item = export(&mut m, &cut, Value::Null);
    assert_eq!(m.states(item), ("done".into(), "notChecked".into()));
    let ran = m.run("qc.run", json!({"queueItem": item, "profile": profile, "wait": true}));
    assert_eq!(ran["verdict"], "INCOMPLETE", "{ran}");
    assert_eq!(m.states(item).1, "incomplete");
    let report_path = format!("{cut}.cfa4qc.json");
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(on_disk["schema"], "cfa4-qc-report/1");
    assert_eq!(on_disk["profile"]["id"], "e2e-structure");
    assert_eq!(on_disk["render"]["status"], "done");
    let check = |r: &Value, id: &str| r["checks"].as_array().and_then(|a| a.iter().find(|c| c["id"] == id)).map(|c| c["status"].clone());
    assert_eq!(check(&on_disk, "integrity.decode"), Some(json!("PASS")), "{on_disk}");
    assert_eq!(check(&on_disk, "integrity.frameCount"), Some(json!("PASS")), "25 frames as planned: {on_disk}");

    // 3. a finished review that leaves a question open, or reports a problem without a finding,
    //    is not a pass
    let mut open = clean_review();
    open["answers"].as_array_mut().unwrap().pop();
    assert_eq!(m.run("qc.review.save", json!({"path": cut, "review": open}))["verdict"], "INCOMPLETE");
    assert_eq!(m.states(item).1, "incomplete");
    let mut flagged = clean_review();
    flagged["answers"][0]["answer"] = json!("yes");
    assert_eq!(m.run("qc.review.save", json!({"path": cut, "review": flagged}))["verdict"], "INCOMPLETE");
    assert_eq!(m.states(item).1, "incomplete");

    // 4. a clean, complete review: "quality checked"
    let passed = m.run("qc.review.save", json!({"path": cut, "review": clean_review()}));
    assert_eq!(passed["verdict"], "PASS", "{passed}");
    assert_eq!(m.states(item), ("done".into(), "passed".into()));

    // 5. the finding's severity decides: minor warns (still checked), critical fails
    flagged["findings"] = json!([finding("minor")]);
    assert_eq!(m.run("qc.review.save", json!({"path": cut, "review": flagged}))["verdict"], "WARN");
    assert_eq!(m.states(item).1, "warned");
    flagged["findings"] = json!([finding("critical")]);
    assert_eq!(m.run("qc.review.save", json!({"path": cut, "review": flagged}))["verdict"], "FAIL");
    assert_eq!(m.states(item), ("done".into(), "failed".into()), "the render stays done whatever QC says");
    let r = m.run("qc.report.get", json!({"path": cut}));
    assert_eq!(r["full"]["human"]["findings"][0]["at"]["timecode"], "00:00:00:12");

    // 6. a report edited to say PASS is refused, not trusted
    let mut forged: Value = serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    forged["summary"]["verdict"] = json!("PASS");
    std::fs::write(&report_path, serde_json::to_string_pretty(&forged).unwrap()).unwrap();
    assert!(m.try_run("qc.report.get", json!({"path": cut})).is_err(), "a forged report was read");
    assert!(m.try_run("qc.review.save", json!({"path": cut, "review": clean_review()})).is_err(), "a forged report was signed off");
    assert_eq!(m.states(item).1, "failed", "refusing the forged report leaves the state as it was");
}
