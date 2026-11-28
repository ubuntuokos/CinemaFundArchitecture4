use serde_json::json;

use super::*;
use crate::testdb::{count, setup};

fn uuid(n: u32) -> String {
    format!("0190a000-0000-7000-8000-{n:012x}")
}

fn hash(fill: u8) -> ContentHash {
    ContentHash { alg: HashAlg::Sha256Sampled1, digest: [fill; 32] }
}

#[tokio::test]
async fn a_file_seen_again_keeps_its_asset() {
    let Some((_db, store)) = setup().await else { return };
    let mi = store.mi();
    let first = mi.register_asset(&uuid(1), Some(&hash(7)), Some("/media/a.mov"), Some(1_000)).await.expect("register");
    assert_eq!((first.id.as_str(), first.index_state), (uuid(1).as_str(), IndexState::Pending));
    assert_eq!(first.hash, Some(hash(7)));
    // the same content under another path and another proposed id: the first id and row win
    let again = mi.register_asset(&uuid(2), Some(&hash(7)), Some("/moved/a.mov"), None).await.expect("again");
    assert_eq!(again.id, uuid(1));
    assert_eq!((again.last_path.as_deref(), again.duration_ticks), (Some("/moved/a.mov"), Some(1_000)), "the old duration is kept when none is given");
    // the same digest from another algorithm is another asset
    let full = ContentHash { alg: HashAlg::Sha256, digest: [7; 32] };
    assert_eq!(mi.register_asset(&uuid(3), Some(&full), None, None).await.expect("full").id, uuid(3));
    assert_eq!(count(&store, "SELECT count(*) FROM mi.asset").await, 2);
    assert_eq!(mi.asset(&uuid(1)).await.expect("get").map(|a| a.id), Some(uuid(1)));
    assert_eq!(mi.asset(&uuid(9)).await.expect("get"), None);
}

#[tokio::test]
async fn an_asset_without_a_hash_waits_and_a_duplicate_is_reported_not_merged() {
    let Some((_db, store)) = setup().await else { return };
    let mi = store.mi();
    let pending = mi.register_asset(&uuid(1), None, Some("/media/b.mov"), None).await.expect("register");
    assert_eq!((pending.hash, pending.index_state), (None, IndexState::PendingHash));
    assert!(mi.set_index_state(&uuid(1), IndexState::Indexing).await.is_ok_and(|changed| !changed), "no state change before the hash is known");
    assert!(mi.set_index_state(&uuid(1), IndexState::PendingHash).await.is_err());
    mi.register_asset(&uuid(2), Some(&hash(9)), None, None).await.expect("known file");
    // the hash turns out to be the one of asset 2: nothing is written, the caller learns whose it is
    assert_eq!(mi.set_hash(&uuid(1), &hash(9)).await.expect("set"), Err(uuid(2)));
    assert_eq!(mi.asset(&uuid(1)).await.expect("get").map(|a| a.index_state), Some(IndexState::PendingHash));
    assert_eq!(mi.set_hash(&uuid(1), &hash(10)).await.expect("set"), Ok(()));
    let done = mi.asset(&uuid(1)).await.expect("get").expect("row");
    assert_eq!((done.hash, done.index_state), (Some(hash(10)), IndexState::Pending));
    assert!(mi.set_hash(&uuid(1), &hash(11)).await.is_err(), "a hash is set once");
    assert!(mi.set_index_state(&uuid(1), IndexState::Indexing).await.expect("state"));
}

#[tokio::test]
async fn a_new_generation_replaces_the_old_one_in_one_step() {
    let Some((_db, store)) = setup().await else { return };
    let mi = store.mi();
    mi.register_asset(&uuid(1), Some(&hash(1)), None, None).await.expect("register");
    assert_eq!(mi.active_generation(&uuid(1)).await.expect("none yet"), None);
    let g1 = mi.begin_generation(&uuid(1)).await.expect("begin");
    assert_eq!(g1, 1);
    assert!(!mi.activate_generation(&uuid(1), 99).await.expect("unknown"), "only a building generation can be activated");
    assert!(mi.activate_generation(&uuid(1), g1).await.expect("activate"));
    assert!(!mi.activate_generation(&uuid(1), g1).await.expect("twice"), "it is no longer building");
    assert_eq!(mi.asset(&uuid(1)).await.expect("get").map(|a| a.index_state), Some(IndexState::Indexed));
    // the rebuild runs next to the active generation, which stays the one that is searched
    let g2 = mi.begin_generation(&uuid(1)).await.expect("begin");
    assert_eq!(g2, 2);
    assert_eq!(mi.active_generation(&uuid(1)).await.expect("active"), Some(1));
    assert!(mi.activate_generation(&uuid(1), g2).await.expect("activate"));
    assert_eq!(mi.active_generation(&uuid(1)).await.expect("active"), Some(2));
    assert_eq!(count(&store, "SELECT count(*) FROM mi.index_generation WHERE state = 'retired'").await, 1);
    assert!(mi.begin_generation(&uuid(5)).await.is_err(), "an unknown asset has no generations");
    // two writers never get the same number
    let id = uuid(1);
    let (a, b) = tokio::join!(mi.begin_generation(&id), mi.begin_generation(&id));
    let (a, b) = (a.expect("a"), b.expect("b"));
    assert!(a != b && a >= 3 && b >= 3, "{a} {b}");
}

