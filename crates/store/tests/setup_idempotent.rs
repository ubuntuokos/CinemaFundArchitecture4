//! `cfa4-db-setup` twice on a real PostgreSQL 18 cluster (plan §5.2 step 7, gate 3).
//!
//! Local runs skip unless `CFA4_DB_SETUP=1`. The PG-1 CI job sets `CFA4_PG_REQUIRE=1` as well,
//! and a skip is then a failure. The test refuses to adopt a cfa4 cluster it did not create.

use std::path::Path;
use std::process::Command;

fn required() -> bool {
    std::env::var("CFA4_PG_REQUIRE").ok().as_deref() == Some("1")
}

fn enabled() -> bool {
    std::env::var("CFA4_DB_SETUP").ok().as_deref() == Some("1")
}

#[test]
fn setup_twice_leaves_the_same_state() {
    if !enabled() {
        if required() {
            panic!("CFA4_PG_REQUIRE=1 but CFA4_DB_SETUP is not 1");
        }
        eprintln!("cfa4-db-setup idempotence: set CFA4_DB_SETUP=1; skipped");
        return;
    }
    if pg_lsclusters().lines().any(|line| line.split_whitespace().nth(1) == Some("cfa4")) {
        panic!("a cfa4 cluster already exists; refusing to modify it");
    }
    let _cleanup = DropCluster { armed: true };
    let config_dir = std::env::temp_dir().join(format!("cfa4-db-setup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config_dir);
    std::fs::create_dir_all(&config_dir).unwrap();
    let os_user = std::env::var("CFA4_OS_USER").unwrap_or_else(|_| current_user());
    let main_before = snapshot_main();

    let first = invoke(&config_dir, &os_user);
    assert_eq!(first.status, 0, "first setup failed: {}", first.stderr);
    assert!(first.stdout.contains("changed=true"), "first run should record a change:\n{}", first.stdout);
    let state = capture(&first.stdout);
    assert!(state.port != 0);

    let second = invoke(&config_dir, &os_user);
    assert_eq!(second.status, 0, "second setup failed: {}", second.stderr);
    assert!(second.stdout.contains("changed=false"), "second run changed something:\n{}", second.stdout);
    let again = capture(&second.stdout);
    assert_eq!(again.port, state.port);
    assert_eq!(again.files, state.files, "files differ after the second run");
    assert_eq!(again.roles, state.roles, "roles differ after the second run");
    assert_eq!(again.members, state.members, "memberships differ after the second run");
    assert_eq!(again.database, state.database);
    assert_eq!(snapshot_main(), main_before, "the default cluster was modified");
    assert!(state.files.iter().any(|(path, body)| path.ends_with("pg_ident.conf") && body.contains(&format!("cfa4map {os_user} cfa4_login"))));
    assert!(state.files.iter().any(|(path, body)| path.ends_with("pg_hba.conf") && body.contains("peer map=cfa4map") && !body.contains("trust")));

    let who = psql(state.port, &os_user, "cfa4_login", "cfa4", "SELECT current_user");
    assert_eq!(who.trim(), "cfa4_login", "peer map did not log {os_user} in as cfa4_login: {who}");
    let version = psql(state.port, "postgres", "postgres", "postgres", "SELECT current_setting('server_version_num')");
    let version_num: i64 = version.trim().parse().expect("server version");
    assert!((180000..190000).contains(&version_num), "PostgreSQL 18 required, got {version}");
    let _ = std::fs::remove_dir_all(&config_dir);
}

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

fn invoke(config_dir: &Path, os_user: &str) -> Run {
    let bin = env!("CARGO_BIN_EXE_cfa4-db-setup");
    let mut cmd = Command::new("sudo");
    cmd.args(["-n", bin, "--config-dir", &config_dir.display().to_string(), "--os-user", os_user]);
    let out = cmd.output().expect("sudo cfa4-db-setup");
    Run {
        status: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

struct State {
    port: u16,
    files: Vec<(String, String)>,
    roles: String,
    members: String,
    database: String,
}

fn capture(stdout: &str) -> State {
    let port = stdout.lines().find_map(|line| line.strip_prefix("port=")).and_then(|v| v.parse().ok()).unwrap_or(0);
    let paths = [
        "/etc/postgresql/18/cfa4/pg_hba.conf",
        "/etc/postgresql/18/cfa4/pg_ident.conf",
        "/etc/postgresql/18/cfa4/conf.d/cfa4.conf",
        "/etc/postgresql/18/cfa4/start.conf",
    ];
    let mut files = Vec::new();
    for path in paths {
        files.push((path.to_owned(), read_as_root(path)));
    }
    if let Some(config) = stdout.lines().find_map(|line| line.strip_prefix("config=")) {
        files.push((config.to_owned(), read_as_root(config)));
    }
    let roles = psql(port, "postgres", "postgres", "postgres", "SELECT rolname || ' ' || rolcanlogin::text FROM pg_roles ORDER BY 1");
    let roles = roles.lines().filter(|line| line.starts_with("cfa4_")).collect::<Vec<_>>().join("\n");
    State {
        port,
        files,
        roles,
        members: psql(
            port,
            "postgres",
            "postgres",
            "postgres",
            "SELECT r.rolname || ' ' || m.rolname FROM pg_auth_members am JOIN pg_roles r ON r.oid = am.roleid JOIN pg_roles m ON m.oid = am.member WHERE m.rolname = 'cfa4_login' ORDER BY 1",
        ),
        database: psql(port, "postgres", "postgres", "postgres", "SELECT datname FROM pg_database WHERE datname = 'cfa4'"),
    }
}

fn psql(port: u16, os_user: &str, db_user: &str, db: &str, sql: &str) -> String {
    let port = port.to_string();
    let mut cmd = Command::new("sudo");
    cmd.args([
        "-n",
        "runuser",
        "-u",
        os_user,
        "--",
        "psql",
        "-X",
        "-v",
        "ON_ERROR_STOP=1",
        "-At",
        "-h",
        "/var/run/postgresql",
        "-p",
        &port,
        "-U",
        db_user,
        "-d",
        db,
        "-c",
        sql,
    ]);
    let out = cmd.output().expect("psql");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "psql failed: {stderr}\n{stdout}");
    stdout.trim().to_owned()
}

fn snapshot_main() -> Vec<(String, String)> {
    ["/etc/postgresql/18/main/pg_hba.conf", "/etc/postgresql/18/main/pg_ident.conf", "/etc/postgresql/18/main/postgresql.conf"]
        .into_iter()
        .map(|path| (path.to_owned(), read_as_root(path)))
        .collect()
}

fn read_as_root(path: &str) -> String {
    let out = Command::new("sudo").args(["-n", "cat", "--", path]).output().expect("sudo cat");
    if out.status.success() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        format!("unreadable {path}: {}", String::from_utf8_lossy(&out.stderr))
    }
}

fn pg_lsclusters() -> String {
    let out = Command::new("pg_lsclusters").output().expect("pg_lsclusters");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn current_user() -> String {
    let out = Command::new("id").arg("-un").output().expect("id");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct DropCluster {
    armed: bool,
}

impl Drop for DropCluster {
    fn drop(&mut self) {
        if self.armed {
            let _ = Command::new("sudo").args(["-n", "pg_dropcluster", "--stop", "18", "cfa4"]).status();
        }
    }
}
