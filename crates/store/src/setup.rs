//! Idempotent `cfa4-db-setup` (plan §5.2, D-1, D-2, D-4).
//!
//! Creates the PostgreSQL 18 cluster named `cfa4` when it is missing, never the default cluster.
//! The cluster listens on a Unix socket only (`listen_addresses = ''`), peer authentication goes
//! through the `cfa4map` identity map (OS user → `cfa4_login`, plus the `postgres` OS account so
//! the setup can run again), and the assigned port is written to `<config dir>/cfa4/db.toml`.
//! A second run with the same arguments leaves the files, the roles and the port as they were.
//!
//! `CREATE EXTENSION vector` is PG-4 and is not done here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::dbtoml::{self, is_pg_ident};
use crate::{ErrorCode, LOGIN_ROLE, StoreConfig};

pub const PG_MAJOR: u32 = 18;
pub const CLUSTER: &str = "cfa4";
pub const DATABASE: &str = "cfa4";
pub const MAP_NAME: &str = "cfa4map";
const SOCKET_DIR: &str = "/var/run/postgresql";

/// Schemas from the plan's map (§4.2). PG-1 creates the roles only; other phases own the DDL.
const SCHEMAS: &[&str] = &["backlot", "cfa4_audit", "cfa4_files", "tasks", "ai", "cache", "mi", "registry", "train", "cfa4_store"];

/// Owners that create objects from this crate's migrations, so they need CREATE on the database.
const MIGRATE_SCHEMAS: &[&str] = &["tasks", "ai", "cache", "cfa4_store"];

pub struct SetupOptions {
    /// Config home. The file is `<config_dir>/cfa4/db.toml`.
    pub config_dir: PathBuf,
    /// Desktop OS account mapped to [`LOGIN_ROLE`]. Not `root` and not `postgres`.
    pub os_user: String,
}

pub struct SetupReport {
    pub port: u16,
    pub changed: bool,
    pub config_path: PathBuf,
}

#[derive(Debug)]
pub struct SetupError {
    pub code: ErrorCode,
    pub detail: String,
}

impl SetupError {
    fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        SetupError { code, detail: detail.into() }
    }
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cluster {
    version: u32,
    name: String,
    port: u16,
    online: bool,
}

/// Create or reuse the `cfa4` cluster. Must run as root. Never edits another cluster.
pub fn run(options: &SetupOptions) -> Result<SetupReport, SetupError> {
    check_os_user(&options.os_user)?;
    if !is_root() {
        return Err(SetupError::new(
            ErrorCode::DbSetupPrivilege,
            "cfa4-db-setup needs root (package postinst, or pkexec on first launch). The default cluster is never modified.",
        ));
    }
    require_postgres_18()?;
    if !user_exists(&options.os_user) {
        return Err(SetupError::new(ErrorCode::DbConfigInvalid, format!("OS user {} does not exist", options.os_user)));
    }

    let before = list_clusters()?;
    reject_foreign_cfa4(&before)?;
    let mut changed = false;
    if !before.iter().any(|c| c.version == PG_MAJOR && c.name == CLUSTER) {
        run_ok("pg_createcluster", &[&PG_MAJOR.to_string(), CLUSTER, "--start-conf=auto"])?;
        changed = true;
    }

    let listed = list_clusters()?;
    let Some(cluster) = listed.into_iter().find(|c| c.version == PG_MAJOR && c.name == CLUSTER) else {
        return Err(SetupError::new(ErrorCode::DbSetupFailed, "pg_createcluster did not leave an 18/cfa4 cluster"));
    };

    let conf_dir = PathBuf::from(format!("/etc/postgresql/{PG_MAJOR}/{CLUSTER}"));
    let include_changed = ensure_include_dir(&conf_dir.join("postgresql.conf"))?;
    let listen_changed = write_if_changed(&conf_dir.join("conf.d").join("cfa4.conf"), &render_listen())?;
    let hba_changed = write_if_changed(&conf_dir.join("pg_hba.conf"), &render_hba())?;
    let ident_changed = write_if_changed(&conf_dir.join("pg_ident.conf"), &render_ident(&options.os_user))?;
    changed |= include_changed || listen_changed || hba_changed || ident_changed;

    if !cluster.online || listen_changed || include_changed {
        let action = if cluster.online { "restart" } else { "start" };
        run_ok("pg_ctlcluster", &[&PG_MAJOR.to_string(), CLUSTER, action])?;
        changed = true;
    } else if hba_changed || ident_changed {
        run_ok("pg_ctlcluster", &[&PG_MAJOR.to_string(), CLUSTER, "reload"])?;
        changed = true;
    }

    if ensure_roles(cluster.port)? {
        changed = true;
    }
    let shown = psql(cluster.port, "SHOW listen_addresses")?;
    if !shown.is_empty() {
        return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("listen_addresses is {shown:?} after setup; TCP must stay off")));
    }

    let stored = StoreConfig { host: SOCKET_DIR.into(), port: cluster.port, user: LOGIN_ROLE.into(), dbname: DATABASE.into(), ..StoreConfig::default() };
    let config_path = options.config_dir.join("cfa4").join("db.toml");
    let wrote = dbtoml::write_if_changed(&config_path, &stored).map_err(|e| SetupError::new(e.code(), e.to_string()))?;
    chown_config(&options.config_dir.join("cfa4"), &options.os_user)?;
    changed |= wrote;

    Ok(SetupReport { port: cluster.port, changed, config_path })
}

