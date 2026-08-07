use chrono::{TimeZone, Utc};
use flowable_engine::agenda::future_operations::PendingFutureRegistry;
use flowable_engine::bpmn::http_handler::{
    HttpHandlerRegistry, HttpResponseHandler, HttpResponseHandlerContext,
};
use flowable_engine::cmd::record_failed_timer_work_cmd::RecordFailedTimerWorkCmd;
use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType, TransactionState,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::engine::timer_worker::TimerWork;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    HttpServiceRuntimeMode, HttpServiceTaskConfiguration, ProcessEngineConfiguration,
    RealHttpClientConfiguration,
};
use serde_json::json;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct UnrecoverableResponseHandler;

impl HttpResponseHandler for UnrecoverableResponseHandler {
    fn handle_response(
        &self,
        _context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        Err(FlowableError::UnrecoverableJobError(
            "response payload cannot be safely processed".to_string(),
        ))
    }
}

struct NestedUnrecoverableResponseHandler;

impl HttpResponseHandler for NestedUnrecoverableResponseHandler {
    fn handle_response(
        &self,
        _context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        Err(
            FlowableError::ExecutionError("response handler wrapper failed".to_string()).caused_by(
                FlowableError::UnrecoverableJobError(
                    "response payload cannot be safely processed".to_string(),
                ),
            ),
        )
    }
}

struct RecordingJobEventListener {
    events: Arc<Mutex<Vec<(EngineEventType, Option<i32>, Option<String>)>>>,
}

impl EngineEventListener for RecordingJobEventListener {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        // P53 layer 1+2: skip non-job events (process/task/activity/sequenceflow
        // events are `EngineEvent::Entity` and have no `RuntimeTimerJobState`).
        let job = match event {
            EngineEvent::Job { job, .. } | EngineEvent::JobExecutionFailure { job, .. } => job,
            EngineEvent::Entity { .. } => return Ok(()),
        };
        self.events
            .lock()
            .unwrap()
            .push((event.event_type(), job.retries, job.job_state.clone()));
        Ok(())
    }
}

struct RecordingJobFailureListener {
    failures: Arc<Mutex<Vec<String>>>,
}

impl EngineEventListener for RecordingJobFailureListener {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        if let Some(error) = event.error() {
            self.failures.lock().unwrap().push(format!("{error:?}"));
        }
        Ok(())
    }
}

struct TransactionRecordingJobEventListener {
    state: TransactionState,
    states: Arc<Mutex<Vec<TransactionState>>>,
}

impl EngineEventListener for TransactionRecordingJobEventListener {
    fn on_event(&self, _event: &EngineEvent) -> Result<(), FlowableError> {
        self.states.lock().unwrap().push(self.state);
        Ok(())
    }

    fn is_fire_on_transaction_lifecycle_event(&self) -> bool {
        true
    }

    fn on_transaction(&self) -> TransactionState {
        self.state
    }
}

struct FatalJobEventListener;

impl EngineEventListener for FatalJobEventListener {
    fn on_event(&self, _event: &EngineEvent) -> Result<(), FlowableError> {
        Err(FlowableError::ExecutionError(
            "fatal job event listener".to_string(),
        ))
    }

    fn is_fail_on_exception(&self) -> bool {
        true
    }
}

struct FatalTransactionJobEventListener {
    state: TransactionState,
    states: Arc<Mutex<Vec<TransactionState>>>,
}

impl EngineEventListener for FatalTransactionJobEventListener {
    fn on_event(&self, _event: &EngineEvent) -> Result<(), FlowableError> {
        self.states.lock().unwrap().push(self.state);
        Err(FlowableError::ExecutionError(format!(
            "fatal {:?} job event listener",
            self.state
        )))
    }

    fn is_fail_on_exception(&self) -> bool {
        true
    }

    fn is_fire_on_transaction_lifecycle_event(&self) -> bool {
        true
    }

    fn on_transaction(&self) -> TransactionState {
        self.state
    }
}

#[test]
fn async_service_task_waits_for_job_and_continues_when_worker_executes_it() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 9, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "async-service-task-success".to_string(),
        time_source.clone(),
        Arc::new(DbStore::new_in_memory().unwrap()),
    );

    deploy(
        &engine,
        async_success_process_xml(),
        "async-success.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(jobs.len(), 1, "asyncBefore should create one runtime job");
    assert_eq!(jobs[0].activity_id, "asyncTask");
    assert_eq!(jobs[0].job_state.as_deref(), Some("async"));
    assert_eq!(jobs[0].retries, Some(3));
    assert_eq!(jobs[0].due_time, Some(time_source.now().timestamp_millis()));
    assert!(jobs[0].lock_owner.is_none());
    // Release the read session so the next command can acquire its own
    // transaction.
    drop(session);

    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .is_empty(),
        "process must not pass the async continuation before the job executes"
    );

    let executed = engine.get_runtime_service().run_due_timers().unwrap();
    assert_eq!(executed, vec![jobs[0].timer_job_id.clone()]);

    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_timer_job_state(&jobs[0].timer_job_id, &mut session)
            .is_none(),
        "successful async continuation job should be consumed"
    );
    drop(session);

    let task_keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(task_keys, vec!["afterAsyncTask".to_string()]);
}

#[test]
fn async_continuation_job_category_literal_is_populated() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 9, 10, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "async-job-category-literal".to_string(),
        time_source,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    deploy(
        &engine,
        async_job_category_process_xml("orders", None),
        "async-job-category-literal.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create one continuation job");
    assert_eq!(job.category.as_deref(), Some("orders"));
    assert_eq!(job.job_state.as_deref(), Some("async"));
}

