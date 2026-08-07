use chrono::{TimeZone, Utc};
use flowable_engine::engine::external_worker_service::{
    ExternalWorkerBpmnErrorRequest, ExternalWorkerFailureRequest, ExternalWorkerFetchAndLockRequest,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use std::sync::Arc;

const TIMER_WAIT_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="external_worker_wait_process" isExecutable="true">
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

const TIMER_START_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="external_worker_start_process" isExecutable="true">
    <bpmn2:startEvent id="start">
      <bpmn2:timerEventDefinition>
        <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
      </bpmn2:timerEventDefinition>
    </bpmn2:startEvent>
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

const BPMN_ERROR_BOUNDARY_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="external_worker_bpmn_error_boundary_process" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="workerScope" />
    <bpmn2:subProcess id="workerScope">
      <bpmn2:startEvent id="scopeStart" />
      <bpmn2:sequenceFlow id="scopeFlow1" sourceRef="scopeStart" targetRef="timer" />
      <bpmn2:intermediateCatchEvent id="timer">
        <bpmn2:timerEventDefinition>
          <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
        </bpmn2:timerEventDefinition>
      </bpmn2:intermediateCatchEvent>
      <bpmn2:sequenceFlow id="scopeFlow2" sourceRef="timer" targetRef="scopeAfterErrorShouldNotRun" />
      <bpmn2:userTask id="scopeAfterErrorShouldNotRun" name="Should Not Run" />
      <bpmn2:sequenceFlow id="scopeFlow3" sourceRef="scopeAfterErrorShouldNotRun" targetRef="scopeEnd" />
      <bpmn2:endEvent id="scopeEnd" />
    </bpmn2:subProcess>
    <bpmn2:boundaryEvent id="catchBusinessError" attachedToRef="workerScope">
      <bpmn2:errorEventDefinition errorCode="BUSINESS_ERROR" />
    </bpmn2:boundaryEvent>
    <bpmn2:sequenceFlow id="errorFlow" sourceRef="catchBusinessError" targetRef="errorTask" />
    <bpmn2:userTask id="errorTask" name="Error Task" />
    <bpmn2:sequenceFlow id="errorEndFlow" sourceRef="errorTask" targetRef="end" />
    <bpmn2:sequenceFlow id="normalFlow" sourceRef="workerScope" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

const BPMN_ERROR_EVENT_SUBPROCESS_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="external_worker_bpmn_error_event_subprocess_process" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="workerScope" />
    <bpmn2:subProcess id="workerScope">
      <bpmn2:startEvent id="scopeStart" />
      <bpmn2:sequenceFlow id="scopeFlow1" sourceRef="scopeStart" targetRef="timer" />
      <bpmn2:intermediateCatchEvent id="timer">
        <bpmn2:timerEventDefinition>
          <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
        </bpmn2:timerEventDefinition>
      </bpmn2:intermediateCatchEvent>
      <bpmn2:sequenceFlow id="scopeFlow2" sourceRef="timer" targetRef="normalTask" />
      <bpmn2:userTask id="normalTask" name="Normal Task" />
      <bpmn2:sequenceFlow id="scopeFlow3" sourceRef="normalTask" targetRef="scopeEnd" />
      <bpmn2:endEvent id="scopeEnd" />

      <bpmn2:subProcess id="errorEventSubProcess" triggeredByEvent="true">
        <bpmn2:startEvent id="errorStart" isInterrupting="true">
          <bpmn2:errorEventDefinition errorCode="BUSINESS_ERROR" />
        </bpmn2:startEvent>
        <bpmn2:sequenceFlow id="errorFlow1" sourceRef="errorStart" targetRef="eventSubprocessErrorTask" />
        <bpmn2:userTask id="eventSubprocessErrorTask" name="Event Subprocess Error Task" />
        <bpmn2:sequenceFlow id="errorFlow2" sourceRef="eventSubprocessErrorTask" targetRef="errorEnd" />
        <bpmn2:endEvent id="errorEnd" />
      </bpmn2:subProcess>
    </bpmn2:subProcess>
    <bpmn2:sequenceFlow id="normalFlow" sourceRef="workerScope" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

fn build_engine() -> (ProcessEngine, Arc<TestTimeSource>) {
    let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let engine = ProcessEngine::build(
        "external-worker-engine".to_string(),
        Arc::clone(&time_source) as Arc<_>,
        db_store,
    );
    (engine, time_source)
}

fn deploy_process(engine: &ProcessEngine, resource_name: &str, bpmn: &str) -> String {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(resource_name.to_string())
                .add_string(resource_name.to_string(), bpmn.to_string()),
        )
        .unwrap();

    repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .last()
        .unwrap()
}

