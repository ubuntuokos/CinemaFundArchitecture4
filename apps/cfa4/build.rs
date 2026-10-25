//! Windows only: embed version info (VERSIONINFO) into `cfa4.exe`. No app icon is embedded:
//! the inherited upstream icon was removed and CFA4 has no Windows `.ico` yet.
//!
//! On every other target this does nothing (`winresource` is only a build-dependency on Windows
//! hosts). A missing resource compiler is a warning, unless `CFA4_REQUIRE_WINRES=1` (for
//! release builds) turns it into an error.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CFA4_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    windows::embed_resources();
}

#[cfg(windows)]
mod windows {
    pub fn embed_resources() {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "CFA4")
            .set("FileDescription", "CFA4 video editor")
            .set("LegalCopyright", "Copyright (c) the CFA4 and upstream contributors (see NOTICE). MIT OR Apache-2.0.")
            .set("OriginalFilename", "cfa4.exe")
            .set("InternalName", "cfa4");
        if let Err(e) = res.compile() {
            if std::env::var_os("CFA4_REQUIRE_WINRES").is_some() {
                panic!("embedding Windows resources failed: {e}");
            }
            println!("cargo:warning=cfa4.exe built without icon/version resources: {e}");
        }
    }
}

/// Cross-compiling for Windows from macOS or Linux: `winresource` is not available on this host.
#[cfg(not(windows))]
mod windows {
    pub fn embed_resources() {
        println!("cargo:warning=cfa4.exe built without icon/version resources (cross-compiled from a non-Windows host)");
    }
}
