//! The PG-2 gate tests (`CFA4-DATA-PG-001` §9, `CFA4-TASKSTORE-PG-001` §10): what the plan asks of
//! the `tasks` schema before PG-2 may be called done.
//!
//! * **Kills.** A client that dies at any point of a transaction leaves no trace: the transaction is
//!   rolled back, the reservation of the caller is untouched, nothing is announced, and a retry
//!   works (and starts a task exactly once). The kill is done on a throw-away cluster with a
//!   trigger that parks the server inside the statement (or inside the commit, with a deferred
//!   constraint trigger) at the point under test; then the connection is terminated
//!   (`pg_terminate_backend`) or the client side is dropped. The server cannot tell that from a
//!   `kill -9` of the CFA4 process: it sees the connection vanish in the middle of a transaction.
//! * **Leases.** The edges of a lease, renewal, and the rule that a lapsed lease never restarts a
//!   task from the beginning by itself.
//! * **Database restart** in the middle of a running task (clean stop and crash), with the measured
//!   recovery time (P9: the cluster is usable again within a minute).
//! * **Schedule.** Exact starts, the grace window, and a missed start across a restart, time zone
//!   and daylight-saving changes, with the database server's own time zone set to unrelated zones.
//! * **Outbox.** A relay that dies after publishing, and a consumer that gets events reordered.
//!
//! Without PostgreSQL the tests skip (as the other store tests do); with `CFA4_PG_REQUIRE=1` (CI)
//! they fail instead.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cfa4_route::Vendor;
use cfa4_tasks::budget::{Budget, BudgetSettings};
use cfa4_tasks::manager::{Class, GpuRequest, Request};
use cfa4_tasks::resources::{Current, GpuCurrent, GpuNominal, Nominal};
use cfa4_tasks::{Manager, MissedStart, Monitor, NewTask, Priority, QueueConfig, QueueError, QueueEvent, Snapshot, StartMode, State, UtcMs};

use crate::listen::{Listener, Wake};
use crate::outbox::OutboxEvent;
use crate::testdb::{TestDb, count, setup};
use crate::{Store, StoreError};

const GIB: u64 = 1 << 30;
/// The grace after which a start counts as missed (the `QueueConfig` default).
const GRACE_MS: u64 = 5 * 60_000;
/// The lease length (the `QueueConfig` default).
const LEASE_MS: u64 = 30_000;

// ---------------------------------------------------------------------------------------------
// fixtures

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

/// A UUID-shaped task id (the form the store keeps ids in once they are `uuid`s, plan D-5).
fn tid(n: u32) -> String {
    format!("0192f000-0000-7000-8000-{n:012}")
}

/// A small task (1 GiB, one CPU, no GPU), so that many of them run side by side.
fn new_task(id: &str) -> NewTask {
    NewTask {
        id: id.into(),
        request: Request { class: Class::Background, ram_bytes: GIB, cpu_milli: 1000, gpu: None },
        priority: Priority::Normal,
        start: StartMode::Now,
        missed: MissedStart::RunLate,
        resumable: true,
        time_zone: None,
    }
}

fn gpu_task(id: &str) -> NewTask {
    NewTask { request: Request { gpu: Some(GpuRequest { vram_bytes: 2 * GIB, node: None, vendor: None }), ..new_task(id).request }, ..new_task(id) }
}

async fn admin(db: &TestDb) -> Store {
    Store::connect(&db.admin_config()).await.expect("connect as the test superuser")
}

/// A new connection pool as the application, as a restarted CFA4 would have.
async fn fresh(db: &TestDb) -> Store {
    Store::connect(&db.config()).await.expect("connect as cfa4_login")
}

async fn exec(admin: &Store, sql: &str) {
    let client = admin.client().await.expect("admin client");
    client.batch_execute(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn text(admin: &Store, sql: &str) -> String {
    let client = admin.client().await.expect("admin client");
    let row = client.query_one(sql, &[]).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
    row.try_get::<_, String>(0).unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// An ISO instant as UTC milliseconds, computed by PostgreSQL (an oracle independent of the
/// scheduler's own calendar code).
async fn at(admin: &Store, iso: &str) -> UtcMs {
    let ms = text(admin, &format!("SELECT (extract(epoch FROM '{iso}'::timestamptz) * 1000)::bigint::text")).await;
    ms.parse().unwrap_or_else(|e| panic!("{iso}: {e}"))
}

fn started_ids(events: &[QueueEvent]) -> Vec<String> {
    events.iter().filter_map(|e| if let QueueEvent::Started { id, .. } = e { Some(id.clone()) } else { None }).collect()
}

fn started_count(events: &[QueueEvent], id: &str) -> usize {
    started_ids(events).iter().filter(|s| s.as_str() == id).count()
}

/// Everything the tables hold about one task, as one string: equal before and after means that
/// nothing at all changed (an empty string: there is no such task).
async fn fingerprint(admin: &Store, id: &str) -> String {
    let client = admin.client().await.expect("admin client");
    let row = client
        .query_opt(
            "SELECT concat_ws('|', t.state, t.version, coalesce(t.lease_owner, '-'), coalesce(t.lease_until::text, '-'), \
                (SELECT count(*) FROM tasks.transition x WHERE x.task_id::text = t.id::text), \
                (SELECT count(*) FROM tasks.outbox o WHERE o.task_id::text = t.id::text), \
                (SELECT count(*) FROM tasks.checkpoint c WHERE c.task_id::text = t.id::text), \
                (SELECT count(*) FROM tasks.schedule s WHERE s.task_id::text = t.id::text), \
                md5(t.doc::text)) \
             FROM tasks.task t WHERE t.id::text = $1",
            &[&id],
        )
        .await
        .expect("fingerprint");
    row.map(|r| r.get::<_, String>(0)).unwrap_or_default()
}

async fn outbox_count(admin: &Store, id: &str, subject: &str) -> i64 {
    count(admin, &format!("SELECT count(*) FROM tasks.outbox WHERE task_id = '{id}' AND subject = '{subject}'")).await
}

const STARTED: &str = "cfa4.execution.job.started";

// ---------------------------------------------------------------------------------------------
// the kill harness

/// Where in a transaction the client is killed.
#[derive(Clone, Copy, Debug)]
enum Point {
    /// While the server runs the statement that writes a row of `tasks.<table>`.
    Statement(&'static str),
    /// While the server runs the commit (a deferred constraint trigger on `tasks.<table>`): every
    /// statement is done, the transaction is not durable yet.
    Commit(&'static str),
}

impl Point {
    fn table(self) -> &'static str {
        match self {
            Point::Statement(t) | Point::Commit(t) => t,
        }
    }
}

/// Park the server for `sleep_s` seconds at `point`.
async fn arm(admin: &Store, point: Point, sleep_s: &str) {
    exec(
        admin,
        &format!("CREATE OR REPLACE FUNCTION public.cfa4_gate() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep({sleep_s}); RETURN NULL; END $$"),
    )
    .await;
    let create = match point {
        Point::Statement(t) => format!("CREATE TRIGGER cfa4_gate AFTER INSERT OR UPDATE ON tasks.{t} FOR EACH ROW EXECUTE FUNCTION public.cfa4_gate()"),
        Point::Commit(t) => format!(
            "CREATE CONSTRAINT TRIGGER cfa4_gate AFTER INSERT OR UPDATE ON tasks.{t} DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.cfa4_gate()"
        ),
    };
    exec(admin, &create).await;
}

async fn disarm(admin: &Store, point: Point) {
    exec(admin, &format!("DROP TRIGGER IF EXISTS cfa4_gate ON tasks.{}", point.table())).await;
}

/// Wait until no application backend is inside a transaction any more (the server noticed that the
/// client is gone and rolled back).
async fn wait_quiet(admin: &Store) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let busy = count(
            admin,
            "SELECT count(*) FROM pg_stat_activity WHERE usename = 'cfa4_login' AND state IN ('active', 'idle in transaction', 'idle in transaction (aborted)')",
        )
        .await;
        if busy == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "an application backend is still inside a transaction after 30 s");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Run `op`, and when the server is parked at the armed point terminate its backend: the client
/// loses its connection in the middle of the transaction. Returns what `op` returned (an error).
async fn kill_while<T: Send + 'static>(admin: &Store, op: impl Future<Output = T> + Send + 'static) -> T {
    let handle = tokio::spawn(op);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let killed = {
            let client = admin.client().await.expect("admin client");
            client
                .query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE wait_event = 'PgSleep' AND usename = 'cfa4_login' AND pid <> pg_backend_pid()",
                    &[],
                )
                .await
                .expect("terminate")
                .len()
        };
        if killed > 0 {
            break;
        }
        assert!(!handle.is_finished(), "the operation finished without reaching the armed point: the point is not part of it");
        assert!(Instant::now() < deadline, "the operation never reached the armed point");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let out = tokio::time::timeout(Duration::from_secs(30), handle).await.expect("the killed operation returned").expect("join");
    wait_quiet(admin).await;
    out
}

/// Like [`kill_while`], but the client side dies: the operation is dropped (and with it its pool)
/// while the server is inside the statement, as when the CFA4 process is killed.
async fn drop_client_while<T: Send + 'static>(admin: &Store, op: impl Future<Output = T> + Send + 'static) {
    let handle = tokio::spawn(op);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let parked = count(admin, "SELECT count(*) FROM pg_stat_activity WHERE wait_event = 'PgSleep' AND usename = 'cfa4_login'").await;
        if parked > 0 {
            break;
        }
        assert!(!handle.is_finished(), "the operation finished without reaching the armed point");
        assert!(Instant::now() < deadline, "the operation never reached the armed point");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();
    let _ = handle.await;
    wait_quiet(admin).await;
}

