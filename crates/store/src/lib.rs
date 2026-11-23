//! CFA4's PostgreSQL client (CFA4-TASKSTORE-PG-001, PG-1 and PG-2): the connection pool, the
//! migrations and the typed `tasks` schema of the shared CFA4 cluster.
//!
//! The driver is `tokio-postgres` with the `deadpool-postgres` pool (owner decision,
//! 2026-11-11; shared with the BacklotHub). PostgreSQL is a separate process (PostgreSQL License,
//! GPL-compatible); CFA4 only connects to it as a client, in pure Rust.
//!
//! - [`Store::connect`] opens the pool and checks one connection. A missing or stopped server is
//!   [`StoreError::Unavailable`] with how to install it: editing, playback and saving never
//!   depend on the database, only the queue and the AI do (`AGENTS.md` §0).
//! - [`Store::migrate`] brings every component's schema up to date, [`Store::rollback`] takes
//!   one step back; each step runs in its own transaction.
//! - [`tasks::TaskStore`] keeps the `cfa4-tasks` queue in the `tasks` schema. The queue's rules
//!   live in [`cfa4_tasks::Queue`] only: every operation locks the rows it touches
//!   (`FOR UPDATE`, the scheduler with `SKIP LOCKED`), runs the same rule on them, and writes back
//!   the changed tasks, their transitions and the outbox events in the same transaction. If the
//!   transaction does not commit, nothing happened, so a task never starts twice.
//!
//! The layer is L5 because the crate does socket operating-system I/O: a Unix-domain socket to the
//! PostgreSQL server, the same class of OS I/O as `cfa4-platform` (P4).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod ai;
pub mod backend;
pub mod cache;
pub mod dbtoml;
pub mod migrate;
pub(crate) mod role;
pub mod setup;
pub mod tasks;

use std::time::Duration;

use deadpool_postgres::{Pool, PoolConfig, Runtime, Timeouts};
use serde::{Deserialize, Serialize};
use tokio_postgres::NoTls;

pub use backend::{MemoryTaskStore, TaskBackend};
pub use tasks::TaskStore;

/// How to reach the CFA4 cluster. The defaults are the local Unix socket (`/var/run/postgresql`)
/// and the `cfa4` database as [`LOGIN_ROLE`] (D-1). The port is whatever `cfa4-db-setup` recorded
/// in `<config dir>/cfa4/db.toml` ([`StoreConfig::load`], D-2); 5432 is only the fallback when that
/// file is absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoreConfig {
    /// A host name, or a directory for a Unix socket (starts with `/`).
    pub host: String,
    pub port: u16,
    pub user: String,
    pub dbname: String,
    /// Not needed with peer authentication over the local socket.
    #[serde(skip_serializing)]
    pub password: Option<String>,
    /// Connections in the pool (few: the 16 GB minimum machine, PG-001 §8).
    pub pool_size: usize,
    pub connect_timeout_ms: u64,
}

impl Default for StoreConfig {
    fn default() -> Self {
        StoreConfig {
            host: "/var/run/postgresql".into(),
            port: 5432,
            user: LOGIN_ROLE.into(),
            dbname: "cfa4".into(),
            password: None,
            pool_size: 4,
            connect_timeout_ms: 3_000,
        }
    }
}

/// The login role `cfa4-db-setup` maps the OS user to (D-1).
pub const LOGIN_ROLE: &str = "cfa4_login";

