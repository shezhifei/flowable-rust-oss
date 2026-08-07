//! Tests proving that an embedded executor and a standalone-style worker
//! can coexist against the same shared database without double-executing
//! timer work.

#[path = "../src/bin/flowable_timer_worker.rs"]
mod flowable_timer_worker;

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::engine::timer_worker::{TimerWork, TimerWorker, TimerWorkerConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

const TIMER_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="standalone_test_process" isExecutable="true">
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

fn standalone_timer_test_guard() -> MutexGuard<'static, ()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unique_sqlite_path(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}.sqlite",
        std::process::id(),
        COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
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

/// Test that a standalone-style worker (using TimerWorker directly,
/// like the standalone binary would) can drive timer work against
/// the same DB that an embedded engine uses.
#[test]
fn test_standalone_worker_drives_timer_work_via_shared_db() {
    let _guard = standalone_timer_test_guard();
    let db_path = unique_sqlite_path("flowable-standalone-worker-shared-db");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    // Engine 1 deploys and starts a process instance
    let engine1 = ProcessEngine::build(
        "embedded_engine".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );

    let builder = engine1
        .get_repository_service()
        .create_deployment()
        .name("standalone_test_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), TIMER_BPMN.to_string());
    engine1.get_repository_service().deploy(builder).unwrap();

    let pd_id = engine1
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine1
        .get_runtime_service()
        .start_process_instance(
            engine1
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    // Verify timer job exists
    let __runtime_store = engine1.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let timer_jobs = __runtime_store.snapshot_timer_job_states(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(timer_jobs.len(), 1);

    // Advance time past timer due date
    mock_time.advance_time(300_001);

    // Engine 2 acts as the "standalone worker" - different owner, same DB
    let engine2 = ProcessEngine::build(
        "standalone_worker".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );

    // Use TimerWorker directly (same pattern as the standalone binary)
    let worker = TimerWorker::new(engine2.get_runtime_service(), "test");
    let works = worker.acquire_due_timers(300_000);
    assert_eq!(
        works.len(),
        1,
        "Standalone worker should acquire the due timer"
    );

    // Execute the timer
    worker.execute_timer(&works[0]);

    // Process should be completed
    let pi_store = engine1.get_runtime_store();
    let mut pi_session = pi_store.create_session().unwrap();
    let pi = pi_store
        .find_process_instance(&process_instance.id, &mut pi_session)
        .unwrap();
    pi_session.rollback().unwrap();
    assert!(
        pi.is_ended,
        "Process instance should be ended after standalone worker executes timer"
    );

    // Timer job should be gone
    let __runtime_store = engine1.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let remaining_jobs = __runtime_store.snapshot_timer_job_states(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(
        remaining_jobs.len(),
        0,
        "Timer job should be deleted after execution"
    );

    remove_sqlite_files(&db_path);
}

/// Ensures that when both an embedded executor and a standalone-style worker
/// compete for the same timer, only one succeeds.
#[test]
fn test_embedded_and_standalone_do_not_double_execute() {
    let _guard = standalone_timer_test_guard();
    let db_path = unique_sqlite_path("flowable-no-double-execute");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    // Embedded engine deploys and starts
    let engine_embedded = Arc::new(ProcessEngine::build(
        "embedded".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));

    let builder = engine_embedded
        .get_repository_service()
        .create_deployment()
        .name("no_double_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), TIMER_BPMN.to_string());
    engine_embedded
        .get_repository_service()
        .deploy(builder)
        .unwrap();

    let pd_id = engine_embedded
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine_embedded
        .get_runtime_service()
        .start_process_instance(
            engine_embedded
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    mock_time.advance_time(300_001);

    // Standalone worker engine
    let engine_standalone = Arc::new(ProcessEngine::build(
        "standalone".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));

    // Race: both try to run due timers concurrently
    let mut embedded_results = Vec::new();
    let mut standalone_results = Vec::new();
    for _ in 0..100 {
        let e1 = Arc::clone(&engine_embedded);
        let e2 = Arc::clone(&engine_standalone);

        let h1 = thread::spawn(move || e1.run_due_timers());
        let h2 = thread::spawn(move || e2.run_due_timers());

        let res1 = h1.join().unwrap();
        let res2 = h2.join().unwrap();
        if !res1.is_empty() || !res2.is_empty() {
            embedded_results = res1;
            standalone_results = res2;
            break;
        }

        thread::sleep(Duration::from_millis(25));
    }

    if embedded_results.is_empty() && standalone_results.is_empty() {
        for _ in 0..40 {
            let res1 = engine_embedded.run_due_timers();
            let res2 = engine_standalone.run_due_timers();
            if !res1.is_empty() || !res2.is_empty() {
                embedded_results = res1;
                standalone_results = res2;
                break;
            }

            thread::sleep(Duration::from_millis(25));
        }
    }

    let total = embedded_results.len() + standalone_results.len();
    assert_eq!(
        total, 1,
        "Timer should execute exactly once. embedded={:?}, standalone={:?}",
        embedded_results, standalone_results
    );

    let pi_store = engine_embedded.get_runtime_store();
    let mut pi_session = pi_store.create_session().unwrap();
    let pi = pi_store
        .find_process_instance(&process_instance.id, &mut pi_session)
        .unwrap();
    pi_session.rollback().unwrap();
    assert!(pi.is_ended, "Process should be ended");

    remove_sqlite_files(&db_path);
}

/// Verifies that after a standalone worker acquires and holds a lease,
/// the embedded engine cannot steal it until the lease expires.
#[test]
fn test_standalone_lease_renewal_uses_correct_owner() {
    use flowable_engine::service::config::ProcessEngineConfiguration;

    let _guard = standalone_timer_test_guard();
    let db_path = unique_sqlite_path("flowable-standalone-lease-hold");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());
    let lease_timeout_ms = 1_000i64;
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.timer_lock_time_ms = lease_timeout_ms as u64;

    let engine = ProcessEngine::build_with_db_store_and_config(
        "embedded_lease_test".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
        config,
    )
    .unwrap();

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("lease_test_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), TIMER_BPMN.to_string());
    engine.get_repository_service().deploy(builder).unwrap();

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    mock_time.advance_time(300_001);

    let standalone_owner = engine.get_runtime_service().timer_owner_id().to_string();

    let fencing_token = engine
        .get_runtime_service()
        .acquire_coordinator_lease(lease_timeout_ms as u64)
        .unwrap()
        .unwrap();

    // Standalone worker acquires with its real runtime owner id.
    let store = engine.get_runtime_store();
    let mut acquire_session = store.create_session().unwrap();
    let (standalone_acquired, _, _) = store.acquire_due_timer_jobs(
        &standalone_owner,
        mock_time.now().timestamp_millis(),
        lease_timeout_ms,
        &mut acquire_session,
    );
    acquire_session.flush_and_commit().unwrap();
    assert_eq!(standalone_acquired.len(), 1);
    assert_eq!(
        standalone_acquired[0].lock_owner.as_deref(),
        Some(standalone_owner.as_str()),
        "Acquisition must be owned by the standalone runtime owner"
    );

    let original_lock_time = standalone_acquired[0]
        .lock_time
        .expect("Acquired timer should have a lock time");

    // Renew with the same owner through the runtime contract.
    mock_time.advance_time(500);
    let worker = TimerWorker::new(engine.get_runtime_service(), "test");
    worker.set_fencing_token(fencing_token);
    let work = TimerWork::RuntimeJob(standalone_acquired[0].clone());
    worker.renew_timer_lease(&work);

    let snapshot_store = engine.get_runtime_store();
    let mut snapshot_session = snapshot_store.create_session().unwrap();
    let renewed_job = snapshot_store
        .snapshot_timer_job_states(&mut snapshot_session)
        .into_iter()
        .next()
        .expect("Expected timer job to remain after renewal");
    let renewed_lock_time = renewed_job
        .1
        .lock_time
        .expect("Renewal should update the lock time");
    snapshot_session.rollback().unwrap();
    assert_eq!(
        renewed_lock_time,
        mock_time.now().timestamp_millis(),
        "Renewal should write the current owner time"
    );
    assert!(
        renewed_lock_time > original_lock_time,
        "Renewal should move the lock forward in time"
    );

    // Embedded engine tries to acquire the same job before the renewed lease expires.
    let mut embedded_session = engine.get_runtime_store().create_session().unwrap();
    let (embedded_acquired, _, _) = engine.get_runtime_store().acquire_due_timer_jobs(
        "embedded_engine_owner",
        mock_time.now().timestamp_millis(),
        lease_timeout_ms,
        &mut embedded_session,
    );
    embedded_session.rollback().unwrap();
    assert_eq!(
        embedded_acquired.len(),
        0,
        "Embedded engine should not steal the renewed standalone lease"
    );

    // Even after the original lease window has elapsed, the renewed lease stays valid.
    mock_time.advance_time(600);
    let mut embedded_session2 = engine.get_runtime_store().create_session().unwrap();
    let (embedded_acquired2, _, _) = engine.get_runtime_store().acquire_due_timer_jobs(
        "embedded_engine_owner",
        mock_time.now().timestamp_millis(),
        lease_timeout_ms,
        &mut embedded_session2,
    );
    embedded_session2.rollback().unwrap();
    assert_eq!(
        embedded_acquired2.len(),
        0,
        "Renewed lease should remain protected before expiry"
    );

    // After the renewed lease expires, reset must run before reacquire.
    mock_time.advance_time(lease_timeout_ms + 1);
    let mut embedded_session3 = engine.get_runtime_store().create_session().unwrap();
    let blocked = engine
        .get_runtime_store()
        .acquire_due_timer_jobs(
            "embedded_engine_owner",
            mock_time.now().timestamp_millis(),
            lease_timeout_ms,
            &mut embedded_session3,
        )
        .0;
    embedded_session3.rollback().unwrap();
    assert!(
        blocked.is_empty(),
        "expired lease must not be reclaimed by acquisition alone"
    );

    let reset = engine
        .get_runtime_service()
        .reset_expired_timer_job_locks(10);
    assert_eq!(reset, 1, "reset must clear the expired renewed lease");

    let mut embedded_session4 = engine.get_runtime_store().create_session().unwrap();
    let (embedded_acquired3, _, _) = engine.get_runtime_store().acquire_due_timer_jobs(
        "embedded_engine_owner",
        mock_time.now().timestamp_millis(),
        lease_timeout_ms,
        &mut embedded_session4,
    );
    embedded_session4.rollback().unwrap();
    assert_eq!(
        embedded_acquired3.len(),
        1,
        "Expired renewed lease should be recoverable after reset"
    );

    remove_sqlite_files(&db_path);
}

/// Verifies that the standalone worker's shutdown path releases leadership
/// so another owner can immediately take over without waiting for timeout.
#[test]
fn test_standalone_worker_releases_leadership_on_shutdown() {
    let _guard = standalone_timer_test_guard();
    let db_path = unique_sqlite_path("flowable-standalone-shutdown-release");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "standalone_shutdown_owner".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );
    let expected_owner = engine.get_runtime_service().timer_owner_id().to_string();

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

    let lease = (0..100)
        .find_map(|_| {
            let lease_store = engine.get_runtime_store();
            let mut lease_session = lease_store.create_session().unwrap();
            let lease =
                lease_store.find_timer_coordinator_lease("timer-coordinator", &mut lease_session);
            lease_session.rollback().unwrap();
            if lease.is_some() {
                lease
            } else {
                thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("Standalone worker should acquire the coordinator lease");
    assert_eq!(lease.owner_node_id, expected_owner);

    shutdown_requested.store(true, Ordering::SeqCst);
    worker_handle
        .join()
        .expect("Standalone worker loop should stop cleanly");

    let released_store = engine.get_runtime_store();
    let mut released_session = released_store.create_session().unwrap();
    let released_lease = released_store
        .find_timer_coordinator_lease("timer-coordinator", &mut released_session)
        .expect("Shutdown should preserve the coordinator lease row");
    released_session.rollback().unwrap();
    assert!(
        released_lease.owner_node_id.is_empty(),
        "Shutdown should clear the coordinator owner"
    );
    assert!(
        released_lease.fencing_token > lease.fencing_token,
        "Shutdown should advance the fencing token"
    );

    let takeover_owner = "standby_after_shutdown".to_string();
    let takeover_engine = ProcessEngine::build(
        takeover_owner.clone(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );
    let expected_takeover_owner = takeover_engine
        .get_runtime_service()
        .timer_owner_id()
        .to_string();
    let reacquired_token = takeover_engine
        .get_runtime_service()
        .acquire_coordinator_lease(1_000)
        .unwrap();
    assert!(
        reacquired_token.is_some(),
        "Another worker should be able to acquire immediately after shutdown"
    );

    let reacquired_store = takeover_engine.get_runtime_store();
    let mut reacquired_session = reacquired_store.create_session().unwrap();
    let reacquired_lease = reacquired_store
        .find_timer_coordinator_lease("timer-coordinator", &mut reacquired_session)
        .expect("Coordinator lease should be visible after reacquire");
    reacquired_session.rollback().unwrap();
    assert_eq!(reacquired_lease.owner_node_id, expected_takeover_owner);
    assert!(
        reacquired_lease.fencing_token > released_lease.fencing_token,
        "Takeover should continue advancing the fencing token"
    );

    remove_sqlite_files(&db_path);
}

/// Java parity: `AcquireTimerJobsRunnable` holds the global acquire lock only
/// for the acquisition cycle (`LockManager.waitForLockRunAndRelease` — "we
/// only need to have the lock during the acquire"). The one-shot
/// `run_due_timers` path must likewise release the coordinator lease after
/// its batch, so another node can take over immediately instead of waiting
/// for the 300s lease to expire.
#[test]
fn test_run_due_timers_releases_coordinator_lease_after_batch() {
    let _guard = standalone_timer_test_guard();
    let db_path = unique_sqlite_path("flowable-run-due-timers-lease-release");

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine_a = ProcessEngine::build(
        "lease_release_a".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );

    let builder = engine_a
        .get_repository_service()
        .create_deployment()
        .name("lease_release_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), TIMER_BPMN.to_string());
    engine_a.get_repository_service().deploy(builder).unwrap();

    let pd_id = engine_a
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    engine_a
        .get_runtime_service()
        .start_process_instance(
            engine_a
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    mock_time.advance_time(300_001);

    let executed = engine_a.run_due_timers();
    assert_eq!(executed.len(), 1, "engine A should execute the due timer");

    // Regression guard: repeated one-shot runs by the same engine keep working
    // (passes both before and after the release fix; no work remains due).
    let executed_again = engine_a.run_due_timers();
    assert!(
        executed_again.is_empty(),
        "second run_due_timers by the same engine must succeed with no work left"
    );

    // Fairness: another node must be able to take over immediately, without
    // waiting for the 300s lease to expire.
    let engine_b = ProcessEngine::build(
        "lease_release_b".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    );
    let taken_over = engine_b
        .get_runtime_service()
        .acquire_coordinator_lease(300_000)
        .unwrap();
    assert!(
        taken_over.is_some(),
        "another engine must be able to acquire the coordinator lease right after run_due_timers"
    );

    remove_sqlite_files(&db_path);
}
