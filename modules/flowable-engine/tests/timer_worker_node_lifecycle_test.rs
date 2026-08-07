//! Timer worker node lifecycle tests
//!
//! Tests that exercise graceful deregistration, lifecycle visibility,
//! and the interaction between node lifecycle and coordinator leadership.

#[path = "../src/bin/flowable_timer_worker.rs"]
mod flowable_timer_worker;

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::engine::timer_worker::{TimerWorker, TimerWorkerConfig};
#[allow(unused_imports)]
use flowable_engine::persistence::runtime_store::{CoordinatorLeadershipStatus, NodeStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

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

/// Verifies that calling deregister() removes the node from the node list
/// while leaving the coordinator lease intact.
#[test]
fn test_worker_deregister_removes_node_from_registry() {
    let db_path = unique_sqlite_path("flowable-worker-deregister-removes-node");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build("lifecycle-test".to_string(), mock_time.clone(), db_store);

    let runtime_service = engine.get_runtime_service();
    let worker = TimerWorker::new(Arc::clone(&runtime_service), "standalone");

    // Heartbeat registers the node
    worker.heartbeat();
    let nodes = engine.list_timer_nodes();
    assert_eq!(nodes.len(), 1, "Worker should appear after heartbeat");
    let _node_id = nodes[0].node_id.clone();

    // Acquire lease so we can verify it survives deregistration
    let token = runtime_service.acquire_coordinator_lease(300_000).unwrap();
    assert!(token.is_some(), "Should acquire lease");

    // Deregister only removes the node, not the lease
    worker.deregister();

    let nodes_after = engine.list_timer_nodes();
    assert!(
        nodes_after.is_empty(),
        "Node should be gone after deregister"
    );

    // Coordinator lease should still be active
    let status = engine.get_timer_coordinator_status();
    assert_eq!(status.status, CoordinatorLeadershipStatus::Active);
    assert!(!status.leader_node_id.is_empty());

    remove_sqlite_files(&db_path);
}

/// Verifies that graceful_shutdown both releases leadership and deregisters
/// the node from the registry.
#[test]
fn test_graceful_shutdown_releases_and_deregisters() {
    let db_path = unique_sqlite_path("flowable-graceful-shutdown");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "graceful-test".to_string(),
        mock_time.clone(),
        db_store.clone(),
    );

    let runtime_service = engine.get_runtime_service();
    let worker = TimerWorker::new(Arc::clone(&runtime_service), "standalone");

    // Heartbeat + acquire lease
    worker.heartbeat();
    let _works = worker.acquire_due_timers(300_000);
    // No timers to acquire, but this registers the node and acquires the lease

    let nodes = engine.list_timer_nodes();
    assert_eq!(nodes.len(), 1, "Worker should be registered after acquire");

    let _status = engine.get_timer_coordinator_status();
    // Status might be Active or NoLeader depending on whether acquire succeeded
    // The key point is the node is registered

    // Graceful shutdown
    worker.graceful_shutdown();

    // Node should be gone
    let nodes_after = engine.list_timer_nodes();
    assert!(
        nodes_after.is_empty(),
        "Node should be deregistered after graceful_shutdown"
    );

    // Leadership should be released
    let status_after = engine.get_timer_coordinator_status();
    assert_eq!(status_after.leader_node_id, "", "Leader should be released");

    remove_sqlite_files(&db_path);
}

/// Verifies multi-node lifecycle: two workers register, each can be
/// independently deregistered without affecting the other.
#[test]
fn test_multi_node_independent_lifecycle() {
    let db_path = unique_sqlite_path("flowable-multi-node-lifecycle");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    // Engine 1
    let engine1 = ProcessEngine::build("node-1".to_string(), mock_time.clone(), db_store.clone());

    // Engine 2
    let engine2 = ProcessEngine::build("node-2".to_string(), mock_time.clone(), db_store.clone());

    let worker1 = TimerWorker::new(engine1.get_runtime_service(), "embedded");
    let worker2 = TimerWorker::new(engine2.get_runtime_service(), "standalone");

    worker1.heartbeat();
    worker2.heartbeat();

    // Both should be visible from either engine
    let nodes = engine1.list_timer_nodes();
    assert_eq!(nodes.len(), 2, "Both nodes should be registered");

    // Deregister node 1 only
    worker1.deregister();

    let nodes_after = engine1.list_timer_nodes();
    assert_eq!(nodes_after.len(), 1, "Only node 2 should remain");
    assert!(
        nodes_after[0].node_id.starts_with("node-2:"),
        "Remaining node should be node-2"
    );

    // Deregister node 2
    worker2.deregister();
    let nodes_final = engine1.list_timer_nodes();
    assert!(nodes_final.is_empty(), "No nodes should remain");

    remove_sqlite_files(&db_path);
}