fn start_timer_wait_process(engine: &ProcessEngine) -> String {
    let process_definition_id = deploy_process(
        engine,
        "external-worker-wait.bpmn20.xml",
        TIMER_WAIT_PROCESS_BPMN,
    );
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

fn deploy_timer_start_process(engine: &ProcessEngine) {
    let _ = deploy_process(
        engine,
        "external-worker-start.bpmn20.xml",
        TIMER_START_PROCESS_BPMN,
    );
}

fn start_bpmn_error_boundary_process(engine: &ProcessEngine) -> String {
    let process_definition_id = deploy_process(
        engine,
        "external-worker-bpmn-error-boundary.bpmn20.xml",
        BPMN_ERROR_BOUNDARY_PROCESS_BPMN,
    );
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

fn start_bpmn_error_event_subprocess_process(engine: &ProcessEngine) -> String {
    let process_definition_id = deploy_process(
        engine,
        "external-worker-bpmn-error-event-subprocess.bpmn20.xml",
        BPMN_ERROR_EVENT_SUBPROCESS_PROCESS_BPMN,
    );
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

#[test]
fn fetch_and_lock_returns_only_supported_runtime_timer_jobs() {
    let (engine, time_source) = build_engine();
    let process_instance_id = start_timer_wait_process(&engine);
    deploy_timer_start_process(&engine);
    time_source.advance_time(300_001);

    let jobs = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 10,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap();

    assert_eq!(jobs.len(), 1, "only runtime timer jobs should be exposed");
    assert_eq!(jobs[0].process_instance_id, process_instance_id);
    assert_eq!(jobs[0].worker_id, "worker-a");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_job = store
        .find_timer_job_state(&jobs[0].id, &mut session)
        .expect("runtime timer job should remain persisted while locked");
    assert_eq!(stored_job.lock_owner.as_deref(), Some("worker-a"));
    assert_eq!(
        stored_job.lock_expiration_time,
        Some(jobs[0].lock_expiration_time)
    );
    // Legacy untyped acquire promotes the row to externalWorker.
    assert_eq!(
        store.find_timer_job_type(&jobs[0].id, &mut session),
        Some(flowable_engine::persistence::runtime_store::RuntimeJobType::ExternalWorker)
    );
}

#[test]
fn fetch_and_lock_skips_typed_timer_history_message_and_definition_schedule() {
    use flowable_engine::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};

    let (engine, time_source) = build_engine();
    let process_instance_id = start_timer_wait_process(&engine);
    // Typed external-worker sibling (should be acquired).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let now = store.time_source().now().timestamp_millis();
    store.insert_timer_job_state_with_type(
        &RuntimeTimerJobState {
            timer_job_id: "typed-ew".to_string(),
            process_instance_id: process_instance_id.clone(),
            execution_id: "exec-ew".to_string(),
            activity_id: "externalTask".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now - 1),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        },
        Some(&RuntimeJobType::ExternalWorker),
        &mut session,
    );
    for (id, job_type, activity_id) in [
        ("typed-timer", RuntimeJobType::Timer, "timerActivity"),
        ("typed-history", RuntimeJobType::History, "async-history"),
        (
            "typed-message",
            RuntimeJobType::Other("message".to_string()),
            "asyncTask",
        ),
        (
            "definition-suspend",
            RuntimeJobType::Timer,
            "process-definition-suspend",
        ),
    ] {
        let mut job = RuntimeTimerJobState {
            timer_job_id: id.to_string(),
            process_instance_id: process_instance_id.clone(),
            execution_id: format!("exec-{id}"),
            activity_id: activity_id.to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now - 1),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        };
        if id == "definition-suspend" {
            job.process_instance_id.clear();
        }
        store.insert_timer_job_state_with_type(&job, Some(&job_type), &mut session);
    }
    session.flush_and_commit().unwrap();
    time_source.advance_time(300_001);

    let jobs = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 20,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap();

    let ids: Vec<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
    // Legacy untyped intermediate timer from the process + typed EW.
    assert!(
        ids.iter().any(|id| *id == "typed-ew"),
        "typed externalWorker must be acquirable: {ids:?}"
    );
    assert_eq!(
        ids.len(),
        2,
        "must acquire only legacy untyped intermediate timer + typed EW, got {ids:?}"
    );
    for forbidden in [
        "typed-timer",
        "typed-history",
        "typed-message",
        "definition-suspend",
    ] {
        assert!(
            !ids.contains(&forbidden),
            "{forbidden} must not be acquired as external-worker"
        );
    }

    // Typed non-EW rows keep their type (not re-stamped).
    let mut session = store.create_session().unwrap();
    assert_eq!(
        store.find_timer_job_type("typed-timer", &mut session),
        Some(RuntimeJobType::Timer)
    );
    assert!(
        store
            .find_timer_job_state("typed-timer", &mut session)
            .unwrap()
            .lock_owner
            .is_none()
    );
}

