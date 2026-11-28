//! The `mi` schema (Media Intelligence, F1): the derived index of media assets.
//!
//! Purely derived data (P9): the `.fcproj` wins and everything here can be rebuilt. This first
//! slice holds what the `index` task type needs: assets identified by content hash, index
//! generations swapped in one transaction (AC-09: the old generation stays active until the new
//! one is complete) and the per-stage checkpoints of an index task. The segment, transcript and
//! search methods come with the stages that fill them.
//!
//! Ids are UUIDs the caller chooses (UUIDv7 by convention); every value from a caller is checked
//! before it reaches SQL, which checks it again.

use cfa4_mi::{ContentHash, HashAlg, IndexState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::Row;

use crate::{Store, StoreError, check_text, check_uuid};

/// A checkpoint detail larger than this is refused (hostile input).
const MAX_DETAIL_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    /// `None` while the asset is `pending_hash`.
    pub hash: Option<ContentHash>,
    pub last_path: Option<String>,
    pub duration_ticks: Option<i64>,
    pub index_state: IndexState,
}

/// The newest finished stage of an index task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCheckpoint {
    pub task_id: String,
    pub stage: u64,
    pub asset_id: String,
    pub generation: u64,
    pub detail: Value,
}

pub struct MiStore {
    store: Store,
}

const ASSET_COLS: &str = "id::text AS id, hash_alg, content_hash, last_path, duration_ticks, index_state";

fn asset_from(row: &Row) -> Result<Asset, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::postgres)?;
    let bad = |e: &dyn std::fmt::Display| StoreError::Corrupt { id: id.clone(), detail: e.to_string() };
    let alg: Option<String> = row.try_get("hash_alg").map_err(|e| bad(&e))?;
    let digest: Option<Vec<u8>> = row.try_get("content_hash").map_err(|e| bad(&e))?;
    let hash = match (alg, digest) {
        (Some(alg), Some(digest)) => {
            let alg = match alg.as_str() {
                "sha256s1" => HashAlg::Sha256Sampled1,
                "sha256" => HashAlg::Sha256,
                other => return Err(bad(&format!("unknown hash algorithm {other}"))),
            };
            let digest: [u8; 32] = digest.try_into().map_err(|_| bad(&"content hash is not 32 bytes"))?;
            Some(ContentHash { alg, digest })
        }
        (None, None) => None,
        _ => return Err(bad(&"hash algorithm and content hash disagree")),
    };
    let state: String = row.try_get("index_state").map_err(|e| bad(&e))?;
    Ok(Asset {
        hash,
        last_path: row.try_get("last_path").map_err(|e| bad(&e))?,
        duration_ticks: row.try_get("duration_ticks").map_err(|e| bad(&e))?,
        index_state: IndexState::parse(&state).map_err(|e| bad(&e))?,
        id,
    })
}

fn to_i64(field: &str, value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Query { detail: format!("{field} is too large") })
}

impl MiStore {
    pub(crate) fn new(store: Store) -> Self {
        Self { store }
    }

    /// Register a media file. With a hash, the asset is found by `(hash_alg, content_hash)`: a file
    /// seen again (moved, renamed, in another project) keeps its id and only its path and duration
    /// change, and the id given here is ignored. Without a hash the asset is new and
    /// `pending_hash` until [`MiStore::set_hash`].
    pub async fn register_asset(
        &self,
        id: &str,
        hash: Option<&ContentHash>,
        last_path: Option<&str>,
        duration_ticks: Option<i64>,
    ) -> Result<Asset, StoreError> {
        check_uuid("asset id", id)?;
        if let Some(p) = last_path {
            check_text("path", p, 1, 4096)?;
        }
        if duration_ticks.is_some_and(|d| d < 0) {
            return Err(StoreError::Query { detail: "duration must not be negative".into() });
        }
        let client = self.store.client().await?;
        let row = match hash {
            Some(h) => {
                let sql = format!(
                    "INSERT INTO mi.asset (id, hash_alg, content_hash, last_path, duration_ticks, index_state) VALUES ($1::text::uuid, $2, $3, $4, $5, 'pending') \
                     ON CONFLICT (hash_alg, content_hash) DO UPDATE SET last_path = COALESCE(EXCLUDED.last_path, mi.asset.last_path), \
                     duration_ticks = COALESCE(EXCLUDED.duration_ticks, mi.asset.duration_ticks), updated_at = now() RETURNING {ASSET_COLS}"
                );
                client.query_one(&sql, &[&id, &h.alg.as_str(), &h.digest.as_slice(), &last_path, &duration_ticks]).await
            }
            None => {
                let sql = format!(
                    "INSERT INTO mi.asset (id, last_path, duration_ticks, index_state) VALUES ($1::text::uuid, $2, $3, 'pending_hash') RETURNING {ASSET_COLS}"
                );
                client.query_one(&sql, &[&id, &last_path, &duration_ticks]).await
            }
        };
        asset_from(&row.map_err(StoreError::postgres)?)
    }

