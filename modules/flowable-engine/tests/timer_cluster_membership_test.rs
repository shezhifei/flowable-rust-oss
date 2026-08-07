use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::engine::timer_worker::TimerWorker;
use flowable_engine::persistence::runtime_store::TimerWorkerNode;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn unique_sqlite_path(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}.sqlite",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let path = path.to_string_lossy().into_owned();
    remove_sqlite_files(&path);
    path
}

fn remove_sqlite_files(path: &str) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-wal"));
    let _ = std::fs::remove_file(format!("{path}-shm"));
}

#[test]
fn test_timer_cluster_membership_and_leadership() {
    let db_path = unique_sqlite_path("flowable-timer-cluster-membership");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    // Node 1 (Embedded)
    let engine1 = ProcessEngine::build(
        "node1".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );
    let worker1 = TimerWorker::new(engine1.get_runtime_service(), "embedded");

    // Node 2 (Standalone)
    let engine2 = ProcessEngine::build(
        "node2".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );
    let worker2 = TimerWorker::new(engine2.get_runtime_service(), "standalone");

    let timeout_ms = 300_000;

    // Both attempt to acquire timers
    let _ = worker1.acquire_due_timers(timeout_ms);
    let _ = worker2.acquire_due_timers(timeout_ms);

    let runtime_store = engine1.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Check membership
    let nodes = runtime_store.snapshot_timer_worker_nodes(&mut session);
    assert_eq!(nodes.len(), 2, "Both nodes should be registered");
    let mut types = nodes
        .values()
        .map(|n| n.worker_type.as_str())
        .collect::<Vec<_>>();
    types.sort();
    assert_eq!(types, vec!["embedded", "standalone"]);

    let node1_id = nodes
        .values()
        .find(|n| n.worker_type == "embedded")
        .unwrap()
        .node_id
        .clone();
    let node2_id = nodes
        .values()
        .find(|n| n.worker_type == "standalone")
        .unwrap()
        .node_id
        .clone();

    assert_eq!(
        nodes.get(&node1_id).unwrap().last_heartbeat,
        mock_time.now().timestamp_millis()
    );
    assert_eq!(
        nodes.get(&node2_id).unwrap().last_heartbeat,
        mock_time.now().timestamp_millis()
    );

    // Check leadership
    let lease = runtime_store
        .find_timer_coordinator_lease("timer-coordinator", &mut session)
        .unwrap();
    assert_eq!(
        lease.owner_node_id, node1_id,
        "First node should hold the lease"
    );
    assert_eq!(
        lease.expiry_time,
        mock_time.now().timestamp_millis() + timeout_ms as i64
    );

    session.rollback().unwrap();

    remove_sqlite_files(&db_path);
}

#[test]
fn test_stale_owner_heartbeat_allows_takeover_before_lease_expiry() {
    let db_path = unique_sqlite_path("flowable-timer-cluster-membership-stale-owner");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine1 = ProcessEngine::build(
        "node1".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );
    let engine2 = ProcessEngine::build(
        "node2".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );

    let timeout_ms = 300_000;

    let leader_token = engine1
        .get_runtime_service()
        .acquire_coordinator_lease(timeout_ms)
        .unwrap()
        .expect("leader should acquire the lease");

    let runtime_store = engine1.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let lease = runtime_store
        .find_timer_coordinator_lease("timer-coordinator", &mut session)
        .expect("expected lease");
    assert_eq!(
        lease.owner_node_id,
        engine1.get_runtime_service().timer_owner_id()
    );
    assert!(
        lease.expiry_time > mock_time.now().timestamp_millis(),
        "lease should still be live before takeover"
    );

    // Make the current owner stale without waiting for lease expiry.
    runtime_store.insert_timer_worker_node(
        TimerWorkerNode {
            node_id: engine1.get_runtime_service().timer_owner_id().to_string(),
            last_heartbeat: now.timestamp_millis() - timeout_ms as i64 - 1,
            worker_type: "embedded".to_string(),
        },
        &mut session,
    );

    session.flush_and_commit().unwrap();
    drop(session);

    let takeover_token = engine2
        .get_runtime_service()
        .acquire_coordinator_lease(timeout_ms)
        .unwrap()
        .expect("standby should take over from a stale owner");

    assert_eq!(takeover_token, leader_token + 1);

    let runtime_store2 = engine2.get_runtime_store();
    let mut session2 = runtime_store2.create_session().unwrap();
    let takeover_lease = runtime_store2
        .find_timer_coordinator_lease("timer-coordinator", &mut session2)
        .expect("expected takeover lease");
    assert_eq!(
        takeover_lease.owner_node_id,
        engine2.get_runtime_service().timer_owner_id()
    );
    assert_eq!(takeover_lease.fencing_token, takeover_token);

    session2.rollback().unwrap();

    remove_sqlite_files(&db_path);
}