#[test]
fn process_suspension_releases_and_restores_external_worker_job_family() {
    let (engine, time_source) = build_engine();
    let process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");
    let original_due_time = job.due_time;
    let original_retries = job.retries;

    engine
        .get_runtime_service()
        .suspend_process_instance(
            process_instance_id.clone(),
            ProcessInstanceUpdate::default(),
        )
        .expect("process suspension should move external worker jobs to suspended");
    let suspended = engine
        .get_management_service()
        .find_suspended_job_by_id(&job.id)
        .expect("external worker job should be visible as suspended");
    assert_eq!(suspended.due_time, original_due_time);
    assert_eq!(suspended.retries, Some(original_retries));
    assert!(suspended.lock_owner.is_none());
    assert!(suspended.lock_time.is_none());
    assert!(suspended.lock_expiration_time.is_none());

    let while_suspended = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap();
    assert!(while_suspended.is_empty());

    engine
        .get_runtime_service()
        .activate_process_instance(process_instance_id, ProcessInstanceUpdate::default())
        .expect("process activation should restore external worker jobs");
    assert!(
        engine
            .get_management_service()
            .find_suspended_job_by_id(&job.id)
            .is_none()
    );

    let reacquired = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap();
    assert_eq!(reacquired.len(), 1);
    assert_eq!(reacquired[0].id, job.id);
    assert_eq!(reacquired[0].worker_id, "worker-b");
    assert_eq!(reacquired[0].due_time, original_due_time);
    assert_eq!(reacquired[0].retries, original_retries);
}

#[test]
fn complete_advances_the_waiting_process_and_deletes_the_job() {
    let (engine, time_source) = build_engine();
    let process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    engine
        .get_external_worker_service()
        .complete(&job.id, "worker-a")
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job.id, &mut session).is_none(),
        "completed job should be deleted"
    );
    assert!(
        store
            .find_process_instance(&process_instance_id, &mut session)
            .expect("process instance should exist")
            .is_ended,
        "completing the external job should resume and finish the process"
    );
}

#[test]
fn complete_with_bpmn_error_triggers_matching_boundary_error_path() {
    let (engine, time_source) = build_engine();
    let process_instance_id = start_bpmn_error_boundary_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    engine
        .get_external_worker_service()
        .complete_with_bpmn_error(
            &job.id,
            ExternalWorkerBpmnErrorRequest {
                worker_id: "worker-a".to_string(),
                error_code: "BUSINESS_ERROR".to_string(),
                error_message: None,
                variables: None,
            },
        )
        .unwrap();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "errorTask");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job.id, &mut session).is_none(),
        "handled BPMN error should consume the external worker job"
    );
}

#[test]
fn complete_with_bpmn_error_triggers_matching_event_subprocess_path() {
    let (engine, time_source) = build_engine();
    let process_instance_id = start_bpmn_error_event_subprocess_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    engine
        .get_external_worker_service()
        .complete_with_bpmn_error(
            &job.id,
            ExternalWorkerBpmnErrorRequest {
                worker_id: "worker-a".to_string(),
                error_code: "BUSINESS_ERROR".to_string(),
                error_message: None,
                variables: None,
            },
        )
        .unwrap();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "eventSubprocessErrorTask");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job.id, &mut session).is_none(),
        "handled BPMN error event subprocess should consume the external worker job"
    );
}

#[test]
fn complete_with_bpmn_error_without_handler_keeps_job_and_returns_error() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    let error = engine
        .get_external_worker_service()
        .complete_with_bpmn_error(
            &job.id,
            ExternalWorkerBpmnErrorRequest {
                worker_id: "worker-a".to_string(),
                error_code: "BUSINESS_ERROR".to_string(),
                error_message: None,
                variables: None,
            },
        )
        .expect_err("unhandled BPMN error should be rejected");

    assert!(
        error.to_string().contains("No matching BPMN error handler"),
        "unexpected unhandled BPMN error result: {error}"
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job.id, &mut session).is_some(),
        "unhandled BPMN error must not consume the external worker job"
    );
}