#[test]
fn async_continuation_job_category_expression_is_evaluated() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 9, 15, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "async-job-category-expression".to_string(),
        time_source,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    deploy(
        &engine,
        async_job_category_process_xml("${categoryValue}", None),
        "async-job-category-expression.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("categoryValue".to_string(), json!("orders")),
        )
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create one continuation job");
    assert_eq!(job.category.as_deref(), Some("orders"));
}

#[test]
fn async_job_uses_configured_java_initial_retry_count() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 9, 30, 0).unwrap(),
    ));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.number_of_retries = 5;
    let engine = ProcessEngine::build_with_config(
        "async-configured-initial-retries".to_string(),
        time_source,
        config,
    )
    .unwrap();
    deploy(
        &engine,
        async_success_process_xml(),
        "async-configured-initial-retries.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create one continuation job");
    assert_eq!(job.retries, Some(5));
}

#[test]
fn management_execute_job_runs_and_consumes_async_continuation() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 9, 45, 0).unwrap(),
    ));
    let observed_events = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_event_listener(Arc::new(RecordingJobEventListener {
        events: Arc::clone(&observed_events),
    }));
    let engine = ProcessEngine::build_with_config(
        "manual-async-job-success".to_string(),
        time_source,
        ProcessEngineConfiguration {
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    deploy(
        &engine,
        async_success_process_xml(),
        "manual-async-job-success.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create one continuation job");
    drop(session);

    engine
        .get_management_service()
        .execute_job(&job.timer_job_id)
        .unwrap();
    assert!(
        engine
            .get_management_service()
            .find_job_by_id(&job.timer_job_id)
            .is_none(),
        "successful manual execution must consume the async job"
    );
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterAsyncTask");
    assert_eq!(
        *observed_events.lock().unwrap(),
        vec![(
            EngineEventType::JobExecutionSuccess,
            Some(3),
            Some("async".to_string())
        )],
        "Java ExecuteAsyncJobCmd emits JOB_EXECUTION_SUCCESS after the job body completes"
    );
}

#[test]
fn direct_hint_executes_job_holding_a_valid_executor_row_lock() {
    // A job pre-locked by the live async executor and handed over via a
    // post-commit hint carries the executor's row lock (owner + non-expired
    // expiration). The direct-hint path re-verifies that lock, skips the timer
    // coordinator lease, and runs the job — no fake fencing token involved.
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build_with_config(
        "direct-hint-valid-lock".to_string(),
        Arc::clone(&time_source) as Arc<dyn TimeSource>,
        ProcessEngineConfiguration::default(),
    )
    .unwrap();
    let (_pi_id, mut job) = create_async_continuation_job(&engine, "direct-hint-valid.bpmn20.xml");

    // Simulate the pre-lock the active executor applied inside the activating
    // transaction: this engine's executor owner + a future expiration.
    let owner = engine.get_runtime_service().timer_owner_id().to_string();
    let now_ms = time_source.now().timestamp_millis();
    job.lock_owner = Some(owner.clone());
    job.lock_time = Some(now_ms);
    job.lock_expiration_time = Some(now_ms + 300_000);
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store.insert_timer_job_state(&job, &mut session);
    session.flush_and_commit().unwrap();

    let executed = engine
        .get_runtime_service()
        .execute_timer_work_direct_hint(&TimerWork::RuntimeJob(job.clone()));
    assert_eq!(
        executed.as_deref(),
        Some(job.timer_job_id.as_str()),
        "a validly pre-locked hint must execute and report the job id"
    );
    assert!(
        engine
            .get_management_service()
            .find_job_by_id(&job.timer_job_id)
            .is_none(),
        "successful direct-hint execution must consume the async job"
    );
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(_pi_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterAsyncTask");
}

#[test]
fn direct_hint_skips_job_whose_row_lock_was_reclaimed_by_another_owner() {
    // If the row was re-acquired (or reset-expired) by a different owner after
    // the hint was queued, the direct-hint path must NOT execute it: the row
    // owner/expiration check fails and the job is left for its new owner. This
    // is the guard that prevents double execution.
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 10, 5, 0).unwrap(),
    ));
    let engine = ProcessEngine::build_with_config(
        "direct-hint-stale-lock".to_string(),
        Arc::clone(&time_source) as Arc<dyn TimeSource>,
        ProcessEngineConfiguration::default(),
    )
    .unwrap();
    let (_pi_id, hinted) = create_async_continuation_job(&engine, "direct-hint-stale.bpmn20.xml");

    // The persisted row is now owned by a *different* node than the hint's
    // expected executor owner (which is this engine's timer owner id).
    let now_ms = time_source.now().timestamp_millis();
    let mut persisted = hinted.clone();
    persisted.lock_owner = Some("some-other-node".to_string());
    persisted.lock_time = Some(now_ms);
    persisted.lock_expiration_time = Some(now_ms + 300_000);
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store.insert_timer_job_state(&persisted, &mut session);
    session.flush_and_commit().unwrap();

    // The hint still carries the row as the executor originally pre-locked it,
    // but the persisted owner no longer matches this engine's executor.
    let executed = engine
        .get_runtime_service()
        .execute_timer_work_direct_hint(&TimerWork::RuntimeJob(hinted.clone()));
    assert!(
        executed.is_none(),
        "a hint for a row now owned by another node must not execute"
    );
    // The job is untouched and still owned by the other node — no double run.
    let still_there = engine
        .get_management_service()
        .find_job_by_id(&hinted.timer_job_id)
        .expect("the reclaimed job must remain for its new owner");
    assert_eq!(still_there.lock_owner.as_deref(), Some("some-other-node"));
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(_pi_id)
        .unwrap();
    assert!(
        tasks.is_empty(),
        "the async continuation must not have advanced the process"
    );
}

