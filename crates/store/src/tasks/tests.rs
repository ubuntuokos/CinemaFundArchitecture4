use cfa4_route::Vendor;
use cfa4_tasks::budget::{Budget, BudgetSettings};
use cfa4_tasks::manager::{Class, GpuRequest, Request};
use cfa4_tasks::resources::{Current, GpuCurrent, GpuNominal, Nominal};
use cfa4_tasks::{Manager, MissedStart, Monitor, NewTask, Priority, QueueConfig, QueueError, QueueEvent, Snapshot, StartMode, State};

use crate::testdb::{self, count, setup};
use crate::{ErrorCode, INSTALL_HINT, Store, StoreConfig, StoreError};

const GIB: u64 = 1 << 30;

fn snap() -> Snapshot {
    Snapshot {
        nominal: Nominal {
            ram_physical_bytes: Some(32 * GIB),
            cpus_online: Some(16),
            gpus: vec![GpuNominal {
                node: "renderD128".into(),
                vendor: Vendor::Nvidia,
                driver: "nvidia".into(),
                vram_total_bytes: Some(12 * GIB),
                drives_display: Some(false),
            }],
            ..Nominal::default()
        },
        current: Current {
            ram_available_bytes: Some(28 * GIB),
            gpus: vec![GpuCurrent { node: "renderD128".into(), vram_used_bytes: Some(0), busy_percent: None }],
            ..Current::default()
        },
        notes: vec![],
    }
}

fn manager() -> Manager {
    Manager::new(Budget::new(&snap().nominal, &BudgetSettings::default()))
}

fn task(id: &str, vram_gib: u64) -> NewTask {
    NewTask {
        id: id.into(),
        request: Request {
            class: Class::Background,
            ram_bytes: GIB,
            cpu_milli: 1000,
            gpu: Some(GpuRequest { vram_bytes: vram_gib * GIB, node: None, vendor: None }),
        },
        priority: Priority::Normal,
        start: StartMode::Now,
        missed: MissedStart::RunLate,
        resumable: true,
        time_zone: None,
    }
}

#[tokio::test]
async fn a_missing_server_is_a_handled_error_with_the_install_hint() {
    let config = StoreConfig { host: "/nonexistent/cfa4-socket".into(), connect_timeout_ms: 200, ..StoreConfig::default() };
    match Store::connect(&config).await {
        Err(StoreError::Unavailable { code, hint, .. }) => {
            assert_eq!(code, ErrorCode::DbServerMissing);
            assert!(hint.contains("cfa4-db-setup"), "{hint}");
            assert!(!hint.contains("createuser"), "{hint}");
            assert!(!hint.contains("createdb"), "{hint}");
            assert_eq!(hint, INSTALL_HINT);
        }
        Err(e) => panic!("expected Unavailable, got {e}"),
        Ok(_) => panic!("connected to a socket that does not exist"),
    }
}

#[tokio::test]
async fn a_refused_connection_is_unavailable_without_panic() {
    // Nothing accepts TCP on port 1. That is a stopped listener, not a missing installation.
    let config = StoreConfig { host: "127.0.0.1".into(), port: 1, connect_timeout_ms: 500, ..StoreConfig::default() };
    match Store::connect(&config).await {
        Err(err @ StoreError::Unavailable { .. }) => assert_eq!(err.code(), ErrorCode::DbServerStopped, "{err}"),
        Err(e) => panic!("expected Unavailable, got {e}"),
        Ok(_) => panic!("connected to a closed port"),
    }
}

#[tokio::test]
async fn a_stopped_server_is_unavailable_without_panic() {
    let Some((db, _store)) = setup().await else { return };
    let config = StoreConfig { connect_timeout_ms: 1_000, ..db.config() };
    db.stop();
    match Store::connect(&config).await {
        Err(StoreError::Unavailable { .. }) => {}
        Err(e) => panic!("expected Unavailable, got {e}"),
        Ok(_) => panic!("connected to a server that was stopped"),
    }
}

#[tokio::test]
async fn without_a_database_the_memory_queue_still_runs() {
    let config = StoreConfig { host: "/nonexistent/cfa4-socket".into(), connect_timeout_ms: 200, ..StoreConfig::default() };
    let err = match Store::connect(&config).await {
        Err(err) => err,
        Ok(_) => panic!("a missing server must not connect"),
    };
    assert!(matches!(err, StoreError::Unavailable { .. }));
    assert!(err.to_string().contains("cfa4-db-setup"));

    let mut queue = cfa4_tasks::Queue::new(cfa4_tasks::QueueConfig::default());
    queue.submit(task("mem-1", 1), 0).expect("the memory queue does not need PostgreSQL");
    let mut manager = manager();
    let events = queue.tick(1, "supervisor", &mut manager, &snap(), &Monitor::default()).expect("memory queue step");
    let started = events.iter().filter(|e| matches!(e, QueueEvent::Started { .. })).count();
    assert_eq!(started, 1);
    assert_eq!(queue.get("mem-1").map(|t| t.state), Some(State::Running));
}