/// Read the wake-ups that are already there.
async fn drain(listener: &mut Listener) {
    loop {
        match listener.wait(Duration::from_millis(150)).await {
            Wake::Timeout => return,
            Wake::Lost => panic!("the listener lost its connection"),
            Wake::Changed(_) => {}
        }
    }
}

fn is_err_not_panic<T: std::fmt::Debug>(point: Point, result: &Result<T, StoreError>) {
    assert!(result.is_err(), "{point:?}: an operation killed in the middle must be an error, got {result:?}");
}

// ---------------------------------------------------------------------------------------------
// kills at every point

#[tokio::test]
async fn a_kill_at_every_point_of_submit_leaves_no_trace_and_the_retry_works() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let mut listener = admin.listen().await.expect("listen");
    let points = [Point::Statement("task"), Point::Statement("schedule"), Point::Statement("outbox"), Point::Commit("outbox")];
    for (n, point) in (0u32..).zip(points) {
        let id = tid(100 + n);
        let outbox_before = count(&admin, "SELECT count(*) FROM tasks.outbox").await;
        arm(&admin, point, "60").await;
        let (store, task) = (fresh(&db).await, new_task(&id));
        let result = kill_while(&admin, async move { store.tasks(QueueConfig::default()).submit(task, 0).await }).await;
        disarm(&admin, point).await;
        is_err_not_panic(point, &result);
        assert_eq!(fingerprint(&admin, &id).await, "", "{point:?}: no row of the task survived");
        assert_eq!(count(&admin, "SELECT count(*) FROM tasks.outbox").await, outbox_before, "{point:?}: no event was written");
        assert_eq!(listener.wait(Duration::from_millis(300)).await, Wake::Timeout, "{point:?}: nothing was announced");
        // the retry is a first submit: not a duplicate
        let store = fresh(&db).await;
        store.tasks(QueueConfig::default()).submit(new_task(&id), 0).await.unwrap_or_else(|e| panic!("{point:?}: retry: {e}"));
        assert_eq!(listener.wait(Duration::from_secs(5)).await, Wake::Changed(vec![id.clone()]), "{point:?}: the commit announces it");
        assert_eq!(outbox_count(&admin, &id, "cfa4.execution.job.created").await, 1, "{point:?}: one created event");
    }
}

