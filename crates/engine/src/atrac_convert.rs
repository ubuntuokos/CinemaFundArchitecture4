//! The AT9/AT3 converter (Tools ▸ Audio Tools ▸ AT9/AT3 Converter…, owner request 2026-11-14):
//! Sony's ATRAC audio files to WAV and back, in CFA4's own code. It replaces BMK-Studio's
//! AT9-AT3_Converter, which runs Sony's SDK tools.
//!
//! - **To WAV:** any audio file CFA4 reads (AT3 with ATRAC3 or ATRAC3plus, AT9, OMA, MP3, WAV, FLAC…)
//!   decoded by our decoders (`cfa4-realaudio` for ATRAC) into 16-bit, 24-bit or 32-bit float WAV,
//!   at the source's rate and channels.
//! - **To AT3:** the same inputs encoded as ATRAC3 (`cfa4-atracenc`) at 66 (LP4, joint stereo),
//!   105 or 132 (LP2) kbit/s, resampled to 44.1 kHz; mono stays mono, more than two channels keep
//!   the front left and right.
//!
//! Encoding ATRAC3plus and ATRAC9 (`.at9`) is not available yet (separate backlog items).
//!
//! Command: `audio.atracConvert` (a background job in `jobs.list`; `wait` runs it in place).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use cfa4_atracenc::atrac3;
use cfa4_export::{Progress, SampleFormat};
use cfa4_time::TICKS_PER_SECOND;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p, u64_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "audio.atracConvert";
/// Frames read from the source at a time.
const CHUNK: usize = 1 << 16;
/// Longest source converted (6 hours): beyond it the samples would not fit in memory comfortably
/// and AT3 files hit RIFF's 4 GiB limit anyway.
const MAX_SECONDS: i64 = 6 * 3600;

/// The output of a conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// WAV at the source's rate and channels.
    Wav(SampleFormat),
    /// ATRAC3 in an AT3 file.
    At3(atrac3::Mode),
}

/// What to convert.
#[derive(Clone, Debug)]
pub struct Request {
    pub input: PathBuf,
    pub output: PathBuf,
    pub target: Target,
}

pub(crate) fn request_from_params(p: &Value) -> Result<Request> {
    let input = str_p(p, "input").ok_or_else(|| bad(CMD, "input: the audio file to convert is required"))?;
    let output = str_p(p, "output").ok_or_else(|| bad(CMD, "output: the file to write is required"))?;
    let format = match str_p(p, "format") {
        Some(f) => f.to_ascii_lowercase(),
        None => Path::new(output).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default(),
    };
    let target = match format.as_str() {
        "wav" | "wave" => Target::Wav(match u64_p(p, "bits") {
            None | Some(16) => SampleFormat::Int16,
            Some(24) => SampleFormat::Int24,
            Some(32) => SampleFormat::Float32,
            Some(b) => return Err(bad(CMD, format!("bits: {b} (16, 24 or 32 for float)"))),
        }),
        "at3" => {
            let kbps = u64_p(p, "bitrateKbps").unwrap_or(132);
            let mode =
                atrac3::MODES.iter().copied().find(|m| u64::from(m.kbps()) == kbps || (kbps == 64 && m.kbps() == 66) || (kbps == 128 && m.kbps() == 132));
            Target::At3(mode.ok_or_else(|| bad(CMD, format!("bitrateKbps: {kbps} (ATRAC3 has 66, 105 and 132)")))?)
        }
        "at9" => return Err(bad(CMD, "format: encoding ATRAC9 (.at9) is not available yet; AT9 files convert to WAV")),
        f => return Err(bad(CMD, format!("format: `{f}` (wav or at3)"))),
    };
    if Path::new(input) == Path::new(output) {
        return Err(bad(CMD, "output: the output must be another file than the input"));
    }
    Ok(Request { input: input.into(), output: output.into(), target })
}

/// What a conversion did.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub bytes: u64,
    pub seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec_in: String,
}

/// Converts `r`; `progress` gets the fraction done and stops the conversion when it returns
/// `false` (then the result is `Err("cancelled")`). The output is written to a temporary file next
/// to it and renamed, so a failed conversion leaves no partial file.
pub fn convert(r: &Request, progress: &mut dyn FnMut(f32, &str) -> bool) -> std::result::Result<Outcome, String> {
    let src = cfa4_media::open_path(&r.input, &cfa4_codecs::openers()).map_err(|e| format!("{}: {e}", r.input.display()))?;
    let info = src.info().clone();
    let stream = info.audio_streams.first().ok_or_else(|| format!("{}: no audio", r.input.display()))?;
    if stream.sample_rate == 0 || stream.channels == 0 {
        return Err(format!("{}: the audio has no sample rate or channels", r.input.display()));
    }
    let src_channels = stream.channels as usize;
    let (rate, out_channels) = match r.target {
        Target::Wav(_) => (stream.sample_rate, src_channels.min(usize::from(u16::MAX))),
        Target::At3(_) => (atrac3::SAMPLE_RATE, src_channels.min(2)),
    };
    let dur = info.duration.0.clamp(0, MAX_SECONDS * TICKS_PER_SECOND);
    let total = usize::try_from((i128::from(dur) * i128::from(rate) + i128::from(TICKS_PER_SECOND) - 1) / i128::from(TICKS_PER_SECOND)).unwrap_or(0);
    if total == 0 {
        return Err(format!("{}: the audio is empty", r.input.display()));
    }
    let share = if matches!(r.target, Target::At3(_)) { 0.5 } else { 0.9 };
    let mut pcm = Vec::with_capacity(total.saturating_mul(out_channels).min(1 << 30));
    let mut at = 0usize;
    while at < total {
        let n = CHUNK.min(total - at);
        let buf = src.audio(at as i64, n, rate).map_err(|e| format!("{}: {e}", r.input.display()))?;
        let got = buf.frames().min(n);
        if got == 0 {
            break;
        }
        for i in 0..got {
            for c in 0..out_channels {
                pcm.push(buf.channels.get(c).and_then(|ch| ch.get(i)).copied().unwrap_or(0.0));
            }
        }
        at += got;
        if !progress(share * at as f32 / total as f32, "decoding") {
            return Err("cancelled".into());
        }
    }
    let channels = u16::try_from(out_channels).map_err(|_| "too many channels".to_string())?;
    let bytes = match r.target {
        Target::Wav(fmt) => cfa4_export::write_wav(&pcm, channels, rate, fmt)?,
        Target::At3(mode) => {
            let mut cancelled = false;
            let file = cfa4_atracenc::riff::encode_at3(&pcm, channels, mode, &mut |f| {
                cancelled = !progress(share + (1.0 - share) * f, "encoding ATRAC3");
                !cancelled
            })
            .map_err(|e| e.to_string())?;
            file.ok_or_else(|| "cancelled".to_string())?
        }
    };
    let tmp = temp_path(&r.output);
    std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, &r.output) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", r.output.display()));
    }
    progress(1.0, "done");
    Ok(Outcome { bytes: bytes.len() as u64, seconds: at as f64 / f64::from(rate), sample_rate: rate, channels, codec_in: stream.codec.clone() })
}

