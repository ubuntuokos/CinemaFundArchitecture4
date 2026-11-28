//! The outbox (D-3, PG-2): the Event Fabric events written with every state change, and the relay
//! that hands them over.
//!
//! **Envelope.** Each row holds the whole Canonical Envelope (`CFA4-EVENT-FABRIC-001` §5.1) with a
//! UUIDv7 `event_id` made when the row is written; it never changes on a repeat (§5.3). The
//! subject is one of the six `cfa4.execution.job.*` subjects of §13.4: the internal states are
//! mapped onto them by [`subject_for`] and the detailed state stays in the payload, together with
//! the task's sequence number (`entity_seq`, Q10) so a consumer can drop a stale `progress` that
//! arrives after `completed`. `correlation_id` is a UUID derived from the task id, so the events of
//! one task share it; `published_at` is filled in when the relay hands the event over.
//!
//! **Relay.** Delivery is **at least once**. A batch is locked (`FOR UPDATE SKIP LOCKED`, so several
//! relays never send the same row at once), handed to the publisher, and the rows it confirms are
//! marked published in the same transaction. If CFA4 stops between the publish and the commit
//! those events go out again with the same `event_id`; consumers drop repeats by it. The order is
//! the outbox order (the commit order of the state changes) within one relay. The Event Fabric
//! client (NATS JetStream) is the publisher's business: this crate does not depend on it.

use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::Transaction;

use crate::{Store, StoreError};

/// Most events handed over in one batch.
pub const MAX_BATCH: usize = 1000;

/// The envelope schema of the job events.
pub const SCHEMA_ID: &str = "cfa4.execution.job.v1";

/// The Event Fabric subject for an internal task state (`to`): the first two states announce the
/// job, `running` starts it, the three end states end it, everything else is progress.
pub fn subject_for(to_state: &str) -> &'static str {
    match to_state {
        "scheduled" | "queued" => "cfa4.execution.job.created",
        "running" => "cfa4.execution.job.started",
        "succeeded" => "cfa4.execution.job.completed",
        "failed" => "cfa4.execution.job.failed",
        "cancelled" => "cfa4.execution.job.cancelled",
        _ => "cfa4.execution.job.progress",
    }
}

/// Write one event, enveloped, in the caller's transaction.
pub(crate) async fn emit(tx: &Transaction<'_>, task_id: &str, subject: &str, payload: &Value) -> Result<(), StoreError> {
    tx.execute(
        "WITH new AS (SELECT tasks.uuid7() AS id) \
         INSERT INTO tasks.outbox (task_id, subject, event_id, event) \
         SELECT $1::text::uuid, $2::text, new.id, jsonb_build_object(\
             'event_id', new.id, 'event_type', $2::text, 'event_version', 1, \
             'occurred_at', to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
             'published_at', NULL, 'producer', 'cfa4-store', 'producer_instance', 'local', \
             'correlation_id', md5($1::text)::uuid, 'causation_id', NULL, 'trace_id', $1::text, \
             'schema_id', $4::text, 'content_type', 'application/json', 'classification', 'internal', \
             'payload', $3::jsonb, 'metadata', '{}'::jsonb) FROM new",
        &[&task_id, &subject, payload, &SCHEMA_ID],
    )
    .await
    .map_err(StoreError::postgres)?;
    Ok(())
}

/// One outbox row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxEvent {
    /// Increasing in commit order; the key the relay deduplicates on.
    pub id: i64,
    /// The key consumers deduplicate on; the same on every repeat.
    pub event_id: String,
    pub task_id: String,
    /// One of the six `cfa4.execution.job.*` subjects.
    pub subject: String,
    /// The Canonical Envelope; `published_at` is set to the time of this hand-over.
    pub envelope: Value,
}

/// Relays the outbox of a store.
pub struct Relay {
    store: Store,
}

impl Store {
    pub fn outbox(&self) -> Relay {
        Relay { store: self.clone() }
    }
}

impl Relay {
    /// Hand up to `limit` unpublished events, oldest first, to `publish`. It answers how many of
    /// them, from the first, are now published (a failed publish answers how many went before it,
    /// or an error); exactly those are marked. Returns that number.
    pub async fn relay_once<F, Fut>(&self, limit: usize, publish: F) -> Result<usize, StoreError>
    where
        F: FnOnce(Vec<OutboxEvent>) -> Fut,
        Fut: Future<Output = Result<usize, String>>,
    {
        let limit = i64::try_from(limit.clamp(1, MAX_BATCH)).unwrap_or(1);
        let mut client = self.store.client().await?;
        let tx = client.transaction().await.map_err(StoreError::postgres)?;
        let rows = tx
            .query(
                "SELECT id, event_id::text AS event_id, task_id::text AS task_id, subject, \
                 jsonb_set(event, '{published_at}', to_jsonb(to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'))) AS envelope \
                 FROM tasks.outbox WHERE published_at IS NULL ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED",
                &[&limit],
            )
            .await
            .map_err(StoreError::postgres)?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id").map_err(StoreError::postgres)?;
            let corrupt = |e: tokio_postgres::Error| StoreError::Corrupt { id: format!("outbox {id}"), detail: e.to_string() };
            events.push(OutboxEvent {
                id,
                event_id: row.try_get("event_id").map_err(corrupt)?,
                task_id: row.try_get("task_id").map_err(corrupt)?,
                subject: row.try_get("subject").map_err(corrupt)?,
                envelope: row.try_get("envelope").map_err(corrupt)?,
            });
        }
        if events.is_empty() {
            return Ok(0);
        }
        let ids: Vec<i64> = events.iter().map(|e| e.id).collect();
        let sent = publish(events).await.map_err(|detail| StoreError::Query { detail: format!("publish: {detail}") })?.min(ids.len());
        let done = ids.get(..sent).unwrap_or_default();
        if !done.is_empty() {
            tx.execute("UPDATE tasks.outbox SET published_at = now() WHERE id = ANY($1)", &[&done]).await.map_err(StoreError::postgres)?;
        }
        tx.commit().await.map_err(StoreError::postgres)?;
        Ok(sent)
    }

    /// Unpublished events waiting.
    pub async fn pending(&self) -> Result<i64, StoreError> {
        let client = self.store.client().await?;
        client.query_one("SELECT count(*) FROM tasks.outbox WHERE published_at IS NULL", &[]).await.and_then(|r| r.try_get(0)).map_err(StoreError::postgres)
    }

    /// Delete events published more than `older_than_days` days ago; how many went.
    pub async fn prune(&self, older_than_days: u32) -> Result<u64, StoreError> {
        let client = self.store.client().await?;
        let days = i32::try_from(older_than_days).unwrap_or(i32::MAX);
        client
            .execute("DELETE FROM tasks.outbox WHERE published_at IS NOT NULL AND published_at < now() - make_interval(days => $1)", &[&days])
            .await
            .map_err(StoreError::postgres)
    }
}

#[cfg(test)]
mod tests;
