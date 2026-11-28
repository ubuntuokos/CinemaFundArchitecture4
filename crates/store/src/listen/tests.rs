use std::time::Duration;

use cfa4_tasks::{MissedStart, NewTask, Priority, QueueConfig, Request, StartMode};

use super::*;
use crate::Store;
use crate::testdb::setup;

fn new_task(id: &str) -> NewTask {
    NewTask {
        id: id.into(),
        request: Request { class: cfa4_tasks::Class::Background, ram_bytes: 1 << 30, cpu_milli: 1000, gpu: None },
        priority: Priority::Normal,
        start: StartMode::Now,
        missed: MissedStart::RunLate,
        resumable: true,
        time_zone: None,
    }
}

#[tokio::test]
async fn a_submit_wakes_the_listener_only_after_the_commit_and_quiet_means_timeout() {
    let Some((_db, store)) = setup().await else { return };
    let mut listener = store.listen().await.expect("listen");
    assert_eq!(listener.wait(Duration::from_millis(100)).await, Wake::Timeout);
    let tasks = store.tasks(QueueConfig::default());
    tasks.submit(new_task("t-wake-1"), 1_700_000_000_000).await.expect("submit");
    tasks.submit(new_task("t-wake-2"), 1_700_000_000_000).await.expect("submit");
    // the two notifications may arrive in one wake-up or in two
    let mut seen = Vec::new();
    while seen.len() < 2 {
        match listener.wait(Duration::from_secs(5)).await {
            Wake::Changed(ids) => seen.extend(ids),
            other => panic!("expected a change, got {other:?}"),
        }
    }
    assert_eq!(seen, ["t-wake-1", "t-wake-2"]);
    // a state change wakes it as well
    let t = tasks.get("t-wake-1").await.expect("get").expect("there");
    tasks.cancel("t-wake-1", t.version, 1_700_000_001_000).await.expect("cancel");
    assert_eq!(listener.wait(Duration::from_secs(5)).await, Wake::Changed(vec!["t-wake-1".into()]));
    assert_eq!(listener.wait(Duration::from_millis(100)).await, Wake::Timeout);
}

#[tokio::test]
async fn a_lost_connection_is_reported_not_hidden() {
    let Some((db, store)) = setup().await else { return };
    let mut listener = store.listen().await.expect("listen");
    db.stop();
    assert_eq!(listener.wait(Duration::from_secs(10)).await, Wake::Lost);
}

#[tokio::test]
async fn without_a_server_listening_is_unavailable() {
    use crate::{ErrorCode, StoreConfig};
    let config = StoreConfig { host: "/nonexistent/cfa4-socket".into(), connect_timeout_ms: 200, ..StoreConfig::default() };
    let store_err = Store::connect(&config).await.err().expect("no server");
    assert!(matches!(store_err, StoreError::Unavailable { code: ErrorCode::DbServerMissing, .. }));
}
