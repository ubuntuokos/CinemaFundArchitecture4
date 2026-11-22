//! A throw-away PostgreSQL cluster for the tests (PG-001 §10): `initdb` and `pg_ctl` in a
//! temporary directory, reachable only over a Unix socket in that directory. When the machine
//! has no PostgreSQL binaries the tests say so and skip. `CFA4_PG_REQUIRE=1` (the PG-1 CI job)
//! turns that skip into a failure and requires major version 18. Run as root (containers), the
//! server runs as the `postgres` user through `runuser`, because PostgreSQL refuses to run as root.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::StoreConfig;

static NEXT: AtomicU32 = AtomicU32::new(0);

pub struct TestDb {
    dir: PathBuf,
    bin: PathBuf,
    as_postgres: bool,
}

fn bin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CFA4_PG_BIN") {
        return Some(PathBuf::from(dir));
    }
    let mut versions: Vec<PathBuf> =
        std::fs::read_dir("/usr/lib/postgresql").ok()?.flatten().map(|e| e.path().join("bin")).filter(|p| p.join("initdb").exists()).collect();
    versions.sort();
    versions.pop()
}

fn is_root() -> bool {
    Command::new("id").arg("-u").output().ok().is_some_and(|o| o.stdout.trim_ascii() == b"0")
}

pub(crate) enum Start {
    Ready(TestDb),
    /// Local development without PostgreSQL. CI (`CFA4_PG_REQUIRE=1`) must treat this as a failure.
    Skip(String),
}

pub(crate) fn required() -> bool {
    std::env::var("CFA4_PG_REQUIRE").ok().as_deref() == Some("1")
}

impl TestDb {
    fn command(&self, tool: &str) -> Command {
        let exe = self.bin.join(tool);
        if self.as_postgres {
            let mut c = Command::new("runuser");
            c.args(["-u", "postgres", "--"]).arg(exe);
            c
        } else {
            Command::new(exe)
        }
    }

    /// Start a cluster, or [`Start::Skip`] when PostgreSQL is not installed.
    pub fn start() -> Start {
        let Some(bin) = bin_dir() else {
            return Start::Skip("cfa4-store tests: no PostgreSQL binaries (set CFA4_PG_BIN); skipped".into());
        };
        match server_major(&bin) {
            Ok(18) => {}
            Ok(major) if required() => {
                return Start::Skip(format!("cfa4-store tests: CFA4_PG_REQUIRE=1 needs PostgreSQL 18, found {major}"));
            }
            Ok(_) => {}
            Err(detail) if required() => return Start::Skip(format!("cfa4-store tests: {detail}")),
            Err(detail) => eprintln!("cfa4-store tests: {detail}"),
        }
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cfa4-store-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        if std::fs::create_dir_all(&dir).is_err() {
            return Start::Skip(format!("cfa4-store tests: cannot create {}", dir.display()));
        }
        let as_postgres = is_root();
        if as_postgres {
            let ok = Command::new("chown").args(["-R", "postgres:postgres"]).arg(&dir).status().ok().is_some_and(|s| s.success());
            if !ok {
                return Start::Skip("cfa4-store tests: running as root and cannot hand the directory to `postgres`; skipped".into());
            }
        }
        let db = TestDb { dir, bin, as_postgres };
        let data = db.dir.join("data");
        let Some(initdb) = db.command("initdb").arg("-D").arg(&data).args(["-U", "cfa4", "-A", "trust", "--no-sync", "-E", "UTF8"]).output().ok() else {
            return Start::Skip("cfa4-store tests: cannot run initdb".into());
        };
        if !initdb.status.success() {
            return Start::Skip(format!("cfa4-store tests: initdb failed: {}", String::from_utf8_lossy(&initdb.stderr)));
        }
        let opts = format!("-k {} -c listen_addresses='' -c fsync=off -c full_page_writes=off", db.dir.display());
        let Some(start) = db.command("pg_ctl").arg("-D").arg(&data).args(["-w", "-o", &opts, "-l"]).arg(db.dir.join("log")).arg("start").output().ok() else {
            return Start::Skip("cfa4-store tests: cannot run pg_ctl".into());
        };
        if !start.status.success() {
            return Start::Skip(format!("cfa4-store tests: pg_ctl start failed: {}", String::from_utf8_lossy(&start.stderr)));
        }
        Start::Ready(db)
    }

    /// Superuser created by `initdb -U cfa4`, used only to install the role model.
    pub fn admin_config(&self) -> StoreConfig {
        StoreConfig { host: self.dir.display().to_string(), port: 5432, user: "cfa4".into(), dbname: "postgres".into(), ..StoreConfig::default() }
    }

    /// The application login ([`crate::LOGIN_ROLE`]) on the throw-away database.
    pub fn config(&self) -> StoreConfig {
        StoreConfig { user: crate::LOGIN_ROLE.into(), ..self.admin_config() }
    }

    /// Stop the server and leave the directory in place so a later connect fails cleanly.
    pub fn stop(&self) {
        let _ = self.command("pg_ctl").arg("-D").arg(self.dir.join("data")).args(["-m", "fast", "stop"]).output();
    }
}

fn server_major(bin: &Path) -> Result<u32, String> {
    let out = Command::new(bin.join("postgres")).arg("--version").output().map_err(|e| format!("postgres --version: {e}"))?;
    if !out.status.success() {
        return Err(format!("postgres --version failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(version) = text.split_whitespace().last() else {
        return Err(format!("cannot parse postgres version from {text}"));
    };
    let Some(major) = version.split('.').next() else {
        return Err(format!("cannot parse postgres version from {text}"));
    };
    major.parse::<u32>().map_err(|_| format!("cannot parse postgres version from {text}"))
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = self.command("pg_ctl").arg("-D").arg(self.dir.join("data")).args(["-m", "immediate", "stop"]).output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
