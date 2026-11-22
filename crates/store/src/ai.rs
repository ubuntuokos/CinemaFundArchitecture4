//! The `ai` schema (PG-3): AI jobs, the model registry and the licence acceptances.
//!
//! CFA4 never ships models. The registry only records what the user downloaded: the SHA-256 of
//! the weights, where they are, the four licence parts, and the user's one-time acceptance of the
//! terms (`license_acceptance`). Weights stay files. A model that has no acceptance is not run:
//! callers ask [`AiStore::license_accepted`] and there is no silent fallback when the answer is no.
//!
//! Ids are UUIDs chosen by the caller (UUIDv7 by convention, P5); times are `timestamptz` in the
//! database and UTC milliseconds here.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::Row;

use crate::{Store, StoreError, check_sha256, check_text, check_uuid};

/// A route decision larger than this is refused (hostile input).
const MAX_ROUTE_BYTES: usize = 64 * 1024;
/// At most this many rows come back from a list.
const MAX_ROWS: i64 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAiJob {
    pub id: String,
    /// The `cfa4-tasks` task this job belongs to (one AI job per task).
    pub task_id: String,
    pub capability: String,
    pub implementation: String,
    pub route_decision: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiJob {
    pub id: String,
    pub task_id: String,
    pub capability: String,
    pub implementation: String,
    pub route_decision: Value,
    /// The project version the Preview was made against.
    pub preview_project_version: Option<String>,
    /// The project version Apply was made against.
    pub apply_project_version: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    Known,
    Downloading,
    Ready,
    Broken,
    Removed,
}

impl ModelState {
    fn as_str(self) -> &'static str {
        match self {
            ModelState::Known => "known",
            ModelState::Downloading => "downloading",
            ModelState::Ready => "ready",
            ModelState::Broken => "broken",
            ModelState::Removed => "removed",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "known" => ModelState::Known,
            "downloading" => ModelState::Downloading,
            "ready" => ModelState::Ready,
            "broken" => ModelState::Broken,
            "removed" => ModelState::Removed,
            _ => return None,
        })
    }
}

/// The four licence parts of a model (manifest v3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLicense {
    pub code: String,
    pub weights: String,
    pub training_data: String,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewModel {
    pub id: String,
    /// SHA-256 of the weights, 64 lowercase hex characters.
    pub sha256: String,
    pub family: String,
    pub revision: String,
    pub variant: String,
    pub format: String,
    pub source: String,
    pub license: ModelLicense,
    pub path: Option<String>,
    pub state: ModelState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[serde(flatten)]
    pub model: NewModel,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseAcceptance {
    pub id: String,
    pub implementation: String,
    pub weight_sha256: String,
    /// Empty: accepted everywhere.
    pub project_ref: String,
    pub accepted_at: i64,
    pub commercial_warning: String,
}

pub struct AiStore {
    store: Store,
}

const JOB_COLS: &str = "id::text AS id, task_id, capability, implementation, route_decision, preview_project_version, apply_project_version, \
     (extract(epoch from created_at) * 1000)::bigint AS created_ms, (extract(epoch from updated_at) * 1000)::bigint AS updated_ms";
const MODEL_COLS: &str = "id::text AS id, sha256, family, revision, variant, format, source, license_code, license_weights, license_training_data, \
     license_output, path, state, (extract(epoch from created_at) * 1000)::bigint AS created_ms, (extract(epoch from updated_at) * 1000)::bigint AS updated_ms";
const ACCEPT_COLS: &str =
    "id::text AS id, implementation, weight_sha256, project_ref, (extract(epoch from accepted_at) * 1000)::bigint AS accepted_ms, commercial_warning";

fn corrupt(id: &str, e: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt { id: id.to_owned(), detail: e.to_string() }
}

fn job_from(row: &Row) -> Result<AiJob, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::postgres)?;
    let get = |e: tokio_postgres::Error| corrupt(&id, e);
    Ok(AiJob {
        task_id: row.try_get("task_id").map_err(get)?,
        capability: row.try_get("capability").map_err(get)?,
        implementation: row.try_get("implementation").map_err(get)?,
        route_decision: row.try_get("route_decision").map_err(get)?,
        preview_project_version: row.try_get("preview_project_version").map_err(get)?,
        apply_project_version: row.try_get("apply_project_version").map_err(get)?,
        created_at: row.try_get("created_ms").map_err(get)?,
        updated_at: row.try_get("updated_ms").map_err(get)?,
        id,
    })
}