#[test]
fn failure_persists_retry_schedule_and_error_details() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    engine
        .get_external_worker_service()
        .handle_failure(
            &job.id,
            ExternalWorkerFailureRequest {
                worker_id: "worker-a".to_string(),
                error_message: Some("worker failed".to_string()),
                error_details: Some("stacktrace".to_string()),
                retries: 3,
                retry_duration_ms: 45_000,
            },
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let persisted = store
        .find_timer_job_state(&job.id, &mut session)
        .expect("failed job should remain persisted");
    assert_eq!(persisted.retries, Some(3));
    assert_eq!(persisted.error_message.as_deref(), Some("worker failed"));
    assert_eq!(persisted.error_details.as_deref(), Some("stacktrace"));
    assert!(
        persisted.lock_owner.is_none(),
        "failure should release the lock"
    );
    assert_eq!(
        persisted.due_time,
        Some(time_source.now().timestamp_millis() + 45_000)
    );
    drop(session);

    let not_yet_due = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap();
    assert!(
        not_yet_due.is_empty(),
        "retry timeout should defer reacquisition"
    );

    time_source.advance_time(45_001);
    let retried = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap();
    assert_eq!(
        retried.len(),
        1,
        "job should become fetchable after retry timeout"
    );
    assert_eq!(retried[0].id, job.id);
}

#[test]
fn wrong_worker_rejected_but_expired_lock_still_operable() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 1_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    let wrong_worker_error = engine
        .get_external_worker_service()
        .complete(&job.id, "worker-b")
        .expect_err("wrong worker should be rejected");
    assert!(
        wrong_worker_error
            .to_string()
            .contains("locked by a different worker"),
        "unexpected wrong-worker error: {wrong_worker_error}"
    );

    // Java parity: lock expiration is NOT checked in the command layer.
    // A worker whose lock has expired can still complete/fail/unlock until
    // the background reset-expired sweep clears the owner. The owning
    // worker's operation succeeds even after the lock time has passed.
    time_source.advance_time(1_001);
    engine
        .get_external_worker_service()
        .complete(&job.id, "worker-a")
        .expect("expired lock must not prevent the owning worker from completing");
}

#[test]
fn owning_worker_can_unlock_legacy_lock_without_expiration() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    // Java resolveJob checks only lockOwner. Preserve that behavior for a
    // legacy/corrupt row whose owner exists without a lock expiration value.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut persisted = store
        .find_timer_job_state(&job.id, &mut session)
        .expect("locked job should exist");
    persisted.lock_expiration_time = None;
    store.insert_timer_job_state(&persisted, &mut session);
    session.flush_and_commit().unwrap();

    engine
        .get_external_worker_service()
        .unlock(&job.id, "worker-a")
        .expect("owning worker should be accepted without an expiration timestamp");

    let mut session = store.create_session().unwrap();
    let unlocked = store
        .find_timer_job_state(&job.id, &mut session)
        .expect("unlocked job should remain");
    assert!(unlocked.lock_owner.is_none());
    assert!(unlocked.lock_expiration_time.is_none());
    session.rollback().unwrap();
}

#[test]
fn unlock_releases_the_job_for_another_worker() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    engine
        .get_external_worker_service()
        .unlock(&job.id, "worker-a")
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let unlocked_state = store
        .find_timer_job_state(&job.id, &mut session)
        .expect("unlocked job should remain persisted");
    assert!(unlocked_state.lock_owner.is_none());
    assert!(unlocked_state.lock_time.is_none());
    assert!(unlocked_state.lock_expiration_time.is_none());
    drop(session);

    let reacquired = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap();
    assert_eq!(reacquired.len(), 1);
    assert_eq!(reacquired[0].id, job.id);
    assert_eq!(reacquired[0].worker_id, "worker-b");
}