fn check_os_user(name: &str) -> Result<(), SetupError> {
    if !is_os_user(name) || name == "postgres" || name == "root" {
        return Err(SetupError::new(ErrorCode::DbConfigInvalid, "the OS user must be the desktop account (not root, not postgres), 1..=32 of [A-Za-z0-9_-]"));
    }
    Ok(())
}

pub(crate) fn is_os_user(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(&first) = bytes.first() else { return false };
    bytes.len() <= 32 && (first.is_ascii_alphabetic() || first == b'_') && bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

fn require_postgres_18() -> Result<(), SetupError> {
    let initdb = PathBuf::from(format!("/usr/lib/postgresql/{PG_MAJOR}/bin/initdb"));
    let tools = ["/usr/bin/pg_lsclusters", "/usr/bin/pg_createcluster", "/usr/bin/pg_ctlcluster"];
    if !initdb.is_file() || tools.iter().any(|p| !Path::new(p).is_file()) {
        return Err(SetupError::new(
            ErrorCode::DbServerMissing,
            "PostgreSQL 18 or postgresql-common is not installed. Editing, playback and saving work without it; the task queue and AI stay off until this command can create the cfa4 cluster.",
        ));
    }
    Ok(())
}

fn list_clusters() -> Result<Vec<Cluster>, SetupError> {
    let text = command_stdout("pg_lsclusters", &[])?;
    parse_clusters(&text)
}

pub(crate) fn parse_clusters(text: &str) -> Result<Vec<Cluster>, SetupError> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("Ver ") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(ver) = parts.next() else { continue };
        let Some(name) = parts.next() else {
            return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("pg_lsclusters line {}: {line}", index.saturating_add(1))));
        };
        let Some(port) = parts.next() else {
            return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("pg_lsclusters line {}: {line}", index.saturating_add(1))));
        };
        let Some(status) = parts.next() else {
            return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("pg_lsclusters line {}: {line}", index.saturating_add(1))));
        };
        if !is_pg_ident(name) && name != CLUSTER {
            return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("refusing cluster name {name}")));
        }
        let version: u32 = ver.parse().map_err(|_| SetupError::new(ErrorCode::DbSetupFailed, format!("bad version {ver}")))?;
        let port: u16 = port.parse().map_err(|_| SetupError::new(ErrorCode::DbSetupFailed, format!("bad port {port}")))?;
        if port == 0 {
            return Err(SetupError::new(ErrorCode::DbSetupFailed, "pg_lsclusters reported port 0"));
        }
        out.push(Cluster { version, name: name.to_owned(), port, online: status.starts_with("online") });
        if out.len() > 32 {
            return Err(SetupError::new(ErrorCode::DbSetupFailed, "too many PostgreSQL clusters"));
        }
    }
    Ok(out)
}

fn reject_foreign_cfa4(rows: &[Cluster]) -> Result<(), SetupError> {
    if let Some(other) = rows.iter().find(|c| c.name == CLUSTER && c.version != PG_MAJOR) {
        return Err(SetupError::new(
            ErrorCode::DbClusterOldMajor,
            format!("the cfa4 cluster is PostgreSQL {}; CFA4 requires {PG_MAJOR}. It was not modified and was not upgraded.", other.version),
        ));
    }
    Ok(())
}

