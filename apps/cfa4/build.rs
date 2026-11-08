//! With the `ffmpeg` feature on Linux: where the binary finds the FFmpeg libraries at run time
//! (ADR 0005, Ubuntu model of docs/licensing.md §7). Two builds share one set of library names: the
//! base build (`/opt/cfa4/ffmpeg-9.0.2`, no patent-restricted codecs) and the build of the
//! `cfa4-restricted-extras` package (`…-restricted`, with x264 and x265). The restricted one is
//! searched first, so installing the extras switches every FFmpeg library at once, and removing
//! it falls back to the base build. `CFA4_FFMPEG_PREFIX` moves the base build. The same rule is in
//! `apps/cfa4-cli/build.rs` and `crates/ffmpeg/build.rs`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFA4_FFMPEG_PREFIX");
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let base = std::env::var("CFA4_FFMPEG_PREFIX").unwrap_or_else(|_| "/opt/cfa4/ffmpeg-9.0.2".into());
    println!("cargo:rustc-link-arg-bins=-Wl,--enable-new-dtags");
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{base}-restricted/lib:{base}/lib");
}
