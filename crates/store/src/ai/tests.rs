use serde_json::json;

use super::*;
use crate::testdb::{count, setup};

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn uuid(n: u32) -> String {
    format!("0190a000-0000-7000-8000-{n:012x}")
}

fn model(n: u32, sha: &str) -> NewModel {
    NewModel {
        id: uuid(n),
        sha256: sha.into(),
        family: "whisper".into(),
        revision: "r1".into(),
        variant: "small".into(),
        format: "onnx".into(),
        source: "https://huggingface.co/example".into(),
        license: ModelLicense { code: "MIT".into(), weights: "Apache-2.0".into(), training_data: "unknown".into(), output: "none".into() },
        path: None,
        state: ModelState::Known,
    }
}

#[tokio::test]
async fn a_job_is_kept_per_task_and_a_reroute_keeps_the_project_versions() {
    let Some((_db, store)) = setup().await else { return };
    let ai = store.ai();
    let new =
        NewAiJob { id: uuid(1), task_id: "t-1".into(), capability: "asr".into(), implementation: "whisper".into(), route_decision: json!({"path": "gpu"}) };
    let job = ai.put_job(&new).await.expect("put");
    assert_eq!(job.route_decision, json!({"path": "gpu"}));
    assert_eq!(job.id, uuid(1));
    assert!(job.created_at > 1_700_000_000_000, "{}", job.created_at);
    assert!(ai.set_project_versions("t-1", Some("v3"), None).await.expect("versions"));
    assert!(ai.set_project_versions("t-1", None, Some("v4")).await.expect("versions"));
    assert!(!ai.set_project_versions("nope", Some("v1"), None).await.expect("versions"));
    let again = NewAiJob { id: uuid(2), route_decision: json!({"path": "cpu"}), ..new };
    let job = ai.put_job(&again).await.expect("re-route");
    assert_eq!(job.id, uuid(1), "the first id stays");
    assert_eq!(job.route_decision, json!({"path": "cpu"}));
    assert_eq!(job.preview_project_version.as_deref(), Some("v3"));
    assert_eq!(job.apply_project_version.as_deref(), Some("v4"));
    assert_eq!(ai.job_for_task("t-1").await.expect("get"), Some(job));
    assert_eq!(ai.job_for_task("t-2").await.expect("get"), None);
    assert_eq!(count(&store, "SELECT count(*) FROM ai.job").await, 1);
}

#[tokio::test]
async fn models_are_one_record_per_weights_hash_and_link_to_capabilities() {
    let Some((_db, store)) = setup().await else { return };
    let ai = store.ai();
    let first = ai.put_model(&model(1, SHA_A)).await.expect("put");
    assert_eq!(first.model.state, ModelState::Known);
    let mut update = model(9, SHA_A);
    update.state = ModelState::Ready;
    update.path = Some("/models/a.onnx".into());
    let second = ai.put_model(&update).await.expect("update");
    assert_eq!(second.model.id, uuid(1), "same weights, same record");
    assert_eq!(second.model.state, ModelState::Ready);
    ai.put_model(&model(2, SHA_B)).await.expect("put b");
    assert!(ai.link_model_use(SHA_A, "asr").await.expect("link"));
    assert!(ai.link_model_use(SHA_A, "asr").await.expect("link twice"));
    assert!(ai.link_model_use(SHA_B, "asr").await.expect("link"));
    assert!(!ai.link_model_use(&"c".repeat(64), "asr").await.expect("unknown model"));
    assert_eq!(ai.models_for("asr").await.expect("list").len(), 2);
    assert!(ai.models_for("tts").await.expect("list").is_empty());
    assert!(ai.set_model_state(SHA_B, ModelState::Broken, Some("/models/b.onnx")).await.expect("state"));
    let b = ai.model_by_sha(SHA_B).await.expect("get").expect("known");
    assert_eq!((b.model.state, b.model.path.as_deref()), (ModelState::Broken, Some("/models/b.onnx")));
    assert!(!ai.set_model_state(&"c".repeat(64), ModelState::Ready, None).await.expect("unknown"));
}