#[test]
fn direct_hint_skips_job_whose_row_lock_has_expired() {
    // Same guard, expiration branch: the executor still owns the row but the
    // lock has expired, so a reset-expired sweep could hand it to anyone. The
    // direct-hint path declines to execute rather than run on a dead lock.
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 10, 10, 0).unwrap(),
    ));
    let engine = ProcessEngine::build_with_config(
        "direct-hint-expired-lock".to_string(),
        Arc::clone(&time_source) as Arc<dyn TimeSource>,
        ProcessEngineConfiguration::default(),
    )
    .unwrap();
    let (_pi_id, hinted) = create_async_continuation_job(&engine, "direct-hint-expired.bpmn20.xml");

    let owner = engine.get_runtime_service().timer_owner_id().to_string();
    let now_ms = time_source.now().timestamp_millis();
    let mut persisted = hinted.clone();
    persisted.lock_owner = Some(owner);
    persisted.lock_time = Some(now_ms - 600_000);
    // Expiration in the past relative to the engine clock.
    persisted.lock_expiration_time = Some(now_ms - 1);
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store.insert_timer_job_state(&persisted, &mut session);
    session.flush_and_commit().unwrap();

    let executed = engine
        .get_runtime_service()
        .execute_timer_work_direct_hint(&TimerWork::RuntimeJob(hinted.clone()));
    assert!(
        executed.is_none(),
        "a hint whose row lock has expired must not execute"
    );
    assert!(
        engine
            .get_management_service()
            .find_job_by_id(&hinted.timer_job_id)
            .is_some(),
        "the job must remain for a fresh acquisition"
    );
}

#[test]
fn management_delete_job_emits_job_canceled_before_committing_deletion() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 9, 46, 0).unwrap(),
    ));
    let observed_events = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_event_listener(Arc::new(RecordingJobEventListener {
        events: Arc::clone(&observed_events),
    }));
    let engine = ProcessEngine::build_with_config(
        "manual-async-job-canceled".to_string(),
        time_source,
        ProcessEngineConfiguration {
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    let (_process_instance_id, job) =
        create_async_continuation_job(&engine, "manual-async-job-canceled.bpmn20.xml");

    engine
        .get_management_service()
        .delete_job(&job.timer_job_id)
        .unwrap();

    assert!(
        engine
            .get_management_service()
            .find_job_by_id(&job.timer_job_id)
            .is_none()
    );
    assert_eq!(
        *observed_events.lock().unwrap(),
        vec![(
            EngineEventType::JobCanceled,
            Some(3),
            Some("async".to_string())
        )],
        "Java DeleteJobCmd emits JOB_CANCELED with the pre-delete job snapshot"
    );
}

#[test]
fn async_service_task_failure_releases_lock_and_decrements_retries() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap(),
    ));
    let engine = http_enabled_engine("async-service-task-retry", time_source.clone());

    deploy(
        &engine,
        async_failing_http_process_xml("R2/PT1M"),
        "async-failing-retry.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let initial_job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create a job");
    assert_eq!(initial_job.job_state.as_deref(), Some("async"));
    assert_eq!(
        initial_job.retries,
        Some(3),
        "Java creates the job with asyncExecutorNumberOfRetries before applying the BPMN retry cycle"
    );
    // Release the read session before invoking the next command.
    drop(session);

    let executed = engine.get_runtime_service().run_due_timers().unwrap();
    assert!(
        executed.is_empty(),
        "failed async work must not be reported as executed"
    );

    let retried_job = engine
        .get_management_service()
        .find_timer_job_by_id(&initial_job.timer_job_id)
        .expect("Java moves a failed async job to the timer family until its retry is due");
    assert_eq!(retried_job.job_state.as_deref(), Some("timer"));
    assert_eq!(retried_job.retries, Some(1));
    assert_eq!(
        retried_job.due_time,
        Some(time_source.now().timestamp_millis() + 60_000)
    );
    assert!(retried_job.lock_owner.is_none());
    assert!(retried_job.lock_time.is_none());
    assert!(retried_job.lock_expiration_time.is_none());
    assert_eq!(retried_job.execution_id, initial_job.execution_id);
    assert_eq!(retried_job.process_instance_id, process_instance.id);
    assert!(
        retried_job
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("127.0.0.1:9")),
        "retry job should retain the failure message, got {:?}",
        retried_job.error_message
    );
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id(&initial_job.timer_job_id)
            .is_none(),
        "job should not move to deadletter until retries are exhausted"
    );
    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty(),
        "retry delay should keep the failed async job from immediate reacquisition"
    );
}