pub(crate) fn render_hba() -> String {
    "\
# Written by cfa4-db-setup (CFA4-DATA-PG-001 P6, D-1).\n\
# Unix socket only. Peer authentication uses the cfa4map identity map.\n\
local   all   all   peer map=cfa4map\n"
        .to_owned()
}

pub(crate) fn render_ident(os_user: &str) -> String {
    format!(
        "# Written by cfa4-db-setup (CFA4-DATA-PG-001 D-1).\n\
         # MAPNAME  SYSTEM-USERNAME  PG-USERNAME\n\
         {MAP_NAME} postgres postgres\n\
         {MAP_NAME} {os_user} {LOGIN_ROLE}\n"
    )
}

fn render_listen() -> String {
    "\
# Written by cfa4-db-setup (CFA4-DATA-PG-001).\n\
# No TCP. Memory and connection counts stay at the package defaults until measured (plan §5.5).\n\
listen_addresses = ''\n"
        .to_owned()
}

fn ensure_include_dir(conf: &Path) -> Result<bool, SetupError> {
    let text = fs::read_to_string(conf).map_err(|e| SetupError::new(ErrorCode::DbSetupFailed, format!("cannot read {}: {e}", conf.display())))?;
    let active = text.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.starts_with('#') && trimmed.replace(' ', "").starts_with("include_dir=")
    });
    if active {
        return Ok(false);
    }
    let mut next = text;
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str("include_dir = 'conf.d'\n");
    write_if_changed(conf, &next)
}

fn ensure_roles(port: u16) -> Result<bool, SetupError> {
    let mut changed = false;
    let roles = psql(port, "SELECT rolname FROM pg_roles")?;
    let have = |name: &str| roles.lines().any(|line| line == name);
    if !have(LOGIN_ROLE) {
        psql(port, &format!("CREATE ROLE {LOGIN_ROLE} LOGIN"))?;
        changed = true;
    }
    for schema in SCHEMAS {
        for kind in ["owner", "app"] {
            let role = format!("cfa4_{kind}_{schema}");
            let role = checked_ident(&role)?.to_owned();
            if !have(&role) {
                psql(port, &format!("CREATE ROLE {role} NOLOGIN"))?;
                changed = true;
            }
        }
    }
    let databases = psql(port, "SELECT datname FROM pg_database")?;
    if !databases.lines().any(|line| line == DATABASE) {
        psql(port, &format!("CREATE DATABASE {DATABASE}"))?;
        changed = true;
    }
    let members = psql(
        port,
        "SELECT r.rolname || ' ' || m.rolname FROM pg_auth_members am \
         JOIN pg_roles r ON r.oid = am.roleid JOIN pg_roles m ON m.oid = am.member \
         WHERE m.rolname = 'cfa4_login'",
    )?;
    for schema in SCHEMAS {
        for kind in ["owner", "app"] {
            let role = format!("cfa4_{kind}_{schema}");
            let role = checked_ident(&role)?.to_owned();
            let line = format!("{role} {LOGIN_ROLE}");
            if !members.lines().any(|have| have == line) {
                psql(port, &format!("GRANT {role} TO {LOGIN_ROLE}"))?;
                changed = true;
            }
        }
    }
    for schema in MIGRATE_SCHEMAS {
        let role = format!("cfa4_owner_{schema}");
        let role = checked_ident(&role)?.to_owned();
        let flag = psql(port, &format!("SELECT has_database_privilege('{role}', '{DATABASE}', 'CREATE')"))?;
        if flag != "t" {
            psql(port, &format!("GRANT CREATE ON DATABASE {DATABASE} TO {role}"))?;
            changed = true;
        }
    }
    Ok(changed)
}

fn checked_ident(name: &str) -> Result<&str, SetupError> {
    if is_pg_ident(name) { Ok(name) } else { Err(SetupError::new(ErrorCode::DbSetupFailed, format!("refusing identifier {name}"))) }
}