#[test]
fn failure_with_zero_retries_moves_job_to_deadletter() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    engine
        .get_external_worker_service()
        .handle_failure(
            &job.id,
            ExternalWorkerFailureRequest {
                worker_id: "worker-a".to_string(),
                error_message: Some("terminal failure".to_string()),
                error_details: Some("stack".to_string()),
                retries: 0,
                retry_duration_ms: 0,
            },
        )
        .unwrap();

    let deadletter = engine
        .get_management_service()
        .find_deadletter_job_by_id(&job.id)
        .expect("exhausted external worker job must be queryable as deadletter");
    assert_eq!(deadletter.job_state.as_deref(), Some("deadletter"));
    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(
        deadletter.error_message.as_deref(),
        Some("terminal failure")
    );
    assert_eq!(deadletter.error_details.as_deref(), Some("stack"));
    assert!(deadletter.lock_owner.is_none());

    let reacquired = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap();
    assert!(
        reacquired.is_empty(),
        "deadletter external worker jobs must not be reacquired"
    );

    engine
        .get_management_service()
        .move_deadletter_job_to_executable_job(&job.id, 1)
        .expect("external-worker deadletter should return to its extension job family");

    let revived = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-b".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap();
    assert_eq!(revived.len(), 1);
    assert_eq!(revived[0].id, job.id);
}

#[test]
fn failure_with_negative_retries_decrements_and_may_deadletter() {
    let (engine, time_source) = build_engine();
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let job = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one external worker job");

    // Seed a known retries counter (fetch maps retries, default 1 from timer creation).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut seeded = store
        .find_timer_job_state(&job.id, &mut session)
        .expect("job");
    seeded.retries = Some(2);
    store.insert_timer_job_state(&seeded, &mut session);
    session.flush_and_commit().unwrap();
    drop(session);

    // retries < 0 means decrement current retries (Java default builder behavior).
    engine
        .get_external_worker_service()
        .handle_failure(
            &job.id,
            ExternalWorkerFailureRequest {
                worker_id: "worker-a".to_string(),
                error_message: Some("first fail".to_string()),
                error_details: None,
                retries: -1,
                retry_duration_ms: 1_000,
            },
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let after_first = store
        .find_timer_job_state(&job.id, &mut session)
        .expect("job after first failure");
    assert_eq!(after_first.retries, Some(1));
    assert_ne!(after_first.job_state.as_deref(), Some("deadletter"));
    assert!(after_first.lock_owner.is_none());
    drop(session);

    time_source.advance_time(1_001);
    let job2 = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-a".to_string(),
            max_jobs: 1,
            lock_duration_ms: 30_000,
            topic: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .expect("retryable job should reacquire");

    engine
        .get_external_worker_service()
        .handle_failure(
            &job2.id,
            ExternalWorkerFailureRequest {
                worker_id: "worker-a".to_string(),
                error_message: Some("final fail".to_string()),
                error_details: None,
                retries: -1,
                retry_duration_ms: 0,
            },
        )
        .unwrap();

    let deadletter = engine
        .get_management_service()
        .find_deadletter_job_by_id(&job.id)
        .expect("decrement-to-zero must move to deadletter");
    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(deadletter.job_state.as_deref(), Some("deadletter"));
}

// ── P54b S5: ExternalWorker service-task job creation (skip / category / interceptor) ──

const EXTERNAL_WORKER_SERVICE_TASK_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL"
                   xmlns:flowable="http://flowable.org/bpmn"
                   targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="external_worker_service_task_process" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="externalWorkerTask" />
    <bpmn2:serviceTask id="externalWorkerTask" name="External Worker"
                      flowable:type="external-worker" flowable:topic="orders"
                      flowable:skipExpression="${shouldSkip}">
      <bpmn2:extensionElements>
        <flowable:jobCategory>${jobCat}</flowable:jobCategory>
      </bpmn2:extensionElements>
    </bpmn2:serviceTask>
    <bpmn2:sequenceFlow id="flow2" sourceRef="externalWorkerTask" targetRef="afterTask" />
    <bpmn2:userTask id="afterTask" name="After External Worker" />
    <bpmn2:sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

const EXTERNAL_WORKER_SERVICE_TASK_NO_SKIP_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL"
                   xmlns:flowable="http://flowable.org/bpmn"
                   targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="external_worker_service_task_plain" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="externalWorkerTask" />
    <bpmn2:serviceTask id="externalWorkerTask" name="External Worker"
                      flowable:type="external-worker" flowable:topic="simple">
      <bpmn2:extensionElements>
        <flowable:jobCategory>priority-high</flowable:jobCategory>
      </bpmn2:extensionElements>
    </bpmn2:serviceTask>
    <bpmn2:sequenceFlow id="flow2" sourceRef="externalWorkerTask" targetRef="afterTask" />
    <bpmn2:userTask id="afterTask" name="After External Worker" />
    <bpmn2:sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

fn start_external_worker_service_task(
    engine: &ProcessEngine,
    bpmn: &str,
    resource: &str,
    variables: Vec<(String, serde_json::Value)>,
) -> String {
    let process_definition_id = deploy_process(engine, resource, bpmn);
    let mut builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    for (name, value) in variables {
        builder = builder.variable(name, value);
    }
    engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap()
        .id
}

/// S5-1: skipExpression true (with enable switch) leaves without creating an EW job.
#[test]
fn external_worker_service_task_skip_expression_leaves_without_job() {
    use serde_json::json;

    let (engine, _time_source) = build_engine();
    let process_instance_id = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_SERVICE_TASK_BPMN,
        "ew-skip.bpmn20.xml",
        vec![
            (
                "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
                json!(true),
            ),
            ("shouldSkip".to_string(), json!(true)),
            ("jobCat".to_string(), json!("ignored")),
        ],
    );

    let active = engine.get_external_worker_service().list_active_timer_jobs();
    assert!(
        active
            .iter()
            .all(|j| j.process_instance_id != process_instance_id),
        "skipExpression=true must not create an external-worker job"
    );

    let tasks = engine
        .get_task_service()
        .create_task_query()
        .process_instance_id(process_instance_id)
        .list()
        .unwrap();
    assert_eq!(tasks.len(), 1, "skip should leave to afterTask");
    assert_eq!(tasks[0].name, "After External Worker");
}