    pub async fn asset(&self, id: &str) -> Result<Option<Asset>, StoreError> {
        check_uuid("asset id", id)?;
        let client = self.store.client().await?;
        let sql = format!("SELECT {ASSET_COLS} FROM mi.asset WHERE id = $1::text::uuid");
        let row = client.query_opt(&sql, &[&id]).await.map_err(StoreError::postgres)?;
        row.as_ref().map(asset_from).transpose()
    }

    /// Record the hash the background indexer computed for a `pending_hash` asset and make it
    /// `pending`. If another asset already has that hash, nothing changes and the id of that
    /// asset is returned (the caller merges: the two rows are the same media).
    pub async fn set_hash(&self, id: &str, hash: &ContentHash) -> Result<Result<(), String>, StoreError> {
        check_uuid("asset id", id)?;
        let client = self.store.client().await?;
        let other = client
            .query_opt(
                "SELECT id::text FROM mi.asset WHERE hash_alg = $1 AND content_hash = $2 AND id <> $3::text::uuid",
                &[&hash.alg.as_str(), &hash.digest.as_slice(), &id],
            )
            .await
            .map_err(StoreError::postgres)?;
        if let Some(row) = other {
            return Ok(Err(row.try_get(0).map_err(StoreError::postgres)?));
        }
        let n = client
            .execute(
                "UPDATE mi.asset SET hash_alg = $1, content_hash = $2, index_state = 'pending', updated_at = now() WHERE id = $3::text::uuid AND index_state = 'pending_hash'",
                &[&hash.alg.as_str(), &hash.digest.as_slice(), &id],
            )
            .await
            .map_err(StoreError::postgres)?;
        if n == 0 {
            return Err(StoreError::Query { detail: "no pending_hash asset with that id".into() });
        }
        Ok(Ok(()))
    }

    /// Set the index state. `pending_hash` is refused: it only exists before the hash is known.
    pub async fn set_index_state(&self, id: &str, state: IndexState) -> Result<bool, StoreError> {
        check_uuid("asset id", id)?;
        if state.needs_hash() {
            return Err(StoreError::Query { detail: "an asset cannot go back to pending_hash".into() });
        }
        let client = self.store.client().await?;
        let n = client
            .execute(
                "UPDATE mi.asset SET index_state = $1, updated_at = now() WHERE id = $2::text::uuid AND index_state <> 'pending_hash'",
                &[&state.as_str(), &id],
            )
            .await
            .map_err(StoreError::postgres)?;
        Ok(n > 0)
    }

