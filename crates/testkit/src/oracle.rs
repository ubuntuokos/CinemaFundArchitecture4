//! Discovery of the external oracle tools (ffmpeg, ffprobe).
//!
//! Search order for a tool `T` (`ffmpeg` or `ffprobe`):
//! 1. `CFA4_FFMPEG` / `CFA4_FFPROBE`: an explicit path (a set-but-wrong path is an error,
//!    not a silent skip);
//! 2. every directory on `PATH`;
//! 3. well-known install locations (`/usr/local/bin`, `/usr/bin`, `/snap/bin`).
//!
//! When a tool is missing, tests print `SKIPPED: …` and pass — unless
//! `CFA4_REQUIRE_ORACLES=1` is set (CI), which turns every skip into a failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Environment variable that makes missing oracles a hard failure.
pub const REQUIRE_ENV: &str = "CFA4_REQUIRE_ORACLES";

const KNOWN_DIRS: &[&str] = &["/usr/local/bin", "/usr/bin", "/snap/bin"];

fn exe_names(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

fn is_file(p: &Path) -> bool {
    p.is_file()
}

/// Find `name` given an explicit override, a `PATH`-style list and extra directories.
/// (Pure function of its inputs; [`find_tool`] supplies the process environment.)
pub fn find_tool_in(name: &str, explicit: Option<&std::ffi::OsStr>, path: Option<&std::ffi::OsStr>, extra: &[&str]) -> Result<Option<PathBuf>, String> {
    if let Some(e) = explicit.filter(|e| !e.is_empty()) {
        let p = PathBuf::from(e);
        return if is_file(&p) { Ok(Some(p)) } else { Err(format!("{} is set but `{}` is not a file", env_var(name), p.display())) };
    }
    let names = exe_names(name);
    if let Some(path) = path {
        for dir in std::env::split_paths(path) {
            for n in &names {
                let p = dir.join(n);
                if is_file(&p) {
                    return Ok(Some(p));
                }
            }
        }
    }
    for dir in extra {
        for n in &names {
            let p = Path::new(dir).join(n);
            if is_file(&p) {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}

fn env_var(name: &str) -> String {
    format!("CFA4_{}", name.to_ascii_uppercase())
}

/// Find a tool using the process environment (uncached).
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let explicit = std::env::var_os(env_var(name));
    let path = std::env::var_os("PATH");
    match find_tool_in(name, explicit.as_deref(), path.as_deref(), KNOWN_DIRS) {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    }
}

/// Path of ffmpeg, if available (cached for the process).
pub fn ffmpeg() -> Option<PathBuf> {
    static F: OnceLock<Option<PathBuf>> = OnceLock::new();
    F.get_or_init(|| find_tool("ffmpeg")).clone()
}

/// Path of ffprobe, if available (cached for the process).
pub fn ffprobe() -> Option<PathBuf> {
    static F: OnceLock<Option<PathBuf>> = OnceLock::new();
    F.get_or_init(|| find_tool("ffprobe")).clone()
}

/// Whether `CFA4_REQUIRE_ORACLES` is set to a true value.
pub fn oracles_required() -> bool {
    std::env::var(REQUIRE_ENV).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Report a skipped test: prints `SKIPPED (<context>): <reason>`. With
/// `CFA4_REQUIRE_ORACLES=1` it panics instead, so CI cannot pass by skipping.
pub fn skip(context: &str, reason: &str) {
    if oracles_required() {
        panic!("{context}: {reason}, and {REQUIRE_ENV}=1 forbids skipping oracle tests");
    }
    eprintln!("SKIPPED ({context}): {reason}");
}

/// Whether every tool in `names` is on `PATH` (only `PATH`: where product code that spawns the
/// tool by name will look). Otherwise [`skip`]s with `hint` (e.g. the packages to install) and
/// returns `false`; with `CFA4_REQUIRE_ORACLES=1` it panics.
pub fn on_path_or_skip(context: &str, names: &[&str], hint: &str) -> bool {
    let path = std::env::var_os("PATH");
    let missing: Vec<&str> = names.iter().copied().filter(|n| !matches!(find_tool_in(n, None, path.as_deref(), &[]), Ok(Some(_)))).collect();
    if missing.is_empty() {
        return true;
    }
    skip(context, &format!("{missing:?} not on PATH ({hint})"));
    false
}

fn tool_or_skip(name: &str, found: Option<PathBuf>, context: &str) -> Option<PathBuf> {
    if found.is_none() {
        skip(context, &format!("{name} not found (set {} or put {name} on PATH)", env_var(name)));
    }
    found
}

/// ffmpeg, or `None` after reporting a skip (see [`skip`]).
pub fn ffmpeg_or_skip(context: &str) -> Option<PathBuf> {
    tool_or_skip("ffmpeg", ffmpeg(), context)
}

/// Whether `ffmpeg -encoders` lists `encoder` (e.g. `libx265`): lets a test tell "this ffmpeg lacks the encoder"
/// (skip) from "the encoder is there but failed" (a real failure).
pub fn ffmpeg_has_encoder(ffmpeg: &Path, encoder: &str) -> bool {
    let Ok(out) = Command::new(ffmpeg).args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).output() else { return false };
    encoders_output_lists(&String::from_utf8_lossy(&out.stdout), encoder)
}

/// Patent-restricted encoders that CFA4's FFmpeg never contains (docs/licensing.md §7, the Ubuntu
/// model): x264 and x265 come from the Ubuntu archive at the user's own risk (`cfa4-restricted-extras`),
/// so neither the development nor the CI FFmpeg 9.0.2 has them. A test whose fixture needs one skips
/// without it, even with `CFA4_REQUIRE_ORACLES=1`.
pub const RESTRICTED_ENCODERS: &[&str] = &["libx264", "libx265"];

/// The first of [`RESTRICTED_ENCODERS`] named in the ffmpeg arguments `args` that `ffmpeg` lacks.
pub fn missing_fixture_encoder<S: AsRef<str>>(ffmpeg: &Path, args: &[S]) -> Option<&'static str> {
    RESTRICTED_ENCODERS.iter().copied().find(|enc| args.iter().any(|a| a.as_ref() == *enc) && !ffmpeg_has_encoder(ffmpeg, enc))
}

/// Report a test skipped because `encoder` is missing: a plain `SKIPPED` for a [restricted
/// encoder](RESTRICTED_ENCODERS), which no CFA4 FFmpeg has; [`skip`] (a failure with
/// `CFA4_REQUIRE_ORACLES=1`) for any other.
fn skip_missing_encoder(context: &str, encoder: &str) {
    if RESTRICTED_ENCODERS.contains(&encoder) {
        eprintln!("SKIPPED ({context}): this ffmpeg has no {encoder} encoder (patent-restricted, never in CFA4's FFmpeg)");
    } else {
        skip(context, &format!("this ffmpeg has no {encoder} encoder"));
    }
}

/// Whether `ffmpeg` has every encoder in `encoders`. Otherwise reports the skip, naming the first
/// missing one (see [`RESTRICTED_ENCODERS`]), and returns `false`.
pub fn encoders_or_skip(context: &str, ffmpeg: &Path, encoders: &[&str]) -> bool {
    match encoders.iter().find(|enc| !ffmpeg_has_encoder(ffmpeg, enc)) {
        None => true,
        Some(enc) => {
            skip_missing_encoder(context, enc);
            false
        }
    }
}

/// [`encoders_or_skip`] for the restricted encoders named in the ffmpeg arguments `args`.
pub fn fixture_encoders_or_skip<S: AsRef<str>>(context: &str, ffmpeg: &Path, args: &[S]) -> bool {
    match missing_fixture_encoder(ffmpeg, args) {
        None => true,
        Some(enc) => {
            skip_missing_encoder(context, enc);
            false
        }
    }
}

/// Pure function of the `ffmpeg -encoders` output: whether `encoder` is one of the listed names.
fn encoders_output_lists(output: &str, encoder: &str) -> bool {
    // rows look like ` V....D libx264   libx264 H.264 …`; the name is the second whitespace-separated token
    output.lines().any(|line| line.split_whitespace().nth(1) == Some(encoder))
}

/// ffprobe, or `None` after reporting a skip (see [`skip`]).
pub fn ffprobe_or_skip(context: &str) -> Option<PathBuf> {
    tool_or_skip("ffprobe", ffprobe(), context)
}

/// Whether `ffmpeg -formats` output lists `format_name` (e.g. `apv`, new in
/// ffmpeg 8.0). Pure function of the command output; matches a whole
/// whitespace-separated token so `APV` in a description cannot match.
fn formats_output_lists(output: &str, format_name: &str) -> bool {
    output.lines().any(|line| line.split_whitespace().any(|token| token == format_name))
}

/// Whether `ff` can read `format_name`. An ffmpeg that cannot run (or prints
/// nothing usable) counts as unsupported; the caller skips rather than fails.
pub fn ffmpeg_supports_format(ff: &Path, format_name: &str) -> bool {
    let Ok(out) = std::process::Command::new(ff).args(["-hide_banner", "-formats"]).output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    formats_output_lists(&String::from_utf8_lossy(&out.stdout), format_name)
}

/// The `ffmpeg -version` version string in banner output (`6.1.1-3ubuntu5`,
/// `8.0.1`, …), if the first line is an ffmpeg banner. Pure function of the
/// command output.
fn ffmpeg_version_in(output: &str) -> Option<String> {
    let first = output.lines().next()?;
    let rest = first.strip_prefix("ffmpeg version")?.trim_start();
    let version: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    if version.is_empty() { None } else { Some(version) }
}

/// The `ffmpeg -version` version string (`6.1.1-3ubuntu5`, `8.0.1`, …), if `ff`
/// reports one. `None` when the tool cannot run or its banner is unexpected.
pub fn ffmpeg_version(ff: &Path) -> Option<String> {
    let out = std::process::Command::new(ff).arg("-version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    ffmpeg_version_in(&String::from_utf8_lossy(&out.stdout))
}

/// ffmpeg that can read `format_name`, or `None` after reporting a skip (see
/// [`skip`]). An ffmpeg predating the format (APV needs 8.0+) skips with its
/// detected version named, exactly like a missing tool — and fails under
/// `CFA4_REQUIRE_ORACLES=1`.
pub fn ffmpeg_with_format_or_skip(context: &str, format_name: &str, min_version: &str) -> Option<PathBuf> {
    let ff = ffmpeg_or_skip(context)?;
    if ffmpeg_supports_format(&ff, format_name) {
        return Some(ff);
    }
    let version = ffmpeg_version(&ff).map_or_else(|| "unknown version".to_string(), |v| format!("version {v}"));
    skip(context, &format!("ffmpeg {version} cannot read {format_name} (needs {min_version}+)"));
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_lookup_matches_the_name_column_only() {
        let out = " V....D libx264              libx264 H.264 / AVC\n V....D libx265              libx265 H.265 / HEVC (libx265)\n A....D aac                  AAC (libx264 inside)\n";
        assert!(encoders_output_lists(out, "libx264"));
        assert!(encoders_output_lists(out, "libx265"));
        assert!(!encoders_output_lists(out, "libsvtav1"));
        assert!(!encoders_output_lists(out, "H.264"));
    }
    use std::ffi::OsString;

    fn tmpdir(_tag: &str) -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }

    #[test]
    fn explicit_path_wins_and_bad_path_errors() {
        let d = tmpdir("explicit");
        let f = d.join("my-ffmpeg");
        std::fs::write(&f, b"").unwrap();
        let got = find_tool_in("ffmpeg", Some(f.as_os_str()), None, &[]).unwrap();
        assert_eq!(got, Some(f.clone()));
        let missing = d.join("nope");
        assert!(find_tool_in("ffmpeg", Some(missing.as_os_str()), None, &[]).is_err());
        // empty value = unset
        assert_eq!(find_tool_in("ffmpeg", Some(OsString::new().as_os_str()), None, &[]).unwrap(), None);
    }

    #[test]
    fn path_search_finds_the_tool() {
        let a = tmpdir("path-a");
        let b = tmpdir("path-b");
        std::fs::write(b.join("ffprobe"), b"").unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        let got = find_tool_in("ffprobe", None, Some(&path), &[]).unwrap();
        assert_eq!(got, Some(b.join("ffprobe")));
        std::fs::write(a.join("ffprobe"), b"").unwrap();
        let got = find_tool_in("ffprobe", None, Some(&path), &[]).unwrap();
        assert!(got.is_some_and(|p| p.starts_with(&a)), "earlier PATH entry wins");
    }

    #[test]
    fn known_dirs_are_the_fallback() {
        let d = tmpdir("known");
        std::fs::write(d.join("ffmpeg"), b"").unwrap();
        let ds = d.to_string_lossy().to_string();
        let got = find_tool_in("ffmpeg", None, Some(OsString::new().as_os_str()), &[ds.as_str()]).unwrap();
        assert_eq!(got, Some(d.join("ffmpeg")));
        assert_eq!(find_tool_in("ffmpeg-does-not-exist", None, None, &[ds.as_str()]).unwrap(), None);
    }

    const FORMATS_WITH_APV: &str =
        "File formats:\n D  mov             QuickTime / MOV\n DE apv             APV raw bitstream\n D  mp4             MP4 (MPEG-4 Part 14)\n";
    const FORMATS_WITHOUT_APV: &str =
        "File formats:\n D  mov             QuickTime / MOV\n D  mp4             MP4 (MPEG-4 Part 14)\n D  oapv            Obsolete APV alias\n";

    #[test]
    fn formats_listing_detects_a_supported_format() {
        assert!(formats_output_lists(FORMATS_WITH_APV, "apv"));
        // No apv entry: neither a longer name containing it (oapv) nor an
        // uppercase description mention (APV) counts as support.
        assert!(!formats_output_lists(FORMATS_WITHOUT_APV, "apv"));
        assert!(!formats_output_lists("", "apv"));
    }

    #[test]
    fn version_string_parses_ffmpeg_banner() {
        assert_eq!(ffmpeg_version_in("ffmpeg version 7:6.1.1-3ubuntu5 Copyright (c) 2000-2023"), Some("7:6.1.1-3ubuntu5".to_string()));
        assert_eq!(ffmpeg_version_in("ffmpeg version 8.0.1-full_build-www.gyan.dev xxx"), Some("8.0.1-full_build-www.gyan.dev".to_string()));
        assert_eq!(ffmpeg_version_in(""), None);
        assert_eq!(ffmpeg_version_in("ffprobe version 6.1"), None);
        assert_eq!(ffmpeg_version_in("ffmpeg version "), None);
    }

    #[test]
    fn live_ffmpeg_reports_its_own_apv_support() {
        let Some(ff) = ffmpeg() else { return };
        let supported = ffmpeg_supports_format(&ff, "apv");
        assert_eq!(
            supported,
            formats_output_lists(&String::from_utf8_lossy(&std::process::Command::new(&ff).args(["-hide_banner", "-formats"]).output().unwrap().stdout), "apv")
        );
        assert!(ffmpeg_version(&ff).is_some_and(|v| !v.is_empty()));
    }
}