#[tokio::test]
async fn a_licence_acceptance_is_once_per_model_and_scoped_to_everywhere_or_a_project() {
    let Some((_db, store)) = setup().await else { return };
    let ai = store.ai();
    assert!(!ai.license_accepted("whisper", SHA_A, "/p/a.fcproj").await.expect("check"));
    let first = ai.accept_license(&uuid(1), "whisper", SHA_A, "/p/a.fcproj", "non-commercial weights").await.expect("accept");
    let again = ai.accept_license(&uuid(2), "whisper", SHA_A, "/p/a.fcproj", "other text").await.expect("accept twice");
    assert_eq!(first, again, "the first acceptance stays");
    assert!(ai.license_accepted("whisper", SHA_A, "/p/a.fcproj").await.expect("check"));
    assert!(!ai.license_accepted("whisper", SHA_A, "/p/b.fcproj").await.expect("other project"));
    assert!(!ai.license_accepted("whisper", SHA_B, "/p/a.fcproj").await.expect("other weights"));
    ai.accept_license(&uuid(3), "whisper", SHA_B, "", "").await.expect("everywhere");
    assert!(ai.license_accepted("whisper", SHA_B, "/anything.fcproj").await.expect("everywhere"));
    assert_eq!(ai.revoke_license("whisper", SHA_B, "").await.expect("revoke"), 1);
    assert!(!ai.license_accepted("whisper", SHA_B, "/anything.fcproj").await.expect("revoked"));
}

#[tokio::test]
async fn hostile_input_is_an_error_not_a_panic_and_nothing_is_written() {
    let Some((_db, store)) = setup().await else { return };
    let ai = store.ai();
    let ok = NewAiJob { id: uuid(1), task_id: "t".into(), capability: "c".into(), implementation: "i".into(), route_decision: json!({}) };
    for bad in [
        NewAiJob { id: "not-a-uuid".into(), ..ok.clone() },
        NewAiJob { task_id: String::new(), ..ok.clone() },
        NewAiJob { capability: "x".repeat(129), ..ok.clone() },
        NewAiJob { implementation: "nul\0".into(), ..ok.clone() },
        NewAiJob { route_decision: json!({"big": "x".repeat(70_000)}), ..ok.clone() },
    ] {
        assert!(matches!(ai.put_job(&bad).await, Err(StoreError::Query { .. })), "{bad:?}");
    }
    assert!(ai.put_model(&model(1, "NOT-A-HASH")).await.is_err());
    assert!(ai.put_model(&model(1, &"A".repeat(64))).await.is_err(), "upper case hex is refused");
    assert!(ai.model_by_sha("short").await.is_err());
    assert!(ai.accept_license("x", "i", SHA_A, "", "").await.is_err());
    assert_eq!(count(&store, "SELECT (SELECT count(*) FROM ai.job) + (SELECT count(*) FROM ai.model) + (SELECT count(*) FROM ai.license_acceptance)").await, 0);
}

#[tokio::test]
async fn the_ai_schema_migrates_back_and_forth() {
    let Some((_db, store)) = setup().await else { return };
    assert_eq!(store.rollback("ai").await.expect("down"), Some(1));
    assert_eq!(count(&store, "SELECT count(*) FROM pg_namespace WHERE nspname = 'ai'").await, 0);
    assert_eq!(store.rollback("ai").await.expect("nothing left"), None);
    assert_eq!(store.migrate().await.expect("up"), vec![("ai".to_owned(), 1)]);
    assert_eq!(count(&store, "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'ai'").await, 4);
}

#[tokio::test]
async fn without_a_server_the_ai_store_reports_unavailable() {
    use crate::{ErrorCode, StoreConfig};
    let config = StoreConfig { host: "/nonexistent/cfa4-socket".into(), connect_timeout_ms: 200, ..StoreConfig::default() };
    assert!(matches!(Store::connect(&config).await, Err(StoreError::Unavailable { code: ErrorCode::DbServerMissing, .. })));
}
