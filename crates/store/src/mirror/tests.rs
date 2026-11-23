use std::time::Duration;

use cfa4_tasks::{MissedStart, NewTask, Priority, Queue, QueueConfig, Request, StartMode, Task, TaskJournal};
use serde_json::json;

use super::*;
use crate::testdb::{count, setup};

fn task(id: &str) -> Task {
    let mut q = Queue::new(QueueConfig::default());
    let new = NewTask {
        id: id.into(),
        request: Request { class: cfa4_tasks::Class::Background, ram_bytes: 0, cpu_milli: 0, gpu: None },
        priority: Priority::Normal,
        start: StartMode::Now,
        missed: MissedStart::RunLate,
        resumable: false,
        time_zone: None,
    };
    q.submit(new, 1_700_000_000_000).expect("submit").clone()
}

fn row(id: &str, state: &str, version: u64) -> (String, String, u64, Value) {
    (id.into(), state.into(), version, json!({ "id": id }))
}

#[tokio::test]
async fn rows_are_written_per_session_and_an_older_version_never_replaces_a_newer_one() {
    let Some((_db, store)) = setup().await else { return };
    let rows = store.session_tasks();
    rows.write("1-1", &[row("a", "queued", 0), row("b", "queued", 0)]).await.expect("write");
    rows.write("1-1", &[row("a", "running", 2)]).await.expect("write");
    rows.write("1-1", &[row("a", "queued", 1)]).await.expect("stale write is ignored");
    rows.write("2-2", &[row("a", "queued", 0)]).await.expect("another session");
    let got = rows.list("1-1").await.expect("list");
    assert_eq!(got.iter().map(|r| (r.id.as_str(), r.state.as_str(), r.version)).collect::<Vec<_>>(), [("a", "running", 2), ("b", "queued", 0)]);
    assert_eq!(rows.list("2-2").await.expect("list").len(), 1);
    // the database scheduler never sees them
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.task").await, 0);
}

#[tokio::test]
async fn the_open_tasks_of_a_dead_session_are_closed_as_interrupted_and_a_live_one_is_left() {
    let Some((_db, store)) = setup().await else { return };
    let rows = store.session_tasks();
    rows.write("10-1", &[row("a", "running", 1), row("done", "succeeded", 3)]).await.expect("write");
    rows.write("20-1", &[row("b", "queued", 0)]).await.expect("write");
    rows.write("30-1", &[row("c", "queued", 0)]).await.expect("write");
    let n = rows.close_dead("30-1", |s| s == "20-1").await.expect("close");
    assert_eq!(n, 1);
    let states = |r: Vec<SessionRow>| r.into_iter().map(|r| (r.id, r.state)).collect::<Vec<_>>();
    assert_eq!(states(rows.list("10-1").await.expect("list")), [("a".into(), "interrupted".into()), ("done".into(), "succeeded".into())]);
    assert_eq!(states(rows.list("20-1").await.expect("list")), [("b".into(), "queued".into())]);
    assert_eq!(states(rows.list("30-1").await.expect("list")), [("c".into(), "queued".into())]);
    // nothing is old enough to prune
    assert_eq!(rows.prune(KEEP_DAYS).await.expect("prune"), 0);
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.session_task").await, 4);
    assert_eq!(rows.prune(0).await.expect("prune"), 4);
}

#[test]
fn a_session_id_names_a_process_that_can_be_checked() {
    assert!(session_alive(&new_session_id()));
    assert!(!session_alive("4294967295-1"));
    assert!(session_alive("not-a-pid"), "an id nobody can check is never closed");
}

#[tokio::test]
async fn the_engines_changes_reach_the_database_through_the_writer_thread() {
    let Some((_db, store)) = setup().await else { return };
    let config = store.config.clone();
    let (session, dropped) = tokio::task::spawn_blocking(move || {
        let mut mirror = SessionMirror::start(config, Duration::from_secs(20)).expect("start");
        let mut t = task("m-1");
        mirror.record(&t);
        t.version = 4;
        mirror.record(&t);
        mirror.record(&task("m-2"));
        let out = (mirror.session().to_owned(), mirror.dropped());
        drop(mirror); // writes what is left
        out
    })
    .await
    .expect("thread");
    assert_eq!(dropped, 0);
    let got = store.session_tasks().list(&session).await.expect("list");
    assert_eq!(got.iter().map(|r| (r.id.as_str(), r.version)).collect::<Vec<_>>(), [("m-1", 4), ("m-2", task("m-2").version)]);
}

#[test]
fn without_a_server_there_is_no_mirror_and_it_says_so_quickly() {
    let config = StoreConfig { host: "/nonexistent/cfa4-socket".into(), connect_timeout_ms: 200, ..StoreConfig::default() };
    let t = std::time::Instant::now();
    assert!(SessionMirror::start(config, Duration::from_secs(10)).is_err());
    assert!(t.elapsed() < Duration::from_secs(5));
}