/// English error codes (R4). The text is stable; callers match on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// PostgreSQL 18 is not installed, or nothing is listening on the configured socket.
    #[serde(rename = "DB_SERVER_MISSING")]
    DbServerMissing,
    /// The server was there and is not accepting connections (stopped, refused, timed out).
    #[serde(rename = "DB_SERVER_STOPPED")]
    DbServerStopped,
    /// The socket answered, and peer or password authentication failed.
    #[serde(rename = "DB_AUTH_FAILED")]
    DbAuthFailed,
    /// `db.toml` or a setup argument is not usable.
    #[serde(rename = "DB_CONFIG_INVALID")]
    DbConfigInvalid,
    /// A `cfa4` cluster exists on a major version other than 18. It was not modified.
    #[serde(rename = "DB_CLUSTER_OLD_MAJOR")]
    DbClusterOldMajor,
    /// `cfa4-db-setup` was not run as root (`postinst` or `pkexec`).
    #[serde(rename = "DB_SETUP_PRIVILEGE")]
    DbSetupPrivilege,
    /// The setup command ran and PostgreSQL rejected a step. Nothing was left half-applied
    /// beyond what that step's own tool rolled back.
    #[serde(rename = "DB_SETUP_FAILED")]
    DbSetupFailed,
    #[serde(rename = "DB_MIGRATION_FAILED")]
    DbMigrationFailed,
    #[serde(rename = "DB_QUERY_FAILED")]
    DbQueryFailed,
    #[serde(rename = "DB_CORRUPT_ROW")]
    DbCorruptRow,
    #[serde(rename = "DB_QUEUE_REJECTED")]
    DbQueueRejected,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::DbServerMissing => "DB_SERVER_MISSING",
            ErrorCode::DbServerStopped => "DB_SERVER_STOPPED",
            ErrorCode::DbAuthFailed => "DB_AUTH_FAILED",
            ErrorCode::DbConfigInvalid => "DB_CONFIG_INVALID",
            ErrorCode::DbClusterOldMajor => "DB_CLUSTER_OLD_MAJOR",
            ErrorCode::DbSetupPrivilege => "DB_SETUP_PRIVILEGE",
            ErrorCode::DbSetupFailed => "DB_SETUP_FAILED",
            ErrorCode::DbMigrationFailed => "DB_MIGRATION_FAILED",
            ErrorCode::DbQueryFailed => "DB_QUERY_FAILED",
            ErrorCode::DbCorruptRow => "DB_CORRUPT_ROW",
            ErrorCode::DbQueueRejected => "DB_QUEUE_REJECTED",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every failure is a value: the caller shows it, CFA4 keeps running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum StoreError {
    /// No server to talk to (not installed, not running, wrong socket or credentials).
    Unavailable { code: ErrorCode, detail: String, hint: String },
    /// A migration step failed; the transaction was rolled back.
    Migration { component: String, version: u32, detail: String },
    /// `db.toml` is present and not usable. The defaults are not applied in its place.
    Config { detail: String },
    /// A query failed (the transaction was rolled back).
    Query { detail: String },
    /// A stored row does not decode: reported, never trusted, never a panic.
    Corrupt { id: String, detail: String },
    /// The queue refused the operation.
    Queue { error: cfa4_tasks::QueueError },
}

/// Shown when the server cannot be reached. Points at `cfa4-db-setup`, not at `createuser` /
/// `createdb` on the default cluster (P6, D-4).
pub const INSTALL_HINT: &str = "CFA4's task queue and AI features need the CFA4 PostgreSQL cluster. Run `cfa4-db-setup` (a .deb runs it from postinst when PostgreSQL is installed; otherwise the application offers it through pkexec on first launch). Editing, playback and saving work without it.";

impl StoreConfig {
    /// The cluster recorded by `cfa4-db-setup`, or [`StoreConfig::default`] when `db.toml` is absent.
    pub fn load() -> Result<Self, StoreError> {
        match dbtoml::default_path() {
            Some(path) if path.is_file() => Self::load_from(&path),
            _ => Ok(Self::default()),
        }
    }

    /// Read one `db.toml`. A missing file is an error here; [`StoreConfig::load`] treats absence as
    /// the defaults.
    pub fn load_from(path: &std::path::Path) -> Result<Self, StoreError> {
        dbtoml::load(path)
    }
}

impl StoreError {
    pub fn code(&self) -> ErrorCode {
        match self {
            StoreError::Unavailable { code, .. } => *code,
            StoreError::Config { .. } => ErrorCode::DbConfigInvalid,
            StoreError::Migration { .. } => ErrorCode::DbMigrationFailed,
            StoreError::Query { .. } => ErrorCode::DbQueryFailed,
            StoreError::Corrupt { .. } => ErrorCode::DbCorruptRow,
            StoreError::Queue { .. } => ErrorCode::DbQueueRejected,
        }
    }