fn model_from(row: &Row) -> Result<Model, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::postgres)?;
    let get = |e: tokio_postgres::Error| corrupt(&id, e);
    let state: String = row.try_get("state").map_err(get)?;
    let state = ModelState::parse(&state).ok_or_else(|| corrupt(&id, format!("unknown model state {state}")))?;
    Ok(Model {
        model: NewModel {
            sha256: row.try_get("sha256").map_err(get)?,
            family: row.try_get("family").map_err(get)?,
            revision: row.try_get("revision").map_err(get)?,
            variant: row.try_get("variant").map_err(get)?,
            format: row.try_get("format").map_err(get)?,
            source: row.try_get("source").map_err(get)?,
            license: ModelLicense {
                code: row.try_get("license_code").map_err(get)?,
                weights: row.try_get("license_weights").map_err(get)?,
                training_data: row.try_get("license_training_data").map_err(get)?,
                output: row.try_get("license_output").map_err(get)?,
            },
            path: row.try_get("path").map_err(get)?,
            state,
            id: id.clone(),
        },
        created_at: row.try_get("created_ms").map_err(get)?,
        updated_at: row.try_get("updated_ms").map_err(get)?,
    })
}

fn acceptance_from(row: &Row) -> Result<LicenseAcceptance, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::postgres)?;
    let get = |e: tokio_postgres::Error| corrupt(&id, e);
    Ok(LicenseAcceptance {
        implementation: row.try_get("implementation").map_err(get)?,
        weight_sha256: row.try_get("weight_sha256").map_err(get)?,
        project_ref: row.try_get("project_ref").map_err(get)?,
        accepted_at: row.try_get("accepted_ms").map_err(get)?,
        commercial_warning: row.try_get("commercial_warning").map_err(get)?,
        id,
    })
}

impl AiStore {
    pub(crate) fn new(store: Store) -> Self {
        AiStore { store }
    }

    /// Record the AI part of a task. A second call for the same task replaces what was asked and
    /// the route decision (a re-route) and keeps the id and the recorded project versions.
    pub async fn put_job(&self, new: &NewAiJob) -> Result<AiJob, StoreError> {
        check_uuid("job id", &new.id)?;
        check_text("task id", &new.task_id, 1, 256)?;
        check_text("capability", &new.capability, 1, 128)?;
        check_text("implementation", &new.implementation, 1, 128)?;
        if serde_json::to_vec(&new.route_decision).map_err(StoreError::query)?.len() > MAX_ROUTE_BYTES {
            return Err(StoreError::Query { detail: format!("route decision is larger than {MAX_ROUTE_BYTES} bytes") });
        }
        let client = self.store.client().await?;
        let sql = format!(
            "INSERT INTO ai.job (id, task_id, capability, implementation, route_decision) VALUES ($1::text::uuid, $2, $3, $4, $5) \
             ON CONFLICT (task_id) DO UPDATE SET capability = EXCLUDED.capability, implementation = EXCLUDED.implementation, \
             route_decision = EXCLUDED.route_decision, updated_at = now() RETURNING {JOB_COLS}"
        );
        let row =
            client.query_one(&sql, &[&new.id, &new.task_id, &new.capability, &new.implementation, &new.route_decision]).await.map_err(StoreError::postgres)?;
        job_from(&row)
    }

    pub async fn job_for_task(&self, task_id: &str) -> Result<Option<AiJob>, StoreError> {
        check_text("task id", task_id, 1, 256)?;
        let client = self.store.client().await?;
        let row = client.query_opt(&format!("SELECT {JOB_COLS} FROM ai.job WHERE task_id = $1"), &[&task_id]).await.map_err(StoreError::postgres)?;
        row.as_ref().map(job_from).transpose()
    }