fn psql(port: u16, sql: &str) -> Result<String, SetupError> {
    let port = port.to_string();
    let mut cmd = Command::new("runuser");
    cmd.args(["-u", "postgres", "--", "psql", "-X", "-v", "ON_ERROR_STOP=1", "-At", "-h", SOCKET_DIR, "-p", &port, "-d", "postgres", "-c", sql]);
    cmd.env("LC_ALL", "C");
    let out = output(&mut cmd)?;
    if !out.status.success() {
        return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("psql failed: {}", tail(&out.stderr))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool, SetupError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| SetupError::new(ErrorCode::DbSetupFailed, format!("cannot create {}: {e}", parent.display())))?;
    }
    if path.is_file() {
        match fs::read_to_string(path) {
            Ok(existing) if existing == contents => return Ok(false),
            Ok(_) => {}
            Err(e) => return Err(SetupError::new(ErrorCode::DbSetupFailed, format!("cannot read {}: {e}", path.display()))),
        }
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).map_err(|e| SetupError::new(ErrorCode::DbSetupFailed, format!("cannot write {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|e| SetupError::new(ErrorCode::DbSetupFailed, format!("cannot replace {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o640));
        if let Some(text) = path.to_str() {
            let _ = Command::new("chgrp").args(["postgres", text]).status();
        }
    }
    Ok(true)
}

fn chown_config(dir: &Path, user: &str) -> Result<(), SetupError> {
    let Some(text) = dir.to_str() else {
        return Err(SetupError::new(ErrorCode::DbConfigInvalid, "config directory is not UTF-8"));
    };
    let owner = format!("{user}:{user}");
    let status = Command::new("chown").args(["-R", &owner, text]).status().map_err(|e| SetupError::new(ErrorCode::DbSetupFailed, format!("chown: {e}")))?;
    if status.success() { Ok(()) } else { Err(SetupError::new(ErrorCode::DbSetupFailed, format!("chown {owner} {text} failed"))) }
}

fn user_exists(name: &str) -> bool {
    Command::new("id").arg(name).status().ok().is_some_and(|s| s.success())
}

fn is_root() -> bool {
    command_stdout("id", &["-u"]).ok().is_some_and(|text| text.trim() == "0")
}

fn run_ok(program: &str, args: &[&str]) -> Result<(), SetupError> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.env("LC_ALL", "C");
    let out = output(&mut cmd)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(SetupError::new(ErrorCode::DbSetupFailed, format!("{program} {} failed: {}", args.join(" "), tail(&out.stderr))))
    }
}

fn command_stdout(program: &str, args: &[&str]) -> Result<String, SetupError> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.env("LC_ALL", "C");
    let out = output(&mut cmd)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(SetupError::new(ErrorCode::DbSetupFailed, format!("{program} failed: {}", tail(&out.stderr))))
    }
}

fn output(cmd: &mut Command) -> Result<Output, SetupError> {
    cmd.output().map_err(|e| SetupError::new(ErrorCode::DbSetupFailed, format!("cannot run command: {e}")))
}

fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    if text.len() <= 2000 { text.to_owned() } else { text.get(text.len().saturating_sub(2000)..).unwrap_or(text).to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_map_and_hba_are_stable() {
        let ident = render_ident("istvan");
        assert_eq!(ident, render_ident("istvan"));
        assert!(ident.contains("cfa4map postgres postgres"));
        assert!(ident.contains("cfa4map istvan cfa4_login"));
        assert!(render_hba().contains("peer map=cfa4map"));
        assert!(!render_hba().contains("trust"));
        assert!(!render_hba().contains("host "));
        assert!(!is_os_user(""));
        assert!(!is_os_user("root\n"));
        assert!(!is_os_user("has space"));
        assert!(is_os_user("istvan"));
    }

    #[test]
    fn cluster_listing_keeps_the_default_cluster_apart_and_rejects_an_old_cfa4() {
        let text = "\
Ver Cluster Port Status Owner    Data directory              Log file
18  main    5432 online postgres /var/lib/postgresql/18/main /var/log/postgresql/postgresql-18-main.log
18  cfa4    5433 down   postgres /var/lib/postgresql/18/cfa4 /var/log/postgresql/postgresql-18-cfa4.log
";
        let rows = parse_clusters(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "main");
        assert!(rows[0].online);
        assert_eq!((rows[1].name.as_str(), rows[1].port, rows[1].online), ("cfa4", 5433, false));
        let old = "16  cfa4    5434 online postgres /var/lib/postgresql/16/cfa4 /var/log/x\n";
        let rows = parse_clusters(old).unwrap();
        let err = reject_foreign_cfa4(&rows).unwrap_err();
        assert_eq!(err.code, ErrorCode::DbClusterOldMajor);
        assert!(parse_clusters("18  cfa4    0 online postgres /x /y\n").is_err());
        assert!(parse_clusters("nope\n").is_err());
    }
}