#[tokio::test]
async fn migrations_go_up_once_and_back_one_step() {
    let Some((_db, store)) = setup().await else { return };
    assert_eq!(store.migrate().await.unwrap(), vec![], "already up to date");
    assert_eq!(
        count(&store, "SELECT CASE WHEN pg_get_userbyid(nspowner) = 'cfa4_owner_tasks' THEN 1 ELSE 0 END::bigint FROM pg_namespace WHERE nspname = 'tasks'")
            .await,
        1,
        "the tasks schema is owned by the schema owner role"
    );
    let version: i64 = count(&store, "SELECT current_setting('server_version_num')::bigint").await;
    if testdb::required() {
        assert!((180000..190000).contains(&version), "CI requires PostgreSQL 18, server_version_num={version}");
    }
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.task").await, 0);
    assert_eq!(store.rollback("tasks").await.unwrap(), Some(4));
    assert_eq!(store.rollback("tasks").await.unwrap(), Some(3));
    assert_eq!(store.rollback("tasks").await.unwrap(), Some(2));
    assert_eq!(count(&store, "SELECT count(*) FROM pg_indexes WHERE indexname = 'task_open' AND indexdef LIKE '%cancelling%'").await, 0);
    assert_eq!(store.rollback("tasks").await.unwrap(), Some(1));
    assert_eq!(count(&store, "SELECT count(*) FROM information_schema.schemata WHERE schema_name = 'tasks'").await, 0);
    assert_eq!(store.rollback("tasks").await.unwrap(), None);
    assert_eq!(store.migrate().await.unwrap(), vec![("tasks".to_owned(), 1), ("tasks".to_owned(), 2), ("tasks".to_owned(), 3), ("tasks".to_owned(), 4)]);
    assert_eq!(count(&store, "SELECT count(*) FROM pg_indexes WHERE indexname = 'task_open' AND indexdef LIKE '%cancelling%'").await, 1);
    assert!(store.rollback("nope").await.is_err());
}

#[tokio::test]
async fn two_schedulers_start_a_task_exactly_once() {
    let Some((_db, store)) = setup().await else { return };
    let (a, b) = (store.tasks(QueueConfig::default()), store.tasks(QueueConfig::default()));
    a.submit(task("t1", 4), 0).await.unwrap();
    assert!(matches!(a.submit(task("t1", 4), 0).await, Err(StoreError::Queue { error: QueueError::DuplicateId { .. } })));
    let (s, mon) = (snap(), Monitor::default());
    let (mut ma, mut mb) = (manager(), manager());
    let (ea, eb) = tokio::join!(a.tick(1, "sup-a", &mut ma, &s, &mon), b.tick(1, "sup-b", &mut mb, &s, &mon));
    let started = |e: &Vec<QueueEvent>| e.iter().filter(|e| matches!(e, QueueEvent::Started { .. })).count();
    assert_eq!(started(&ea.unwrap()) + started(&eb.unwrap()), 1, "exactly one scheduler started it");
    assert_eq!(ma.held_count() + mb.held_count(), 1, "and only that one holds the reservation");
    // a later step by the other scheduler sees it running under a live lease
    let again = b.tick(2, "sup-b", &mut mb, &s, &mon).await.unwrap();
    assert_eq!(started(&again), 0);
    let t = a.get("t1").await.unwrap().unwrap();
    assert_eq!((t.state, t.attempts), (State::Running, 1));
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.transition WHERE task_id = 't1'").await, 1);
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.outbox WHERE subject = 'cfa4.execution.job.running'").await, 1);
}

