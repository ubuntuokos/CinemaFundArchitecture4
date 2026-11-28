//! `ingest.*` commands (plan `CFA4-TV-INGEST-RUNDOWN-001`, phase IN-1b): the recorder core of
//! [`cfa4_ingest`] as engine commands, so the GUI, the CLI, the control channel and MCP use the
//! same ones.
//!
//! Only the **synthetic** backend exists in this phase (`"backend": "synthetic"`, required): it
//! writes no file and records nothing real. The libobs (live studio S5) and DeckLink backends
//! connect to the same contract in their own phases; recovery, integrity and the File Services
//! states (IN-3, IN-4) are not here yet. Nothing in the project changes.

use cfa4_ingest::testsrc::TestRecorder;
use cfa4_ingest::{AudioTrackSpec, Container, IngestError, RecordHandle, RecordJob, Recorder, StagedTarget, Trigger, VideoTrackSpec};
use cfa4_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, str_p, u64_p};
use crate::{EngineError, Result, Session};

/// The recorder core's state in a session.
#[derive(Default)]
pub struct IngestState {
    rec: TestRecorder,
}

fn map(cmd: &str, e: IngestError) -> EngineError {
    bad(cmd, e.to_string())
}

fn synthetic(cmd: &str, p: &Value) -> Result<()> {
    match str_p(p, "backend") {
        Some("synthetic") => Ok(()),
        Some(other) => Err(bad(cmd, format!("backend `{other}` does not exist yet; only `synthetic` (no file is written)"))),
        None => Err(bad(cmd, "`backend` is required: only `synthetic` exists yet (no file is written)")),
    }
}

fn handle(cmd: &str, p: &Value) -> Result<RecordHandle> {
    u64_p(p, "handle").map(RecordHandle).ok_or_else(|| bad(cmd, "`handle` is required"))
}

fn status_json(s: &IngestState, h: RecordHandle) -> Value {
    let state = s.rec.state(h);
    let extent = s.rec.extent(h).ok();
    json!({
        "handle": h.0,
        "state": state,
        "extent": extent.map(|e| json!({"available": e.available.0, "frames": e.frames, "state": e.state})),
    })
}

fn start(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "ingest.start";
    synthetic(CMD, p)?;
    let path = str_p(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(CMD, "`path` is required"))?;
    let container = match str_p(p, "container").unwrap_or("mkv") {
        "mkv" => Container::Mkv,
        "mp4" => Container::Mp4,
        "fmp4" => Container::FragmentedMp4,
        "mov" => Container::Mov,
        "mxf" => Container::Mxf,
        other => return Err(bad(CMD, format!("unknown container `{other}` (mkv, mp4, fmp4, mov, mxf)"))),
    };
    let width = u32::try_from(u64_p(p, "width").unwrap_or(1920)).map_err(|_| bad(CMD, "`width` out of range"))?;
    let height = u32::try_from(u64_p(p, "height").unwrap_or(1080)).map_err(|_| bad(CMD, "`height` out of range"))?;
    let (num, den) = match (u64_p(p, "fpsNum"), u64_p(p, "fpsDen")) {
        (None, None) => (25, 1),
        (Some(n), Some(d)) => {
            (i64::try_from(n).map_err(|_| bad(CMD, "`fpsNum` out of range"))?, i64::try_from(d).map_err(|_| bad(CMD, "`fpsDen` out of range"))?)
        }
        _ => return Err(bad(CMD, "give both `fpsNum` and `fpsDen`, or neither")),
    };
    let channels = u16::try_from(u64_p(p, "audioChannels").unwrap_or(2)).map_err(|_| bad(CMD, "`audioChannels` out of range"))?;
    let audio = if channels == 0 { Vec::new() } else { vec![AudioTrackSpec { codec: "pcm".into(), channels, sample_rate: 48_000 }] };
    let job = RecordJob {
        target: StagedTarget { path: path.into() },
        container,
        video: vec![VideoTrackSpec { codec: "synthetic".into(), width, height, frame_rate: FrameRate { num, den } }],
        audio,
        split: None,
        start_timecode: p.get("startTimecode").and_then(Value::as_i64).map(Tick),
    };
    let h = s.ingest.rec.start(job).map_err(|e| map(CMD, e))?;
    Ok(status_json(&s.ingest, h))
}

