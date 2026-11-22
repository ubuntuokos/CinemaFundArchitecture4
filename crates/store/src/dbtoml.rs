//! `<config dir>/cfa4/db.toml` (D-2). `cfa4-db-setup` writes the port the cluster was given;
//! [`crate::StoreConfig::load`] reads it. The file is small and hostile: a bad one is
//! [`crate::ErrorCode::DbConfigInvalid`], never a panic.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{StoreConfig, StoreError};

const MAX_BYTES: u64 = 65_536;

/// `$CFA4_DB_TOML`, or `<config home>/cfa4/db.toml`.
///
/// The config home is `$CFA4_CONFIG_DIR`, then `$XDG_CONFIG_HOME`, then `$HOME/.config`.
pub(crate) fn default_path() -> Option<PathBuf> {
    let env = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(path) = env("CFA4_DB_TOML") {
        return Some(path);
    }
    let home = env("CFA4_CONFIG_DIR").or_else(|| env("XDG_CONFIG_HOME")).or_else(|| env("HOME").map(|h| h.join(".config")))?;
    Some(home.join("cfa4").join("db.toml"))
}

pub(crate) fn load(path: &Path) -> Result<StoreConfig, StoreError> {
    let meta = fs::metadata(path).map_err(|e| StoreError::config(format!("cannot read {}: {e}", path.display())))?;
    if meta.len() > MAX_BYTES {
        return Err(StoreError::config(format!("{} is larger than {MAX_BYTES} bytes", path.display())));
    }
    let bytes = fs::read(path).map_err(|e| StoreError::config(format!("cannot read {}: {e}", path.display())))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| StoreError::config(format!("{} is not UTF-8", path.display())))?;
    parse(text)
}

pub(crate) fn parse(text: &str) -> Result<StoreConfig, StoreError> {
    let mut cfg = StoreConfig::default();
    let mut seen_host = false;
    let mut seen_port = false;
    let mut seen_user = false;
    let mut seen_db = false;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index.saturating_add(1);
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(StoreError::config(format!("line {line_no}: expected key = value")));
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "host" => {
                if seen_host {
                    return Err(StoreError::config(format!("line {line_no}: host is set twice")));
                }
                seen_host = true;
                cfg.host = parse_host(value, line_no)?;
            }
            "port" => {
                if seen_port {
                    return Err(StoreError::config(format!("line {line_no}: port is set twice")));
                }
                seen_port = true;
                cfg.port = parse_port(value, line_no)?;
            }
            "user" => {
                if seen_user {
                    return Err(StoreError::config(format!("line {line_no}: user is set twice")));
                }
                seen_user = true;
                cfg.user = parse_ident(value, "user", line_no)?;
            }
            "dbname" => {
                if seen_db {
                    return Err(StoreError::config(format!("line {line_no}: dbname is set twice")));
                }
                seen_db = true;
                cfg.dbname = parse_ident(value, "dbname", line_no)?;
            }
            _ => return Err(StoreError::config(format!("line {line_no}: unknown key {key}"))),
        }
    }
    Ok(cfg)
}

pub(crate) fn render(config: &StoreConfig) -> String {
    format!(
        "# Written by cfa4-db-setup. The port is the one the cfa4 cluster was given (D-2).\n\
         host = \"{}\"\n\
         port = {}\n\
         user = \"{}\"\n\
         dbname = \"{}\"\n",
        config.host, config.port, config.user, config.dbname
    )
}

