use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::engine::timer_worker::{TimerWork, TimerWorker};
use std::sync::Arc;

#[test]
fn test_timer_lease_renewal_prevents_false_stale_recovery() {
    use flowable_engine::service::config::ProcessEngineConfiguration;

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let lock_timeout_ms = 300_000i64;
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.timer_lock_time_ms = lock_timeout_ms as u64;

    let engine = ProcessEngine::build_with_config(
        "timer_lease_renewal_engine".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        config,
    )
    .unwrap();

    let bpmn_xml = r#"
    <bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
      <bpmn2:process id="timer_lease_test_process" isExecutable="true">
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

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("timer_lease_test_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), bpmn_xml.to_string());
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

    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let timer_jobs = __runtime_store.snapshot_timer_job_states(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(timer_jobs.len(), 1);

    // 1. Advance time to make timer due
    mock_time.advance_time(300_001); // 5 mins + 1ms

    let worker = TimerWorker::new(engine.get_runtime_service(), "test");

    // Acquire and lock the job (simulate first worker)
    let works = worker.acquire_due_timers(300_000);
    assert_eq!(works.len(), 1);

    let job_id = match &works[0] {
        TimerWork::RuntimeJob(j) => j.timer_job_id.clone(),
        _ => panic!("Expected runtime job"),
    };

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job_before_renew = runtime_store
        .find_timer_job_state(&job_id, &mut session)
        .unwrap();
    let initial_lock_time = job_before_renew.lock_time.unwrap();
    session.rollback().unwrap();
    drop(session);

    // 3. Advance time just below the timeout (timeout is 5 mins = 300,000 ms)
    mock_time.advance_time(290_000); // 4 mins 50 secs later

    // 4. Renew lease
    worker.renew_timer_lease(&works[0]);

    let mut session = runtime_store.create_session().unwrap();
    let job_after_renew = runtime_store
        .find_timer_job_state(&job_id, &mut session)
        .unwrap();
    assert!(
        job_after_renew.lock_time.unwrap() > initial_lock_time,
        "Lock time should have advanced"
    );
    assert_eq!(
        job_after_renew.lock_time.unwrap(),
        mock_time.now().timestamp_millis()
    );

    // 5. Advance time by another 2 minutes (total 6m 50s since first lock, > 5m)
    mock_time.advance_time(120_000);

    // 6. Try to acquire as a different worker (simulated via runtime store call)
    let current_now = mock_time.now().timestamp_millis();
    let (stolen_jobs, _, _) = runtime_store.acquire_due_timer_jobs(
        "stealing_worker",
        current_now,
        lock_timeout_ms,
        &mut session,
    );

    // It should NOT be stolen because we renewed the lease 2 minutes ago, and timeout is 5 mins!
    assert_eq!(
        stolen_jobs.len(),
        0,
        "Job was stolen despite lease renewal!"
    );

    // 7. Advance time by another 4 mins (now 6 mins since renewal, > 5m)
    mock_time.advance_time(240_000);
    // Expired locks require reset before reacquire.
    let still_locked = runtime_store
        .acquire_due_timer_jobs(
            "stealing_worker",
            mock_time.now().timestamp_millis(),
            lock_timeout_ms,
            &mut session,
        )
        .0;
    assert!(
        still_locked.is_empty(),
        "acquisition must not reclaim expired leases directly"
    );
    session.rollback().unwrap();

    let reset = engine
        .get_runtime_service()
        .reset_expired_timer_job_locks(10);
    assert_eq!(reset, 1, "reset must clear the expired renewed lease");

    let mut session = runtime_store.create_session().unwrap();
    let (stolen_jobs, _, _) = runtime_store.acquire_due_timer_jobs(
        "stealing_worker",
        mock_time.now().timestamp_millis(),
        lock_timeout_ms,
        &mut session,
    );

    // After reset, the job is acquirable again.
    assert_eq!(
        stolen_jobs.len(),
        1,
        "Job was not stolen after renewed lease expired and was reset!"
    );
    assert_eq!(stolen_jobs[0].timer_job_id, job_id);
    assert_eq!(
        stolen_jobs[0].lock_owner.as_deref(),
        Some("stealing_worker")
    );
    session.rollback().unwrap();
}

/// Verifies that automatic heartbeat within the embedded executor keeps
/// the lease alive when work takes longer than the lock timeout.
///
/// Strategy: We cannot easily inject slow work, but we can verify the
/// executor's heartbeat mechanism is properly wired by manually calling
/// renew in a loop (simulating what the heartbeat thread does) and
/// verifying that the lock_time advances each time.
#[test]
fn test_manual_lease_renewal_extends_lock_multiple_times() {
    use flowable_engine::service::config::ProcessEngineConfiguration;

    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));
    let lock_timeout_ms = 300_000i64;
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.timer_lock_time_ms = lock_timeout_ms as u64;

    let engine = ProcessEngine::build_with_config(
        "multi_renewal_engine".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        config,
    )
    .unwrap();

    let bpmn_xml = r#"
    <bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
      <bpmn2:process id="multi_renewal_process" isExecutable="true">
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

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("multi_renewal_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), bpmn_xml.to_string());
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

    let worker = TimerWorker::new(engine.get_runtime_service(), "test");
    let works = worker.acquire_due_timers(300_000);
    assert_eq!(works.len(), 1);

    let job_id = match &works[0] {
        TimerWork::RuntimeJob(j) => j.timer_job_id.clone(),
        _ => panic!("Expected runtime job"),
    };

    // Simulate multiple heartbeat renewals (as the executor's heartbeat thread would do)
    let runtime_store = engine.get_runtime_store();
    for i in 0..5 {
        // Advance time by 1 minute each iteration
        mock_time.advance_time(60_000);

        worker.renew_timer_lease(&works[0]);

        // Create a fresh session AFTER renew commits, so reads see the committed write
        let mut session = runtime_store.create_session().unwrap();
        let job = runtime_store
            .find_timer_job_state(&job_id, &mut session)
            .unwrap();
        let expected_lock_time = mock_time.now().timestamp_millis();
        assert_eq!(
            job.lock_time.unwrap(),
            expected_lock_time,
            "Renewal {} should update lock_time",
            i
        );

        // Verify a competitor cannot steal the job
        let (stolen, _, _) = runtime_store.acquire_due_timer_jobs(
            "competitor",
            mock_time.now().timestamp_millis(),
            lock_timeout_ms,
            &mut session,
        );
        assert_eq!(
            stolen.len(),
            0,
            "Job should not be stealable after renewal {}",
            i
        );
        session.rollback().unwrap();
    }

    // Now wait past the lock timeout without renewing
    mock_time.advance_time(lock_timeout_ms + 1);

    let mut session = runtime_store.create_session().unwrap();
    let blocked = runtime_store
        .acquire_due_timer_jobs(
            "competitor",
            mock_time.now().timestamp_millis(),
            lock_timeout_ms,
            &mut session,
        )
        .0;
    assert!(
        blocked.is_empty(),
        "acquisition must not reclaim expired leases without reset"
    );
    session.rollback().unwrap();

    let reset = engine
        .get_runtime_service()
        .reset_expired_timer_job_locks(10);
    assert_eq!(reset, 1);

    let mut session = runtime_store.create_session().unwrap();
    let (stolen, _, _) = runtime_store.acquire_due_timer_jobs(
        "competitor",
        mock_time.now().timestamp_millis(),
        lock_timeout_ms,
        &mut session,
    );
    assert_eq!(
        stolen.len(),
        1,
        "Job should be stealable after lock timeout and reset"
    );
    session.rollback().unwrap();
}

/// Verifies that once work completes, further renewal calls are no-ops
/// (the timer job is deleted by execution so renewal finds nothing to update).
#[test]
fn test_renewal_after_completion_is_noop() {
    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));

    let engine = ProcessEngine::with_time_source(
        "renewal_after_complete_engine".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
    );

    let bpmn_xml = r#"
    <bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
      <bpmn2:process id="renewal_complete_process" isExecutable="true">
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

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("renewal_complete_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), bpmn_xml.to_string());
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

    let worker = TimerWorker::new(engine.get_runtime_service(), "test");
    let works = worker.acquire_due_timers(300_000);
    assert_eq!(works.len(), 1);

    // Execute the timer work - this triggers the timer and completes the process
    worker.execute_timer(&works[0]);

    // Timer job should now be deleted
    let job_id = match &works[0] {
        TimerWork::RuntimeJob(j) => j.timer_job_id.clone(),
        _ => panic!("Expected runtime job"),
    };
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_timer_job_state(&job_id, &mut session)
            .is_none(),
        "Timer job should be deleted after execution"
    );
    session.rollback().unwrap();
    drop(session);

    // Renewal should be a safe no-op (job doesn't exist, so UPDATE affects 0 rows)
    worker.renew_timer_lease(&works[0]);

    // No crash, no side effects
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_timer_job_state(&job_id, &mut session)
            .is_none(),
        "Timer job should remain deleted after stale renewal"
    );
    session.rollback().unwrap();
}