#[tokio::test]
async fn a_kill_at_every_point_of_the_start_transaction_starts_nothing_and_the_retry_starts_it_exactly_once() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let mut listener = admin.listen().await.expect("listen");
    let points = [Point::Statement("task"), Point::Statement("schedule"), Point::Statement("transition"), Point::Statement("outbox"), Point::Commit("outbox")];
    for (n, point) in (0u32..).zip(points) {
        let id = tid(200 + n);
        fresh(&db).await.tasks(QueueConfig::default()).submit(new_task(&id), 0).await.expect("submit");
        drain(&mut listener).await;
        let before = fingerprint(&admin, &id).await;
        let ticks_before = count(&admin, "SELECT count(*) FROM tasks.transition").await;
        arm(&admin, point, "60").await;
        let store = fresh(&db).await;
        let (result, held) = kill_while(&admin, async move {
            let mut m = manager();
            let r = store.tasks(QueueConfig::default()).tick(1, "sup-a", &mut m, &snap(), &Monitor::default()).await;
            (r, m.held_count())
        })
        .await;
        disarm(&admin, point).await;
        is_err_not_panic(point, &result);
        assert_eq!(held, 0, "{point:?}: the killed scheduler holds no reservation");
        assert_eq!(fingerprint(&admin, &id).await, before, "{point:?}: the task is exactly as it was: queued, no lease, no log");
        assert_eq!(count(&admin, "SELECT count(*) FROM tasks.transition").await, ticks_before, "{point:?}: no transition was written");
        assert_eq!(outbox_count(&admin, &id, STARTED).await, 0, "{point:?}: no `started` event");
        assert_eq!(listener.wait(Duration::from_millis(300)).await, Wake::Timeout, "{point:?}: nothing was announced");

        // two schedulers retry at once: exactly one of them starts it
        let (a, b) = (fresh(&db).await, fresh(&db).await);
        let (qa, qb) = (a.tasks(QueueConfig::default()), b.tasks(QueueConfig::default()));
        let (mut ma, mut mb) = (manager(), manager());
        let (s, mon) = (snap(), Monitor::default());
        let (ea, eb) = tokio::join!(qa.tick(2, "sup-a", &mut ma, &s, &mon), qb.tick(2, "sup-b", &mut mb, &s, &mon));
        let (ea, eb) = (ea.expect("retry a"), eb.expect("retry b"));
        assert_eq!(started_count(&ea, &id) + started_count(&eb, &id), 1, "{point:?}: started exactly once");
        assert_eq!(ma.held_count() + mb.held_count(), 1, "{point:?}: and held once");
        assert_eq!(outbox_count(&admin, &id, STARTED).await, 1, "{point:?}: one `started` event");
        assert_eq!(count(&admin, &format!("SELECT count(*) FROM tasks.transition WHERE task_id = '{id}'")).await, 1, "{point:?}: one transition");
        assert_eq!(count(&admin, &format!("SELECT (doc->>'attempts')::bigint FROM tasks.task WHERE id = '{id}'")).await, 1, "{point:?}: attempt 1");
        assert_eq!(listener.wait(Duration::from_secs(5)).await, Wake::Changed(vec![id.clone()]), "{point:?}: the commit announces it");
    }
}

#[tokio::test]
async fn a_client_that_vanishes_inside_the_start_transaction_leaves_it_either_whole_or_not_at_all() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    // inside a statement the unfinished transaction is rolled back when the connection ends; once
    // the COMMIT has been sent the server finishes it whether anyone is listening or not (the
    // classic commit in doubt): then the task is started whole, and the next scheduler must see
    // that and not start it a second time
    for (n, (point, committed)) in (0u32..).zip([(Point::Statement("transition"), false), (Point::Commit("outbox"), true)]) {
        let id = tid(250 + n);
        fresh(&db).await.tasks(QueueConfig::default()).submit(new_task(&id), 0).await.expect("submit");
        let before = fingerprint(&admin, &id).await;
        arm(&admin, point, "1.5").await;
        let store = fresh(&db).await;
        drop_client_while(&admin, async move {
            let mut m = manager();
            store.tasks(QueueConfig::default()).tick(1, "sup-a", &mut m, &snap(), &Monitor::default()).await
        })
        .await;
        disarm(&admin, point).await;
        let mut m = manager();
        let events = fresh(&db).await.tasks(QueueConfig::default()).tick(2, "sup-b", &mut m, &snap(), &Monitor::default()).await.expect("restart");
        if committed {
            assert_ne!(fingerprint(&admin, &id).await, before, "{point:?}: the commit that was already sent went through");
            assert_eq!(started_count(&events, &id), 0, "{point:?}: the restarted scheduler does not start it again");
            let t = fresh(&db).await.tasks(QueueConfig::default()).get(&id).await.expect("get").expect("task");
            assert_eq!((t.state, t.attempts, t.lease_owner.as_deref()), (State::Running, 1, Some("sup-a")), "{point:?}");
        } else {
            assert_eq!(started_count(&events, &id), 1, "{point:?}: the abandoned transaction was rolled back, so the restarted scheduler starts it");
        }
        assert_eq!(outbox_count(&admin, &id, STARTED).await, 1, "{point:?}: one `started` event either way");
        assert_eq!(count(&admin, &format!("SELECT count(*) FROM tasks.transition WHERE task_id = '{id}'")).await, 1, "{point:?}: one transition");
    }
}

#[tokio::test]
async fn a_kill_at_every_point_of_finish_keeps_the_reservation_and_the_retry_releases_it() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let points = [Point::Statement("task"), Point::Statement("schedule"), Point::Statement("transition"), Point::Statement("outbox"), Point::Commit("outbox")];
    for (n, point) in (0u32..).zip(points) {
        let id = tid(300 + n);
        let q = fresh(&db).await;
        let q = q.tasks(QueueConfig::default());
        q.submit(gpu_task(&id), 0).await.expect("submit");
        let mut m = manager();
        q.tick(1, "sup-a", &mut m, &snap(), &Monitor::default()).await.expect("start");
        assert_eq!(m.held_count(), 1);
        let before = fingerprint(&admin, &id).await;
        arm(&admin, point, "60").await;
        let (store, task) = (fresh(&db).await, id.clone());
        let (result, m) = kill_while(&admin, async move {
            let mut m = m;
            let r = store.tasks(QueueConfig::default()).finish(&task, "sup-a", State::Succeeded, 2, "done", &mut m).await;
            (r, m)
        })
        .await;
        disarm(&admin, point).await;
        is_err_not_panic(point, &result);
        assert_eq!(m.held_count(), 1, "{point:?}: the reservation is released only after the commit");
        assert_eq!(fingerprint(&admin, &id).await, before, "{point:?}: still running under its lease");
        let (mut m, store) = (m, fresh(&db).await);
        store.tasks(QueueConfig::default()).finish(&id, "sup-a", State::Succeeded, 3, "done", &mut m).await.unwrap_or_else(|e| panic!("{point:?}: retry: {e}"));
        assert_eq!(m.held_count(), 0, "{point:?}: released by the retry");
        assert_eq!(outbox_count(&admin, &id, "cfa4.execution.job.completed").await, 1, "{point:?}: one `completed` event");
    }
}

#[tokio::test]
async fn a_kill_at_every_point_of_cancel_leaves_the_task_running_and_the_retry_cancels_it() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let points = [Point::Statement("task"), Point::Statement("schedule"), Point::Statement("transition"), Point::Statement("outbox"), Point::Commit("outbox")];
    for (n, point) in (0u32..).zip(points) {
        let id = tid(400 + n);
        let q = fresh(&db).await;
        let q = q.tasks(QueueConfig::default());
        q.submit(new_task(&id), 0).await.expect("submit");
        q.tick(1, "sup-a", &mut manager(), &snap(), &Monitor::default()).await.expect("start");
        let version = q.get(&id).await.expect("get").expect("task").version;
        let before = fingerprint(&admin, &id).await;
        arm(&admin, point, "60").await;
        let (store, task) = (fresh(&db).await, id.clone());
        let result = kill_while(&admin, async move { store.tasks(QueueConfig::default()).cancel(&task, version, 2).await }).await;
        disarm(&admin, point).await;
        is_err_not_panic(point, &result);
        assert_eq!(fingerprint(&admin, &id).await, before, "{point:?}: the cancel left no trace");
        let state = fresh(&db).await.tasks(QueueConfig::default()).cancel(&id, version, 3).await.unwrap_or_else(|e| panic!("{point:?}: retry: {e}"));
        assert_eq!(state, State::Cancelling, "{point:?}");
    }
}

