//! `SET LOCAL ROLE` for the schema owner (plan §4.1, §6). Object ownership follows the role that
//! executes the DDL, so migrations assume `cfa4_owner_<schema>` when the login is a member.
//! A cluster that has no such roles (a throw-away superuser) keeps running as itself.

use tokio_postgres::Transaction;

use crate::StoreError;
use crate::dbtoml::is_pg_ident;

/// Assume `role` for the rest of this transaction when it exists and the current user is a member.
/// Does nothing otherwise.
pub(crate) async fn assume(tx: &Transaction<'_>, role: &str) -> Result<(), StoreError> {
    if !is_pg_ident(role) {
        return Err(StoreError::Query { detail: format!("refusing role name {role}") });
    }
    // A previous SET ROLE in this transaction would make current_user the other owner, who is
    // not a member of this one. RESET ROLE returns to the login, which holds both memberships.
    tx.batch_execute("RESET ROLE").await.map_err(StoreError::postgres)?;
    let row = tx
        .query_one("SELECT (SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)) AND pg_has_role(current_user, $1, 'MEMBER')", &[&role])
        .await
        .map_err(StoreError::postgres)?;
    let member: bool = row.try_get(0).map_err(StoreError::postgres)?;
    if !member {
        return Ok(());
    }
    let row = tx.query_one("SELECT format('SET LOCAL ROLE %I', $1::text)", &[&role]).await.map_err(StoreError::postgres)?;
    let sql: String = row.try_get(0).map_err(StoreError::postgres)?;
    if !sql.starts_with("SET LOCAL ROLE ") || sql.contains(';') {
        return Err(StoreError::Query { detail: "refusing an unexpected SET ROLE statement".into() });
    }
    tx.batch_execute(&sql).await.map_err(StoreError::postgres)?;
    Ok(())
}