/// S5-2: jobCategory extension is resolved onto the created EW job.
#[test]
fn external_worker_service_task_job_category_on_created_job() {
    use flowable_engine::persistence::runtime_store::RuntimeJobType;
    use serde_json::json;

    let (engine, _time_source) = build_engine();
    let process_instance_id = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_SERVICE_TASK_BPMN,
        "ew-category.bpmn20.xml",
        vec![
            ("shouldSkip".to_string(), json!(false)),
            ("jobCat".to_string(), json!("category-from-var")),
        ],
    );

    let jobs = engine.get_external_worker_service().list_active_timer_jobs();
    let job = jobs
        .iter()
        .find(|j| j.process_instance_id == process_instance_id)
        .expect("external-worker service task must create a typed EW job");
    assert_eq!(job.activity_id, "externalWorkerTask");
    assert_eq!(job.category.as_deref(), Some("category-from-var"));
    assert_eq!(
        job.job_handler_configuration.as_deref(),
        Some("orders"),
        "topic must be stored as jobHandlerConfiguration"
    );
    assert_eq!(
        job.handler_type.as_deref(),
        Some("external-worker-complete")
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert_eq!(
        store.find_timer_job_type(&job.timer_job_id, &mut session),
        Some(RuntimeJobType::ExternalWorker)
    );
}

/// S5-3: CreateExternalWorkerJobInterceptor before/after hooks and topic override.
#[test]
fn external_worker_service_task_create_interceptor_overrides_topic() {
    use flowable_engine::engine::external_worker_service::{
        CreateExternalWorkerJobAfterContext, CreateExternalWorkerJobBeforeContext,
        CreateExternalWorkerJobInterceptor,
    };
    use flowable_engine::service::config::ProcessEngineConfiguration;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingInterceptor {
        before: AtomicUsize,
        after: AtomicUsize,
    }

    impl CreateExternalWorkerJobInterceptor for CountingInterceptor {
        fn before_create(&self, context: &mut CreateExternalWorkerJobBeforeContext<'_>) {
            self.before.fetch_add(1, Ordering::SeqCst);
            // Java test: topic + "Test"
            let base = context.service_task.topic.clone().unwrap_or_default();
            context.job_topic_expression = Some(format!("{base}Test"));
        }

        fn after_create(&self, _context: &CreateExternalWorkerJobAfterContext<'_>) {
            self.after.fetch_add(1, Ordering::SeqCst);
        }
    }

    let interceptor = Arc::new(CountingInterceptor {
        before: AtomicUsize::new(0),
        after: AtomicUsize::new(0),
    });

    let now = chrono::Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let mut config = ProcessEngineConfiguration::default();
    config.create_external_worker_job_interceptor =
        Some(interceptor.clone() as Arc<dyn CreateExternalWorkerJobInterceptor + Send + Sync>);
    let engine = ProcessEngine::build_with_db_store_and_config(
        "external-worker-interceptor-engine".to_string(),
        Arc::clone(&time_source) as Arc<_>,
        db_store,
        config,
    )
    .unwrap();

    let process_instance_id = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_SERVICE_TASK_NO_SKIP_BPMN,
        "ew-interceptor.bpmn20.xml",
        vec![],
    );

    assert_eq!(interceptor.before.load(Ordering::SeqCst), 1);
    assert_eq!(interceptor.after.load(Ordering::SeqCst), 1);

    let jobs = engine.get_external_worker_service().list_active_timer_jobs();
    let job = jobs
        .iter()
        .find(|j| j.process_instance_id == process_instance_id)
        .expect("interceptor path must still create the job");
    assert_eq!(
        job.job_handler_configuration.as_deref(),
        Some("simpleTest"),
        "before interceptor must override topic expression"
    );
    assert_eq!(job.category.as_deref(), Some("priority-high"));

    // Complete advances past the wait state.
    let locked = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "worker-interceptor".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: None,
        })
        .unwrap();
    assert_eq!(locked.len(), 1);
    engine
        .get_external_worker_service()
        .complete(&locked[0].id, "worker-interceptor")
        .unwrap();

    let tasks = engine
        .get_task_service()
        .create_task_query()
        .process_instance_id(process_instance_id)
        .list()
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After External Worker");
}