    /// Open the next index generation of an asset (state `building`) and return its number.
    /// Concurrent callers get different numbers.
    pub async fn begin_generation(&self, asset_id: &str) -> Result<u64, StoreError> {
        check_uuid("asset id", asset_id)?;
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::postgres)?;
        // the asset row serialises the writers of one asset's generations
        let locked = tx.query_opt("SELECT 1 FROM mi.asset WHERE id = $1::text::uuid FOR UPDATE", &[&asset_id]).await.map_err(StoreError::postgres)?;
        if locked.is_none() {
            return Err(StoreError::Query { detail: "unknown asset".into() });
        }
        let row = tx
            .query_one(
                "INSERT INTO mi.index_generation (asset_id, generation, state) SELECT $1::text::uuid, COALESCE(MAX(generation), 0) + 1, 'building' \
                 FROM mi.index_generation WHERE asset_id = $1::text::uuid RETURNING generation",
                &[&asset_id],
            )
            .await
            .map_err(StoreError::postgres)?;
        let generation: i64 = row.try_get(0).map_err(StoreError::postgres)?;
        tx.commit().await.map_err(StoreError::postgres)?;
        u64::try_from(generation).map_err(|_| StoreError::Corrupt { id: asset_id.into(), detail: "negative generation".into() })
    }

    /// Make a finished `building` generation the active one and retire the previous active one, in
    /// one transaction: a reader sees the old generation or the new one, never none. `false` when
    /// that generation is not `building`.
    pub async fn activate_generation(&self, asset_id: &str, generation: u64) -> Result<bool, StoreError> {
        check_uuid("asset id", asset_id)?;
        let generation = to_i64("generation", generation)?;
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::postgres)?;
        tx.query_opt("SELECT 1 FROM mi.asset WHERE id = $1::text::uuid FOR UPDATE", &[&asset_id]).await.map_err(StoreError::postgres)?;
        let building = tx
            .query_opt(
                "SELECT 1 FROM mi.index_generation WHERE asset_id = $1::text::uuid AND generation = $2 AND state = 'building' FOR UPDATE",
                &[&asset_id, &generation],
            )
            .await
            .map_err(StoreError::postgres)?;
        if building.is_none() {
            tx.rollback().await.map_err(StoreError::postgres)?;
            return Ok(false);
        }
        tx.execute("UPDATE mi.index_generation SET state = 'retired' WHERE asset_id = $1::text::uuid AND state = 'active'", &[&asset_id])
            .await
            .map_err(StoreError::postgres)?;
        tx.execute("UPDATE mi.index_generation SET state = 'active' WHERE asset_id = $1::text::uuid AND generation = $2", &[&asset_id, &generation])
            .await
            .map_err(StoreError::postgres)?;
        tx.execute("UPDATE mi.asset SET index_state = 'indexed', updated_at = now() WHERE id = $1::text::uuid AND index_state <> 'pending_hash'", &[&asset_id])
            .await
            .map_err(StoreError::postgres)?;
        tx.commit().await.map_err(StoreError::postgres)?;
        Ok(true)
    }

    /// The active generation of an asset, if any.
    pub async fn active_generation(&self, asset_id: &str) -> Result<Option<u64>, StoreError> {
        check_uuid("asset id", asset_id)?;
        let client = self.store.client().await?;
        let row = client
            .query_opt("SELECT generation FROM mi.index_generation WHERE asset_id = $1::text::uuid AND state = 'active'", &[&asset_id])
            .await
            .map_err(StoreError::postgres)?;
        row.map(|r| r.try_get::<_, i64>(0).map_err(StoreError::postgres).map(|g| u64::try_from(g).unwrap_or(0))).transpose()
    }

    /// Record that `stage` of an index task is finished. Repeating it replaces the detail (an
    /// at-least-once worker may report a stage twice); the same stage for another asset or
    /// generation is an error.
    pub async fn save_checkpoint(&self, task_id: &str, stage: u64, asset_id: &str, generation: u64, detail: &Value) -> Result<(), StoreError> {
        check_text("task id", task_id, 1, 256)?;
        check_uuid("asset id", asset_id)?;
        let stage = to_i64("stage", stage)?;
        let generation = to_i64("generation", generation)?;
        if detail.to_string().len() > MAX_DETAIL_BYTES {
            return Err(StoreError::Query { detail: format!("checkpoint detail is larger than {MAX_DETAIL_BYTES} bytes") });
        }
        let client = self.store.client().await?;
        let n = client
            .execute(
                "INSERT INTO mi.job_checkpoint (task_id, stage, asset_id, generation, detail) VALUES ($1, $2, $3::text::uuid, $4, $5) \
                 ON CONFLICT (task_id, stage) DO UPDATE SET detail = EXCLUDED.detail, at = now() \
                 WHERE mi.job_checkpoint.asset_id = EXCLUDED.asset_id AND mi.job_checkpoint.generation = EXCLUDED.generation",
                &[&task_id, &stage, &asset_id, &generation, detail],
            )
            .await
            .map_err(StoreError::postgres)?;
        if n == 0 {
            return Err(StoreError::Query { detail: "this stage of the task is already recorded for another asset or generation".into() });
        }
        Ok(())
    }

    /// The newest finished stage of a task, or `None` before the first.
    pub async fn last_checkpoint(&self, task_id: &str) -> Result<Option<IndexCheckpoint>, StoreError> {
        check_text("task id", task_id, 1, 256)?;
        let client = self.store.client().await?;
        let row = client
            .query_opt(
                "SELECT task_id, stage, asset_id::text AS asset_id, generation, detail FROM mi.job_checkpoint WHERE task_id = $1 ORDER BY stage DESC LIMIT 1",
                &[&task_id],
            )
            .await
            .map_err(StoreError::postgres)?;
        let Some(row) = row else { return Ok(None) };
        let bad = |e: &dyn std::fmt::Display| StoreError::Corrupt { id: task_id.into(), detail: e.to_string() };
        let stage: i64 = row.try_get("stage").map_err(|e| bad(&e))?;
        let generation: i64 = row.try_get("generation").map_err(|e| bad(&e))?;
        Ok(Some(IndexCheckpoint {
            task_id: row.try_get("task_id").map_err(|e| bad(&e))?,
            stage: u64::try_from(stage).map_err(|e| bad(&e))?,
            asset_id: row.try_get("asset_id").map_err(|e| bad(&e))?,
            generation: u64::try_from(generation).map_err(|e| bad(&e))?,
            detail: row.try_get("detail").map_err(|e| bad(&e))?,
        }))
    }
}

#[cfg(test)]
mod tests;