#[tokio::test]
async fn a_kill_in_a_checkpoint_or_a_renewal_changes_nothing_and_a_stage_is_recorded_once() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let id = tid(500);
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    q.submit(new_task(&id), 0).await.expect("submit");
    q.tick(0, "sup-a", &mut manager(), &snap(), &Monitor::default()).await.expect("start");
    drop(q);

    for point in [Point::Statement("task"), Point::Statement("checkpoint"), Point::Commit("checkpoint")] {
        let before = fingerprint(&admin, &id).await;
        arm(&admin, point, "60").await;
        let (store, task) = (fresh(&db).await, id.clone());
        let result =
            kill_while(&admin, async move { store.tasks(QueueConfig::default()).checkpoint_keyed(&task, "sup-a", 4, Some("blake3:stage-4")).await }).await;
        disarm(&admin, point).await;
        is_err_not_panic(point, &result);
        assert_eq!(fingerprint(&admin, &id).await, before, "{point:?}: no stage, no checkpoint in the document");
    }
    for point in [Point::Statement("task"), Point::Commit("task")] {
        let before = fingerprint(&admin, &id).await;
        arm(&admin, point, "60").await;
        let (store, task) = (fresh(&db).await, id.clone());
        let result = kill_while(&admin, async move { store.tasks(QueueConfig::default()).renew(&task, "sup-a", 20_000).await }).await;
        disarm(&admin, point).await;
        is_err_not_panic(point, &result);
        assert_eq!(fingerprint(&admin, &id).await, before, "{point:?}: the lease did not move");
    }
    // retries: twice the same stage is one row
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    q.checkpoint_keyed(&id, "sup-a", 4, Some("blake3:stage-4")).await.expect("checkpoint");
    q.checkpoint_keyed(&id, "sup-a", 4, Some("blake3:stage-4")).await.expect("the same stage again");
    assert_eq!(count(&admin, &format!("SELECT count(*) FROM tasks.checkpoint WHERE task_id = '{id}'")).await, 1);
    assert_eq!(q.renew(&id, "sup-a", 20_000).await.expect("renew"), 20_000 + LEASE_MS);
}

// ---------------------------------------------------------------------------------------------
// leases

#[tokio::test]
async fn a_lease_lapses_only_after_its_end_and_a_renewal_moves_the_end() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let (plain, renewed) = (tid(600), tid(601));
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    let (s, mon) = (snap(), Monitor::default());
    q.submit(new_task(&plain), 0).await.expect("submit");
    q.submit(new_task(&renewed), 0).await.expect("submit");
    let mut m = manager();
    q.tick(0, "sup-a", &mut m, &s, &mon).await.expect("start both");
    assert_eq!(q.get(&plain).await.expect("get").expect("task").lease_until, Some(LEASE_MS));
    assert_eq!(q.renew(&renewed, "sup-a", 20_000).await.expect("renew"), 20_000 + LEASE_MS);
    assert_eq!(
        text(&admin, &format!("SELECT (extract(epoch FROM lease_until) * 1000)::bigint::text FROM tasks.task WHERE id = '{renewed}'")).await,
        (20_000 + LEASE_MS).to_string()
    );

    // at the last millisecond of the lease the task is still alive
    let e = q.tick(LEASE_MS, "sup-b", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.iter().all(|e| !matches!(e, QueueEvent::LeaseLost { .. })), "{e:?}");
    // one millisecond later the plain one lapsed (never checkpointed: it fails), the renewed lives
    let e = q.tick(LEASE_MS + 1, "sup-b", &mut manager(), &s, &mon).await.expect("tick");
    assert_eq!(e, vec![QueueEvent::LeaseLost { id: plain.clone(), resumed: false }]);
    assert_eq!(q.get(&plain).await.expect("get").map(|t| t.state), Some(State::Failed));
    assert_eq!(q.get(&renewed).await.expect("get").map(|t| t.state), Some(State::Running));
    assert_eq!(text(&admin, &format!("SELECT coalesce(lease_owner, 'none') FROM tasks.task WHERE id = '{plain}'")).await, "none", "a lapsed lease is cleared");
    // the renewed lease ends at 50 000 exactly like that
    let e = q.tick(20_000 + LEASE_MS, "sup-b", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.is_empty(), "{e:?}");
    let e = q.tick(20_000 + LEASE_MS + 1, "sup-b", &mut manager(), &s, &mon).await.expect("tick");
    assert_eq!(e, vec![QueueEvent::LeaseLost { id: renewed.clone(), resumed: false }]);
}

#[tokio::test]
async fn a_lapsed_lease_never_starts_a_task_from_the_beginning_by_itself() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let (resumable, one_shot, no_stage) = (tid(610), tid(611), tid(612));
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    let (s, mon) = (snap(), Monitor::default());
    q.submit(new_task(&resumable), 0).await.expect("submit");
    q.submit(NewTask { resumable: false, ..new_task(&one_shot) }, 0).await.expect("submit");
    q.submit(new_task(&no_stage), 0).await.expect("submit");
    q.tick(0, "sup-a", &mut manager(), &s, &mon).await.expect("start");
    q.checkpoint(&resumable, "sup-a", 7).await.expect("checkpoint");
    q.checkpoint(&one_shot, "sup-a", 7).await.expect("checkpoint");

    // the first lapse: the checkpointed resumable task continues from stage 7, the others fail
    let e = q.tick(100_000, "sup-b", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.contains(&QueueEvent::LeaseLost { id: resumable.clone(), resumed: true }), "{e:?}");
    assert!(e.contains(&QueueEvent::Started { id: resumable.clone(), gpu_node: None, resumed_from: Some(7) }), "{e:?}");
    assert!(e.contains(&QueueEvent::LeaseLost { id: one_shot.clone(), resumed: false }), "{e:?}");
    assert!(e.contains(&QueueEvent::LeaseLost { id: no_stage.clone(), resumed: false }), "{e:?}");
    for id in [&one_shot, &no_stage] {
        assert_eq!(q.get(id).await.expect("get").map(|t| (t.state, t.attempts)), Some((State::Failed, 1)), "{id}: failed after its only start");
    }
    // the second lapse continues from the newer stage
    q.checkpoint(&resumable, "sup-b", 9).await.expect("checkpoint");
    let e = q.tick(200_000, "sup-c", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.contains(&QueueEvent::Started { id: resumable.clone(), gpu_node: None, resumed_from: Some(9) }), "{e:?}");
    assert_eq!(q.get(&resumable).await.expect("get").map(|t| t.attempts), Some(3));
    // days later, any number of scheduler steps: the failed ones are never started again
    for day in 1..=3u64 {
        let e = q.tick(day * 86_400_000, "sup-d", &mut manager(), &s, &mon).await.expect("tick");
        assert_eq!(started_count(&e, &one_shot) + started_count(&e, &no_stage), 0, "{e:?}");
    }
    for id in [&one_shot, &no_stage] {
        assert_eq!(outbox_count(&admin, id, STARTED).await, 1, "{id}: one `started` event ever");
        assert_eq!(outbox_count(&admin, id, "cfa4.execution.job.failed").await, 1, "{id}: one `failed` event");
    }
}