/// Verifies that after admin step-down, a worker with the old fencing token
/// cannot execute timer work.
#[test]
fn test_step_down_blocks_stale_token_execution() {
    let db_path = unique_sqlite_path("flowable-step-down-blocks-stale");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let bpmn = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="step_down_test_process" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="timer" />
    <bpmn2:intermediateCatchEvent id="timer">
      <bpmn2:timerEventDefinition>
        <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
      </bpmn2:timerEventDefinition>
    </bpmn2:intermediateCatchEvent>
    <bpmn2:sequenceFlow id="flow2" sourceRef="timer" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

    let engine = ProcessEngine::build(
        "stale-token-test".to_string(),
        mock_time.clone(),
        db_store.clone(),
    );

    // Deploy and start
    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("step_down_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), bpmn.to_string());
    engine.get_repository_service().deploy(builder).unwrap();

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    // Advance time past timer
    mock_time.advance_time(300_001);

    // Worker acquires timer with current token
    let worker = TimerWorker::new(engine.get_runtime_service(), "test");
    let works = worker.acquire_due_timers(300_000);
    assert_eq!(works.len(), 1, "Should acquire due timer");

    let old_token = worker.get_fencing_token();
    assert!(old_token > 0, "Worker should hold a valid fencing token");

    // Admin step-down: this advances the fencing token
    let (success, new_token) = engine.admin_step_down();
    assert!(success, "Admin step-down should succeed");
    assert!(new_token > old_token, "New fencing token should be higher");

    // Worker still has the old token. Execution should be rejected
    // because the fencing token no longer matches.
    worker.execute_timer(&works[0]);

    // Process instance should NOT be completed
    let pi_store = engine.get_runtime_store();
    let mut pi_session = pi_store.create_session().unwrap();
    let pi_after = pi_store
        .find_process_instance(&pi.id, &mut pi_session)
        .unwrap();
    assert!(
        !pi_after.is_ended,
        "Process should NOT be ended - stale token should be rejected"
    );

    pi_session.rollback().unwrap();
    drop(pi_session);

    // Timer job should still exist (not consumed)
    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let remaining = __runtime_store.snapshot_timer_job_states(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "Timer job should still exist after rejected execution"
    );

    remove_sqlite_files(&db_path);
}

/// Verifies that the standalone worker loop deregisters its node on shutdown,
/// and a second worker can see the empty node list through the control surface.
#[test]
fn test_standalone_loop_deregisters_on_shutdown() {
    let db_path = unique_sqlite_path("flowable-standalone-loop-deregister");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "standalone-loop-test".to_string(),
        mock_time.clone(),
        db_store.clone(),
    );

    let _expected_owner = engine.get_runtime_service().timer_owner_id().to_string();
    let worker = TimerWorker::new(engine.get_runtime_service(), "standalone");
    let config = TimerWorkerConfig {
        poll_interval_ms: 1,
        heartbeat_interval_ms: 1,
        max_jitter_ms: 0,
        coordinator_lease_timeout_ms: 1_000,
    };

    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown_requested);
    let worker_handle = thread::spawn(move || {
        flowable_timer_worker::run_worker_loop(worker, config, worker_shutdown);
    });

    // Wait for the worker to register its node
    let found_node = (0..100).find_map(|_| {
        let nodes = engine.list_timer_nodes();
        if !nodes.is_empty() {
            Some(nodes[0].node_id.clone())
        } else {
            thread::sleep(Duration::from_millis(10));
            None
        }
    });
    assert!(
        found_node.is_some(),
        "Standalone worker should register its node"
    );

    // Request shutdown
    shutdown_requested.store(true, Ordering::SeqCst);
    worker_handle
        .join()
        .expect("Worker loop should stop cleanly");

    // After shutdown, node should be deregistered
    let nodes_after = engine.list_timer_nodes();
    assert!(
        nodes_after.is_empty(),
        "Node should be deregistered after standalone worker shutdown"
    );

    // Coordinator lease should show no leader
    let status = engine.get_timer_coordinator_status();
    assert_eq!(
        status.leader_node_id, "",
        "Leadership should be released after shutdown"
    );

    remove_sqlite_files(&db_path);
}

/// Verifies that stale/expired nodes are visible through the control surface
/// and can be cleaned up without affecting active nodes.
#[test]
fn test_expired_node_visibility_and_selective_cleanup() {
    let db_path = unique_sqlite_path("flowable-expired-node-visibility");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    // Two workers
    let engine1 = ProcessEngine::build(
        "active-node".to_string(),
        mock_time.clone(),
        db_store.clone(),
    );
    let engine2 = ProcessEngine::build(
        "stale-node".to_string(),
        mock_time.clone(),
        db_store.clone(),
    );

    let worker1 = TimerWorker::new(engine1.get_runtime_service(), "embedded");
    let worker2 = TimerWorker::new(engine2.get_runtime_service(), "standalone");

    // Both heartbeat at time 0
    worker1.heartbeat();
    worker2.heartbeat();

    let nodes = engine1.list_timer_nodes();
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|n| n.status == NodeStatus::Active));

    // Advance time by 10 minutes (past the 5-minute heartbeat timeout)
    mock_time.advance_time(600_000);

    // Only worker1 heartbeats again
    worker1.heartbeat();

    let nodes = engine1.list_timer_nodes();
    assert_eq!(nodes.len(), 2, "Both nodes should still be visible");

    // Check statuses
    let active_count = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Active)
        .count();
    let expired_count = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Expired)
        .count();
    assert_eq!(active_count, 1, "Only worker1 should be active");
    assert_eq!(expired_count, 1, "Worker2 should be expired");

    // Cleanup should only remove the expired node
    let cleaned = engine1.cleanup_expired_timer_nodes();
    assert_eq!(cleaned, 1, "Should clean exactly 1 expired node");

    let nodes_final = engine1.list_timer_nodes();
    assert_eq!(nodes_final.len(), 1, "Only active node should remain");
    assert!(nodes_final[0].node_id.starts_with("active-node:"));

    remove_sqlite_files(&db_path);
}
