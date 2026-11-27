//! DCP mastering with DCP-o-matic (ADR 0011), as commands every client can call.
//!
//! - `dcpomatic.status`: whether the DCP-o-matic build CFA4 ships is there, where, and its
//!   version.
//! - `dcpomatic.makeDcp`: makes a DCP from content files (typically a master exported with
//!   `file.exportMedia`, e.g. ProRes 4444 or a TIFF sequence plus WAV) and subtitle files as a
//!   background job (`jobs.list`, `jobs.cancel`), or at once with `wait`; with `encrypt` the track
//!   files are encrypted and the content keys stay in the film folder (`output`), which must be kept.
//! - `dcpomatic.makeKdm`: KDMs for such an encrypted DCP, for a projector certificate (plan
//!   CFA4-DCPOMATIC-001 D4, phase DCPOM-3).
//!
//! DCP-o-matic runs as separate processes (`cfa4-dcpomatic`); nothing of it is in the engine.
//! CFA4's own DCP export (`format: "dcp"`) stays the default.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, PoisonError};

use cfa4_dcpomatic::{
    Container, Content, ContentKind, DcpRequest, Dcpomatic, DcpomaticError, Formulation, KdmRequest, Phase, Resolution, Standard, Validity, ValidityUnit,
};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, checked_u32_p};
use crate::{EngineError, Result, Session};

/// The progress scale of a job: creating the film is the first 2 %, encoding the rest.
const STEPS: u64 = 1000;
const CREATE_SHARE: f32 = 0.02;

fn status(_: &mut Session, _: &Value) -> Result<Value> {
    let d = match Dcpomatic::locate() {
        Ok(d) => d,
        Err(e) => {
            return Ok(json!({
                "available": false,
                "minimumVersion": cfa4_dcpomatic::MIN_VERSION.to_string(),
                "dir": Dcpomatic::search_dir().display().to_string(),
                "error": e.to_string(),
            }));
        }
    };
    let (version, error) = match d.version() {
        Ok(v) => (Some(v.to_string()), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(json!({
        "available": version.is_some(),
        "version": version,
        "minimumVersion": cfa4_dcpomatic::MIN_VERSION.to_string(),
        "dir": Dcpomatic::search_dir().display().to_string(),
        "create": d.create_program().display().to_string(),
        "cli": d.cli_program().display().to_string(),
        "kdm": d.kdm_program().map(|k| k.display().to_string()),
        "error": error,
    }))
}

fn enum_p<T: serde::de::DeserializeOwned>(cmd: &str, p: &Value, k: &str) -> Result<Option<T>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|_| bad(cmd, format!("{k}: {v} is not one of the allowed values"))),
    }
}

/// A whole number: fractions, negatives and values over `u32::MAX` are refused, never truncated.
fn u32_p(cmd: &str, p: &Value, k: &str) -> Result<Option<u32>> {
    checked_u32_p(p, k, cmd)
}

/// A string; only a missing or `null` parameter is absent, anything else (`42`, `[]`) is refused,
/// so a mistyped value never falls back to a default.
fn text_p<'a>(cmd: &str, p: &'a Value, k: &str) -> Result<Option<&'a str>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(v) => Err(bad(cmd, format!("{k}: {v} is not a string"))),
    }
}

/// `true` or `false`; anything else that is there (`"true"`, `1`) is refused, so a mistyped
/// flag is never read as its default.
fn flag_p(cmd: &str, p: &Value, k: &str, default: bool) -> Result<bool> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(v) => Err(bad(cmd, format!("{k}: {v} is not true or false"))),
    }
}

/// The film folders of the DCP-o-matic jobs that are running: two jobs never write into the
/// same folder, and one job's clean-up never removes another's.
static FILM_DIRS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// A film folder held by one job until it is dropped (finished, failed, cancelled, or never
/// started).
struct FilmDirClaim(PathBuf);

impl FilmDirClaim {
    fn take(cmd: &str, dir: &Path) -> Result<Self> {
        let key = film_dir_key(dir);
        let mut held = FILM_DIRS.lock().unwrap_or_else(PoisonError::into_inner);
        // the same folder, or one inside the other: a cancelled job removes its whole folder
        if held.iter().any(|h| h.starts_with(&key) || key.starts_with(h)) {
            return Err(bad(cmd, format!("output: another DCP-o-matic job is already writing into or around {}", dir.display())));
        }
        held.insert(key.clone());
        Ok(Self(key))
    }
}

