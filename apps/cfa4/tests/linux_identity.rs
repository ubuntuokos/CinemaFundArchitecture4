#![cfg(target_os = "linux")]

use std::process::Command;

fn run_app(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cfa4")).args(args).output().expect("run CFA4 desktop CLI");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn version_uses_cfa4_identity() {
    assert!(run_app(&["--version"]).starts_with("CFA4 "));
}

#[test]
fn help_uses_cfa4_command_name() {
    assert!(run_app(&["--help"]).starts_with("Usage: cfa4 "));
}
