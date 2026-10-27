//! With the `ffmpeg` feature on Linux, the FFmpeg run-time search path of this crate's own test
//! binaries: the restricted build first, the base build second, as `apps/cfa4/build.rs`
//! (`ffmpeg_runpath`) sets it for the apps. `rustc-link-arg` reaches every linked target of the
//! package (tests, examples), not the library itself.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFA4_FFMPEG_PREFIX");
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let base = std::env::var("CFA4_FFMPEG_PREFIX").unwrap_or_else(|_| "/opt/cfa4/ffmpeg-9.0.2".into());
    println!("cargo:rustc-link-arg=-Wl,--enable-new-dtags");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{base}-restricted/lib:{base}/lib");
}