    pub(crate) fn query(e: impl std::fmt::Display) -> Self {
        StoreError::Query { detail: e.to_string() }
    }

    /// `tokio_postgres::Error`'s `Display` is often only "db error"; the SQLSTATE and message
    /// are what the caller can act on.
    pub(crate) fn postgres(e: tokio_postgres::Error) -> Self {
        StoreError::Query { detail: postgres_detail(&e) }
    }

    pub(crate) fn unavailable(err: impl std::error::Error) -> Self {
        let detail = error_chain(&err);
        StoreError::Unavailable { code: classify_unavailable(&detail), detail, hint: INSTALL_HINT.into() }
    }

    pub(crate) fn config(detail: impl std::fmt::Display) -> Self {
        StoreError::Config { detail: detail.to_string() }
    }
}

/// Input checks shared by the typed schemas: every value from a caller is hostile, so lengths,
/// the SHA-256 form and the UUID form are checked before they reach SQL (which checks them again).
pub(crate) fn check_text(field: &str, value: &str, min: usize, max: usize) -> Result<(), StoreError> {
    let n = value.chars().count();
    if n < min || n > max || value.contains('\0') {
        return Err(StoreError::Query { detail: format!("{field} must be {min}..={max} characters without NUL") });
    }
    Ok(())
}

pub(crate) fn check_sha256(field: &str, value: &str) -> Result<(), StoreError> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Ok(())
    } else {
        Err(StoreError::Query { detail: format!("{field} must be 64 lowercase hex characters") })
    }
}

pub(crate) fn check_uuid(field: &str, value: &str) -> Result<(), StoreError> {
    let ok = value.len() == 36 && value.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() });
    if ok { Ok(()) } else { Err(StoreError::Query { detail: format!("{field} must be a UUID") }) }
}

pub(crate) fn postgres_detail(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        let mut text = format!("{}: {}", db.code().code(), db.message());
        if let Some(detail) = db.detail() {
            text.push_str(": ");
            text.push_str(detail);
        }
        return text;
    }
    error_chain(e)
}

/// Map a driver error onto a server code. Anything we cannot classify is "missing": the caller
/// still gets [`StoreError::Unavailable`] and the setup hint, never a panic.
/// Display plus `Error::source`, because the pool's own message hides "connection refused".
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut parts = Vec::new();
    let mut current = Some(err);
    let mut depth = 0u8;
    while let Some(item) = current {
        if depth == 8 {
            break;
        }
        depth = depth.saturating_add(1);
        let text = item.to_string();
        if parts.last().is_none_or(|prev: &String| prev != &text) {
            parts.push(text);
        }
        current = item.source();
    }
    let joined = parts.join(": ");
    if joined.len() <= 2_000 { joined } else { joined.get(..2_000).unwrap_or(&joined).to_owned() }
}

pub(crate) fn classify_unavailable(detail: &str) -> ErrorCode {
    let d = detail.to_ascii_lowercase();
    if d.contains("authentication failed") || d.contains("pg_hba") || d.contains("peer authentication") {
        return ErrorCode::DbAuthFailed;
    }
    if d.contains("connection refused")
        || d.contains("timed out")
        || d.contains("timeout")
        || d.contains("os error 111")
        || d.contains("os error 110")
        || d.contains("unreachable")
    {
        return ErrorCode::DbServerStopped;
    }
    ErrorCode::DbServerMissing
}