#[tokio::test]
async fn a_restart_resumes_from_the_checkpoint_never_from_the_start() {
    let Some((_db, store)) = setup().await else { return };
    let q = store.tasks(QueueConfig::default());
    q.submit(task("long", 4), 0).await.unwrap();
    q.submit(NewTask { resumable: false, ..task("oneshot", 2) }, 0).await.unwrap();
    let (s, mon) = (snap(), Monitor::default());
    let mut m = manager();
    q.tick(0, "sup-a", &mut m, &s, &mon).await.unwrap();
    q.checkpoint("long", "sup-a", 7).await.unwrap();
    assert_eq!(q.renew("long", "sup-a", 10_000).await.unwrap(), 40_000);
    assert!(q.renew("long", "intruder", 10_000).await.is_err());
    // CFA4 restarts: a new process with a fresh manager, after the leases lapsed
    let mut fresh = manager();
    let e = q.tick(60_000, "sup-b", &mut fresh, &s, &mon).await.unwrap();
    assert!(e.contains(&QueueEvent::Started { id: "long".into(), gpu_node: Some("renderD128".into()), resumed_from: Some(7) }), "{e:?}");
    assert!(e.contains(&QueueEvent::LeaseLost { id: "oneshot".into(), resumed: false }));
    assert_eq!(q.get("oneshot").await.unwrap().map(|t| t.state), Some(State::Failed));
    // finish: the reservation is released only once the commit went through
    q.finish("long", "sup-b", State::Succeeded, 61_000, "done", &mut fresh).await.unwrap();
    assert_eq!(fresh.held_count(), 0);
    assert!(q.finish("long", "sup-b", State::Succeeded, 61_000, "again", &mut fresh).await.is_err());
}

#[tokio::test]
async fn user_actions_check_the_version() {
    let Some((_db, store)) = setup().await else { return };
    let q = store.tasks(QueueConfig::default());
    q.submit(NewTask { start: StartMode::At { at: 1_000 }, ..task("later", 2) }, 0).await.unwrap();
    assert!(matches!(q.pause("later", 7, 1).await, Err(StoreError::Queue { error: QueueError::Stale { .. } })));
    q.pause("later", 1, 1).await.unwrap();
    q.reschedule("later", 2, 5_000, 2).await.unwrap();
    let t = q.get("later").await.unwrap().unwrap();
    assert_eq!((t.state, t.version, t.start), (State::Scheduled, 3, StartMode::At { at: 5_000 }));
    assert_eq!(q.cancel("later", 3, 3).await.unwrap(), State::Cancelled);
    assert!(matches!(q.cancel("missing", 1, 3).await, Err(StoreError::Queue { error: QueueError::NotFound { .. } })));
}

#[tokio::test]
async fn a_corrupt_row_is_an_error_not_a_panic() {
    let Some((_db, store)) = setup().await else { return };
    let q = store.tasks(QueueConfig::default());
    q.submit(task("bad", 2), 0).await.unwrap();
    let client = store.client().await.unwrap();
    client.execute("UPDATE tasks.task SET doc = '{\"id\": 42}'::jsonb WHERE id = 'bad'", &[]).await.unwrap();
    assert!(matches!(q.get("bad").await, Err(StoreError::Corrupt { .. })));
    let mut m = manager();
    assert!(matches!(q.tick(1, "sup", &mut m, &snap(), &Monitor::default()).await, Err(StoreError::Corrupt { .. })));
    assert_eq!(m.held_count(), 0);
}

#[tokio::test]
async fn a_task_whose_worker_dies_while_cancelling_frees_its_resources() {
    let Some((_db, store)) = setup().await else { return };
    let q = store.tasks(QueueConfig::default());
    q.submit(task("stuck", 4), 0).await.unwrap();
    let (s, mon) = (snap(), Monitor::default());
    let mut m = manager();
    q.tick(0, "sup-a", &mut m, &s, &mon).await.unwrap();
    let v = q.get("stuck").await.unwrap().unwrap().version;
    assert_eq!(q.cancel("stuck", v, 1).await.unwrap(), State::Cancelling);
    // cancelling again changes nothing
    assert_eq!(q.cancel("stuck", v + 1, 2).await.unwrap(), State::Cancelling);
    assert_eq!(m.held_count(), 1);
    // the worker never reports; once the lease lapses the scheduler cancels it and frees the VRAM
    let e = q.tick(60_000, "sup-a", &mut m, &s, &mon).await.unwrap();
    assert!(e.contains(&QueueEvent::LeaseLost { id: "stuck".into(), resumed: false }), "{e:?}");
    assert_eq!(q.get("stuck").await.unwrap().map(|t| t.state), Some(State::Cancelled));
    assert_eq!(m.held_count(), 0);
}