#[test]
fn async_service_task_failure_moves_to_deadletter_when_retries_are_exhausted() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 0, 0).unwrap(),
    ));
    let engine = http_enabled_engine("async-service-task-deadletter", time_source);

    deploy(
        &engine,
        async_failing_http_process_xml("R1/PT1M"),
        "async-failing-deadletter.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let initial_job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create a job");
    assert_eq!(
        initial_job.retries,
        Some(3),
        "R1 is applied on the first failure, not when the async job is created"
    );
    // Release the read session before invoking the next command.
    drop(session);

    let executed = engine.get_runtime_service().run_due_timers().unwrap();
    assert!(
        executed.is_empty(),
        "failed async work must not be reported as executed"
    );

    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id(&initial_job.timer_job_id)
            .is_none(),
        "exhausted async job should leave the executable set"
    );

    let deadletter = engine
        .get_management_service()
        .find_deadletter_job_by_id(&initial_job.timer_job_id)
        .expect("exhausted async job should be visible as deadletter");
    assert_eq!(deadletter.job_state.as_deref(), Some("deadletter"));
    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(deadletter.execution_id, initial_job.execution_id);
    assert_eq!(deadletter.process_instance_id, process_instance.id);
    assert!(deadletter.lock_owner.is_none());
    assert!(
        deadletter
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("127.0.0.1:9")),
        "deadletter should retain the failure message, got {:?}",
        deadletter.error_message
    );
    assert!(
        deadletter
            .error_details
            .as_deref()
            .is_some_and(|details| details.contains("127.0.0.1:9")),
        "deadletter should retain failure details, got {:?}",
        deadletter.error_details
    );

    let moved = engine
        .get_management_service()
        .move_deadletter_job_to_executable_job(&initial_job.timer_job_id, 1)
        .expect("move async deadletter job back to executable");
    assert_eq!(moved.job_state.as_deref(), Some("async"));
    assert_eq!(moved.retries, Some(1));
}

#[test]
fn async_http_job_retry_replays_one_external_post_per_command_attempt() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 30, 0).unwrap(),
    ));
    let (address, request_count, server) = spawn_counted_http_server(vec![500, 200]);
    let engine = http_job_retry_engine("async-http-side-effect-retry", time_source.clone());
    deploy(
        &engine,
        async_http_retry_process_xml(
            "asyncHttpSideEffectRetry",
            &format!("http://{address}/orders"),
            Some("${retryCycle}"),
            None,
        ),
        "async-http-side-effect-retry.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("retryCycle".to_string(), json!("R2/PT1M")),
        )
        .unwrap();
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        0,
        "creating an async continuation job must not execute the HTTP side effect"
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let initial_job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async HTTP task should create one continuation job");
    assert_eq!(
        initial_job.retries,
        Some(3),
        "the expression-backed retry cycle must not replace the initial Java retry count"
    );
    drop(session);

    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty(),
        "the first 500 response must fail and roll back the command attempt"
    );
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "the first command attempt must issue exactly one POST"
    );
    let retry_job = engine
        .get_management_service()
        .find_timer_job_by_id(&initial_job.timer_job_id)
        .expect("one retry should remain after the first failed attempt");
    assert_eq!(retry_job.retries, Some(1));
    assert_eq!(
        retry_job.due_time,
        Some(time_source.now().timestamp_millis() + 60_000)
    );
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .is_empty(),
        "the failed command attempt must not commit outgoing work"
    );

    time_source.advance_time(60_000);
    assert_eq!(
        engine.get_runtime_service().run_due_timers().unwrap(),
        vec![initial_job.timer_job_id.clone()],
        "the second command attempt should consume the job after HTTP succeeds"
    );
    assert_eq!(server.join().unwrap(), 2);
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id(&initial_job.timer_job_id)
            .is_none()
    );
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id(&initial_job.timer_job_id)
            .is_none()
    );
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterAsyncHttp");
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance.id, "httpResult".to_string())
            .unwrap()
            .unwrap()["response"]["statusCode"],
        json!(200)
    );
}

#[test]
fn exhausted_async_http_job_stops_replaying_external_post_in_deadletter() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 45, 0).unwrap(),
    ));
    let (address, request_count, server) = spawn_counted_http_server(vec![500, 500]);
    let engine = http_job_retry_engine("async-http-side-effect-deadletter", time_source.clone());
    deploy(
        &engine,
        async_http_retry_process_xml(
            "asyncHttpSideEffectDeadletter",
            &format!("http://{address}/orders"),
            Some("R2/PT1M"),
            None,
        ),
        "async-http-side-effect-deadletter.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async HTTP task should create one continuation job");
    drop(session);

    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty()
    );
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    time_source.advance_time(60_000);
    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty()
    );
    assert_eq!(server.join().unwrap(), 2);

    let deadletter = engine
        .get_management_service()
        .find_deadletter_job_by_id(&job.timer_job_id)
        .expect("R2 must move the job to deadletter after the second failure");
    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
    time_source.advance_time(60_000);
    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty(),
        "deadletter jobs must not be reacquired automatically"
    );
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        2,
        "no HTTP side effect may occur after the job reaches deadletter"
    );
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id)
            .unwrap()
            .is_empty(),
        "failed attempts must not commit the outgoing user task"
    );
}

#[test]
fn async_http_job_without_retry_cycle_uses_java_default_failed_job_wait() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 55, 0).unwrap(),
    ));
    let (address, request_count, server) = spawn_counted_http_server(vec![500]);
    let engine = http_job_retry_engine("async-http-default-retry-wait", time_source.clone());
    deploy(
        &engine,
        async_http_retry_process_xml(
            "asyncHttpDefaultRetryWait",
            &format!("http://{address}/orders"),
            None,
            None,
        ),
        "async-http-default-retry-wait.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async HTTP task should create one continuation job");
    assert_eq!(job.retries, Some(3));
    drop(session);

    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty()
    );
    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    let retry_job = engine
        .get_management_service()
        .find_timer_job_by_id(&job.timer_job_id)
        .expect("the default retry policy should move the job to the timer family");
    assert_eq!(retry_job.retries, Some(2));
    assert_eq!(
        retry_job.due_time,
        Some(time_source.now().timestamp_millis() + 10_000),
        "Java asyncFailedJobWaitTime defaults to ten seconds"
    );
    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty(),
        "the failed job must not be reacquired before the default wait expires"
    );
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
}