/// One key per folder on disk, however it is written: the deepest existing ancestor is
/// canonicalized (so `..` and symbolic links resolve), the not yet existing rest is appended with
/// `.` dropped and `..` taken off lexically.
fn film_dir_key(dir: &Path) -> PathBuf {
    use std::path::Component;
    let abs = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut base = abs.as_path();
    let mut rest = Vec::new();
    let mut key = loop {
        if let Ok(c) = base.canonicalize() {
            break c;
        }
        match (base.parent(), base.components().next_back()) {
            (Some(parent), Some(last)) => {
                rest.push(last);
                base = parent;
            }
            _ => break base.to_path_buf(),
        }
    };
    for c in rest.into_iter().rev() {
        match c {
            Component::ParentDir => {
                key.pop();
            }
            Component::Normal(n) => key.push(n),
            _ => {}
        }
    }
    key
}

impl Drop for FilmDirClaim {
    fn drop(&mut self) {
        FILM_DIRS.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.0);
    }
}

/// `content`: a path, or a list of paths and `{path, eye?, channel?, gainDb?}` objects.
fn content_p(cmd: &str, p: &Value) -> Result<Vec<Content>> {
    let one = |v: &Value| -> Result<Content> {
        match v {
            Value::String(s) => Ok(Content::new(s)),
            Value::Object(_) => serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("content: {e}"))),
            _ => Err(bad(cmd, "content: each entry is a path or an object with `path`")),
        }
    };
    match p.get("content") {
        Some(Value::Array(a)) => a.iter().map(one).collect(),
        Some(v @ (Value::String(_) | Value::Object(_))) => Ok(vec![one(v)?]),
        _ => Err(bad(cmd, "content: the content files are required")),
    }
}

pub(crate) fn request_from_params(cmd: &str, p: &Value) -> Result<DcpRequest> {
    let content = content_p(cmd, p)?;
    let film = text_p(cmd, p, "output")?.ok_or_else(|| bad(cmd, "output: the new DCP-o-matic film folder is required (the DCP is written inside it)"))?;
    let name = match text_p(cmd, p, "name")? {
        Some(n) => n.to_string(),
        None => Path::new(film).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "DCP".into()),
    };
    let mut r = DcpRequest::new(content, film, name);
    r.content_kind = enum_p::<ContentKind>(cmd, p, "contentKind")?.unwrap_or_default();
    r.standard = enum_p::<Standard>(cmd, p, "standard")?.unwrap_or_default();
    r.resolution = enum_p::<Resolution>(cmd, p, "resolution")?;
    r.container = enum_p::<Container>(cmd, p, "container")?;
    r.frame_rate = u32_p(cmd, p, "fps")?;
    r.audio_channels = u32_p(cmd, p, "audioChannels")?;
    r.video_bit_rate_mbps = u32_p(cmd, p, "videoBitRateMbps")?;
    r.three_d = flag_p(cmd, p, "threeD", false)?;
    r.encrypt = flag_p(cmd, p, "encrypt", false)?;
    r.subtitles = match p.get("subtitles") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![PathBuf::from(s)],
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(PathBuf::from).ok_or_else(|| bad(cmd, "subtitles: a list of subtitle file paths"))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(cmd, "subtitles: a list of subtitle file paths")),
    };
    r.isdcf_name = flag_p(cmd, p, "isdcfName", true)?;
    r.threads = u32_p(cmd, p, "threads")?;
    r.config_dir = text_p(cmd, p, "configDir")?.map(PathBuf::from);
    Ok(r)
}