#[tokio::test]
async fn a_worker_that_lost_its_lease_cannot_touch_the_task_any_more() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let id = tid(620);
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    let (s, mon) = (snap(), Monitor::default());
    q.submit(gpu_task(&id), 0).await.expect("submit");
    let mut zombie_manager = manager();
    q.tick(0, "sup-a", &mut zombie_manager, &s, &mon).await.expect("start");
    q.checkpoint(&id, "sup-a", 2).await.expect("checkpoint");
    // sup-a stalls; sup-b takes the task over from stage 2
    let mut m = manager();
    let e = q.tick(60_000, "sup-b", &mut m, &s, &mon).await.expect("takeover");
    assert!(e.contains(&QueueEvent::Started { id: id.clone(), gpu_node: Some("renderD128".into()), resumed_from: Some(2) }), "{e:?}");
    let before = fingerprint(&admin, &id).await;
    // sup-a wakes up: every call is refused and changes nothing
    assert!(matches!(q.renew(&id, "sup-a", 61_000).await, Err(StoreError::Queue { error: QueueError::NotLeaseOwner { .. } })));
    assert!(matches!(q.checkpoint(&id, "sup-a", 3).await, Err(StoreError::Queue { error: QueueError::NotLeaseOwner { .. } })));
    assert!(matches!(
        q.finish(&id, "sup-a", State::Succeeded, 61_000, "late", &mut zombie_manager).await,
        Err(StoreError::Queue { error: QueueError::NotLeaseOwner { .. } })
    ));
    assert_eq!(fingerprint(&admin, &id).await, before);
    // the owner finishes it, once
    q.finish(&id, "sup-b", State::Succeeded, 62_000, "done", &mut m).await.expect("finish");
    assert_eq!(m.held_count(), 0);
    assert!(q.finish(&id, "sup-b", State::Succeeded, 63_000, "again", &mut m).await.is_err());
    assert_eq!(outbox_count(&admin, &id, "cfa4.execution.job.completed").await, 1);
}

#[tokio::test]
async fn two_schedulers_that_both_see_a_lapsed_lease_resume_the_task_once() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let id = tid(630);
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    q.submit(new_task(&id), 0).await.expect("submit");
    q.tick(0, "sup-a", &mut manager(), &snap(), &Monitor::default()).await.expect("start");
    q.checkpoint(&id, "sup-a", 3).await.expect("checkpoint");
    let (a, b) = (fresh(&db).await, fresh(&db).await);
    let (qa, qb) = (a.tasks(QueueConfig::default()), b.tasks(QueueConfig::default()));
    let (s, mon) = (snap(), Monitor::default());
    let (mut ma, mut mb) = (manager(), manager());
    let (ea, eb) = tokio::join!(qa.tick(60_000, "sup-b", &mut ma, &s, &mon), qb.tick(60_000, "sup-c", &mut mb, &s, &mon));
    let (ea, eb) = (ea.expect("tick"), eb.expect("tick"));
    assert_eq!(started_count(&ea, &id) + started_count(&eb, &id), 1, "{ea:?} {eb:?}");
    assert_eq!(ma.held_count() + mb.held_count(), 1);
    let t = q.get(&id).await.expect("get").expect("task");
    assert_eq!((t.state, t.attempts, t.checkpoint), (State::Running, 2, Some(3)));
    assert_eq!(outbox_count(&admin, &id, STARTED).await, 2, "the first start and the resume");
}

// ---------------------------------------------------------------------------------------------
// database restart

