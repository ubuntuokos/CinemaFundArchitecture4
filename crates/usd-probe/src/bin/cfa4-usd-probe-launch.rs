//! `cfa4-usd-probe-launch [--read PATH]... [--out PATH]... [--exec PROGRAM]... -- WORKER ARGS...`
//!
//! Confines itself (Landlock + seccomp, `cfa4-sandbox`, strict mode) and then replaces itself with the OpenUSD probe
//! worker. The confinement is inherited through `exec`: the worker reads only the `--read` paths (plus the
//! system libraries), writes only the `--out` files, opens no network socket and starts no other program
//! (docs/plans/CFA4-OPENUSD-SOURCEBUILD-001-0.1.md, S5, SB-G6). If confinement is not possible, nothing starts: the error is
//! printed as `{"error": ...}` and the exit code is 2.

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use cfa4_sandbox::{Context, Mode, Network, ProcessKind, Profile, apply};

fn fail(msg: &str) -> ExitCode {
    println!("{}", serde_json::json!({ "error": msg }));
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let mut read = Vec::new();
    let mut out = Vec::new();
    let mut exec = Vec::new();
    loop {
        match args.next() {
            Some(a) if a == "--read" => match args.next() {
                Some(p) => read.push(PathBuf::from(p)),
                None => return fail("--read needs a path"),
            },
            Some(a) if a == "--out" => match args.next() {
                Some(p) => out.push(PathBuf::from(p)),
                None => return fail("--out needs a path"),
            },
            // Extra programs the worker may start (an interpreter of a script worker; the host never passes it).
            Some(a) if a == "--exec" => match args.next() {
                Some(p) => exec.push(PathBuf::from(p)),
                None => return fail("--exec needs a path"),
            },
            Some(a) if a == "--" => break,
            _ => return fail("usage: cfa4-usd-probe-launch [--read PATH]... [--out PATH]... -- WORKER ARGS..."),
        }
    }
    let Some(worker) = args.next() else { return fail("no worker given") };
    let worker = PathBuf::from(worker);

    // The model-converter shape (read some sources, write single output files, no network) plus the right to
    // start exactly this worker.
    let ctx = Context::default();
    let mut profile = Profile::for_process(ProcessKind::ModelConverter, &ctx);
    profile.read = read;
    profile.read.push(worker.clone());
    profile.outputs = out;
    profile.exec = std::iter::once(worker.clone()).chain(exec).collect();
    profile.network = Network::None;
    if let Err(e) = apply(&profile, Mode::Strict) {
        return fail(&format!("cannot confine the OpenUSD probe worker: {e}"));
    }
    let err = Command::new(&worker).args(args).exec();
    fail(&format!("cannot start the OpenUSD probe worker: {err}"))
}