#[tokio::test]
async fn the_checkpoint_of_an_index_task_survives_a_repeat() {
    let Some((_db, store)) = setup().await else { return };
    let mi = store.mi();
    mi.register_asset(&uuid(1), Some(&hash(1)), None, None).await.expect("register");
    mi.register_asset(&uuid(2), Some(&hash(2)), None, None).await.expect("register");
    let g = mi.begin_generation(&uuid(1)).await.expect("begin");
    let task = format!("mi-index-{}-{g}", uuid(1));
    assert_eq!(mi.last_checkpoint(&task).await.expect("none"), None);
    mi.save_checkpoint(&task, 0, &uuid(1), g, &json!({"stage": "hash"})).await.expect("stage 0");
    mi.save_checkpoint(&task, 1, &uuid(1), g, &json!({"stage": "metadata"})).await.expect("stage 1");
    mi.save_checkpoint(&task, 1, &uuid(1), g, &json!({"stage": "metadata", "again": true})).await.expect("repeat");
    let last = mi.last_checkpoint(&task).await.expect("last").expect("row");
    assert_eq!((last.stage, last.generation, last.asset_id.as_str()), (1, g, uuid(1).as_str()));
    assert_eq!(last.detail, json!({"stage": "metadata", "again": true}));
    assert_eq!(count(&store, "SELECT count(*) FROM mi.job_checkpoint").await, 2);
    // a stage cannot be moved to another asset, and a generation that does not exist is refused
    mi.save_checkpoint(&task, 1, &uuid(2), g, &json!({})).await.expect_err("the stage belongs to asset 1");
    assert_eq!(mi.last_checkpoint(&task).await.expect("last").map(|c| c.asset_id), Some(uuid(1)));
    assert!(mi.save_checkpoint(&task, 2, &uuid(1), g + 5, &json!({})).await.is_err());
    // deleting the asset takes its generations and checkpoints with it (the index is derived data)
    store.client().await.expect("client").execute("DELETE FROM mi.asset WHERE id = $1::text::uuid", &[&uuid(1)]).await.expect("delete");
    assert_eq!(count(&store, "SELECT count(*) FROM mi.job_checkpoint").await, 0);
}

#[tokio::test]
async fn hostile_input_is_refused_before_it_reaches_the_tables() {
    let Some((_db, store)) = setup().await else { return };
    let mi = store.mi();
    assert!(mi.register_asset("not-a-uuid", None, None, None).await.is_err());
    assert!(mi.register_asset(&uuid(1), None, Some(""), None).await.is_err());
    assert!(mi.register_asset(&uuid(1), None, Some("nul\0"), None).await.is_err());
    assert!(mi.register_asset(&uuid(1), None, Some(&"p".repeat(4097)), None).await.is_err());
    assert!(mi.register_asset(&uuid(1), None, None, Some(-1)).await.is_err());
    assert!(mi.asset("x").await.is_err() && mi.begin_generation("x").await.is_err() && mi.active_generation("x").await.is_err());
    assert!(mi.last_checkpoint("").await.is_err());
    assert!(mi.last_checkpoint(&"t".repeat(257)).await.is_err());
    let g = u64::MAX;
    assert!(mi.activate_generation(&uuid(1), g).await.is_err());
    assert!(mi.save_checkpoint("t", u64::MAX, &uuid(1), 1, &json!({})).await.is_err());
    assert!(mi.save_checkpoint("t", 0, &uuid(1), 1, &json!({"big": "x".repeat(70_000)})).await.is_err());
    assert_eq!(count(&store, "SELECT (SELECT count(*) FROM mi.asset) + (SELECT count(*) FROM mi.job_checkpoint)").await, 0);
}