// ── P68: topic-filtered fetch + complete variables writeback ────────────────

const EXTERNAL_WORKER_TOPIC_A_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL"
                   xmlns:flowable="http://flowable.org/bpmn"
                   targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="ew_topic_a" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="ew" />
    <bpmn2:serviceTask id="ew" name="EW A"
                      flowable:type="external-worker" flowable:topic="orders" />
    <bpmn2:sequenceFlow id="flow2" sourceRef="ew" targetRef="after" />
    <bpmn2:userTask id="after" name="After A" />
    <bpmn2:sequenceFlow id="flow3" sourceRef="after" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

const EXTERNAL_WORKER_TOPIC_B_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL"
                   xmlns:flowable="http://flowable.org/bpmn"
                   targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="ew_topic_b" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="ew" />
    <bpmn2:serviceTask id="ew" name="EW B"
                      flowable:type="external-worker" flowable:topic="shipments" />
    <bpmn2:sequenceFlow id="flow2" sourceRef="ew" targetRef="after" />
    <bpmn2:userTask id="after" name="After B" />
    <bpmn2:sequenceFlow id="flow3" sourceRef="after" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

/// P68: fetch/acquire by topic only returns jobs subscribed to that topic.
/// Java `AcquireExternalWorkerJobsCmd.java:55-58` + entity manager topic filter.
#[test]
fn fetch_and_lock_filters_by_topic() {
    let (engine, _time_source) = build_engine();
    let pi_orders = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_TOPIC_A_BPMN,
        "ew-topic-a.bpmn20.xml",
        vec![],
    );
    let pi_shipments = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_TOPIC_B_BPMN,
        "ew-topic-b.bpmn20.xml",
        vec![],
    );

    let orders = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "orders-worker".to_string(),
            max_jobs: 10,
            lock_duration_ms: 60_000,
            topic: Some("orders".to_string()),
        })
        .unwrap();
    assert_eq!(orders.len(), 1, "only the orders topic job must be acquired");
    assert_eq!(orders[0].process_instance_id, pi_orders);
    assert_eq!(orders[0].topic.as_deref(), Some("orders"));

    let shipments = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "ship-worker".to_string(),
            max_jobs: 10,
            lock_duration_ms: 60_000,
            topic: Some("shipments".to_string()),
        })
        .unwrap();
    assert_eq!(shipments.len(), 1);
    assert_eq!(shipments[0].process_instance_id, pi_shipments);
    assert_eq!(shipments[0].topic.as_deref(), Some("shipments"));

    let none = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "other-worker".to_string(),
            max_jobs: 10,
            lock_duration_ms: 60_000,
            topic: Some("unknown".to_string()),
        })
        .unwrap();
    assert!(none.is_empty(), "unrelated topic must not match");
}

/// P68: complete with variables writes them back onto the process instance.
/// Java `ExternalWorkerJobCompleteCmd.java:75-81` (no out-parameters path).
#[test]
fn complete_with_variables_writes_process_variables() {
    use serde_json::json;
    use std::collections::HashMap;

    let (engine, _time_source) = build_engine();
    let process_instance_id = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_TOPIC_A_BPMN,
        "ew-complete-vars.bpmn20.xml",
        vec![("seed".to_string(), json!("before"))],
    );

    let locked = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "var-worker".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: Some("orders".to_string()),
        })
        .unwrap();
    assert_eq!(locked.len(), 1);
    // No in-parameters: acquired job carries full process variables.
    assert_eq!(
        locked[0].variables.get("seed"),
        Some(&json!("before")),
        "acquire should project process variables when no in-parameters"
    );

    let mut variables = HashMap::new();
    variables.insert("result".to_string(), json!("done"));
    variables.insert("count".to_string(), json!(42));
    engine
        .get_external_worker_service()
        .complete_with_variables(&locked[0].id, "var-worker", Some(variables))
        .unwrap();

    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id.clone(), "result".to_string())
            .unwrap(),
        Some(json!("done"))
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id.clone(), "count".to_string())
            .unwrap(),
        Some(json!(42))
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id, "seed".to_string())
            .unwrap(),
        Some(json!("before")),
        "pre-existing process variables must survive complete"
    );
}