fn temp_path(out: &Path) -> PathBuf {
    let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    out.with_file_name(format!(".{name}.cfa4-partial"))
}

const STEPS: u64 = 1000;

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let request = request_from_params(p)?;
    let wait = bool_p(p, "wait").unwrap_or(false);
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let name = request.input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let label = match request.target {
        Target::Wav(_) => format!("AT9/AT3 Converter: {name} → WAV"),
        Target::At3(m) => format!("AT9/AT3 Converter: {name} → AT3 {} kbit/s", m.kbps()),
    };
    let job = crate::Job { id, label, progress: Default::default(), result: Default::default() };
    job.progress.total.store(STEPS, Ordering::Relaxed);
    let (prog, res): (Arc<Progress>, _) = (job.progress.clone(), job.result.clone());
    let req = request.clone();
    let work = move || {
        let t0 = web_time::Instant::now();
        let pp = prog.clone();
        let r = convert(&req, &mut |f, what| {
            pp.done.store((f.clamp(0.0, 1.0) * STEPS as f32) as u64, Ordering::Relaxed);
            *pp.status.lock().unwrap_or_else(PoisonError::into_inner) = format!("{what}, {:.0} %", f * 100.0);
            !pp.cancel.load(Ordering::Relaxed)
        });
        let r = r.map(|o| {
            *prog.status.lock().unwrap_or_else(PoisonError::into_inner) =
                format!("{}: {:.1} s, {} Hz, {} ch, {} bytes", req.output.display(), o.seconds, o.sample_rate, o.channels, o.bytes);
            prog.done.store(STEPS, Ordering::Relaxed);
            cfa4_export::Report {
                path: req.output.display().to_string(),
                frames: 0,
                seconds: t0.elapsed().as_secs_f64(),
                bytes: o.bytes,
                render_fps: 0.0,
                extra_files: Vec::new(),
            }
        });
        if let Err(e) = &r {
            *prog.status.lock().unwrap_or_else(PoisonError::into_inner) = e.clone();
            *prog.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.clone());
        }
        prog.finished.store(true, Ordering::Relaxed);
        *res.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
    };
    let work = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), work);
    let result = job.result.clone();
    if wait {
        s.jobs.push(job);
        work();
        let r = result.lock().unwrap_or_else(PoisonError::into_inner).clone();
        return match r {
            Some(Ok(rep)) => Ok(json!({"job": id, "output": rep.path, "bytes": rep.bytes})),
            Some(Err(e)) => Err(bad(CMD, e)),
            None => Err(bad(CMD, "the job did not finish")),
        };
    }
    std::thread::Builder::new().name("atrac-convert".into()).spawn(work).map_err(|e| EngineError::Other(e.to_string()))?;
    s.jobs.push(job);
    Ok(json!({"job": id}))
}

/// The formats and rates the converter offers (for the dialog).
fn formats(_s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(json!({
        "inputs": ["at3", "at9", "oma", "omg", "aa3", "wav", "mp3", "flac", "ogg", "m4a"],
        "outputs": [
            {"format": "wav", "label": "WAV (PCM)", "bits": [16, 24, 32]},
            {"format": "at3", "label": "AT3 (ATRAC3, PSP / PS3)", "bitrateKbps": atrac3::MODES.iter().map(|m| m.kbps()).collect::<Vec<_>>(), "default": 132},
        ],
        "notYet": ["AT3 with ATRAC3plus", "AT9 (ATRAC9)", "MP3"],
    }))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD,
            label: "AT9/AT3 Converter…",
            menu: &[],
            shortcut: None,
            params: r#"{"input":str,"output":str,"format":"wav|at3"?,"bitrateKbps":66|105|132=132,"bits":16|24|32=16,"wait":bool=false}"#,
            enabled: always,
            run,
            journal: false,
        },
        CommandSpec {
            id: "audio.atracConvert.formats",
            label: "AT9/AT3 Converter Formats",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: formats,
            journal: false,
        },
    ]
}

#[cfg(test)]
#[path = "atrac_convert_tests.rs"]
mod tests;