    /// Record the project version Preview and/or Apply used; `None` leaves a value as it is.
    /// `false`: no job for that task.
    pub async fn set_project_versions(&self, task_id: &str, preview: Option<&str>, apply: Option<&str>) -> Result<bool, StoreError> {
        check_text("task id", task_id, 1, 256)?;
        for v in [preview, apply].into_iter().flatten() {
            check_text("project version", v, 1, 256)?;
        }
        let client = self.store.client().await?;
        let n = client
            .execute(
                "UPDATE ai.job SET preview_project_version = COALESCE($2, preview_project_version), \
                 apply_project_version = COALESCE($3, apply_project_version), updated_at = now() WHERE task_id = $1",
                &[&task_id, &preview, &apply],
            )
            .await
            .map_err(StoreError::postgres)?;
        Ok(n == 1)
    }

    /// Add a model, or update what is known about the weights with this SHA-256 (the id of the
    /// first record stays).
    pub async fn put_model(&self, new: &NewModel) -> Result<Model, StoreError> {
        check_uuid("model id", &new.id)?;
        check_sha256("sha256", &new.sha256)?;
        check_text("family", &new.family, 1, 128)?;
        check_text("revision", &new.revision, 0, 128)?;
        check_text("variant", &new.variant, 0, 128)?;
        check_text("format", &new.format, 1, 64)?;
        check_text("source", &new.source, 0, 1024)?;
        for (field, v) in [
            ("code licence", &new.license.code),
            ("weights licence", &new.license.weights),
            ("training data licence", &new.license.training_data),
            ("output licence", &new.license.output),
        ] {
            check_text(field, v, 0, 256)?;
        }
        if let Some(p) = &new.path {
            check_text("path", p, 1, 4096)?;
        }
        let client = self.store.client().await?;
        let sql = format!(
            "INSERT INTO ai.model (id, sha256, family, revision, variant, format, source, license_code, license_weights, license_training_data, license_output, path, state) \
             VALUES ($1::text::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             ON CONFLICT (sha256) DO UPDATE SET family = EXCLUDED.family, revision = EXCLUDED.revision, variant = EXCLUDED.variant, format = EXCLUDED.format, \
             source = EXCLUDED.source, license_code = EXCLUDED.license_code, license_weights = EXCLUDED.license_weights, \
             license_training_data = EXCLUDED.license_training_data, license_output = EXCLUDED.license_output, path = EXCLUDED.path, \
             state = EXCLUDED.state, updated_at = now() RETURNING {MODEL_COLS}"
        );
        let row = client
            .query_one(
                &sql,
                &[
                    &new.id,
                    &new.sha256,
                    &new.family,
                    &new.revision,
                    &new.variant,
                    &new.format,
                    &new.source,
                    &new.license.code,
                    &new.license.weights,
                    &new.license.training_data,
                    &new.license.output,
                    &new.path,
                    &new.state.as_str(),
                ],
            )
            .await
            .map_err(StoreError::postgres)?;
        model_from(&row)
    }

    pub async fn model_by_sha(&self, sha256: &str) -> Result<Option<Model>, StoreError> {
        check_sha256("sha256", sha256)?;
        let client = self.store.client().await?;
        let row = client.query_opt(&format!("SELECT {MODEL_COLS} FROM ai.model WHERE sha256 = $1"), &[&sha256]).await.map_err(StoreError::postgres)?;
        row.as_ref().map(model_from).transpose()
    }

    /// Change the state (and, with `Some`, the path) of a known model. `false`: unknown model.
    pub async fn set_model_state(&self, sha256: &str, state: ModelState, path: Option<&str>) -> Result<bool, StoreError> {
        check_sha256("sha256", sha256)?;
        if let Some(p) = path {
            check_text("path", p, 1, 4096)?;
        }
        let client = self.store.client().await?;
        let n = client
            .execute("UPDATE ai.model SET state = $2, path = COALESCE($3, path), updated_at = now() WHERE sha256 = $1", &[&sha256, &state.as_str(), &path])
            .await
            .map_err(StoreError::postgres)?;
        Ok(n == 1)
    }

