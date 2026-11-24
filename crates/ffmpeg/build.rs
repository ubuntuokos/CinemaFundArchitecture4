//! With the `ffmpeg` feature on Linux, the FFmpeg run-time search path of this crate's own test
//! binaries: the restricted build first, the base build second, as `apps/cfa4/build.rs`
//! (`ffmpeg_runpath`) sets it for the apps. `rustc-link-arg` reaches every linked target of the
//! package (tests, examples), not the library itself.
//!
//! Always: `MIN_VERSION` (`src/lib.rs`) from `CFA4_FFMPEG_MIN_VERSION` of
//! `packaging/ffmpeg-version.env`, written to `$OUT_DIR/ffmpeg_version.rs`.
//!
//! Errors (an unreadable or malformed version file) are returned from `main`, so Cargo reports them
//! as a failed build script with the message, without a panic.

use std::path::Path;

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFA4_FFMPEG_PREFIX");
    let env = read_version_file()?;
    write_min_version(&env)?;
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return Ok(());
    }
    let version = pinned(&env, "CFA4_FFMPEG_VERSION")?;
    let base = std::env::var("CFA4_FFMPEG_PREFIX").unwrap_or_else(|_| format!("/opt/cfa4/ffmpeg-{version}"));
    println!("cargo:rustc-link-arg=-Wl,--enable-new-dtags");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{base}-restricted/lib:{base}/lib");
    Ok(())
}

/// `packaging/ffmpeg-version.env`, the one place CFA4's pinned FFmpeg is set
/// (docs/plans/CFA4-FFMPEG-9.1-001.md, R1).
fn read_version_file() -> Result<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/ffmpeg-version.env");
    println!("cargo:rerun-if-changed={}", path.display());
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// A `CFA4_FFMPEG_*` value of the version file.
fn pinned(env: &str, key: &str) -> Result<String, String> {
    env.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_owned())
        .ok_or_else(|| format!("{key} is not set in packaging/ffmpeg-version.env"))
}

/// `MAJOR.MINOR` or `MAJOR.MINOR.PATCH` (FFmpeg names x.y releases without a patch number).
fn parse_min_version(min: &str) -> Result<(u32, u32, u32), String> {
    let bad = || format!("CFA4_FFMPEG_MIN_VERSION is not MAJOR.MINOR[.PATCH]: {min}");
    let parts = min.split('.').map(|p| p.parse::<u32>().map_err(|_| bad())).collect::<Result<Vec<_>, _>>()?;
    match parts[..] {
        [a, b] => Ok((a, b, 0)),
        [a, b, c] => Ok((a, b, c)),
        _ => Err(bad()),
    }
}

/// `pub const MIN_VERSION: (u32, u32, u32)` for `src/lib.rs`.
fn write_min_version(env: &str) -> Result<(), String> {
    let (a, b, c) = parse_min_version(&pinned(env, "CFA4_FFMPEG_MIN_VERSION")?)?;
    let out_dir = std::env::var("OUT_DIR").map_err(|e| format!("OUT_DIR: {e}"))?;
    let out = Path::new(&out_dir).join("ffmpeg_version.rs");
    let code = format!(
        "/// Lowest FFmpeg release the backend accepts (ADR 0005; `CFA4_FFMPEG_MIN_VERSION` of\n\
         /// `packaging/ffmpeg-version.env`).\npub const MIN_VERSION: (u32, u32, u32) = ({a}, {b}, {c});\n"
    );
    std::fs::write(&out, code).map_err(|e| format!("cannot write {}: {e}", out.display()))
}
