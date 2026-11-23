//! The `cache` schema (PG-3): the index of the content-addressed cache.
//!
//! The files stay on disk; a row says which key lives in which file, how big it is, which tier it
//! belongs to and when it was last used. [`CacheIndex::evict_to`] picks the least recently used
//! entries until the cache fits a byte budget and returns them; the caller deletes the files, so a
//! crash between the two leaves an orphan file, never an index row that points at nothing.

use serde::{Deserialize, Serialize};
use tokio_postgres::Row;

use crate::{Store, StoreError, check_text};

/// At most this many entries are evicted in one call.
const MAX_EVICT: i64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Preview,
    Final,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Tier::Preview => "preview",
            Tier::Final => "final",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub key: String,
    pub file: String,
    pub size_bytes: u64,
    pub tier: Tier,
    pub last_used_at: i64,
}

pub struct CacheIndex {
    store: Store,
}

const COLS: &str = "key, file, size_bytes, tier, (extract(epoch from last_used_at) * 1000)::bigint AS used_ms";

fn entry_from(row: &Row) -> Result<CacheEntry, StoreError> {
    let key: String = row.try_get("key").map_err(StoreError::postgres)?;
    let bad = |e: &dyn std::fmt::Display| StoreError::Corrupt { id: key.clone(), detail: e.to_string() };
    let size: i64 = row.try_get("size_bytes").map_err(|e| bad(&e))?;
    let tier: String = row.try_get("tier").map_err(|e| bad(&e))?;
    Ok(CacheEntry {
        file: row.try_get("file").map_err(|e| bad(&e))?,
        size_bytes: u64::try_from(size).map_err(|e| bad(&e))?,
        tier: match tier.as_str() {
            "preview" => Tier::Preview,
            "final" => Tier::Final,
            other => return Err(bad(&format!("unknown tier {other}"))),
        },
        last_used_at: row.try_get("used_ms").map_err(|e| bad(&e))?,
        key: key.clone(),
    })
}

impl CacheIndex {
    pub(crate) fn new(store: Store) -> Self {
        CacheIndex { store }
    }

    /// Add or replace an entry; it counts as used now.
    pub async fn put(&self, key: &str, file: &str, size_bytes: u64, tier: Tier) -> Result<(), StoreError> {
        check_text("key", key, 1, 256)?;
        check_text("file", file, 1, 4096)?;
        let size = i64::try_from(size_bytes).map_err(|_| StoreError::Query { detail: "size does not fit a bigint".into() })?;
        let client = self.store.client().await?;
        client
            .execute(
                "INSERT INTO cache.entry (key, file, size_bytes, tier) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (key) DO UPDATE SET file = EXCLUDED.file, size_bytes = EXCLUDED.size_bytes, tier = EXCLUDED.tier, last_used_at = now()",
                &[&key, &file, &size, &tier.as_str()],
            )
            .await
            .map_err(StoreError::postgres)?;
        Ok(())
    }

    /// Look an entry up and mark it used now.
    pub async fn get(&self, key: &str) -> Result<Option<CacheEntry>, StoreError> {
        check_text("key", key, 1, 256)?;
        let client = self.store.client().await?;
        let row = client
            .query_opt(&format!("UPDATE cache.entry SET last_used_at = now() WHERE key = $1 RETURNING {COLS}"), &[&key])
            .await
            .map_err(StoreError::postgres)?;
        row.as_ref().map(entry_from).transpose()
    }

    /// Forget an entry; `true` when it existed. The caller deletes the file.
    pub async fn remove(&self, key: &str) -> Result<bool, StoreError> {
        check_text("key", key, 1, 256)?;
        let client = self.store.client().await?;
        let n = client.execute("DELETE FROM cache.entry WHERE key = $1", &[&key]).await.map_err(StoreError::postgres)?;
        Ok(n == 1)
    }

    /// The bytes of every entry, or of one tier.
    pub async fn total_bytes(&self, tier: Option<Tier>) -> Result<u64, StoreError> {
        let client = self.store.client().await?;
        let tier = tier.map(Tier::as_str);
        let row = client
            .query_one("SELECT COALESCE(sum(size_bytes), 0)::bigint FROM cache.entry WHERE $1::text IS NULL OR tier = $1", &[&tier])
            .await
            .map_err(StoreError::postgres)?;
        let total: i64 = row.try_get(0).map_err(StoreError::postgres)?;
        Ok(u64::try_from(total).unwrap_or(0))
    }

    /// Remove the least recently used entries of `tier` (preview and final when `None`) until the
    /// remaining bytes fit `budget_bytes`, and return them so the caller can delete their files.
    /// One transaction: the rows the cache picked are the rows that are gone.
    pub async fn evict_to(&self, budget_bytes: u64, tier: Option<Tier>) -> Result<Vec<CacheEntry>, StoreError> {
        let budget = i64::try_from(budget_bytes).unwrap_or(i64::MAX);
        let tier = tier.map(Tier::as_str);
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::postgres)?;
        // newest first: running size of what is kept; whatever pushes the sum over the budget goes
        let rows = tx
            .query(
                "WITH ranked AS (SELECT key, sum(size_bytes) OVER (ORDER BY last_used_at DESC, key DESC)::bigint AS kept FROM cache.entry \
                     WHERE $1::text IS NULL OR tier = $1), \
                     gone AS (DELETE FROM cache.entry e USING ranked r WHERE e.key = r.key AND r.kept > $2 \
                     AND e.key IN (SELECT key FROM ranked WHERE kept > $2 ORDER BY kept DESC LIMIT $3) RETURNING e.*) \
                     SELECT key, file, size_bytes, tier, (extract(epoch from last_used_at) * 1000)::bigint AS used_ms FROM gone ORDER BY last_used_at, key",
                &[&tier, &budget, &MAX_EVICT],
            )
            .await
            .map_err(StoreError::postgres)?;
        let out = rows.iter().map(entry_from).collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(StoreError::postgres)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