/// Retry `get` on a pool that may hold dead connections, until the server answers; how long it took
/// and how many answers were errors on the way.
async fn recover(q: &crate::TaskStore, id: &str, since: Instant) -> (Duration, u32) {
    let mut errors = 0u32;
    loop {
        match q.get(id).await {
            Ok(_) => return (since.elapsed(), errors),
            Err(e) => {
                assert!(matches!(e, StoreError::Unavailable { .. } | StoreError::Query { .. }), "an outage is an error value: {e:?}");
                errors += 1;
            }
        }
        assert!(since.elapsed() < Duration::from_secs(120), "the database did not come back within two minutes");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_database_restart_in_the_middle_of_running_tasks_loses_nothing_and_recovers_within_a_minute() {
    let Some((db, store)) = setup().await else { return };
    let (resumable, one_shot, late) = (tid(700), tid(701), tid(702));
    let q = store.tasks(QueueConfig::default());
    q.submit(new_task(&resumable), 0).await.expect("submit");
    q.submit(NewTask { resumable: false, ..new_task(&one_shot) }, 0).await.expect("submit");
    let mut m = manager();
    q.tick(0, "sup-a", &mut m, &snap(), &Monitor::default()).await.expect("start");
    q.checkpoint(&resumable, "sup-a", 3).await.expect("checkpoint");
    q.renew(&resumable, "sup-a", 10_000).await.expect("renew");
    assert_eq!(m.held_count(), 2);

    for mode in ["fast", "immediate"] {
        let before = (fingerprint(&admin(&db).await, &resumable).await, fingerprint(&admin(&db).await, &one_shot).await);
        let outage = Instant::now();
        db.stop_with(mode);
        // while the server is down nothing starts, nothing is lost, and nothing panics
        let down = q.tick(1, "sup-a", &mut m, &snap(), &Monitor::default()).await;
        assert!(matches!(down, Err(StoreError::Unavailable { .. })), "{mode}: {down:?}");
        assert!(matches!(q.submit(new_task(&late), 1).await, Err(StoreError::Unavailable { .. })));
        assert!(matches!(q.renew(&resumable, "sup-a", 1).await, Err(StoreError::Unavailable { .. })));
        assert_eq!(m.held_count(), 2, "{mode}: the reservations are kept through an outage");
        assert!(matches!(Store::connect(&db.config()).await, Err(StoreError::Unavailable { .. })));

        let restart = Instant::now();
        assert!(db.start_again(), "{mode}: the cluster starts again");
        let (rto, errors) = recover(&q, &resumable, restart).await;
        eprintln!(
            "PG-2 RTO ({mode} stop, throw-away cluster): usable {} ms after the start command, {} ms after the stop began, {errors} refused calls on the way",
            rto.as_millis(),
            outage.elapsed().as_millis()
        );
        assert!(rto < Duration::from_secs(60), "{mode}: RTO {rto:?} exceeds the one minute of P9");

        let after = (fingerprint(&admin(&db).await, &resumable).await, fingerprint(&admin(&db).await, &one_shot).await);
        assert_eq!(before, after, "{mode}: running tasks, leases, checkpoints and logs came back exactly");
        // the supervisor goes on with the same lease
        assert_eq!(q.renew(&resumable, "sup-a", 10_000).await.expect("renew"), 10_000 + LEASE_MS);
    }

    // a task submitted before a crash is started once, however often the scheduler retries across it
    q.submit(new_task(&late), 1).await.expect("submit");
    db.stop_with("immediate");
    let restart = Instant::now();
    assert!(db.start_again());
    let mut starts = 0;
    let mut tries = 0u32;
    loop {
        match q.tick(2, "sup-c", &mut m, &snap(), &Monitor::default()).await {
            Ok(events) => {
                starts += started_count(&events, &late);
                break;
            }
            Err(e) => {
                assert!(matches!(e, StoreError::Unavailable { .. } | StoreError::Query { .. }), "{e:?}");
                tries += 1;
            }
        }
        assert!(restart.elapsed() < Duration::from_secs(120), "the database did not come back");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    eprintln!("PG-2: the scheduler needed {tries} retries after the crash");
    assert_eq!(starts, 1);
    let admin = admin(&db).await;
    assert_eq!(outbox_count(&admin, &late, STARTED).await, 1);

    // and when the outage lasted past the leases, the first step afterwards follows the lease rule
    let e = q.tick(1_000_000, "sup-d", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
    assert!(e.contains(&QueueEvent::Started { id: resumable.clone(), gpu_node: None, resumed_from: Some(3) }), "{e:?}");
    assert!(e.contains(&QueueEvent::LeaseLost { id: one_shot.clone(), resumed: false }), "{e:?}");
    assert_eq!(q.get(&one_shot).await.expect("get").map(|t| t.state), Some(State::Failed));
    assert_eq!(outbox_count(&admin, &resumable, STARTED).await, 2);
    assert_eq!(outbox_count(&admin, &one_shot, STARTED).await, 1);
}

#[tokio::test]
async fn a_listener_notices_a_restart_and_a_new_one_wakes_again() {
    let Some((db, store)) = setup().await else { return };
    let mut listener = store.listen().await.expect("listen");
    db.stop_with("fast");
    assert_eq!(listener.wait(Duration::from_secs(10)).await, Wake::Lost);
    assert!(db.start_again());
    let q = store.tasks(QueueConfig::default());
    let id = tid(710);
    let mut new_listener = loop {
        match store.listen().await {
            Ok(l) => break l,
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    };
    loop {
        if q.submit(new_task(&id), 0).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(new_listener.wait(Duration::from_secs(5)).await, Wake::Changed(vec![id]));
}

// ---------------------------------------------------------------------------------------------
// schedule

#[tokio::test]
async fn timed_starts_are_exact_and_the_grace_window_has_hard_edges() {
    let Some((db, store)) = setup().await else { return };
    let admin = admin(&db).await;
    let q = store.tasks(QueueConfig::default());
    let (s, mon) = (snap(), Monitor::default());

    // exactly on time: not a millisecond before
    let (exact, with_resources) = (tid(800), tid(801));
    q.submit(NewTask { start: StartMode::At { at: 10_000 }, ..new_task(&exact) }, 0).await.expect("submit");
    q.submit(NewTask { start: StartMode::AtWithResources { at: 10_000 }, ..new_task(&with_resources) }, 0).await.expect("submit");
    assert!(q.tick(9_999, "sup", &mut manager(), &s, &mon).await.expect("tick").is_empty());
    assert_eq!(q.get(&exact).await.expect("get").map(|t| t.state), Some(State::Scheduled));
    let e = q.tick(10_000, "sup", &mut manager(), &s, &mon).await.expect("tick");
    assert_eq!((started_count(&e, &exact), started_count(&e, &with_resources)), (1, 1), "{e:?}");

    // the last millisecond of the grace window is still on time, the next one is a missed start
    let (edge, over) = (tid(802), tid(803));
    q.submit(NewTask { start: StartMode::At { at: 1_000_000 }, ..new_task(&edge) }, 0).await.expect("submit");
    let e = q.tick(1_000_000 + GRACE_MS, "sup", &mut manager(), &s, &mon).await.expect("tick");
    assert_eq!(started_count(&e, &edge), 1);
    assert!(e.iter().all(|e| !matches!(e, QueueEvent::Missed { .. })), "{e:?}");
    q.submit(NewTask { start: StartMode::At { at: 2_000_000 }, ..new_task(&over) }, 0).await.expect("submit");
    let e = q.tick(2_000_000 + GRACE_MS + 1, "sup", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.contains(&QueueEvent::Missed { id: over.clone(), scheduled_at: 2_000_000, action: MissedStart::RunLate, next_at: None }), "{e:?}");
    assert_eq!(started_count(&e, &over), 1, "run late: it still starts");
    // each of them started once
    for id in [&exact, &with_resources, &edge, &over] {
        assert_eq!(outbox_count(&admin, id, STARTED).await, 1, "{id}");
    }
}

#[tokio::test]
async fn a_missed_start_moves_to_the_same_local_time_across_a_restart_and_daylight_saving_changes() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    // (zone, planned, the machine is back at, the next slot)
    let cases = [
        ("Europe/Budapest", "2026-11-12T01:00:00Z", "2026-11-12T08:00:00Z", "2026-11-13T01:00:00Z"),
        // 02:30 does not exist on 2027-03-28: the first moment after the gap (03:30 summer time)
        ("Europe/Budapest", "2027-03-27T01:30:00Z", "2027-03-27T12:00:00Z", "2027-03-28T01:30:00Z"),
        // 22:00 summer time missed, the clocks go back that night: 22:00 winter time, an hour later in UTC
        ("Europe/Budapest", "2026-10-24T20:00:00Z", "2026-10-25T06:00:00Z", "2026-10-25T21:00:00Z"),
        // 02:30 exists twice on 2026-10-25: the first one
        ("Europe/Budapest", "2026-10-24T00:30:00Z", "2026-10-24T12:00:00Z", "2026-10-25T00:30:00Z"),
        ("America/New_York", "2027-03-13T13:00:00Z", "2027-03-13T14:00:00Z", "2027-03-14T12:00:00Z"),
        // the southern hemisphere changes in April: 10:00 Sydney is 11 hours ahead, then 10
        ("Australia/Sydney", "2027-04-02T23:00:00Z", "2027-04-03T05:00:00Z", "2027-04-04T00:00:00Z"),
        // half an hour offset, no changes
        ("Asia/Kolkata", "2026-11-12T03:30:00Z", "2026-11-12T10:00:00Z", "2026-11-13T03:30:00Z"),
        ("UTC", "2026-11-12T01:00:00Z", "2026-11-12T08:00:00Z", "2026-11-13T01:00:00Z"),
    ];
    for (n, (zone, planned, back, next)) in (0u32..).zip(cases) {
        let id = tid(900 + n);
        let (planned, back, next) = (at(&admin, planned).await, at(&admin, back).await, at(&admin, next).await);
        fresh(&db)
            .await
            .tasks(QueueConfig::default())
            .submit(NewTask { start: StartMode::At { at: planned }, missed: MissedStart::NextSlot, time_zone: Some(zone.into()), ..new_task(&id) }, 0)
            .await
            .expect("submit");
        // CFA4 starts again after the machine was off: a new process, a new connection pool
        let q = fresh(&db).await;
        let q = q.tasks(QueueConfig::default());
        let e = q.tick(back, "sup", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
        assert!(e.contains(&QueueEvent::Missed { id: id.clone(), scheduled_at: planned, action: MissedStart::NextSlot, next_at: Some(next) }), "{zone}: {e:?}");
        let t = q.get(&id).await.expect("get").expect("task");
        assert_eq!((t.state, t.version, t.start), (State::Scheduled, 2, StartMode::At { at: next }), "{zone}");
        // the database has the new slot, with the zone it was planned in
        let ms = |col: &str, table: &str| {
            format!(
                "SELECT round(extract(epoch FROM {col}) * 1000)::bigint::text FROM tasks.{table} WHERE {} = '{id}'",
                if table == "task" { "id" } else { "task_id" }
            )
        };
        assert_eq!(text(&admin, &ms("start_at", "schedule")).await, next.to_string(), "{zone}: schedule.start_at");
        assert_eq!(text(&admin, &ms("start_at", "task")).await, next.to_string(), "{zone}: task.start_at");
        assert_eq!(text(&admin, &format!("SELECT time_zone FROM tasks.schedule WHERE task_id = '{id}'")).await, zone);
        assert_eq!(outbox_count(&admin, &id, "cfa4.execution.job.progress").await, 1, "{zone}: the missed start is announced once");
        // a second scheduler (or the same one again) does not move it a second time
        let again = fresh(&db).await.tasks(QueueConfig::default()).tick(back, "sup-2", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
        assert!(again.is_empty(), "{zone}: {again:?}");
        assert_eq!(q.get(&id).await.expect("get").map(|t| t.version), Some(2), "{zone}");
        // and it starts at the new slot, not an hour or a millisecond early
        let early = q.tick(next - 1, "sup", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
        assert_eq!(started_count(&early, &id), 0, "{zone}: not before the slot");
        let on_time = q.tick(next, "sup", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
        assert_eq!(started_count(&on_time, &id), 1, "{zone}: exactly at the slot");
    }
}

#[tokio::test]
async fn the_other_missed_start_rules_and_an_unusable_zone_pause_the_task_for_the_user() {
    let Some((db, store)) = setup().await else { return };
    let admin = admin(&db).await;
    let q = store.tasks(QueueConfig::default());
    let (s, mon) = (snap(), Monitor::default());
    let (ask, bad_zone) = (tid(950), tid(951));
    q.submit(NewTask { start: StartMode::At { at: 1_000 }, missed: MissedStart::AskUser, ..new_task(&ask) }, 0).await.expect("submit");
    q.submit(
        NewTask { start: StartMode::At { at: 1_000 }, missed: MissedStart::NextSlot, time_zone: Some("Europe/Budapest".into()), ..new_task(&bad_zone) },
        0,
    )
    .await
    .expect("submit");
    // the stored zone became unknown (a stale tzdata, a hand edit): never a panic, never a guess
    exec(&admin, &format!("UPDATE tasks.task SET doc = jsonb_set(doc, '{{timeZone}}', '\"Mars/Olympus_Mons\"') WHERE id = '{bad_zone}'")).await;
    let e = q.tick(GRACE_MS * 10, "sup", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.contains(&QueueEvent::Missed { id: ask.clone(), scheduled_at: 1_000, action: MissedStart::AskUser, next_at: None }), "{e:?}");
    assert!(e.contains(&QueueEvent::Missed { id: bad_zone.clone(), scheduled_at: 1_000, action: MissedStart::NextSlot, next_at: None }), "{e:?}");
    assert!(started_ids(&e).is_empty());
    for (id, reason) in [(&ask, "missed start"), (&bad_zone, "missed start, next slot unknown")] {
        assert_eq!(q.get(id).await.expect("get").map(|t| t.state), Some(State::Paused), "{id}");
        assert_eq!(text(&admin, &format!("SELECT reason FROM tasks.transition WHERE task_id = '{id}' ORDER BY seq DESC LIMIT 1")).await, reason);
    }
    // later steps leave them paused until the user decides
    let e = q.tick(GRACE_MS * 20, "sup", &mut manager(), &s, &mon).await.expect("tick");
    assert!(e.is_empty(), "{e:?}");
}

#[tokio::test]
async fn the_stored_instants_do_not_depend_on_the_time_zone_of_the_database_server() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    // an instant in the repeated hour of Budapest (the hardest one), and one in its skipped hour
    let instants =
        [at(&admin, "2026-10-25T00:30:00Z").await + 123, at(&admin, "2026-10-25T01:30:00Z").await + 7, at(&admin, "2027-03-28T00:59:59Z").await + 999];
    for (n, zone) in (0u32..).zip(["Pacific/Auckland", "Europe/Budapest", "America/Los_Angeles", "UTC"]) {
        exec(&admin, &format!("ALTER DATABASE postgres SET timezone TO '{zone}'")).await;
        let pool = fresh(&db).await;
        let session = pool.client().await.expect("client").query_one("SHOW TimeZone", &[]).await.expect("show").get::<_, String>(0);
        assert_eq!(session, zone, "the application's sessions run in the server's zone");
        let q = pool.tasks(QueueConfig::default());
        for (k, instant) in (0u32..).zip(instants) {
            let id = tid(980 + n * 10 + k);
            q.submit(NewTask { start: StartMode::At { at: instant }, ..new_task(&id) }, 0).await.expect("submit");
            for sql in [
                format!("SELECT round(extract(epoch FROM start_at) * 1000)::bigint::text FROM tasks.task WHERE id = '{id}'"),
                format!("SELECT round(extract(epoch FROM start_at) * 1000)::bigint::text FROM tasks.schedule WHERE task_id = '{id}'"),
            ] {
                assert_eq!(text(&admin, &sql).await, instant.to_string(), "{zone}: {sql}");
            }
            // due exactly then, whatever the server thinks the local time is
            let early = q.tick(instant - 1, "sup", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
            assert_eq!(started_count(&early, &id), 0, "{zone}: not a millisecond early");
            let e = q.tick(instant, "sup", &mut manager(), &snap(), &Monitor::default()).await.expect("tick");
            assert_eq!(started_count(&e, &id), 1, "{zone}: {e:?}");
        }
    }
    exec(&admin, "ALTER DATABASE postgres RESET timezone").await;
}

// ---------------------------------------------------------------------------------------------
// outbox

fn collect(sink: &Arc<Mutex<Vec<OutboxEvent>>>) -> Vec<OutboxEvent> {
    sink.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[tokio::test]
async fn a_relay_that_dies_after_publishing_hands_the_same_events_over_again_with_the_same_ids() {
    let Some((db, _store)) = setup().await else { return };
    let admin = admin(&db).await;
    let id = tid(1100);
    let q = fresh(&db).await;
    let q = q.tasks(QueueConfig::default());
    q.submit(new_task(&id), 0).await.expect("submit");
    q.tick(1, "sup", &mut manager(), &snap(), &Monitor::default()).await.expect("start");
    q.finish(&id, "sup", State::Succeeded, 2, "done", &mut manager()).await.ok();
    let pending = fresh(&db).await.outbox().pending().await.expect("pending");
    assert!(pending >= 3, "created, started, progress: {pending}");

    // the publisher gets the batch and sends it; the relay dies before its transaction commits
    let sink = Arc::new(Mutex::new(Vec::new()));
    let (store, first) = (fresh(&db).await, sink.clone());
    let handle = tokio::spawn(async move {
        store
            .outbox()
            .relay_once(100, |events| {
                first.lock().unwrap_or_else(PoisonError::into_inner).extend(events);
                async {
                    tokio::time::sleep(Duration::from_secs(120)).await;
                    Ok(0)
                }
            })
            .await
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while collect(&sink).is_empty() {
        assert!(Instant::now() < deadline, "the relay never reached its publisher");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let killed = count(
        &admin,
        "SELECT count(*) FROM (SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = 'cfa4_login' AND state = 'idle in transaction') x",
    )
    .await;
    assert_eq!(killed, 1, "one relay transaction was waiting for its publisher");
    assert!(handle.await.expect("join").is_err(), "the killed relay reports an error");
    wait_quiet(&admin).await;
    assert_eq!(
        count(&admin, "SELECT count(*) FROM tasks.outbox WHERE published_at IS NULL").await,
        pending,
        "nothing was marked: the sent events are still due"
    );

    // the next relay sends them again: the same event_ids in the same order, which is what
    // consumers deduplicate on
    let second = Arc::new(Mutex::new(Vec::new()));
    let (store, sink2) = (fresh(&db).await, second.clone());
    let n = store
        .outbox()
        .relay_once(100, |events| {
            sink2.lock().unwrap_or_else(PoisonError::into_inner).extend(events);
            let n = collect(&sink2).len();
            async move { Ok(n) }
        })
        .await
        .expect("relay");
    assert_eq!(i64::try_from(n).ok(), Some(pending));
    let ids = |v: &[OutboxEvent]| v.iter().map(|e| (e.id, e.event_id.clone())).collect::<Vec<_>>();
    assert_eq!(ids(&collect(&sink)), ids(&collect(&second)));
    assert_eq!(count(&admin, "SELECT count(*) FROM tasks.outbox WHERE published_at IS NULL").await, 0);
}

#[tokio::test]
async fn the_entity_sequence_lets_a_consumer_restore_order_after_reordered_and_repeated_delivery() {
    let Some((db, store)) = setup().await else { return };
    let admin = admin(&db).await;
    let id = tid(1200);
    let q = store.tasks(QueueConfig::default());
    let (s, mon) = (snap(), Monitor::default());
    q.submit(new_task(&id), 0).await.expect("submit");
    let mut m = manager();
    q.tick(1, "sup-a", &mut m, &s, &mon).await.expect("start");
    q.checkpoint(&id, "sup-a", 1).await.expect("checkpoint");
    q.pause(&id, 2, 2).await.ok();
    q.tick(100_000, "sup-b", &mut manager(), &s, &mon).await.expect("lease lapses, resumes");
    q.finish(&id, "sup-b", State::Succeeded, 100_001, "done", &mut m).await.ok();
    let state = q.get(&id).await.expect("get").expect("task").state;

    let sink = Arc::new(Mutex::new(Vec::new()));
    let sink2 = sink.clone();
    store
        .outbox()
        .relay_once(100, |events| {
            sink2.lock().unwrap_or_else(PoisonError::into_inner).extend(events);
            let n = collect(&sink2).len();
            async move { Ok(n) }
        })
        .await
        .expect("relay");
    let events = collect(&sink);
    let seq = |e: &OutboxEvent| e.envelope.pointer("/payload/entity_seq").and_then(serde_json::Value::as_u64);
    // along the outbox order the sequence numbers of one task only go up, and they are the
    // numbers of the transition log
    let seqs: Vec<u64> = events.iter().filter_map(seq).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    let logged = text(&admin, &format!("SELECT string_agg(seq::text, ',' ORDER BY seq) FROM tasks.transition WHERE task_id = '{id}'")).await;
    let from_events: Vec<String> = seqs.iter().skip(1).map(u64::to_string).collect();
    assert_eq!(from_events.join(","), logged, "entity_seq follows tasks.transition.seq (created is 0)");

    // a consumer that sees them reversed, and some twice, still ends in the real state
    let mut delivered: Vec<&OutboxEvent> = events.iter().rev().collect();
    delivered.extend(events.iter().take(2));
    let mut seen_ids = std::collections::HashSet::new();
    let (mut last_seq, mut last_state): (Option<u64>, Option<String>) = (None, None);
    for e in delivered {
        if !seen_ids.insert(e.event_id.clone()) {
            continue; // a repeat: dropped by event_id
        }
        let Some(n) = seq(e) else { continue }; // a scheduler decision carries no sequence
        if last_seq.is_none_or(|l| n > l) {
            last_seq = Some(n);
            last_state = e.envelope.pointer("/payload/state").and_then(|v| v.as_str()).map(str::to_owned);
        }
    }
    assert_eq!(last_state.as_deref(), Some(crate::tasks::state_name(state).as_str()), "the newest sequence number carries the final state");
}
