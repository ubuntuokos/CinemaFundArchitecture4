//! Estimates and finished runs of tasks (`tasks.estimate`, `tasks.run_history`).
//!
//! Both are append-only records. The estimate is what the planner promised before and during a
//! run, as a range; the run history is what a finished run really cost, so the estimator can
//! learn from it (CFA4-TASKSTORE-PG-001 §3). Every value from a caller is checked before SQL.

use cfa4_tasks::Confidence;
use serde::{Deserialize, Serialize};

use super::TaskStore;
use crate::{StoreError, check_task_id, check_text};

/// How many run records one lookup returns at most.
const MAX_RUNS: i64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EstimateBasis {
    Measurement,
    History,
    Manifest,
}

impl EstimateBasis {
    fn as_str(self) -> &'static str {
        match self {
            EstimateBasis::Measurement => "measurement",
            EstimateBasis::History => "history",
            EstimateBasis::Manifest => "manifest",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateRecord {
    pub min_s: u64,
    pub max_s: u64,
    pub confidence: Confidence,
    pub basis: EstimateBasis,
    /// UTC milliseconds.
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub task_id: String,
    pub implementation: String,
    /// The execution path that ran (reference or a result-preserving switch).
    pub path: String,
    /// A fingerprint of the hardware it ran on.
    pub hardware: String,
    pub input_bytes: u64,
    pub duration_ms: u64,
    pub peak_ram_bytes: u64,
    pub peak_vram_bytes: u64,
}

fn big(field: &str, v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError::Query { detail: format!("{field} does not fit a bigint") })
}

fn back(field: &str, v: i64) -> Result<u64, StoreError> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt { id: field.to_owned(), detail: format!("negative {field}") })
}

impl TaskStore {
    /// Record an estimate for a task (the newest one is the current one). `min_s <= max_s`;
    /// an unknown task is an error.
    pub async fn record_estimate(&self, task_id: &str, min_s: u64, max_s: u64, confidence: Confidence, basis: EstimateBasis) -> Result<(), StoreError> {
        check_task_id(task_id)?;
        if min_s > max_s {
            return Err(StoreError::Query { detail: "estimate minimum is above its maximum".into() });
        }
        let confidence = match confidence {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
        };
        let client = self.store.client().await?;
        client
            .execute(
                "INSERT INTO tasks.estimate (task_id, min_s, max_s, confidence, basis) VALUES ($1::text::uuid, $2, $3, $4, $5)",
                &[&task_id, &big("minimum", min_s)?, &big("maximum", max_s)?, &confidence, &basis.as_str()],
            )
            .await
            .map_err(StoreError::postgres)?;
        Ok(())
    }

    /// The newest estimate of a task.
    pub async fn current_estimate(&self, task_id: &str) -> Result<Option<EstimateRecord>, StoreError> {
        check_task_id(task_id)?;
        let client = self.store.client().await?;
        let row = client
            .query_opt(
                "SELECT min_s, max_s, confidence, basis, (extract(epoch from at) * 1000)::bigint AS at_ms FROM tasks.estimate WHERE task_id = $1::text::uuid ORDER BY id DESC LIMIT 1",
                &[&task_id],
            )
            .await
            .map_err(StoreError::postgres)?;
        let Some(row) = row else { return Ok(None) };
        let bad = |e: tokio_postgres::Error| StoreError::Corrupt { id: task_id.to_owned(), detail: e.to_string() };
        let (min_s, max_s): (i64, i64) = (row.try_get("min_s").map_err(bad)?, row.try_get("max_s").map_err(bad)?);
        let confidence: String = row.try_get("confidence").map_err(bad)?;
        let basis: String = row.try_get("basis").map_err(bad)?;
        let corrupt = |what: &str, v: &str| StoreError::Corrupt { id: task_id.to_owned(), detail: format!("unknown {what} {v}") };
        Ok(Some(EstimateRecord {
            min_s: back("minimum", min_s)?,
            max_s: back("maximum", max_s)?,
            confidence: match confidence.as_str() {
                "low" => Confidence::Low,
                "medium" => Confidence::Medium,
                other => return Err(corrupt("confidence", other)),
            },
            basis: match basis.as_str() {
                "measurement" => EstimateBasis::Measurement,
                "history" => EstimateBasis::History,
                "manifest" => EstimateBasis::Manifest,
                other => return Err(corrupt("basis", other)),
            },
            at: row.try_get("at_ms").map_err(|e| StoreError::Corrupt { id: task_id.to_owned(), detail: e.to_string() })?,
        }))
    }

    /// Record what a finished run cost.
    pub async fn record_run(&self, run: &RunRecord) -> Result<(), StoreError> {
        check_task_id(&run.task_id)?;
        check_text("implementation", &run.implementation, 1, 128)?;
        check_text("path", &run.path, 0, 128)?;
        check_text("hardware", &run.hardware, 0, 256)?;
        let client = self.store.client().await?;
        client
            .execute(
                "INSERT INTO tasks.run_history (task_id, implementation, path, hardware, input_bytes, duration_ms, peak_ram_bytes, peak_vram_bytes) \
                 VALUES ($1::text::uuid, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &run.task_id,
                    &run.implementation,
                    &run.path,
                    &run.hardware,
                    &big("input size", run.input_bytes)?,
                    &big("duration", run.duration_ms)?,
                    &big("peak RAM", run.peak_ram_bytes)?,
                    &big("peak VRAM", run.peak_vram_bytes)?,
                ],
            )
            .await
            .map_err(StoreError::postgres)?;
        Ok(())
    }

    /// The newest `limit` runs (at most 1000) of an implementation on a hardware fingerprint,
    /// newest first: what the estimator learns from.
    pub async fn runs_for(&self, implementation: &str, hardware: &str, limit: u32) -> Result<Vec<RunRecord>, StoreError> {
        check_text("implementation", implementation, 1, 128)?;
        check_text("hardware", hardware, 0, 256)?;
        let limit = i64::from(limit).min(MAX_RUNS);
        let client = self.store.client().await?;
        let rows = client
            .query(
                "SELECT task_id::text AS task_id, implementation, path, hardware, input_bytes, duration_ms, peak_ram_bytes, peak_vram_bytes FROM tasks.run_history \
                 WHERE implementation = $1 AND hardware = $2 ORDER BY id DESC LIMIT $3",
                &[&implementation, &hardware, &limit],
            )
            .await
            .map_err(StoreError::postgres)?;
        rows.iter()
            .map(|row| {
                let bad = |e: tokio_postgres::Error| StoreError::Corrupt { id: implementation.to_owned(), detail: e.to_string() };
                Ok(RunRecord {
                    task_id: row.try_get("task_id").map_err(bad)?,
                    implementation: row.try_get("implementation").map_err(bad)?,
                    path: row.try_get("path").map_err(bad)?,
                    hardware: row.try_get("hardware").map_err(bad)?,
                    input_bytes: back("input size", row.try_get("input_bytes").map_err(bad)?)?,
                    duration_ms: back("duration", row.try_get("duration_ms").map_err(bad)?)?,
                    peak_ram_bytes: back("peak RAM", row.try_get("peak_ram_bytes").map_err(bad)?)?,
                    peak_vram_bytes: back("peak VRAM", row.try_get("peak_vram_bytes").map_err(bad)?)?,
                })
            })
            .collect()
    }
}