    /// Say that a capability uses a model. Idempotent; `false`: unknown model.
    pub async fn link_model_use(&self, sha256: &str, capability: &str) -> Result<bool, StoreError> {
        check_sha256("sha256", sha256)?;
        check_text("capability", capability, 1, 128)?;
        let client = self.store.client().await?;
        let n = client
            .execute(
                "INSERT INTO ai.model_use (model_id, capability) SELECT id, $2 FROM ai.model WHERE sha256 = $1 ON CONFLICT DO NOTHING",
                &[&sha256, &capability],
            )
            .await
            .map_err(StoreError::postgres)?;
        if n == 1 {
            return Ok(true);
        }
        let row = client.query_one("SELECT EXISTS (SELECT 1 FROM ai.model WHERE sha256 = $1)", &[&sha256]).await.map_err(StoreError::postgres)?;
        row.try_get(0).map_err(StoreError::postgres)
    }

    /// The models a capability uses, oldest first (at most 1000).
    pub async fn models_for(&self, capability: &str) -> Result<Vec<Model>, StoreError> {
        check_text("capability", capability, 1, 128)?;
        let client = self.store.client().await?;
        let sql =
            format!("SELECT {MODEL_COLS} FROM ai.model WHERE id IN (SELECT model_id FROM ai.model_use WHERE capability = $1) ORDER BY created_at, id LIMIT $2");
        let rows = client.query(&sql, &[&capability, &MAX_ROWS]).await.map_err(StoreError::postgres)?;
        rows.iter().map(model_from).collect()
    }

    /// Record the user's acceptance of a model's terms. The first acceptance stays: a second call
    /// for the same implementation, weights and project returns it unchanged.
    pub async fn accept_license(
        &self,
        id: &str,
        implementation: &str,
        weight_sha256: &str,
        project_ref: &str,
        commercial_warning: &str,
    ) -> Result<LicenseAcceptance, StoreError> {
        check_uuid("acceptance id", id)?;
        check_text("implementation", implementation, 1, 128)?;
        check_sha256("weight sha256", weight_sha256)?;
        check_text("project", project_ref, 0, 4096)?;
        check_text("commercial warning", commercial_warning, 0, 4096)?;
        let client = self.store.client().await?;
        client
            .execute(
                "INSERT INTO ai.license_acceptance (id, implementation, weight_sha256, project_ref, commercial_warning) \
                 VALUES ($1::text::uuid, $2, $3, $4, $5) ON CONFLICT (implementation, weight_sha256, project_ref) DO NOTHING",
                &[&id, &implementation, &weight_sha256, &project_ref, &commercial_warning],
            )
            .await
            .map_err(StoreError::postgres)?;
        let row = client
            .query_one(
                &format!("SELECT {ACCEPT_COLS} FROM ai.license_acceptance WHERE implementation = $1 AND weight_sha256 = $2 AND project_ref = $3"),
                &[&implementation, &weight_sha256, &project_ref],
            )
            .await
            .map_err(StoreError::postgres)?;
        acceptance_from(&row)
    }

    /// Has the user accepted this model's terms, for everywhere or for this project?
    pub async fn license_accepted(&self, implementation: &str, weight_sha256: &str, project_ref: &str) -> Result<bool, StoreError> {
        check_text("implementation", implementation, 1, 128)?;
        check_sha256("weight sha256", weight_sha256)?;
        check_text("project", project_ref, 0, 4096)?;
        let client = self.store.client().await?;
        let row = client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM ai.license_acceptance WHERE implementation = $1 AND weight_sha256 = $2 AND project_ref IN ('', $3))",
                &[&implementation, &weight_sha256, &project_ref],
            )
            .await
            .map_err(StoreError::postgres)?;
        row.try_get(0).map_err(StoreError::postgres)
    }

    /// Take an acceptance back; how many rows were removed.
    pub async fn revoke_license(&self, implementation: &str, weight_sha256: &str, project_ref: &str) -> Result<u64, StoreError> {
        check_text("implementation", implementation, 1, 128)?;
        check_sha256("weight sha256", weight_sha256)?;
        check_text("project", project_ref, 0, 4096)?;
        let client = self.store.client().await?;
        client
            .execute(
                "DELETE FROM ai.license_acceptance WHERE implementation = $1 AND weight_sha256 = $2 AND project_ref = $3",
                &[&implementation, &weight_sha256, &project_ref],
            )
            .await
            .map_err(StoreError::postgres)
    }
}

#[cfg(test)]
mod tests;