#[test]
fn unrecoverable_async_http_handler_failure_moves_directly_to_deadletter() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 58, 0).unwrap(),
    ));
    let (address, request_count, server) = spawn_counted_http_server(vec![200]);
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_response_handler(
        "com.example.UnrecoverableResponseHandler",
        Arc::new(UnrecoverableResponseHandler),
    );
    let observed_events = Arc::new(Mutex::new(Vec::new()));
    let observed_failures = Arc::new(Mutex::new(Vec::new()));
    let transaction_states = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_event_listener(Arc::new(RecordingJobEventListener {
        events: Arc::clone(&observed_events),
    }));
    event_dispatcher.add_typed_event_listener(
        EngineEventType::JobExecutionFailure,
        Arc::new(RecordingJobFailureListener {
            failures: Arc::clone(&observed_failures),
        }),
    );
    for state in [
        TransactionState::Committing,
        TransactionState::Committed,
        TransactionState::RollingBack,
        TransactionState::RolledBack,
    ] {
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobRetriesDecremented,
            Arc::new(TransactionRecordingJobEventListener {
                state,
                states: Arc::clone(&transaction_states),
            }),
        );
    }
    let pending_futures = Arc::new(PendingFutureRegistry::new());
    let engine = ProcessEngine::build_with_config(
        "async-http-unrecoverable-handler".to_string(),
        time_source.clone(),
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            http_handler_registry: Some(handlers),
            pending_future_registry: Arc::clone(&pending_futures),
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    deploy(
        &engine,
        async_http_retry_process_xml(
            "asyncHttpUnrecoverableHandler",
            &format!("http://{address}/orders"),
            Some("R5/PT1M"),
            Some("com.example.UnrecoverableResponseHandler"),
        ),
        "async-http-unrecoverable-handler.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async HTTP task should create one continuation job");
    assert_eq!(job.retries, Some(3));
    drop(session);

    let error = engine
        .get_management_service()
        .execute_job(&job.timer_job_id)
        .unwrap_err();
    assert!(matches!(
        error,
        FlowableError::UnrecoverableJobError(ref message)
            if message == "response payload cannot be safely processed"
    ));
    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert!(pending_futures.is_empty());
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id(&job.timer_job_id)
            .is_none()
    );
    let deadletter = engine
        .get_management_service()
        .find_deadletter_job_by_id(&job.timer_job_id)
        .expect("unrecoverable failure must bypass the remaining R5 retry schedule");
    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(
        deadletter.error_message.as_deref(),
        Some("response payload cannot be safely processed")
    );
    assert!(
        deadletter
            .error_details
            .as_deref()
            .is_some_and(|details| details.contains("UnrecoverableJobError"))
    );
    assert!(deadletter.lock_owner.is_none());
    assert!(deadletter.lock_time.is_none());
    assert!(deadletter.lock_expiration_time.is_none());
    assert_eq!(
        *observed_events.lock().unwrap(),
        vec![
            (
                EngineEventType::JobExecutionFailure,
                Some(3),
                Some("async".to_string())
            ),
            (
                EngineEventType::JobMovedToDeadLetter,
                Some(0),
                Some("deadletter".to_string())
            ),
            (
                EngineEventType::EntityUpdated,
                Some(0),
                Some("deadletter".to_string())
            ),
            (
                EngineEventType::JobRetriesDecremented,
                Some(0),
                Some("deadletter".to_string())
            ),
        ],
        "Java JobRetryCmd dispatches ENTITY_UPDATED before JOB_RETRIES_DECREMENTED"
    );
    assert_eq!(
        *observed_failures.lock().unwrap(),
        vec!["UnrecoverableJobError(\"response payload cannot be safely processed\")".to_string()],
        "JOB_EXECUTION_FAILURE must retain the typed Rust error corresponding to Java's exception event"
    );
    assert_eq!(
        *transaction_states.lock().unwrap(),
        vec![TransactionState::Committing, TransactionState::Committed],
        "a successful retry command must not invoke rollback lifecycle listeners"
    );
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id)
            .unwrap()
            .is_empty(),
        "the unrecoverable command attempt must roll back outgoing work"
    );

    time_source.advance_time(60_000);
    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "deadletter state must prevent a second external side effect"
    );
}

