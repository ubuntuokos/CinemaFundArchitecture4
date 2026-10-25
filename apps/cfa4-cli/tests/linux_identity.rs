#![cfg(target_os = "linux")]

use std::process::Command;

fn run_cli(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cfa4-cli")).args(args).output().expect("run CFA4 CLI");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn version_uses_cfa4_cli_identity() {
    assert!(run_cli(&["--version"]).starts_with("CFA4 CLI "));
}

#[test]
fn help_uses_cfa4_cli_name() {
    let help = run_cli(&["--help"]);
    assert!(help.starts_with("cfa4-cli: drive CFA4 "));
    assert!(help.contains("  cfa4-cli [OPTIONS]"));
}
