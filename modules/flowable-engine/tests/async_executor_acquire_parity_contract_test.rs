//! Contract tests for async-executor acquisition / failure / tenant semantics.
//!
//! Locks Java-aligned behavior that the P5-A race investigation depends on:
//! exclusive coordinator lease, missing-job execute is a conflict (not panic),
//! deadletter rows are not re-acquired, and default lock/batch/retry knobs.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::engine::timer_worker::TimerWork;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::service::config::{AsyncExecutorConfiguration, ProcessEngineConfiguration};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

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

fn now_fixed() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 26, 12, 0, 0).unwrap()
}

/// Java AsyncJobExecutorConfiguration defaults that Rust mirrors.
#[test]
fn async_executor_default_acquire_and_retry_knobs_match_java() {
    let config = AsyncExecutorConfiguration::default();
    assert_eq!(
        config.max_jobs_per_acquisition, 512,
        "Java maxAsyncJobsDuePerAcquisition / maxTimerJobsPerAcquisition default is 512"
    );
    assert_eq!(
        config.async_job_lock_time_ms, 3_600_000,
        "Java asyncJobLockTime default is 1 hour"
    );
    assert_eq!(
        config.timer_lock_time_ms, 3_600_000,
        "Java timerLockTime default is 1 hour"
    );
    assert_eq!(
        config.number_of_retries, 3,
        "Java asyncExecutorNumberOfRetries default is 3"
    );
    assert!(
        config.tenant_ids.is_empty(),
        "empty tenant_ids means shared mode (all tenants)"
    );
}

/// Concurrent one-shot `run_due_timers` against a shared DB must execute a due
/// intermediate timer exactly once (the flake that motivated P5-A).
#[test]
fn concurrent_run_due_timers_executes_intermediate_timer_exactly_once() {
    let now = now_fixed();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_path = unique_sqlite_path("p5a-concurrent-lease");
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine1 = Arc::new(ProcessEngine::build(
        "p5a-e1".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));
    let bpmn_xml = r#"
    <bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
      <bpmn2:process id="p5a_concurrent_timer" isExecutable="true">
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
    engine1
        .get_repository_service()
        .deploy(
            engine1
                .get_repository_service()
                .create_deployment()
                .name("p5a-concurrent".to_string())
                .add_string("process.bpmn20.xml".to_string(), bpmn_xml.to_string()),
        )
        .unwrap();

    let engine2 = Arc::new(ProcessEngine::build(
        "p5a-e2".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));

    let pd_id = engine1
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine1
        .get_runtime_service()
        .start_process_instance(
            engine1
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    mock_time.advance_time(300_001);

    let e1 = Arc::clone(&engine1);
    let e2 = Arc::clone(&engine2);
    let h1 = thread::spawn(move || e1.run_due_timers());
    let h2 = thread::spawn(move || e2.run_due_timers());
    let r1 = h1.join().unwrap();
    let r2 = h2.join().unwrap();
    let total = r1.len() + r2.len();
    assert_eq!(
        total, 1,
        "exactly one engine must execute the timer; res1={r1:?} res2={r2:?}"
    );

    let store = engine1.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let ended = store
        .snapshot_process_instances(&mut session)
        .get(&pi.id)
        .unwrap()
        .is_ended;
    session.rollback().unwrap();
    assert!(ended, "process must complete after exclusive timer fire");
    remove_sqlite_files(&db_path);
}

/// Two engines racing for the coordinator lease: at most one holds a live token.
#[test]
fn concurrent_coordinator_lease_is_exclusive() {
    let now = now_fixed();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let db_path = unique_sqlite_path("p5a-lease-exclusive");
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine1 = Arc::new(ProcessEngine::build(
        "lease-e1".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));
    let engine2 = Arc::new(ProcessEngine::build(
        "lease-e2".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));

    let e1 = Arc::clone(&engine1);
    let e2 = Arc::clone(&engine2);
    let h1 = thread::spawn(move || {
        e1.get_runtime_service()
            .acquire_coordinator_lease(300_000)
            .unwrap()
    });
    let h2 = thread::spawn(move || {
        e2.get_runtime_service()
            .acquire_coordinator_lease(300_000)
            .unwrap()
    });
    let t1 = h1.join().unwrap();
    let t2 = h2.join().unwrap();
    let winners = [t1.is_some(), t2.is_some()]
        .into_iter()
        .filter(|w| *w)
        .count();
    assert_eq!(
        winners, 1,
        "exactly one concurrent first-writer must win the lease; t1={t1:?} t2={t2:?}"
    );

    // Live lease must not be stolen while the owner node is missing but lease is fresh
    // (the pre-fix "missing node == dead" path).
    let loser = if t1.is_some() { &engine2 } else { &engine1 };
    let steal = loser
        .get_runtime_service()
        .acquire_coordinator_lease(300_000)
        .unwrap();
    assert!(
        steal.is_none(),
        "standby must not steal a still-valid lease from an unregistered owner"
    );

    remove_sqlite_files(&db_path);
}

/// Execute of a deleted job is a soft conflict, not a panic (async executor race).
#[test]
fn execute_timer_work_missing_job_is_conflict_not_panic() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let engine = ProcessEngine::build_with_config(
        "p5a-missing-job".to_string(),
        time_source,
        ProcessEngineConfiguration::default(),
    )
    .unwrap();

    let ghost = RuntimeTimerJobState {
        timer_job_id: "already-gone".to_string(),
        process_instance_id: "pi".to_string(),
        execution_id: "ex".to_string(),
        activity_id: "asyncTask".to_string(),
        job_state: Some("async".to_string()),
        due_time: Some(now_fixed().timestamp_millis()),
        retries: Some(3),
        lock_owner: Some(engine.get_runtime_service().timer_owner_id().to_string()),
        lock_time: Some(now_fixed().timestamp_millis()),
        lock_expiration_time: Some(now_fixed().timestamp_millis() + 60_000),
        ..Default::default()
    };
    let work = TimerWork::RuntimeJob(ghost);
    // fencing_token 0 + async job skips coordinator lease and hits the job re-read.
    let result = engine.get_runtime_service().execute_timer_work(&work, 0);
    assert!(
        result.is_none(),
        "missing job must return None (conflict), not panic"
    );
}