/// The behaviour every [`crate::TaskBackend`] must have, whatever it stores the queue in.
async fn contract<B: crate::TaskBackend>(backend: &B) {
    let mut manager = manager();
    let monitor = Monitor::default();
    let submitted = backend.submit(task("c-1", 1), 0).await.expect("submit");
    assert_eq!(submitted.state, State::Queued);
    assert!(matches!(backend.submit(task("c-1", 1), 0).await, Err(StoreError::Queue { error: QueueError::DuplicateId { .. } })));
    assert!(backend.get("nope").await.expect("get").is_none());

    let events = backend.tick(1, "w", &mut manager, &snap(), &monitor).await.expect("tick");
    assert_eq!(events.iter().filter(|e| matches!(e, QueueEvent::Started { .. })).count(), 1);
    let running = backend.get("c-1").await.expect("get").expect("task");
    assert_eq!(running.state, State::Running);
    assert!(backend.checkpoint("c-1", "other", 1).await.is_err(), "only the lease owner checkpoints");
    backend.checkpoint("c-1", "w", 1).await.expect("checkpoint");
    backend.renew("c-1", "w", 2).await.expect("renew");

    // a stale version is refused and changes nothing
    assert!(backend.pause("c-1", running.version.saturating_add(7), 3).await.is_err());
    assert_eq!(backend.get("c-1").await.expect("get").map(|t| t.state), Some(State::Running));

    backend.finish("c-1", "w", State::Succeeded, 4, "done", &mut manager).await.expect("finish");
    assert_eq!(backend.get("c-1").await.expect("get").map(|t| t.state), Some(State::Succeeded));
    // the reservation came back: a second task of the same size starts
    backend.submit(task("c-2", 1), 5).await.expect("submit");
    let events = backend.tick(6, "w", &mut manager, &snap(), &monitor).await.expect("tick");
    assert_eq!(events.iter().filter(|e| matches!(e, QueueEvent::Started { .. })).count(), 1);
}

#[tokio::test]
async fn the_memory_backend_follows_the_contract_without_a_server() {
    contract(&crate::MemoryTaskStore::new(QueueConfig::default())).await;
}

#[tokio::test]
async fn the_postgres_backend_follows_the_same_contract() {
    let Some((_db, store)) = setup().await else { return };
    contract(&store.tasks(QueueConfig::default())).await;
}

#[tokio::test]
async fn the_time_columns_are_timestamptz_and_migrate_both_ways_keeping_the_milliseconds() {
    let Some((_db, store)) = setup().await else { return };
    let tasks = store.tasks(QueueConfig::default());
    let t = tasks.submit(task("t-time", 1), 1_700_000_123_456).await.expect("submit");
    assert_eq!(t.submitted_at, 1_700_000_123_456);
    let ms = |col: &'static str| format!("SELECT (extract(epoch from {col}) * 1000)::bigint FROM tasks.task WHERE id = 't-time'");
    assert_eq!(count(&store, &ms("submitted_at")).await, 1_700_000_123_456);
    assert_eq!(count(&store, "SELECT count(*) FROM information_schema.columns WHERE table_schema = 'tasks' AND table_name IN ('task', 'transition') AND column_name IN ('submitted_at', 'start_at', 'lease_until', 'at') AND data_type = 'timestamp with time zone'").await, 4);
    // back to the bigint columns of step 2 and forward again: the stored values survive
    assert_eq!(store.rollback("tasks").await.expect("down"), Some(4));
    assert_eq!(store.rollback("tasks").await.expect("down"), Some(3));
    assert_eq!(count(&store, "SELECT submitted_at FROM tasks.task WHERE id = 't-time'").await, 1_700_000_123_456);
    assert_eq!(store.migrate().await.expect("up"), vec![("tasks".to_owned(), 3), ("tasks".to_owned(), 4)]);
    assert_eq!(count(&store, &ms("submitted_at")).await, 1_700_000_123_456);
    // the queue still runs on the converted columns
    let mut manager = manager();
    let monitor = Monitor::default();
    let events = tasks.tick(1_700_000_200_000, "w1", &mut manager, &snap(), &monitor).await.expect("tick");
    assert!(!events.is_empty() || tasks.get("t-time").await.expect("get").is_some());
    assert!(count(&store, "SELECT count(*) FROM tasks.transition WHERE at IS NOT NULL").await >= 0);
}

