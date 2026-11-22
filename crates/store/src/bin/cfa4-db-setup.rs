//! `cfa4-db-setup`: the idempotent CFA4 cluster setup (CFA4-DATA-PG-001 §5.2).

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use cfa4_store::ErrorCode;
use cfa4_store::setup::{PG_MAJOR, SetupOptions};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(exit) => exit,
    }
}

fn run() -> Result<(), ExitCode> {
    let options = match options_from_args() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            return Err(exit_code(&error));
        }
    };
    match cfa4_store::setup::run(&options) {
        Ok(report) => {
            println!("changed={}", report.changed);
            println!("port={}", report.port);
            println!("config={}", report.config_path.display());
            println!("server={PG_MAJOR}");
            Ok(())
        }
        Err(error) => {
            eprintln!("{error}");
            Err(exit_code_for(error.code))
        }
    }
}

fn exit_code(error: &cfa4_store::setup::SetupError) -> ExitCode {
    exit_code_for(error.code)
}

fn exit_code_for(code: ErrorCode) -> ExitCode {
    // Missing PostgreSQL is a handled result (ADR-0006): exit 2, no panic.
    if code == ErrorCode::DbServerMissing { ExitCode::from(2) } else { ExitCode::from(1) }
}

fn options_from_args() -> Result<SetupOptions, cfa4_store::setup::SetupError> {
    let mut args = std::env::args().skip(1);
    let mut config_dir: Option<PathBuf> = None;
    let mut os_user: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "cfa4-db-setup --config-dir DIR --os-user NAME\n\
                     \n\
                     Create or reuse the PostgreSQL {PG_MAJOR} cluster named cfa4 (never the default\n\
                     cluster). Unix socket only, peer map cfa4map, roles from the CFA4 role model.\n\
                     Writes <config dir>/cfa4/db.toml with the port the cluster was given.\n\
                     A second run changes nothing. Must run as root."
                );
                std::process::exit(0);
            }
            "--config-dir" => {
                let Some(value) = args.next() else {
                    return Err(setup_err(ErrorCode::DbConfigInvalid, "--config-dir needs a directory"));
                };
                config_dir = Some(PathBuf::from(value));
            }
            "--os-user" => {
                let Some(value) = args.next() else {
                    return Err(setup_err(ErrorCode::DbConfigInvalid, "--os-user needs a name"));
                };
                os_user = Some(value);
            }
            other => return Err(setup_err(ErrorCode::DbConfigInvalid, format!("unknown argument {other}"))),
        }
    }
    let os_user = match os_user {
        Some(user) => user,
        None => match std::env::var("SUDO_USER") {
            Ok(user) if !user.is_empty() => user,
            _ => current_user()?,
        },
    };
    let config_dir = match config_dir {
        Some(dir) => dir,
        None => home_config(&os_user)?,
    };
    Ok(SetupOptions { config_dir, os_user })
}

fn current_user() -> Result<String, cfa4_store::setup::SetupError> {
    let out = Command::new("id").arg("-un").output().map_err(|e| setup_err(ErrorCode::DbSetupFailed, format!("id -un: {e}")))?;
    if !out.status.success() {
        return Err(setup_err(ErrorCode::DbSetupFailed, "id -un failed"));
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if name.is_empty() || name == "root" {
        return Err(setup_err(ErrorCode::DbConfigInvalid, "running as root requires --os-user or SUDO_USER"));
    }
    Ok(name)
}

fn home_config(user: &str) -> Result<PathBuf, cfa4_store::setup::SetupError> {
    let out = Command::new("getent").args(["passwd", user]).output().map_err(|e| setup_err(ErrorCode::DbSetupFailed, format!("getent: {e}")))?;
    if !out.status.success() {
        return Err(setup_err(ErrorCode::DbConfigInvalid, format!("cannot look up OS user {user}")));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(home) = text.trim().split(':').nth(5).filter(|h| h.starts_with('/')) else {
        return Err(setup_err(ErrorCode::DbConfigInvalid, format!("OS user {user} has no home directory")));
    };
    Ok(PathBuf::from(home).join(".config"))
}

fn setup_err(code: ErrorCode, detail: impl Into<String>) -> cfa4_store::setup::SetupError {
    // SetupError's fields are public; keep construction in one place.
    cfa4_store::setup::SetupError { code, detail: detail.into() }
}
