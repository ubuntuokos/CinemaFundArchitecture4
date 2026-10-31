//! `cfa4-cli qc`: check a media file with a QC profile and print or save the report.
//!
//! The report is written atomically (`--out`), so an interrupted run never leaves a half-written
//! report behind. The exit code says the verdict: 0 PASS, 3 WARN, 4 FAIL, 5 INCOMPLETE (1 is an
//! error, 2 a usage error, as for every subcommand).

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use cfa4_qc::report::{ComputeMode, RunInfo};
use cfa4_qc::{Expectation, Options, QcProfile, QcReport, Verdict};

/// Exit code for a verdict.
pub fn exit_code(v: Verdict) -> i32 {
    match v {
        Verdict::Pass => 0,
        Verdict::Warn => 3,
        Verdict::Fail => 4,
        Verdict::Incomplete => 5,
    }
}

/// A built-in profile id, or a profile file (`.toml`).
fn profile(arg: Option<&str>) -> Result<QcProfile, String> {
    let arg = arg.unwrap_or("delivery-web");
    if arg.ends_with(".toml") {
        let text = std::fs::read_to_string(arg).map_err(|e| format!("{arg}: {e}"))?;
        return QcProfile::from_toml(&text).map_err(|e| format!("{arg}: {e}"));
    }
    QcProfile::builtin(arg).map_err(|e| {
        let ids: Vec<&str> = cfa4_qc::profile::BUILTIN.iter().map(|(id, _)| *id).collect();
        format!("{e} (built-in: {}; or a .toml file)", ids.join(", "))
    })
}

fn expectation(arg: Option<&str>) -> Result<Expectation, String> {
    let Some(path) = arg else { return Ok(Expectation::default()) };
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    Expectation::from_json(&text).map_err(|e| format!("{path}: {e}"))
}

/// Whether two paths name one file (resolving links and `..`; a sheet that does not exist yet is
/// compared by its folder).
fn same_file(a: &str, b: &str) -> bool {
    let canon = |p: &str| {
        let p = Path::new(p);
        p.canonicalize().ok().or_else(|| {
            let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
            Some(dir.canonicalize().ok()?.join(p.file_name()?))
        })
    };
    match (canon(a), canon(b)) {
        (Some(x), Some(y)) => x == y,
        _ => Path::new(a) == Path::new(b),
    }
}

/// Run the QC. `decode`: false skips the full decode (integrity, audio and picture checks report
/// NOT_RUN). `sheet`: where to write the contact sheet PNG (its path goes into the report).
pub fn run(media: &str, profile_arg: Option<&str>, expect_arg: Option<&str>, decode: bool, sheet: Option<&str>) -> Result<QcReport, String> {
    if let Some(path) = sheet
        && same_file(path, media)
    {
        return Err(format!("{path}: the contact sheet would overwrite the file being checked; name another file"));
    }
    let profile = profile(profile_arg)?;
    let expect = expectation(expect_arg)?;
    let started = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let t0 = Instant::now();
    let tool_path = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let opts = Options { decode, run: RunInfo { started_unix_ms: started, tool_path, compute: ComputeMode::Cpu, ..RunInfo::default() }, ..Options::default() };
    let checked = cfa4_qc::check_file_with_outputs(Path::new(media), &profile, &expect, opts, &AtomicBool::new(false)).map_err(|e| e.to_string())?;
    let mut report = checked.report;
    if let (Some(path), Some(png)) = (sheet, checked.contact_sheet) {
        cfa4_format::atomic::atomic_write(Path::new(path), &png).map_err(|e| format!("{path}: {e}"))?;
        if let Some(r) = report.checks.iter_mut().find(|r| r.id == "video.contactSheet") {
            r.detail.insert("file".into(), serde_json::json!(path));
        }
    }
    report.run.duration_ms = t0.elapsed().as_millis() as u64;
    Ok(report)
}

/// Write the report to `path` atomically.
pub fn save(report: &QcReport, path: &str) -> Result<(), String> {
    let json = report.to_json().map_err(|e| e.to_string())?;
    cfa4_format::atomic::atomic_write(Path::new(path), json.as_bytes()).map_err(|e| format!("{path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review of #306: `--contact-sheet` naming the checked file is refused before anything is
    /// written, however the path is spelled.
    #[test]
    fn contact_sheet_cannot_overwrite_the_media() {
        let dir = std::env::temp_dir().join(format!("cfa4-qc-sheet-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let media = dir.join("clip.mp4");
        std::fs::write(&media, b"not really a movie").unwrap();
        let m = media.to_string_lossy().into_owned();
        let other = dir.join("sub").join("..").join("clip.mp4").to_string_lossy().into_owned();
        assert!(same_file(&m, &m) && same_file(&other, &m));
        assert!(!same_file(&dir.join("clip.png").to_string_lossy(), &m));
        let err = run(&m, None, None, false, Some(&other)).unwrap_err();
        assert!(err.contains("would overwrite"), "{err}");
        assert_eq!(std::fs::read(&media).unwrap(), b"not really a movie");
        std::fs::remove_dir_all(&dir).ok();
    }
}