fn stop(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "ingest.stop";
    let h = handle(CMD, p)?;
    let report = s.ingest.rec.stop(h).map_err(|e| map(CMD, e))?;
    let mut v = status_json(&s.ingest, h);
    v["report"] = serde_json::to_value(report).unwrap_or(Value::Null);
    Ok(v)
}

fn status(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "ingest.status";
    let h = handle(CMD, p)?;
    if s.ingest.rec.state(h).is_none() {
        return Err(bad(CMD, format!("no recording {}", h.0)));
    }
    let mut v = status_json(&s.ingest, h);
    let journal = s.ingest.rec.session(h).map(|x| x.journal().to_vec()).unwrap_or_default();
    v["journal"] = serde_json::to_value(journal).unwrap_or(Value::Null);
    Ok(v)
}

fn events(s: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = s.ingest.rec.events().iter().map(|e| json!({"name": e.name(), "event": e})).collect();
    Ok(json!({"events": list}))
}

fn manual(s: &mut Session, p: &Value, cmd: &str, trigger: Trigger) -> Result<Value> {
    let h = handle(cmd, p)?;
    let reason = str_p(p, "reason").unwrap_or("");
    s.ingest.rec.transition(h, trigger, reason).map_err(|e| map(cmd, e))?;
    Ok(status_json(&s.ingest, h))
}

fn quarantine(s: &mut Session, p: &Value) -> Result<Value> {
    manual(s, p, "ingest.quarantine", Trigger::Quarantine)
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    manual(s, p, "ingest.release", Trigger::Release)
}

fn synthetic_push(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "ingest.synthetic.push";
    let h = handle(CMD, p)?;
    let frames = u64_p(p, "frames").ok_or_else(|| bad(CMD, "`frames` is required"))?;
    s.ingest.rec.push_frames(h, frames, u64_p(p, "dropped").unwrap_or(0)).map_err(|e| map(CMD, e))?;
    Ok(status_json(&s.ingest, h))
}

fn synthetic_fault(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "ingest.synthetic.fault";
    let h = handle(CMD, p)?;
    let reason = str_p(p, "reason").unwrap_or("");
    s.ingest.rec.inject_fault(h, reason).map_err(|e| map(CMD, e))?;
    Ok(status_json(&s.ingest, h))
}

pub fn commands() -> Vec<CommandSpec> {
    let spec = |id: &'static str, label: &'static str, params: &'static str, run: crate::commands::Run, journal: bool| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params,
        enabled: always,
        run,
        journal,
    };
    vec![
        spec(
            "ingest.start",
            "Start Recording (synthetic backend)",
            r#"{"backend":"synthetic" (required),"path":str,"container":"mkv"|"mp4"|"fmp4"|"mov"|"mxf"="mkv","width":px=1920,"height":px=1080,"fpsNum":n?,"fpsDen":n?,"audioChannels":n=2 (0 = none),"startTimecode":ticks?}"#,
            start,
            false,
        ),
        spec("ingest.stop", "Stop Recording", r#"{"handle":n}"#, stop, false),
        spec("ingest.status", "Recording Status", r#"{"handle":n}"#, status, false),
        spec("ingest.events", "Recording Events", "{}", events, false),
        spec("ingest.quarantine", "Quarantine Recording", r#"{"handle":n,"reason":str (required)}"#, quarantine, false),
        spec("ingest.release", "Release Recording from Quarantine", r#"{"handle":n,"reason":str (required)}"#, release, false),
        spec("ingest.synthetic.push", "Synthetic Recording: Deliver Frames", r#"{"handle":n,"frames":n,"dropped":n=0}"#, synthetic_push, false),
        spec("ingest.synthetic.fault", "Synthetic Recording: Inject Fault", r#"{"handle":n,"reason":str (required)}"#, synthetic_fault, false),
    ]
}