#[tokio::test]
async fn the_tables_refuse_what_the_rules_forbid() {
    let Some((_db, store)) = setup().await else { return };
    let mi = store.mi();
    mi.register_asset(&uuid(1), Some(&hash(1)), None, None).await.expect("register");
    let g = mi.begin_generation(&uuid(1)).await.expect("begin");
    let client = store.client().await.expect("client");
    let a = uuid(1);
    let seg = |n: u32, start: i64, end: i64, kind: &str, prov: &str| {
        let sql = format!(
            "INSERT INTO mi.segment (id, asset_id, generation, start_tick, end_tick, kind, provenance) VALUES ('{}', '{a}', {g}, {start}, {end}, '{kind}', '{prov}')",
            uuid(100 + n)
        );
        let client = &client;
        async move { client.execute(&sql, &[]).await }
    };
    assert!(seg(1, 0, 254_016_000_000, "speech", "SOURCE_DERIVED").await.is_ok());
    assert!(seg(2, 5, 5, "speech", "SOURCE_DERIVED").await.is_err(), "an empty range");
    assert!(seg(3, 9, 4, "speech", "SOURCE_DERIVED").await.is_err(), "a reversed range");
    assert!(seg(4, -1, 4, "speech", "SOURCE_DERIVED").await.is_err(), "a negative start");
    assert!(seg(5, 0, 4, "face", "SOURCE_DERIVED").await.is_err(), "an unknown kind");
    assert!(seg(6, 0, 4, "shot", "VERIFIED").await.is_err(), "an unknown provenance class");
    // the transcript search index is generated from the text
    client
        .execute(&format!("INSERT INTO mi.transcript (segment_id, text, lang) VALUES ('{}', 'Az ablak mögött esik az eső', 'hu')", uuid(101)), &[])
        .await
        .expect("transcript");
    assert_eq!(count(&store, "SELECT count(*) FROM mi.transcript WHERE tsv @@ plainto_tsquery('simple', 'eső')").await, 1);
    assert_eq!(count(&store, "SELECT count(*) FROM mi.transcript WHERE tsv @@ plainto_tsquery('simple', 'hó')").await, 0);
    assert!(client.execute(&format!("INSERT INTO mi.transcript (segment_id, text, confidence) VALUES ('{}', 'x', 1.5)", uuid(101)), &[]).await.is_err());
    // an embedding model: the pgvector HNSW limits are part of the schema (2000 / 4000 dimensions)
    let model = |n: u32, dims: i32, storage: &str| {
        format!(
            "INSERT INTO mi.embedding_model (id, implementation, weight_sha256, dims, metric, storage, modality, state) VALUES ('{}', 'clip', '{}', {dims}, 'cosine', '{storage}', 'image', 'new')",
            uuid(200 + n),
            format!("{n:x}").repeat(64).chars().take(64).collect::<String>()
        )
    };
    assert!(client.execute(&model(1, 2000, "vector"), &[]).await.is_ok());
    assert!(client.execute(&model(2, 2001, "vector"), &[]).await.is_err());
    assert!(client.execute(&model(3, 4000, "halfvec"), &[]).await.is_ok());
    assert!(client.execute(&model(4, 4001, "halfvec"), &[]).await.is_err());
    assert!(client.execute(&model(5, 0, "vector"), &[]).await.is_err());
    // faces: switched on only with who and when
    assert!(client.execute("INSERT INTO mi.face_consent (project_ref, enabled) VALUES ('/p/a.fcproj', true)", &[]).await.is_err());
    assert!(client.execute("INSERT INTO mi.face_consent (project_ref) VALUES ('/p/a.fcproj')", &[]).await.is_ok());
    assert!(client.execute("UPDATE mi.face_consent SET enabled = true, enabled_at = now(), enabled_by = 'owner'", &[]).await.is_ok());
    // a hash and its algorithm come together, and an indexed asset has a hash
    assert!(client.execute(&format!("INSERT INTO mi.asset (id, hash_alg, index_state) VALUES ('{}', 'sha256s1', 'pending')", uuid(7)), &[]).await.is_err());
    assert!(client.execute(&format!("INSERT INTO mi.asset (id, index_state) VALUES ('{}', 'indexed')", uuid(8)), &[]).await.is_err());
}

#[tokio::test]
async fn the_mi_schema_migrates_back_and_forth() {
    let Some((_db, store)) = setup().await else { return };
    assert_eq!(store.rollback("mi").await.expect("down"), Some(1));
    assert_eq!(count(&store, "SELECT count(*) FROM pg_namespace WHERE nspname = 'mi'").await, 0);
    assert_eq!(store.rollback("mi").await.expect("nothing left"), None);
    assert_eq!(store.migrate().await.expect("up"), vec![("mi".to_owned(), 1)]);
    assert_eq!(count(&store, "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'mi'").await, 10);
}