/// Write `db.toml` for `config` when the bytes would change. `true` when the file was replaced.
pub(crate) fn write_if_changed(path: &Path, config: &StoreConfig) -> Result<bool, StoreError> {
    let text = render(config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| StoreError::config(format!("cannot create {}: {e}", parent.display())))?;
    }
    if path.is_file() {
        match fs::read_to_string(path) {
            Ok(existing) if existing == text => return Ok(false),
            Ok(_) => {}
            Err(e) => return Err(StoreError::config(format!("cannot read {}: {e}", path.display()))),
        }
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &text).map_err(|e| StoreError::config(format!("cannot write {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|e| StoreError::config(format!("cannot replace {}: {e}", path.display())))?;
    Ok(true)
}

fn unquote(value: &str, line_no: usize) -> Result<&str, StoreError> {
    let Some(inner) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return Err(StoreError::config(format!("line {line_no}: expected a quoted string")));
    };
    if inner.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        return Err(StoreError::config(format!("line {line_no}: unsupported character in string")));
    }
    Ok(inner)
}

fn parse_host(value: &str, line_no: usize) -> Result<String, StoreError> {
    let inner = unquote(value, line_no)?;
    if inner.is_empty() || inner.len() > 4096 {
        return Err(StoreError::config(format!("line {line_no}: host length is not usable")));
    }
    let ok = if inner.starts_with('/') {
        inner.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
    } else {
        inner.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
    };
    if !ok {
        return Err(StoreError::config(format!("line {line_no}: host has a character db.toml does not accept")));
    }
    Ok(inner.to_owned())
}

fn parse_ident(value: &str, what: &str, line_no: usize) -> Result<String, StoreError> {
    let inner = unquote(value, line_no)?;
    if !is_pg_ident(inner) {
        return Err(StoreError::config(format!("line {line_no}: {what} is not a PostgreSQL identifier")));
    }
    Ok(inner.to_owned())
}

fn parse_port(value: &str, line_no: usize) -> Result<u16, StoreError> {
    let text = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(value);
    if !text.bytes().all(|b| b.is_ascii_digit()) || text.is_empty() || text.len() > 5 {
        return Err(StoreError::config(format!("line {line_no}: port is not an integer")));
    }
    let port: u32 = text.parse().map_err(|_| StoreError::config(format!("line {line_no}: port is not an integer")))?;
    u16::try_from(port).ok().filter(|p| *p != 0).ok_or_else(|| StoreError::config(format!("line {line_no}: port must be 1..=65535")))
}

pub(crate) fn is_pg_ident(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(&first) = bytes.first() else { return false };
    bytes.len() <= 63 && (first.is_ascii_alphabetic() || first == b'_') && bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_reads_back_and_a_second_write_does_not_change_it() {
        let dir = std::env::temp_dir().join(format!("cfa4-dbtoml-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.toml");
        let cfg = StoreConfig { port: 5433, ..StoreConfig::default() };
        assert!(write_if_changed(&path, &cfg).unwrap());
        assert!(!write_if_changed(&path, &cfg).unwrap());
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.port, 5433);
        assert_eq!(loaded.user, crate::LOGIN_ROLE);
        assert_eq!(loaded.dbname, "cfa4");
        assert_eq!(loaded.host, "/var/run/postgresql");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_db_toml_is_a_config_error() {
        let bad = [
            "port = 0\n",
            "port = 65536\n",
            "port = -1\n",
            "port = \"no\"\n",
            "port = 5432\nport = 5433\n",
            "host = \"/tmp/odd socket\"\n",
            "host = \"/tmp/a\\nb\"\n",
            "user = \"\"\n",
            "user = \"cfa4-login\"\n",
            "mystery = 1\n",
            "not a line\n",
        ];
        for text in bad {
            let err = parse(text).expect_err(text);
            assert_eq!(err.code(), crate::ErrorCode::DbConfigInvalid, "{text} -> {err}");
        }
        let huge = "a".repeat(70_000);
        let dir = std::env::temp_dir().join(format!("cfa4-dbtoml-huge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.toml");
        fs::write(&path, huge).unwrap();
        assert_eq!(load(&path).unwrap_err().code(), crate::ErrorCode::DbConfigInvalid);
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert_eq!(load(&path).unwrap_err().code(), crate::ErrorCode::DbConfigInvalid);
        let _ = fs::remove_dir_all(&dir);
    }
}