// ── P75b: doNotIncludeVariables on external-worker service tasks ────────────

const EXTERNAL_WORKER_NO_VARS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL"
                   xmlns:flowable="http://flowable.org/bpmn"
                   targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="ew_no_vars" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="ew" />
    <bpmn2:serviceTask id="ew" name="EW No Vars"
                      flowable:type="external-worker" flowable:topic="simple"
                      flowable:doNotIncludeVariables="true" />
    <bpmn2:sequenceFlow id="flow2" sourceRef="ew" targetRef="after" />
    <bpmn2:userTask id="after" name="After" />
    <bpmn2:sequenceFlow id="flow3" sourceRef="after" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

/// P75b: default (no doNotIncludeVariables, no in-parameters) still projects
/// full process variables — must not regress P68.
/// Java DefaultInternalJobManager.java:106 `return executionEntity.getVariables()`.
#[test]
fn p75b_fetch_default_includes_full_process_variables() {
    use serde_json::json;

    let (engine, _time_source) = build_engine();
    let _pi = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_TOPIC_A_BPMN,
        "ew-p75b-default-vars.bpmn20.xml",
        vec![
            ("alpha".to_string(), json!("a")),
            ("beta".to_string(), json!(2)),
        ],
    );

    let locked = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "p75b-default".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: Some("orders".to_string()),
        })
        .unwrap();
    assert_eq!(locked.len(), 1);
    assert_eq!(
        locked[0].variables.get("alpha"),
        Some(&json!("a")),
        "P75b/P68: default fetch must include full process variables"
    );
    assert_eq!(locked[0].variables.get("beta"), Some(&json!(2)));
}

/// P75b: `flowable:doNotIncludeVariables="true"` with no in-parameters yields
/// an empty variable map on fetch (DefaultInternalJobManager.java:102-103;
/// ExternalWorkerServiceTaskTest.testWithNoInputVariables).
#[test]
fn p75b_fetch_do_not_include_variables_returns_empty() {
    use serde_json::json;

    let (engine, _time_source) = build_engine();
    let _pi = start_external_worker_service_task(
        &engine,
        EXTERNAL_WORKER_NO_VARS_BPMN,
        "ew-p75b-no-vars.bpmn20.xml",
        vec![
            ("secret".to_string(), json!("should-not-leak")),
            ("count".to_string(), json!(99)),
        ],
    );

    let locked = engine
        .get_external_worker_service()
        .fetch_and_lock(ExternalWorkerFetchAndLockRequest {
            worker_id: "p75b-no-vars".to_string(),
            max_jobs: 1,
            lock_duration_ms: 60_000,
            topic: Some("simple".to_string()),
        })
        .unwrap();
    assert_eq!(locked.len(), 1);
    assert!(
        locked[0].variables.is_empty(),
        "P75b: doNotIncludeVariables=true must yield empty variables on fetch; got {:?}",
        locked[0].variables
    );
}

/// P75b converter parity: `flowable:doNotIncludeVariables` is parsed onto the
/// ServiceTask model (ServiceTaskXMLConverter.convertExternalWorkerTaskXMLProperties).
#[test]
fn p75b_converter_parses_do_not_include_variables_on_service_task() {
    use flowable_bpmn_converter::BpmnXMLConverter;
    use flowable_bpmn_model::model::FlowElementEnum;

    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(EXTERNAL_WORKER_NO_VARS_BPMN);
    let process = model.main_process.as_ref().expect("main process");
    let FlowElementEnum::ServiceTask(st) = process
        .flow_element_map
        .get("ew")
        .expect("external worker service task")
    else {
        panic!("expected ServiceTask");
    };
    assert!(
        st.do_not_include_variables,
        "converter must set do_not_include_variables from flowable:doNotIncludeVariables"
    );
    assert_eq!(st.task_type.as_deref(), Some("external-worker"));
    assert_eq!(st.topic.as_deref(), Some("simple"));
}