#[test]
fn nested_unrecoverable_async_http_handler_failure_preserves_typed_cause_and_deadletters() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 58, 30).unwrap(),
    ));
    let (address, request_count, server) = spawn_counted_http_server(vec![200]);
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_response_handler(
        "com.example.NestedUnrecoverableResponseHandler",
        Arc::new(NestedUnrecoverableResponseHandler),
    );
    let observed_events = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_event_listener(Arc::new(RecordingJobEventListener {
        events: Arc::clone(&observed_events),
    }));
    let pending_futures = Arc::new(PendingFutureRegistry::new());
    let engine = ProcessEngine::build_with_config(
        "async-http-nested-unrecoverable-handler".to_string(),
        time_source.clone(),
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            http_handler_registry: Some(handlers),
            pending_future_registry: Arc::clone(&pending_futures),
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    deploy(
        &engine,
        async_http_retry_process_xml(
            "asyncHttpNestedUnrecoverableHandler",
            &format!("http://{address}/orders"),
            Some("R5/PT1M"),
            Some("com.example.NestedUnrecoverableResponseHandler"),
        ),
        "async-http-nested-unrecoverable-handler.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async HTTP task should create one continuation job");
    assert_eq!(job.retries, Some(3));
    drop(session);

    let error = engine
        .get_management_service()
        .execute_job(&job.timer_job_id)
        .unwrap_err();
    assert_eq!(
        error.raw_primary_message(),
        "response handler wrapper failed"
    );
    assert_eq!(
        error.to_string(),
        "Execution error: response handler wrapper failed"
    );
    assert!(matches!(
        error.primary_error(),
        FlowableError::ExecutionError(message) if message == "response handler wrapper failed"
    ));
    let source = std::error::Error::source(&error)
        .expect("nested unrecoverable error must expose its typed source");
    let source = source
        .downcast_ref::<FlowableError>()
        .expect("nested unrecoverable source must remain a FlowableError");
    assert!(matches!(
        source,
        FlowableError::UnrecoverableJobError(message)
            if message == "response payload cannot be safely processed"
    ));

    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert!(pending_futures.is_empty());
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id(&job.timer_job_id)
            .is_none()
    );
    let deadletter = engine
        .get_management_service()
        .find_deadletter_job_by_id(&job.timer_job_id)
        .expect("nested unrecoverable cause must bypass the remaining R5 retry schedule");
    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(
        deadletter.error_message.as_deref(),
        Some("response handler wrapper failed")
    );
    let error_details = deadletter
        .error_details
        .as_deref()
        .expect("dead-letter details must retain the complete typed cause chain");
    assert!(error_details.contains("ExecutionError"));
    assert!(error_details.contains("response handler wrapper failed"));
    assert!(error_details.contains("UnrecoverableJobError"));
    assert!(error_details.contains("response payload cannot be safely processed"));
    assert!(deadletter.lock_owner.is_none());
    assert!(deadletter.lock_time.is_none());
    assert!(deadletter.lock_expiration_time.is_none());
    assert_eq!(
        *observed_events.lock().unwrap(),
        vec![
            (
                EngineEventType::JobExecutionFailure,
                Some(3),
                Some("async".to_string())
            ),
            (
                EngineEventType::JobMovedToDeadLetter,
                Some(0),
                Some("deadletter".to_string())
            ),
            (
                EngineEventType::EntityUpdated,
                Some(0),
                Some("deadletter".to_string())
            ),
            (
                EngineEventType::JobRetriesDecremented,
                Some(0),
                Some("deadletter".to_string())
            ),
        ]
    );
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id)
            .unwrap()
            .is_empty(),
        "the nested unrecoverable command attempt must roll back outgoing work"
    );

    time_source.advance_time(60_000);
    assert!(
        engine
            .get_runtime_service()
            .run_due_timers()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "dead-letter state must prevent a second external side effect"
    );
}