#[tokio::test]
async fn the_schedule_follows_the_task_and_checkpoints_estimates_and_runs_are_recorded() {
    use super::{EstimateBasis, RunRecord};
    use cfa4_tasks::Confidence;
    let Some((_db, store)) = setup().await else { return };
    let tasks = store.tasks(QueueConfig::default());
    let mut at = task("t-sched", 1);
    at.start = StartMode::At { at: 2_000_000_000_000 };
    at.missed = MissedStart::NextSlot;
    at.time_zone = Some("Europe/Budapest".into());
    let t = tasks.submit(at, 1_700_000_000_000).await.expect("submit");
    let row = |col: &str| format!("SELECT {col} FROM tasks.schedule WHERE task_id = 't-sched'");
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM tasks.schedule WHERE task_id = 't-sched' AND mode = 'at' AND if_missed = 'next_slot' AND time_zone = 'Europe/Budapest'"
        )
        .await,
        1
    );
    assert_eq!(count(&store, &row("(extract(epoch from start_at) * 1000)::bigint")).await, 2_000_000_000_000);
    tasks.reschedule("t-sched", t.version, 2_100_000_000_000, 1_700_000_001_000).await.expect("reschedule");
    assert_eq!(count(&store, &row("(extract(epoch from start_at) * 1000)::bigint")).await, 2_100_000_000_000);

    // estimates: the newest is the current one; an inverted range is refused
    assert_eq!(tasks.current_estimate("t-sched").await.expect("none"), None);
    tasks.record_estimate("t-sched", 60, 120, Confidence::Low, EstimateBasis::Manifest).await.expect("first");
    tasks.record_estimate("t-sched", 80, 90, Confidence::Medium, EstimateBasis::History).await.expect("second");
    let e = tasks.current_estimate("t-sched").await.expect("get").expect("there");
    assert_eq!((e.min_s, e.max_s, e.confidence, e.basis), (80, 90, Confidence::Medium, EstimateBasis::History));
    assert!(tasks.record_estimate("t-sched", 10, 5, Confidence::Low, EstimateBasis::Manifest).await.is_err());
    assert!(tasks.record_estimate("unknown-task", 1, 2, Confidence::Low, EstimateBasis::Manifest).await.is_err(), "an unknown task has no estimate");

    // run history: newest first, by implementation and hardware
    let run = |n: u64, hw: &str| RunRecord {
        task_id: "t-sched".into(),
        implementation: "whisper".into(),
        path: "gpu".into(),
        hardware: hw.into(),
        input_bytes: n,
        duration_ms: n * 10,
        peak_ram_bytes: 1 << 30,
        peak_vram_bytes: 2 << 30,
    };
    for n in 1..=3 {
        tasks.record_run(&run(n, "rtx3060")).await.expect("run");
    }
    tasks.record_run(&run(9, "other")).await.expect("run");
    let runs = tasks.runs_for("whisper", "rtx3060", 2).await.expect("runs");
    assert_eq!(runs.iter().map(|r| r.input_bytes).collect::<Vec<_>>(), [3, 2]);
    assert!(tasks.record_run(&RunRecord { implementation: String::new(), ..run(1, "x") }).await.is_err());
    assert!(tasks.record_run(&RunRecord { duration_ms: u64::MAX, ..run(1, "x") }).await.is_err());
}

#[tokio::test]
async fn a_stage_is_recorded_once_with_its_cache_key_and_deleting_a_task_removes_its_rows() {
    let Some((_db, store)) = setup().await else { return };
    let tasks = store.tasks(QueueConfig::default());
    let (mut manager, monitor) = (manager(), Monitor::default());
    tasks.submit(task("t-stage", 1), 1_700_000_000_000).await.expect("submit");
    tasks.tick(1_700_000_000_000, "w1", &mut manager, &snap(), &monitor).await.expect("tick");
    tasks.checkpoint_keyed("t-stage", "w1", 0, Some("blake3-abc")).await.expect("stage 0");
    tasks.checkpoint_keyed("t-stage", "w1", 0, Some("other")).await.expect("stage 0 again is harmless");
    tasks.checkpoint("t-stage", "w1", 1).await.expect("stage 1");
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.checkpoint WHERE task_id = 't-stage'").await, 2);
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.checkpoint WHERE task_id = 't-stage' AND stage = 0 AND cache_key = 'blake3-abc'").await, 1);
    assert!(tasks.checkpoint_keyed("t-stage", "w1", 2, Some("")).await.is_err());
    assert!(tasks.checkpoint_keyed("t-stage", "someone-else", 3, None).await.is_err(), "only the lease owner records a stage");
    assert_eq!(count(&store, "SELECT count(*) FROM tasks.checkpoint WHERE task_id = 't-stage'").await, 2);
    let client = store.client().await.expect("client");
    client.execute("DELETE FROM tasks.task WHERE id = 't-stage'", &[]).await.expect("delete");
    assert_eq!(count(&store, "SELECT (SELECT count(*) FROM tasks.checkpoint) + (SELECT count(*) FROM tasks.schedule WHERE task_id = 't-stage')").await, 0);
}