impl From<cfa4_tasks::QueueError> for StoreError {
    fn from(error: cfa4_tasks::QueueError) -> Self {
        StoreError::Queue { error }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Unavailable { code, detail, hint } => write!(f, "{code}: PostgreSQL is not available ({detail}). {hint}"),
            StoreError::Config { detail } => write!(f, "{}: {detail}", ErrorCode::DbConfigInvalid),
            StoreError::Migration { component, version, detail } => write!(f, "migration {component} {version} failed: {detail}"),
            StoreError::Query { detail } => write!(f, "query failed: {detail}"),
            StoreError::Corrupt { id, detail } => write!(f, "stored row {id} does not decode: {detail}"),
            StoreError::Queue { error } => write!(f, "queue: {error:?}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The pool over the CFA4 cluster.
#[derive(Clone)]
pub struct Store {
    pool: Pool,
}

impl Store {
    /// Open the pool and check that one connection works.
    pub async fn connect(config: &StoreConfig) -> Result<Store, StoreError> {
        let timeout = Duration::from_millis(config.connect_timeout_ms.max(1));
        let mut pg = deadpool_postgres::Config::new();
        pg.host = Some(config.host.clone());
        pg.port = Some(config.port);
        pg.user = Some(config.user.clone());
        pg.dbname = Some(config.dbname.clone());
        pg.password = config.password.clone();
        pg.application_name = Some("cfa4".into());
        pg.connect_timeout = Some(timeout);
        let mut pool_config = PoolConfig::new(config.pool_size.clamp(1, 64));
        pool_config.timeouts = Timeouts { wait: Some(timeout), create: Some(timeout), recycle: Some(timeout) };
        pg.pool = Some(pool_config);
        let pool = pg.create_pool(Some(Runtime::Tokio1), NoTls).map_err(StoreError::unavailable)?;
        let client = pool.get().await.map_err(StoreError::unavailable)?;
        client.simple_query("SELECT 1").await.map_err(StoreError::unavailable)?;
        Ok(Store { pool })
    }

    pub(crate) async fn client(&self) -> Result<deadpool_postgres::Object, StoreError> {
        self.pool.get().await.map_err(StoreError::unavailable)
    }

    /// Bring every component's schema up to date; the versions applied now.
    pub async fn migrate(&self) -> Result<Vec<(String, u32)>, StoreError> {
        migrate::up(self).await
    }

    /// Undo the newest migration step of `component`; the version undone, `None` when nothing
    /// was applied.
    pub async fn rollback(&self, component: &str) -> Result<Option<u32>, StoreError> {
        migrate::down(self, component).await
    }

    /// The AI job store, model registry and licence acceptances in the `ai` schema.
    pub fn ai(&self) -> ai::AiStore {
        ai::AiStore::new(self.clone())
    }

    /// The cache index in the `cache` schema.
    pub fn cache(&self) -> cache::CacheIndex {
        cache::CacheIndex::new(self.clone())
    }

    /// The task queue in the `tasks` schema.
    pub fn tasks(&self, config: cfa4_tasks::QueueConfig) -> TaskStore {
        TaskStore::new(self.clone(), config)
    }
}

#[cfg(test)]
pub(crate) mod testdb;

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn the_default_login_is_the_mapped_role_and_the_hint_names_the_setup_command() {
        let config = StoreConfig::default();
        assert_eq!(config.user, LOGIN_ROLE);
        assert_eq!(LOGIN_ROLE, "cfa4_login");
        assert!(INSTALL_HINT.contains("cfa4-db-setup"));
        assert!(!INSTALL_HINT.contains("createuser"));
        assert!(!INSTALL_HINT.contains("createdb"));
        assert_eq!(ErrorCode::DbServerMissing.as_str(), "DB_SERVER_MISSING");
        assert_eq!(ErrorCode::DbServerStopped.as_str(), "DB_SERVER_STOPPED");
    }

    #[test]
    fn connection_failures_keep_a_stable_code() {
        assert_eq!(classify_unavailable("No such file or directory (os error 2)"), ErrorCode::DbServerMissing);
        assert_eq!(classify_unavailable("error connecting: Connection refused (os error 111)"), ErrorCode::DbServerStopped);
        assert_eq!(classify_unavailable("connection timed out"), ErrorCode::DbServerStopped);
        assert_eq!(classify_unavailable("password authentication failed"), ErrorCode::DbAuthFailed);
        let err = StoreError::unavailable(std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"));
        assert_eq!(err.code(), ErrorCode::DbServerMissing);
        let text = err.to_string();
        assert!(text.starts_with("DB_SERVER_MISSING:"));
        assert!(text.contains("cfa4-db-setup"));
    }
}