#[test]
fn fatal_retry_event_listener_rolls_back_job_update_and_fires_rollback_lifecycle() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 59, 0).unwrap(),
    ));
    let (address, request_count, server) = spawn_counted_http_server(vec![200]);
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_response_handler(
        "com.example.UnrecoverableResponseHandler",
        Arc::new(UnrecoverableResponseHandler),
    );
    let transaction_states = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    for state in [
        TransactionState::Committing,
        TransactionState::Committed,
        TransactionState::RollingBack,
        TransactionState::RolledBack,
    ] {
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobRetriesDecremented,
            Arc::new(TransactionRecordingJobEventListener {
                state,
                states: Arc::clone(&transaction_states),
            }),
        );
    }
    event_dispatcher.add_typed_event_listener(
        EngineEventType::JobRetriesDecremented,
        Arc::new(FatalJobEventListener),
    );
    let engine = ProcessEngine::build_with_config(
        "async-http-fatal-retry-listener".to_string(),
        time_source,
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            http_handler_registry: Some(handlers),
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    deploy(
        &engine,
        async_http_retry_process_xml(
            "asyncHttpFatalRetryListener",
            &format!("http://{address}/orders"),
            Some("R5/PT1M"),
            Some("com.example.UnrecoverableResponseHandler"),
        ),
        "async-http-fatal-retry-listener.bpmn20.xml",
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async HTTP task should create one continuation job");
    drop(session);

    let error = engine
        .get_management_service()
        .execute_job(&job.timer_job_id)
        .unwrap_err();
    assert!(matches!(error, FlowableError::UnrecoverableJobError(_)));
    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        *transaction_states.lock().unwrap(),
        vec![TransactionState::RollingBack, TransactionState::RolledBack],
        "fatal immediate listener must roll back the retry command"
    );

    let unchanged_job = engine
        .get_management_service()
        .find_executable_job_by_id(&job.timer_job_id)
        .expect("rolled-back retry command must leave the original job executable");
    assert_eq!(unchanged_job.retries, Some(3));
    assert!(unchanged_job.error_message.is_none());
    assert!(unchanged_job.error_details.is_none());
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id(&job.timer_job_id)
            .is_none()
    );
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fatal_committing_retry_listener_rolls_back_job_update_before_database_commit() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 59, 10).unwrap(),
    ));
    let transaction_states = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    add_retry_transaction_recorders(&mut event_dispatcher, &transaction_states);
    event_dispatcher.add_typed_event_listener(
        EngineEventType::JobRetriesDecremented,
        Arc::new(FatalTransactionJobEventListener {
            state: TransactionState::Committing,
            states: Arc::clone(&transaction_states),
        }),
    );
    let engine = ProcessEngine::build_with_config(
        "fatal-committing-retry-listener".to_string(),
        time_source,
        ProcessEngineConfiguration {
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    let (process_instance_id, job) =
        create_async_continuation_job(&engine, "fatal-committing-retry-listener.bpmn20.xml");

    let simulated_failure = FlowableError::ExecutionError("simulated job failure".to_string());
    let command =
        RecordFailedTimerWorkCmd::new(TimerWork::RuntimeJob(job.clone()), &simulated_failure);
    let error = engine.get_command_executor().execute(&command).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("fatal Committing job event listener")
    );
    assert_eq!(
        *transaction_states.lock().unwrap(),
        vec![
            TransactionState::Committing,
            TransactionState::Committing,
            TransactionState::RollingBack,
            TransactionState::RolledBack,
        ],
        "a fatal COMMITTING listener must enter the real rollback lifecycle"
    );
    let unchanged_job = engine
        .get_management_service()
        .find_executable_job_by_id(&job.timer_job_id)
        .expect("the retry update must be rolled back before commit");
    assert_eq!(unchanged_job.retries, Some(3));
    assert!(unchanged_job.error_message.is_none());
    assert!(unchanged_job.error_details.is_none());
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id(&job.timer_job_id)
            .is_none()
    );
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fatal_committed_retry_listener_returns_error_without_rolling_back_committed_update() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 11, 59, 20).unwrap(),
    ));
    let transaction_states = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    add_retry_transaction_recorders(&mut event_dispatcher, &transaction_states);
    event_dispatcher.add_typed_event_listener(
        EngineEventType::JobRetriesDecremented,
        Arc::new(FatalTransactionJobEventListener {
            state: TransactionState::Committed,
            states: Arc::clone(&transaction_states),
        }),
    );
    let engine = ProcessEngine::build_with_config(
        "fatal-committed-retry-listener".to_string(),
        time_source,
        ProcessEngineConfiguration {
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .unwrap();
    let (_process_instance_id, job) =
        create_async_continuation_job(&engine, "fatal-committed-retry-listener.bpmn20.xml");

    let simulated_failure = FlowableError::ExecutionError("simulated job failure".to_string());
    let command =
        RecordFailedTimerWorkCmd::new(TimerWork::RuntimeJob(job.clone()), &simulated_failure);
    let error = engine.get_command_executor().execute(&command).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("fatal Committed job event listener")
    );
    assert_eq!(
        *transaction_states.lock().unwrap(),
        vec![
            TransactionState::Committing,
            TransactionState::Committed,
            TransactionState::Committed,
            TransactionState::RollingBack,
            TransactionState::RolledBack,
        ],
        "Java emits post-commit rollback lifecycle notifications without undoing committed state"
    );
    let committed_job = engine
        .get_management_service()
        .find_timer_job_by_id(&job.timer_job_id)
        .expect("the retry update must remain committed");
    assert_eq!(committed_job.retries, Some(2));
    assert_eq!(
        committed_job.error_message.as_deref(),
        Some("simulated job failure")
    );
    assert!(
        committed_job
            .error_details
            .as_deref()
            .is_some_and(|details| details.contains("simulated job failure"))
    );
}

#[test]
fn async_after_service_task_defers_outgoing_flows_until_job_executes() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "async-after-service-task".to_string(),
        time_source.clone(),
        Arc::new(DbStore::new_in_memory().unwrap()),
    );

    deploy(&engine, async_after_process_xml(), "async-after.bpmn20.xml");

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    // After the service task executes, the engine should create an async-after
    // job and *not* take the outgoing sequence flow yet.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    let async_after_job = jobs
        .iter()
        .find(|job| job.job_state.as_deref() == Some("async-after"))
        .expect("asyncAfter should create one async-after job");
    assert_eq!(async_after_job.activity_id, "afterTask");
    assert_eq!(
        async_after_job.time_duration.as_deref(),
        Some("__flowable_async_after")
    );
    // Release the read session so the next command can acquire its own
    // transaction.
    drop(session);

    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .is_empty(),
        "process must not reach the user task before the async-after job runs"
    );

    let executed = engine.get_runtime_service().run_due_timers().unwrap();
    assert_eq!(executed, vec![async_after_job.timer_job_id.clone()]);

    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_timer_job_state(&async_after_job.timer_job_id, &mut session)
            .is_none(),
        "successful async-after job should be consumed"
    );
    // Release the read session so the next command can acquire its own
    // transaction.
    drop(session);

    let task_keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(
        task_keys,
        vec!["afterUserTask".to_string()],
        "async-after job should advance the process to the next activity"
    );
}

fn add_retry_transaction_recorders(
    event_dispatcher: &mut EngineEventDispatcher,
    transaction_states: &Arc<Mutex<Vec<TransactionState>>>,
) {
    for state in [
        TransactionState::Committing,
        TransactionState::Committed,
        TransactionState::RollingBack,
        TransactionState::RolledBack,
    ] {
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobRetriesDecremented,
            Arc::new(TransactionRecordingJobEventListener {
                state,
                states: Arc::clone(transaction_states),
            }),
        );
    }
}

