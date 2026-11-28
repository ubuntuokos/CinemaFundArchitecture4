//! A throw-away PostgreSQL cluster for the tests (PG-001 §10): `initdb` and `pg_ctl` in a
//! temporary directory, reachable only over a Unix socket in that directory. When the machine
//! has no PostgreSQL binaries the tests say so and skip. `CFA4_PG_REQUIRE=1` (the PG-1 CI job)
//! turns that skip into a failure and requires major version 18. Run as root (containers), the
//! server runs as the `postgres` user through `runuser`, because PostgreSQL refuses to run as root.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{LOGIN_ROLE, Store, StoreConfig};

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
    major_from_version(&text).ok_or_else(|| format!("cannot parse postgres version from {text}"))
}

/// The major version from a `postgres --version` line. Ubuntu's build ends with its own package
/// version (`postgres (PostgreSQL) 18.7 (Ubuntu 18.7-1.pgdg26.04+5)`), so the PostgreSQL version is
/// the first token that starts with a digit, not the last one.
fn major_from_version(text: &str) -> Option<u32> {
    let token = text.split_whitespace().find(|token| token.starts_with(|c: char| c.is_ascii_digit()))?;
    token.split('.').next().and_then(|major| major.parse().ok())
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = self.command("pg_ctl").arg("-D").arg(self.dir.join("data")).args(["-m", "immediate", "stop"]).output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const ROLE_SQL: &str = "\
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_login') THEN
    CREATE ROLE cfa4_login LOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_owner_tasks') THEN
    CREATE ROLE cfa4_owner_tasks NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_app_tasks') THEN
    CREATE ROLE cfa4_app_tasks NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_owner_cfa4_store') THEN
    CREATE ROLE cfa4_owner_cfa4_store NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_app_cfa4_store') THEN
    CREATE ROLE cfa4_app_cfa4_store NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_owner_ai') THEN
    CREATE ROLE cfa4_owner_ai NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_app_ai') THEN
    CREATE ROLE cfa4_app_ai NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_owner_cache') THEN
    CREATE ROLE cfa4_owner_cache NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cfa4_app_cache') THEN
    CREATE ROLE cfa4_app_cache NOLOGIN;
  END IF;
END $$;
GRANT cfa4_owner_tasks TO cfa4_login;
GRANT cfa4_app_tasks TO cfa4_login;
GRANT cfa4_owner_cfa4_store TO cfa4_login;
GRANT cfa4_app_cfa4_store TO cfa4_login;
GRANT CREATE ON DATABASE postgres TO cfa4_owner_tasks;
GRANT CREATE ON DATABASE postgres TO cfa4_owner_cfa4_store;
GRANT cfa4_owner_ai TO cfa4_login;
GRANT cfa4_app_ai TO cfa4_login;
GRANT CREATE ON DATABASE postgres TO cfa4_owner_ai;
GRANT cfa4_owner_cache TO cfa4_login;
GRANT cfa4_app_cache TO cfa4_login;
GRANT CREATE ON DATABASE postgres TO cfa4_owner_cache;
";

pub(crate) fn skip_or_fail(reason: impl std::fmt::Display) -> Option<(TestDb, Store)> {
    if required() {
        panic!("PostgreSQL 18 test cluster required but not usable: {reason}");
    }
    eprintln!("{reason}");
    None
}

pub(crate) async fn setup() -> Option<(TestDb, Store)> {
    let db = match TestDb::start() {
        Start::Ready(db) => db,
        Start::Skip(reason) => return skip_or_fail(reason),
    };
    let admin = match Store::connect(&db.admin_config()).await {
        Ok(store) => store,
        Err(e) => return skip_or_fail(format!("cfa4-store tests: cannot connect as the test superuser: {e}")),
    };
    let client = match admin.client().await {
        Ok(client) => client,
        Err(e) => return skip_or_fail(format!("cfa4-store tests: {e}")),
    };
    if let Err(e) = client.batch_execute(ROLE_SQL).await {
        return skip_or_fail(format!("cfa4-store tests: cannot install the role model: {e}"));
    }
    drop(client);
    drop(admin);
    let store = match Store::connect(&db.config()).await {
        Ok(store) => store,
        Err(e) => return skip_or_fail(format!("cfa4-store tests: cannot connect as {LOGIN_ROLE}: {e}")),
    };
    if let Err(e) = store.migrate().await {
        return skip_or_fail(format!("cfa4-store tests: migration failed: {e}"));
    }
    Some((db, store))
}

pub(crate) async fn count(store: &Store, sql: &str) -> i64 {
    let Ok(client) = store.client().await else { return -1 };
    client.query_one(sql, &[]).await.and_then(|r| r.try_get(0)).unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::major_from_version;

    #[test]
    fn the_major_version_is_read_from_the_ubuntu_build_line_too() {
        assert_eq!(major_from_version("postgres (PostgreSQL) 18.7 (Ubuntu 18.7-1.pgdg26.04+5)\n"), Some(18));
        assert_eq!(major_from_version("postgres (PostgreSQL) 16.4\n"), Some(16));
        assert_eq!(major_from_version("postgres (PostgreSQL) 19.0 (Ubuntu 19.0-2.pgdg26.04+1)\n"), Some(19));
        assert_eq!(major_from_version("no version here"), None);
    }
}