pub(crate) fn kdm_request_from_params(cmd: &str, p: &Value) -> Result<KdmRequest> {
    let need = |k: &str, what: &str| text_p(cmd, p, k)?.map(PathBuf::from).ok_or_else(|| bad(cmd, format!("{k}: {what} is required")));
    let validity = match (p.get("validFor"), text_p(cmd, p, "validUntil")?) {
        (Some(v), None) if !v.is_null() => {
            let count = u32_p(cmd, v, "count")?.ok_or_else(|| bad(cmd, "validFor.count is required"))?;
            let unit = enum_p::<ValidityUnit>(cmd, v, "unit")?.ok_or_else(|| bad(cmd, "validFor.unit is required (hours|days|weeks|years)"))?;
            Validity::For { count, unit }
        }
        (None | Some(Value::Null), Some(t)) => Validity::Until(t.to_string()),
        _ => return Err(bad(cmd, "give exactly one of validFor {count, unit} and validUntil")),
    };
    let trusted_devices = match p.get("trustedDevices") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map(PathBuf::from).ok_or_else(|| bad(cmd, "trustedDevices: a list of certificate file paths")))
            .collect::<Result<_>>()?,
        Some(_) => return Err(bad(cmd, "trustedDevices: a list of certificate file paths")),
    };
    Ok(KdmRequest {
        film_dir: need("film", "the DCP-o-matic film folder of the encrypted DCP")?,
        projector_certificate: need("certificate", "the projector's certificate file")?,
        output: need("output", "the folder for the KDM files")?,
        cinema: text_p(cmd, p, "cinema")?.map(str::to_string),
        screen: text_p(cmd, p, "screen")?.map(str::to_string),
        valid_from: text_p(cmd, p, "validFrom")?.filter(|v| *v != "now").map(str::to_string),
        validity,
        formulation: enum_p::<Formulation>(cmd, p, "formulation")?.unwrap_or_default(),
        trusted_devices,
        disable_forensic_marking_picture: flag_p(cmd, p, "disableForensicMarkingPicture", false)?,
        disable_forensic_marking_audio: flag_p(cmd, p, "disableForensicMarkingAudio", false)?,
        config_dir: text_p(cmd, p, "configDir")?.map(PathBuf::from),
    })
}