fn create_async_continuation_job(
    engine: &ProcessEngine,
    resource_name: &str,
) -> (
    String,
    flowable_engine::persistence::runtime_store::RuntimeTimerJobState,
) {
    deploy(engine, async_success_process_xml(), resource_name);
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .next()
        .expect("async service task should create one continuation job");
    (process_instance.id, job)
}

fn deploy(engine: &ProcessEngine, xml: String, resource_name: &str) {
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(resource_name.to_string())
                .add_string(resource_name.to_string(), xml),
        )
        .unwrap();
}

fn http_enabled_engine(name: &str, time_source: Arc<TestTimeSource>) -> ProcessEngine {
    ProcessEngine::build_with_config(
        name.to_string(),
        time_source,
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .expect("HTTP-enabled engine should build")
}

fn http_job_retry_engine(name: &str, time_source: Arc<TestTimeSource>) -> ProcessEngine {
    ProcessEngine::build_with_config(
        name.to_string(),
        time_source,
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    // Isolate engine job retry from the additive transport retry
                    // extension. Java's equivalent fixture sets requestRetryLimit=0.
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .expect("HTTP job-retry engine should build")
}

fn spawn_counted_http_server(
    statuses: Vec<u16>,
) -> (SocketAddr, Arc<AtomicUsize>, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let server_request_count = Arc::clone(&request_count);
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut served = 0;
        while served < statuses.len() && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request).unwrap();
                    server_request_count.fetch_add(1, Ordering::SeqCst);
                    let status = statuses[served];
                    let reason = if status == 200 {
                        "OK"
                    } else {
                        "Internal Server Error"
                    };
                    let body = format!(r#"{{"attempt":{},"status":{status}}}"#, served + 1);
                    write!(
                        stream,
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    served += 1;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("counted HTTP server failed: {error}"),
            }
        }
        served
    });
    (address, request_count, server)
}

fn async_success_process_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="asyncSuccessProcess">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncTask" />
        <serviceTask id="asyncTask" flowable:async="true" />
        <sequenceFlow id="flow2" sourceRef="asyncTask" targetRef="afterAsyncTask" />
        <userTask id="afterAsyncTask" name="After Async" />
    </process>
</definitions>"#
        .to_string()
}

fn async_job_category_process_xml(category_text: &str, process_category: Option<&str>) -> String {
    let process_extension = process_category
        .map(|category| {
            format!(
                r#"
        <extensionElements>
            <flowable:jobCategory>{category}</flowable:jobCategory>
        </extensionElements>"#
            )
        })
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="asyncJobCategoryProcess" isExecutable="true">{process_extension}
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncTask" />
        <serviceTask id="asyncTask" flowable:async="true">
            <extensionElements>
                <flowable:jobCategory>{category_text}</flowable:jobCategory>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="asyncTask" targetRef="afterAsyncTask" />
        <userTask id="afterAsyncTask" name="After Async" />
    </process>
</definitions>"#
    )
}

fn async_failing_http_process_xml(retry_cycle: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="asyncFailingHttpProcess">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncHttpTask" />
        <serviceTask id="asyncHttpTask" flowable:async="true" flowable:type="http">
            <extensionElements>
                <flowable:failedJobRetryTimeCycle>{retry_cycle}</flowable:failedJobRetryTimeCycle>
                <flowable:requestMethod>GET</flowable:requestMethod>
                <flowable:requestUrl>http://127.0.0.1:9/unavailable</flowable:requestUrl>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="asyncHttpTask" targetRef="afterAsyncTask" />
        <userTask id="afterAsyncTask" name="After Async" />
    </process>
</definitions>"#
    )
}

fn async_http_retry_process_xml(
    process_id: &str,
    request_url: &str,
    retry_cycle: Option<&str>,
    response_handler_class: Option<&str>,
) -> String {
    let retry_extension = retry_cycle
        .map(|cycle| {
            format!("<flowable:failedJobRetryTimeCycle>{cycle}</flowable:failedJobRetryTimeCycle>")
        })
        .unwrap_or_default();
    let response_handler = response_handler_class
        .map(|class| format!(r#"<flowable:httpResponseHandler class="{class}" />"#))
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="{process_id}">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncHttpTask" />
        <serviceTask id="asyncHttpTask" flowable:async="true" flowable:type="http"
                     flowable:resultVariableName="httpResult">
            <extensionElements>
                {retry_extension}
                <flowable:field name="requestMethod"><flowable:string>POST</flowable:string></flowable:field>
                <flowable:field name="requestUrl"><flowable:string>{request_url}</flowable:string></flowable:field>
                <flowable:field name="requestBody"><flowable:string>{{"orderId":42}}</flowable:string></flowable:field>
                <flowable:field name="failStatusCodes"><flowable:string>500</flowable:string></flowable:field>
                <flowable:field name="parallelInSameTransaction"><flowable:string>true</flowable:string></flowable:field>
                {response_handler}
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="asyncHttpTask" targetRef="afterAsyncHttp" />
        <userTask id="afterAsyncHttp" name="After Async HTTP" />
    </process>
</definitions>"#
    )
}

fn async_after_process_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="asyncAfterProcess">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="afterTask" />
        <serviceTask id="afterTask" name="Work" flowable:asyncLeave="true" />
        <sequenceFlow id="flow2" sourceRef="afterTask" targetRef="afterUserTask" />
        <userTask id="afterUserTask" name="After Async Leave" />
    </process>
</definitions>"#
        .to_string()
}
