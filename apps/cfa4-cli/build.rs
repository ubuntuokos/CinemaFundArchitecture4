//! With the `ffmpeg` feature on Linux, the FFmpeg run-time search path of `cfa4-cli`: the same rule
//! as `apps/cfa4/build.rs` (`ffmpeg_runpath`), so the CLI and the app load the same FFmpeg build.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFA4_FFMPEG_PREFIX");
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let version = pinned("CFA4_FFMPEG_VERSION");
    let base = std::env::var("CFA4_FFMPEG_PREFIX").unwrap_or_else(|_| format!("/opt/cfa4/ffmpeg-{version}"));
    println!("cargo:rustc-link-arg-bins=-Wl,--enable-new-dtags");
    // Third: the copy a relocatable package (tarball, AppImage, Flatpak) carries next to the binary
    // (packaging/linux/stage-ffmpeg.sh). The .deb installs into /opt/cfa4, the first two entries.
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{base}-restricted/lib:{base}/lib:$ORIGIN/../lib/cfa4/ffmpeg-{version}/lib");
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
