//! With the `ffmpeg` feature on Linux, the FFmpeg run-time search path of this crate's own test
//! binaries: the restricted build first, the base build second, as `apps/cfa4/build.rs`
//! (`ffmpeg_runpath`) sets it for the apps. `rustc-link-arg` reaches every linked target of the
//! package (tests, examples), not the library itself.
//!
//! Always: `MIN_VERSION` (`src/lib.rs`) from `CFA4_FFMPEG_MIN_VERSION` of
//! `packaging/ffmpeg-version.env`, written to `$OUT_DIR/ffmpeg_version.rs`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFA4_FFMPEG_PREFIX");
    write_min_version();
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let version = pinned("CFA4_FFMPEG_VERSION");
    let base = std::env::var("CFA4_FFMPEG_PREFIX").unwrap_or_else(|_| format!("/opt/cfa4/ffmpeg-{version}"));
    println!("cargo:rustc-link-arg=-Wl,--enable-new-dtags");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{base}-restricted/lib:{base}/lib");
}

/// A `CFA4_FFMPEG_*` value of `packaging/ffmpeg-version.env`, the one place CFA4's pinned FFmpeg is
/// set (docs/plans/CFA4-FFMPEG-9.1-001.md, R1).
fn pinned(key: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/ffmpeg-version.env");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_owned())
        .unwrap_or_else(|| panic!("{key} is not set in {}", path.display()))
}

/// `pub const MIN_VERSION: (u32, u32, u32)` for `src/lib.rs`.
fn write_min_version() {
    let min = pinned("CFA4_FFMPEG_MIN_VERSION");
    // MAJOR.MINOR or MAJOR.MINOR.PATCH (FFmpeg names x.y releases without a patch number).
    let bad = || -> ! { panic!("CFA4_FFMPEG_MIN_VERSION is not MAJOR.MINOR[.PATCH]: {min}") };
    let parts: Vec<u32> = min.split('.').map(|p| p.parse().unwrap_or_else(|_| bad())).collect();
    let (a, b, c) = match parts[..] {
        [a, b] => (a, b, 0),
        [a, b, c] => (a, b, c),
        _ => bad(),
    };
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("ffmpeg_version.rs");
    let code = format!(
        "/// Lowest FFmpeg release the backend accepts (ADR 0005; `CFA4_FFMPEG_MIN_VERSION` of\n\
         /// `packaging/ffmpeg-version.env`).\npub const MIN_VERSION: (u32, u32, u32) = ({a}, {b}, {c});\n"
    );
    std::fs::write(&out, code).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
}