/// KDMs are quick (seconds), so the command waits for them.
fn make_kdm(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "dcpomatic.makeKdm";
    let request = kdm_request_from_params(CMD, p)?;
    let dcpomatic = Dcpomatic::locate().map_err(|e| bad(CMD, format!("{e}; see docs/development/dcpomatic.md")))?;
    let out = dcpomatic.make_kdm(&request).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(json!({"kdms": out.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>()}))
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

fn make_dcp(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "dcpomatic.makeDcp";
    let request = request_from_params(CMD, p)?;
    let film_dir = std::path::absolute(&request.film_dir).unwrap_or_else(|_| request.film_dir.clone()).display().to_string();
    let dcpomatic = Dcpomatic::locate().map_err(|e| bad(CMD, format!("{e}; see docs/development/dcpomatic.md")))?;
    let wait = flag_p(CMD, p, "wait", false)?;
    let claim = FilmDirClaim::take(CMD, &request.film_dir)?;
    // the content keys of an encrypted DCP live only in the film folder: every KDM is made from it
    let keep = if request.encrypt {
        json!({"encrypted": true, "keepFilmDir": "the content keys are in this folder; keep it, every KDM is made from it"})
    } else {
        json!({"encrypted": false})
    };
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label: format!("DCP-o-matic: {}", request.name), progress: Default::default(), result: Default::default() };
    job.progress.total.store(STEPS, Ordering::Relaxed);
    let (prog, res) = (job.progress.clone(), job.result.clone());
    let run = move || {
        let _claim = claim;
        let t0 = web_time::Instant::now();
        let (pc, pp) = (prog.clone(), prog.clone());
        let control = cfa4_dcpomatic::Control::new().with_cancel(move || pc.cancel.load(Ordering::Relaxed)).with_progress(move |phase, f| {
            let (overall, text) = match phase {
                Phase::Create => (f * CREATE_SHARE, "DCP-o-matic: creating the film".to_string()),
                Phase::Encode => (CREATE_SHARE + f * (1.0 - CREATE_SHARE), format!("DCP-o-matic: making the DCP, {:.0} %", f * 100.0)),
                Phase::Kdm => (1.0, "DCP-o-matic: making KDMs".to_string()),
            };
            pp.done.store((overall * STEPS as f32) as u64, Ordering::Relaxed);
            *pp.status.lock().unwrap_or_else(PoisonError::into_inner) = text;
        });
        let r = dcpomatic.make_dcp(&request, &control).map_err(|e| match e {
            DcpomaticError::Cancelled => "cancelled".to_string(),
            e => e.to_string(),
        });
        let r = r.map(|out| {
            *prog.status.lock().unwrap_or_else(PoisonError::into_inner) = format!("DCP-o-matic {}: {}", out.version, out.dcp.display());
            prog.done.store(STEPS, Ordering::Relaxed);
            cfa4_export::Report {
                path: out.dcp.display().to_string(),
                frames: 0,
                seconds: t0.elapsed().as_secs_f64(),
                bytes: dir_bytes(&out.dcp),
                render_fps: 0.0,
                extra_files: vec![out.film_dir.display().to_string()],
            }
        });
        if let Err(e) = &r {
            *prog.status.lock().unwrap_or_else(PoisonError::into_inner) = e.clone();
            *prog.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.clone());
        }
        prog.finished.store(true, Ordering::Relaxed);
        *res.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    let result = job.result.clone();
    if wait {
        s.jobs.push(job);
        run();
        let r = result.lock().unwrap_or_else(PoisonError::into_inner).clone();
        return match r {
            Some(Ok(rep)) => {
                let mut out = json!({"job": id, "dcp": rep.path, "filmDir": rep.extra_files.first(), "bytes": rep.bytes});
                merge(&mut out, keep);
                Ok(out)
            }
            Some(Err(e)) => Err(bad(CMD, e)),
            None => Err(bad(CMD, "the job did not finish")),
        };
    }
    // The job is listed only once its thread runs, so a failed start leaves no job without a runner.
    std::thread::Builder::new().name("dcpomatic".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    s.jobs.push(job);
    let mut out = json!({"job": id, "filmDir": film_dir});
    merge(&mut out, keep);
    Ok(out)
}

fn merge(into: &mut Value, from: Value) {
    if let (Value::Object(a), Value::Object(b)) = (into, from) {
        a.extend(b);
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "dcpomatic.status",
            label: "DCP-o-matic Status",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: status,
            journal: false,
        },
        CommandSpec {
            id: "dcpomatic.makeDcp",
            label: "Make DCP with DCP-o-matic…",
            menu: &[],
            shortcut: None,
            params: r#"{"content":str|[str|{"path":str,"eye":"left|right"?,"channel":"L|R|C|Lfe|Ls|Rs|BsL|BsR|HI|VI"?,"gainDb":f64?}],"output":str,"name":str?,"contentKind":"feature|short|trailer|test|transitional|rating|teaser|policy|psa|advertisement"=test,"standard":"smpte|interop"=smpte,"resolution":"2k|4k"?,"container":"flat|scope|1.19|1.33|1.37|1.38|1.66|1.78"?,"fps":24|25|30|48|50|60?,"audioChannels":u32?,"videoBitRateMbps":u32?,"threeD":bool=false,"encrypt":bool=false,"subtitles":str|[str]?,"isdcfName":bool=true,"threads":u32?,"configDir":str?,"wait":bool=false}"#,
            enabled: always,
            run: make_dcp,
            journal: false,
        },
        CommandSpec {
            id: "dcpomatic.makeKdm",
            label: "Make KDM with DCP-o-matic…",
            menu: &[],
            shortcut: None,
            params: r#"{"film":str,"certificate":str,"output":str,"cinema":str?,"screen":str?,"validFrom":"now|YYYY-MM-DD HH:MM:SS (UTC)"=now,"validFor":{"count":u32,"unit":"hours|days|weeks|years"}?,"validUntil":"YYYY-MM-DD HH:MM:SS (UTC)"?,"formulation":"modifiedTransitional1|multipleModifiedTransitional1|dciAny|dciSpecific"=modifiedTransitional1,"trustedDevices":[str]?,"disableForensicMarkingPicture":bool=false,"disableForensicMarkingAudio":bool=false,"configDir":str?}"#,
            enabled: always,
            run: make_kdm,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "dcpomatic_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dcpomatic_real_tests.rs"]
mod real_tests;