/// Deadletter jobs must not be returned by async acquisition.
#[test]
fn deadletter_jobs_are_not_acquired_by_async_path() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let engine = ProcessEngine::build_with_config(
        "p5a-deadletter-skip".to_string(),
        time_source.clone(),
        ProcessEngineConfiguration::default(),
    )
    .unwrap();
    let store = engine.get_runtime_store();
    let now_ms = time_source.now().timestamp_millis();

    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "dl-1".to_string(),
            process_instance_id: "pi-dl".to_string(),
            execution_id: "ex-dl".to_string(),
            activity_id: "asyncTask".to_string(),
            job_state: Some("deadletter".to_string()),
            due_time: Some(now_ms - 1_000),
            retries: Some(0),
            error_message: Some("boom".to_string()),
            ..Default::default()
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "async-1".to_string(),
            process_instance_id: "pi-ok".to_string(),
            execution_id: "ex-ok".to_string(),
            activity_id: "asyncTask".to_string(),
            job_state: Some("async".to_string()),
            due_time: Some(now_ms - 1_000),
            retries: Some(3),
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let acquired = engine.get_runtime_service().acquire_async_jobs(5_000, 10);
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].timer_job_id, "async-1");
    assert_ne!(
        acquired[0].job_state.as_deref(),
        Some("deadletter"),
        "deadletter must never be acquired"
    );
}

/// Failed async job decrements retries and records the error message (Java parity).
#[test]
fn failed_async_job_decrements_retries_and_stores_error_message() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.number_of_retries = 3;
    let engine = ProcessEngine::build_with_config(
        "p5a-retry-decrement".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();

    // Minimal async continuation process with a script that fails is heavy;
    // drive RecordFailedTimerWorkCmd via execute of a job whose execution is gone.
    let store = engine.get_runtime_store();
    let now_ms = time_source.now().timestamp_millis();
    let mut session = store.create_session().unwrap();
    let job = RuntimeTimerJobState {
        timer_job_id: "fail-retry-1".to_string(),
        process_instance_id: "pi-fail".to_string(),
        execution_id: "missing-execution".to_string(),
        activity_id: "asyncTask".to_string(),
        job_state: Some("async".to_string()),
        // Marker used by failure path for async-continuation retry cycle handling.
        time_duration: Some("__flowable_async_continuation".to_string()),
        due_time: Some(now_ms - 1),
        retries: Some(3),
        lock_owner: Some(engine.get_runtime_service().timer_owner_id().to_string()),
        lock_time: Some(now_ms),
        lock_expiration_time: Some(now_ms + 60_000),
        ..Default::default()
    };
    store.insert_timer_job_state(&job, &mut session);
    session.flush_and_commit().unwrap();

    let work = TimerWork::RuntimeJob(job.clone());
    let executed = engine.get_runtime_service().execute_timer_work(&work, 0);
    assert!(
        executed.is_none(),
        "missing execution must fail the job, not succeed"
    );

    let mut session = store.create_session().unwrap();
    let after = store
        .find_timer_job_state("fail-retry-1", &mut session)
        .expect("job row must remain for retry");
    session.rollback().unwrap();
    assert_eq!(
        after.retries,
        Some(2),
        "first failure must decrement retries by 1"
    );
    assert!(
        after.error_message.is_some(),
        "failure message must be persisted"
    );
    assert!(
        after.lock_owner.is_none(),
        "lock must be cleared on failure"
    );
}
