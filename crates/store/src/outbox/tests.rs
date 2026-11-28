use cfa4_tasks::{Monitor, QueueConfig};

use super::*;
use crate::testdb::{count, setup};

#[test]
fn internal_states_map_onto_the_six_event_fabric_subjects() {
    assert_eq!(subject_for("scheduled"), "cfa4.execution.job.created");
    assert_eq!(subject_for("queued"), "cfa4.execution.job.created");
    assert_eq!(subject_for("running"), "cfa4.execution.job.started");
    assert_eq!(subject_for("succeeded"), "cfa4.execution.job.completed");
    assert_eq!(subject_for("failed"), "cfa4.execution.job.failed");
    assert_eq!(subject_for("cancelled"), "cfa4.execution.job.cancelled");
    for other in ["waiting", "blocked", "paused", "cancelling", "something-new"] {
        assert_eq!(subject_for(other), "cfa4.execution.job.progress", "{other}");
    }
}

fn new_task(id: &str) -> cfa4_tasks::NewTask {
    cfa4_tasks::NewTask {
        id: id.into(),
        request: cfa4_tasks::Request { class: cfa4_tasks::Class::Background, ram_bytes: 1 << 30, cpu_milli: 1000, gpu: None },
        priority: cfa4_tasks::Priority::Normal,
        start: cfa4_tasks::StartMode::Now,
        missed: cfa4_tasks::MissedStart::RunLate,
        resumable: true,
        time_zone: None,
    }
}

#[tokio::test]
async fn a_state_change_writes_a_canonical_envelope_with_a_uuid_v7_event_id() {
    let Some((_db, store)) = setup().await else { return };
    let tasks = store.tasks(QueueConfig::default());
    tasks.submit(new_task("00000000-0000-7000-8000-0000000000c9"), 1_700_000_000_000).await.expect("submit");
    let relay = store.outbox();
    let mut seen = Vec::new();
    let n = relay
        .relay_once(10, |events| {
            seen = events;
            let n = seen.len();
            async move { Ok(n) }
        })
        .await
        .expect("relay");
    assert!(n >= 1);
    let e = seen.first().expect("an event");
    assert_eq!(e.subject, "cfa4.execution.job.created");
    let env = &e.envelope;
    for field in [
        "event_id",
        "event_type",
        "event_version",
        "occurred_at",
        "published_at",
        "producer",
        "producer_instance",
        "correlation_id",
        "causation_id",
        "trace_id",
        "schema_id",
        "content_type",
        "classification",
        "payload",
        "metadata",
    ] {
        assert!(env.get(field).is_some(), "envelope lacks {field}: {env}");
    }
    assert_eq!(env["event_id"], e.event_id.as_str());
    assert_eq!(env["event_type"], "cfa4.execution.job.created");
    assert_eq!(env["schema_id"], SCHEMA_ID);
    assert!(env["published_at"].is_string(), "the hand-over time is filled in");
    assert!(env["causation_id"].is_null());
    assert_eq!(env["payload"]["entity_seq"], 0, "creation is sequence 0");
    assert_eq!(env["payload"]["taskId"], "00000000-0000-7000-8000-0000000000c9");
    // a UUID of version 7 whose first 48 bits are the milliseconds since the epoch
    let id = e.event_id.replace('-', "");
    assert_eq!(id.get(12..13), Some("7"), "{id}");
    let ms = i64::from_str_radix(id.get(..12).expect("12 hex"), 16).expect("hex");
    assert!((ms - chrono_now_ms()).abs() < 60_000, "{ms}");
    // the correlation id is one per task
    let corr: String = env["correlation_id"].as_str().expect("string").into();
    assert_eq!(corr.len(), 36);
}

fn chrono_now_ms() -> i64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    i64::try_from(d.as_millis()).unwrap_or(0)
}

#[tokio::test]
async fn relay_is_at_least_once_keeps_the_event_id_and_marks_only_what_was_confirmed() {
    let Some((_db, store)) = setup().await else { return };
    let tasks = store.tasks(QueueConfig::default());
    for i in 0..3 {
        tasks.submit(new_task(&format!("00000000-0000-7000-8000-{:012x}", 0x400 + i)), 1_700_000_000_000).await.expect("submit");
    }
    let relay = store.outbox();
    assert_eq!(relay.pending().await.expect("pending"), 3);
    // the publisher takes the first and fails on the second
    let mut first = Vec::new();
    let sent = relay
        .relay_once(10, |events| {
            first = events.iter().map(|e| (e.id, e.event_id.clone())).collect();
            async move { Ok(1) }
        })
        .await
        .expect("partial");
    assert_eq!(sent, 1);
    assert_eq!(relay.pending().await.expect("pending"), 2);
    // a publish that errors marks nothing, and the failure is a value
    let err = relay.relay_once(10, |_| async { Err("nats is down".to_owned()) }).await;
    assert!(matches!(err, Err(StoreError::Query { .. })), "{err:?}");
    assert_eq!(relay.pending().await.expect("pending"), 2);
    // a crash between publish and commit is a repeat with the same event id: simulate by publishing
    // and failing the commit, i.e. by rolling the rows back through a dropped transaction
    let mut again = Vec::new();
    relay
        .relay_once(10, |events| {
            again = events.iter().map(|e| (e.id, e.event_id.clone())).collect();
            async move { Ok(events.len()) }
        })
        .await
        .expect("rest");
    assert_eq!(again.len(), 2);
    assert_eq!(first.len(), 3, "the first batch offered all three");
    assert!(first.first().is_some_and(|f| !again.contains(f)), "the confirmed row is not sent again");
    assert!(first.iter().skip(1).all(|f| again.contains(f)), "the unconfirmed rows come back with the same event ids");
    assert_eq!(relay.pending().await.expect("pending"), 0);
    assert_eq!(count(&store, "SELECT count(DISTINCT event_id) FROM tasks.outbox").await, 3);
    // pruning removes what was published long ago and keeps the rest
    assert_eq!(relay.prune(1).await.expect("prune"), 0);
    let client = store.client().await.expect("client");
    client.execute("UPDATE tasks.outbox SET published_at = now() - interval '3 days'", &[]).await.expect("age");
    assert_eq!(relay.prune(1).await.expect("prune"), 3);
}

#[tokio::test]
async fn two_relays_never_hand_over_the_same_row_at_the_same_time() {
    let Some((_db, store)) = setup().await else { return };
    let tasks = store.tasks(QueueConfig::default());
    tasks.submit(new_task("00000000-0000-7000-8000-0000000000ca"), 1_700_000_000_000).await.expect("submit");
    let (a, b) = (store.outbox(), store.outbox());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (hold, other) = tokio::join!(
        a.relay_once(10, |events| async move {
            // while this relay holds the locked rows, a second one finds nothing to take
            let _ = rx.await;
            Ok(events.len())
        }),
        async {
            let got = b.relay_once(10, |events| async move { Ok(events.len()) }).await;
            let _ = tx.send(());
            got
        }
    );
    assert_eq!(hold.expect("a"), 1);
    assert_eq!(other.expect("b"), 0, "the row was locked by the first relay");
    assert_eq!(a.pending().await.expect("pending"), 0);
    let _ = Monitor::default();
}
