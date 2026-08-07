use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;

fn deploy_simple_process(engine: &ProcessEngine) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="simpleProcess" name="Simple Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="User Task" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repo = engine.get_repository_service();
    let builder = repo
        .create_deployment()
        .name("test-deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), xml.to_string());
    repo.deploy(builder).unwrap();
    repo.get_process_definition_ids().unwrap()[0].clone()
}

#[test]
fn sync_mode_writes_history_immediately() {
    let engine = ProcessEngine::new("sync_test".to_string());
    let pd_id = deploy_simple_process(&engine);

    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let hist_pis = store.list_historic_process_instances(&mut session);
    session.rollback().unwrap();
    assert!(
        !hist_pis.is_empty(),
        "sync mode should write history immediately"
    );
    assert_eq!(hist_pis[0].id, pi.id);
}

#[test]
fn async_mode_buffers_history_on_start() {
    let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;

    let engine =
        ProcessEngine::build_with_config("async_buffer_test".to_string(), time_source, config)
            .unwrap();

    let pd_id = deploy_simple_process(&engine);

    let _pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();

    // No history should be written synchronously
    let hist_pis = store.list_historic_process_instances(&mut session);
    assert!(
        hist_pis.is_empty(),
        "async mode should not write history synchronously"
    );

    // But a history job should have been created by flush_history
    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .collect();
    session.rollback().unwrap();
    assert_eq!(jobs.len(), 1, "flush_history should create one history job");
    // Java asyncHistoryExecutorNumberOfRetries default is 10.
    assert_eq!(jobs[0].retries, Some(10));
    assert!(jobs[0].time_duration.is_some());
    assert!(
        jobs[0].advanced_job_handler_configuration.is_some(),
        "history job should store advanced handler configuration"
    );
}

#[test]
fn async_mode_replays_via_execute_history_job() {
    let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;

    let engine =
        ProcessEngine::build_with_config("async_replay_test".to_string(), time_source, config)
            .unwrap();

    let pd_id = deploy_simple_process(&engine);

    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();

    // Find the history job
    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .collect();
    session.rollback().unwrap();
    assert_eq!(jobs.len(), 1);

    // Execute via management service — triggers AsyncHistoryJobHandler
    engine
        .get_management_service()
        .execute_history_job(&jobs[0].timer_job_id)
        .unwrap();

    // After replay, history should be present
    let mut session2 = store.create_session().unwrap();
    let hist_pis = store.list_historic_process_instances(&mut session2);
    assert_eq!(hist_pis.len(), 1, "replay should create history records");
    assert_eq!(hist_pis[0].id, pi.id);

    // The history job should be deleted after successful replay
    assert!(
        store
            .find_timer_job_state(&jobs[0].timer_job_id, &mut session2)
            .is_none(),
        "history job should be deleted after replay"
    );
    session2.rollback().unwrap();
}

#[test]
fn async_history_replay_preserves_process_start_user() {
    let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;

    let engine = ProcessEngine::build_with_config(
        "async-history-start-user".to_string(),
        time_source,
        config,
    )
    .unwrap();
    let process_definition_id = deploy_simple_process(&engine);
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .start_user_id("admin".to_string()),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|job| job.job_state.as_deref() == Some("history"))
        .collect();
    assert_eq!(jobs.len(), 1);
    let payload = jobs[0]
        .time_duration
        .as_deref()
        .expect("history job should have a serialized payload");
    assert!(payload.contains("\"start_user_id\":\"admin\""));
    session.rollback().unwrap();

    engine
        .get_management_service()
        .execute_history_job(&jobs[0].timer_job_id)
        .unwrap();

    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_process_instance(&process_instance.id, &mut session)
        .expect("async replay should create historic process instance");
    assert_eq!(historic.start_user_id.as_deref(), Some("admin"));
    session.rollback().unwrap();
}

#[test]
fn async_history_start_payload_without_user_remains_backward_compatible() {
    use flowable_engine::history::async_history_job_handler::{HistoryJobBatch, HistoryJobPayload};

    let batch: HistoryJobBatch = serde_json::from_str(
        r#"{
            "operations": [{
                "ProcessInstanceStart": {
                    "process_instance_id": "pi-legacy",
                    "process_definition_id": "pd-legacy",
                    "business_key": null,
                    "start_time": "2026-06-30T12:00:00Z"
                }
            }]
        }"#,
    )
    .unwrap();

    assert!(matches!(
        &batch.operations[0],
        HistoryJobPayload::ProcessInstanceStart {
            start_user_id: None,
            ..
        }
    ));
}

/// P71: pre-delete_reason ActivityEnd history-job payloads still deserialize.
#[test]
fn async_history_activity_end_payload_without_delete_reason_is_backward_compatible() {
    use flowable_engine::history::async_history_job_handler::{HistoryJobBatch, HistoryJobPayload};

    let batch: HistoryJobBatch = serde_json::from_str(
        r#"{
            "operations": [{
                "ActivityEnd": {
                    "execution_id": "exec-1",
                    "activity_id": "catchTimer",
                    "end_time": "2026-06-30T12:00:00Z"
                }
            }]
        }"#,
    )
    .unwrap();

    assert!(matches!(
        &batch.operations[0],
        HistoryJobPayload::ActivityEnd {
            delete_reason: None,
            ..
        }
    ));
}

/// P71: ActivityEnd carries delete_reason through the async job payload.
#[test]
fn async_history_activity_end_payload_roundtrips_delete_reason() {
    use flowable_engine::history::async_history_job_handler::{HistoryJobBatch, HistoryJobPayload};
    use flowable_engine::history::delete_reason::EVENT_BASED_GATEWAY_CANCEL;

    let batch = HistoryJobBatch {
        operations: vec![HistoryJobPayload::ActivityEnd {
            execution_id: "exec-1".to_string(),
            activity_id: "catchTimer".to_string(),
            end_time: Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap(),
            delete_reason: Some(EVENT_BASED_GATEWAY_CANCEL.to_string()),
        }],
    };
    let json = serde_json::to_string(&batch).unwrap();
    let loaded: HistoryJobBatch = serde_json::from_str(&json).unwrap();
    match &loaded.operations[0] {
        HistoryJobPayload::ActivityEnd {
            delete_reason: Some(reason),
            ..
        } => assert_eq!(reason, EVENT_BASED_GATEWAY_CANCEL),
        other => panic!("expected ActivityEnd with delete_reason, got {other:?}"),
    }
}

#[test]
fn async_mode_replay_produces_task_and_log_entry() {
    let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;

    let engine =
        ProcessEngine::build_with_config("async_task_log_test".to_string(), time_source, config)
            .unwrap();

    let pd_id = deploy_simple_process(&engine);
    let store = engine.get_runtime_store();

    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let mut session = store.create_session().unwrap();
    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .collect();
    session.rollback().unwrap();
    assert_eq!(jobs.len(), 1);

    engine
        .get_management_service()
        .execute_history_job(&jobs[0].timer_job_id)
        .unwrap();

    // Verify task exists
    let tasks_after = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1, "user task should exist after replay");
    assert_eq!(tasks_after[0].name, "User Task".to_string());

    // D3: next_historic_task_log_number should advance from 0 to >0
    let mut session2 = store.create_session().unwrap();
    let log_number = store.next_historic_task_log_number(&mut session2);
    session2.rollback().unwrap();
    assert!(
        log_number > 0,
        "task log entry should increment log number after replay"
    );
}

#[test]
fn async_history_replays_task_metadata_claim_and_reclaim_in_command_order() {
    // Java parity: AsyncHistoryManager records task info changes after creation,
    // including ClaimTaskCmd.java:52/62 re-claims, rather than relying on the
    // original TaskCreated snapshot.
    let now = Utc.with_ymd_and_hms(2026, 7, 28, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;
    let engine = ProcessEngine::build_with_config(
        "async-history-p34-task-updates".to_string(),
        time_source,
        config,
    )
    .unwrap();

    let process_definition_id = deploy_simple_process(&engine);
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();
    let store = engine.get_runtime_store();
    let pending_job_ids = || {
        let mut session = store.create_session().unwrap();
        let ids = store
            .snapshot_timer_job_states(&mut session)
            .into_values()
            .filter(|job| job.job_state.as_deref() == Some("history"))
            .map(|job| job.timer_job_id)
            .collect::<std::collections::HashSet<_>>();
        session.rollback().unwrap();
        ids
    };
    let take_new_job = |before: &std::collections::HashSet<String>| {
        let after = pending_job_ids();
        let new_jobs = after.difference(before).cloned().collect::<Vec<_>>();
        assert_eq!(
            new_jobs.len(),
            1,
            "each command should create one history job"
        );
        (after, new_jobs[0].clone())
    };

    let mut pending = pending_job_ids();
    assert_eq!(pending.len(), 1);
    let start_job = pending.iter().next().unwrap().clone();
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let task_id = task.id.clone();

    engine
        .get_task_service()
        .update_task_by_id(
            task_id.clone(),
            flowable_engine::engine::task_service::TaskUpdate {
                description: Some(Some("async description".to_string())),
                tenant_id: Some(Some("123".to_string())),
                category: Some(Some("456".to_string())),
                form_key: Some(Some("789".to_string())),
                parent_task_id: Some(Some("101112".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
    let (after_update, update_job) = take_new_job(&pending);
    pending = after_update;

    engine
        .get_task_service()
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    let (after_claim, claim_job) = take_new_job(&pending);
    pending = after_claim;
    let first_claim_time = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()[0]
        .claim_time
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(2));
    engine
        .get_task_service()
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    let (after_reclaim, reclaim_job) = take_new_job(&pending);
    pending = after_reclaim;
    let second_claim_time = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()[0]
        .claim_time
        .unwrap();
    assert!(second_claim_time > first_claim_time);

    engine
        .get_task_service()
        .complete_task_by_id(task_id.clone())
        .unwrap();
    let (_after_complete, complete_job) = take_new_job(&pending);

    for job_id in [start_job, update_job, claim_job, reclaim_job, complete_job] {
        engine
            .get_management_service()
            .execute_history_job(&job_id)
            .unwrap();
    }

    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_task_instance(&task_id, &mut session)
        .unwrap();
    session.rollback().unwrap();
    assert_eq!(historic.description.as_deref(), Some("async description"));
    assert_eq!(historic.tenant_id.as_deref(), Some("123"));
    assert_eq!(historic.category.as_deref(), Some("456"));
    assert_eq!(historic.form_key.as_deref(), Some("789"));
    assert_eq!(historic.parent_task_id.as_deref(), Some("101112"));
    assert_eq!(historic.assignee.as_deref(), Some("kermit"));
    assert_eq!(historic.claim_time, Some(second_claim_time));
    assert!(historic.end_time.is_some());
    assert!(
        historic
            .end_time
            .zip(historic.claim_time)
            .is_some_and(|(end, claim)| end >= claim)
    );
}

#[test]
fn historic_task_text_projections_preserve_numeric_strings_and_clear_nulls() {
    let engine = ProcessEngine::new("p34-historic-task-typed-projections".to_string());
    let process_definition_id = deploy_simple_process(&engine);
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();
    let task_id = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap()[0]
        .id
        .clone();

    engine
        .get_task_service()
        .update_task_by_id(
            task_id.clone(),
            flowable_engine::engine::task_service::TaskUpdate {
                tenant_id: Some(Some("123".to_string())),
                category: Some(Some("456".to_string())),
                form_key: Some(Some("789".to_string())),
                parent_task_id: Some(Some("101112".to_string())),
                ..Default::default()
            },
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let load_projection = || {
        let mut session = store.create_session().unwrap();
        let mut params = flowable_engine::persistence::db_session::DbParams::new();
        params.push(task_id.as_str());
        let row = session
            .raw_query_one(
                "SELECT tenant_id, category, form_key, parent_task_id FROM historic_task_instances WHERE id = ?",
                params,
            )
            .unwrap()
            .unwrap();
        session.rollback().unwrap();
        [
            row.get_text("tenant_id"),
            row.get_text("category"),
            row.get_text("form_key"),
            row.get_text("parent_task_id"),
        ]
    };
    assert_eq!(
        load_projection(),
        [
            Some("123".to_string()),
            Some("456".to_string()),
            Some("789".to_string()),
            Some("101112".to_string()),
        ]
    );

    engine
        .get_task_service()
        .update_task_by_id(
            task_id.clone(),
            flowable_engine::engine::task_service::TaskUpdate {
                tenant_id: Some(None),
                category: Some(None),
                form_key: Some(None),
                parent_task_id: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(load_projection(), [None, None, None, None]);
}

#[test]
fn async_history_job_deserialization_error_returns_error() {
    use flowable_engine::history::async_history_job_handler::{
        AsyncHistoryJobHandler, HistoryJobHandler,
    };
    use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;

    let db = Arc::new(DbStore::new_in_memory().unwrap());
    let store =
        flowable_engine::persistence::runtime_store::RuntimeStore::new_with_memory_backend_for_test(
            db.clone(),
        );
    let session = store.create_session().unwrap();
    let mut ctx = flowable_engine::interceptor::command_context::CommandContext::new(
        flowable_engine::engine::deployment_manager::DeploymentManager::new_with_memory_backend_for_test(db),
        store.clone(),
        session,
        Arc::new(ProcessEngineConfiguration::default()),
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let job = RuntimeTimerJobState {
        timer_job_id: "bad-job".to_string(),
        process_instance_id: String::new(),
        execution_id: String::new(),
        activity_id: "async-history".to_string(),
        job_state: Some("history".to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: Some("invalid-json".to_string()),
        time_date: None,
        time_cycle: None,
        end_date: None,
        due_time: Some(1_777_593_600_000i64),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(3),
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    };

    let handler = AsyncHistoryJobHandler;
    let result = handler.execute(&job, &mut ctx);
    assert!(result.is_err(), "invalid JSON should cause error");
}

#[test]
fn async_mode_disabled_default_config() {
    let config = ProcessEngineConfiguration::default();
    assert!(!config.async_history.enabled);
    assert_eq!(config.async_history.handler_type, "default-history");
}

#[test]
fn async_mode_empty_buffer_does_nothing() {
    use flowable_engine::history::history_manager::HistoryManager;
    use flowable_engine::persistence::runtime_store::RuntimeStore;

    let store =
        RuntimeStore::new_with_memory_backend_for_test(Arc::new(DbStore::new_in_memory().unwrap()));
    let manager = HistoryManager::new(store.clone(), true);
    let mut session = store.create_session().unwrap();

    // No record_* calls were made, so buffer is empty
    manager.flush_history(&mut session);

    // No history job should be created
    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .collect();
    session.rollback().unwrap();
    assert!(
        jobs.is_empty(),
        "empty buffer should not create history jobs"
    );
}

#[test]
fn async_mode_single_record_creates_one_history_job() {
    use flowable_engine::history::history_manager::HistoryManager;
    use flowable_engine::persistence::runtime_store::RuntimeStore;

    let db = Arc::new(DbStore::new_in_memory().unwrap());
    let store = RuntimeStore::new_with_memory_backend_for_test(db.clone());
    let manager = HistoryManager::new(store.clone(), true);
    let mut session = store.create_session().unwrap();

    manager.record_audit_event("test-event", Some("pi-1"), None, None, &mut session);

    manager.flush_history(&mut session);

    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .collect();
    session.rollback().unwrap();
    assert_eq!(jobs.len(), 1, "one record -> one history job");
    assert!(jobs[0].time_duration.is_some());

    // Verify the batch has one operation
    let batch: flowable_engine::history::async_history_job_handler::HistoryJobBatch =
        serde_json::from_str(jobs[0].time_duration.as_deref().unwrap()).unwrap();
    assert_eq!(batch.operations.len(), 1);
}

#[test]
fn retry_deadletter_on_invalid_payload() {
    use flowable_engine::history::async_history_job_handler::{
        AsyncHistoryJobHandler, HistoryJobHandler,
    };
    use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;

    let db = Arc::new(DbStore::new_in_memory().unwrap());
    let store =
        flowable_engine::persistence::runtime_store::RuntimeStore::new_with_memory_backend_for_test(
            db.clone(),
        );
    let session = store.create_session().unwrap();
    let mut ctx = flowable_engine::interceptor::command_context::CommandContext::new(
        flowable_engine::engine::deployment_manager::DeploymentManager::new_with_memory_backend_for_test(db),
        store.clone(),
        session,
        Arc::new(ProcessEngineConfiguration::default()),
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let job = RuntimeTimerJobState {
        timer_job_id: "retry-job".to_string(),
        process_instance_id: String::new(),
        execution_id: String::new(),
        activity_id: "async-history".to_string(),
        job_state: Some("history".to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: Some("invalid-json".to_string()),
        time_date: None,
        time_cycle: None,
        end_date: None,
        due_time: Some(1_777_593_600_000i64),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(3),
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    };

    // Insert the job first
    {
        let (s, sess) = ctx.store_and_session();
        s.insert_timer_job_state(&job, sess);
    }

    let handler = AsyncHistoryJobHandler;
    let result = handler.execute(&job, &mut ctx);
    assert!(result.is_err());

    // Commit the changes made by handle_failure
    ctx.session().flush_and_commit().unwrap();
    drop(ctx);

    // Job should still exist but with decremented retries
    let mut session2 = store.create_session().unwrap();
    let updated = store
        .find_timer_job_state("retry-job", &mut session2)
        .unwrap();
    session2.rollback().unwrap();
    assert_eq!(updated.retries, Some(2));
    assert!(updated.due_time.is_some());
    assert!(updated.error_message.is_some());
}

#[test]
fn retry_exhausted_moves_to_deadletter() {
    use flowable_engine::history::async_history_job_handler::{
        AsyncHistoryJobHandler, HistoryJobHandler,
    };
    use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;

    let db = Arc::new(DbStore::new_in_memory().unwrap());
    let store =
        flowable_engine::persistence::runtime_store::RuntimeStore::new_with_memory_backend_for_test(
            db.clone(),
        );
    let session = store.create_session().unwrap();
    let mut ctx = flowable_engine::interceptor::command_context::CommandContext::new(
        flowable_engine::engine::deployment_manager::DeploymentManager::new_with_memory_backend_for_test(db),
        store.clone(),
        session,
        Arc::new(ProcessEngineConfiguration::default()),
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let job = RuntimeTimerJobState {
        timer_job_id: "deadletter-job".to_string(),
        process_instance_id: String::new(),
        execution_id: String::new(),
        activity_id: "async-history".to_string(),
        job_state: Some("history".to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: Some("invalid-json".to_string()),
        time_date: None,
        time_cycle: None,
        end_date: None,
        due_time: Some(1_777_593_600_000i64),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(1), // Last retry
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    };

    // Insert the job first
    {
        let (s, sess) = ctx.store_and_session();
        s.insert_timer_job_state(&job, sess);
    }

    let handler = AsyncHistoryJobHandler;
    let result = handler.execute(&job, &mut ctx);
    assert!(result.is_err());

    // Commit the changes made by handle_failure
    ctx.session().flush_and_commit().unwrap();
    drop(ctx);

    // Job should now be in deadletter
    let mut session2 = store.create_session().unwrap();
    let updated = store
        .find_timer_job_state("deadletter-job", &mut session2)
        .unwrap();
    session2.rollback().unwrap();
    assert_eq!(updated.retries, Some(0));
    assert_eq!(updated.job_state.as_deref(), Some("deadletter"));
    assert!(updated.error_message.is_some());
}

#[test]
fn successful_replay_deletes_job() {
    let now = Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;

    let engine =
        ProcessEngine::build_with_config("replay_delete_test".to_string(), time_source, config)
            .unwrap();

    let pd_id = deploy_simple_process(&engine);
    let store = engine.get_runtime_store();

    let _pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let mut session = store.create_session().unwrap();
    let jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .collect();
    session.rollback().unwrap();
    assert_eq!(jobs.len(), 1);

    engine
        .get_management_service()
        .execute_history_job(&jobs[0].timer_job_id)
        .unwrap();

    // After successful replay, history job should be deleted
    let mut session2 = store.create_session().unwrap();
    assert!(
        store
            .find_timer_job_state(&jobs[0].timer_job_id, &mut session2)
            .is_none(),
        "history job should be deleted after successful replay"
    );

    // But history data should exist
    let hist_pis = store.list_historic_process_instances(&mut session2);
    session2.rollback().unwrap();
    assert!(!hist_pis.is_empty());
}

#[test]
fn async_and_history_acquisition_are_isolated() {
    use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
    use std::collections::HashSet;

    let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let engine = ProcessEngine::build(
        "async-history-acquisition-isolation".to_string(),
        time_source.clone(),
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let store = engine.get_runtime_store();
    let due_time = time_source.now().timestamp_millis();
    let mut session = store.create_session().unwrap();

    for (timer_job_id, job_state) in [
        ("async-job", "async"),
        ("async-after-job", "async-after"),
        ("history-job", "history"),
    ] {
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: timer_job_id.to_string(),
                process_instance_id: "process-instance".to_string(),
                execution_id: "execution".to_string(),
                activity_id: "activity".to_string(),
                job_state: Some(job_state.to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: Some(due_time),
                lock_owner: None,
                lock_time: None,
                lock_expiration_time: None,
                retries: Some(3),
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    let runtime_service = engine.get_runtime_service();
    let async_jobs = runtime_service.acquire_async_jobs(300_000, 10);
    let history_jobs = runtime_service.acquire_history_jobs(300_000, 10);

    assert!(
        async_jobs
            .iter()
            .all(|job| matches!(job.job_state.as_deref(), Some("async" | "async-after")))
    );
    assert!(
        history_jobs
            .iter()
            .all(|job| job.job_state.as_deref() == Some("history"))
    );

    let async_ids = async_jobs
        .iter()
        .map(|job| job.timer_job_id.as_str())
        .collect::<HashSet<_>>();
    let history_ids = history_jobs
        .iter()
        .map(|job| job.timer_job_id.as_str())
        .collect::<HashSet<_>>();
    assert!(async_ids.is_disjoint(&history_ids));
    assert_eq!(async_ids, HashSet::from(["async-job", "async-after-job"]));
    assert_eq!(history_ids, HashSet::from(["history-job"]));
}

#[test]
fn independent_history_executor_lifecycle() {
    let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;
    config.async_history.use_shared_executor = false;
    config.async_history.acquire_interval_ms = 50;

    let engine = ProcessEngine::build_with_config(
        "independent-history-exec-lifecycle".to_string(),
        time_source,
        config,
    )
    .unwrap();

    // With use_shared_executor=false, async_history_executor should exist
    assert!(
        engine.get_async_history_executor().is_some(),
        "AsyncHistoryExecutor should be created when use_shared_executor=false"
    );

    let hist_exec = engine.get_async_history_executor().unwrap();
    assert!(
        !hist_exec.is_active(),
        "executor should not be active before start"
    );

    // Start the executor
    engine.start_timer_executor();
    assert!(
        hist_exec.is_active(),
        "executor should be active after start"
    );

    // Stop the executor
    engine.stop_timer_executor();
    assert!(
        !hist_exec.is_active(),
        "executor should not be active after shutdown"
    );
}

#[test]
fn shared_executor_mode_does_not_create_independent_executor() {
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;
    config.async_history.use_shared_executor = true;

    let engine = ProcessEngine::build_with_config(
        "shared-exec-mode".to_string(),
        Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap(),
        )),
        config,
    )
    .unwrap();

    assert!(
        engine.get_async_history_executor().is_none(),
        "AsyncHistoryExecutor should not be created when use_shared_executor=true"
    );
}

#[test]
fn independent_history_executor_acquires_only_history_jobs() {
    use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;

    let now = Utc.with_ymd_and_hms(2026, 7, 14, 14, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;
    config.async_history.use_shared_executor = false;
    config.async_history.acquire_interval_ms = 50;
    // Keep main async executor off so only the independent history executor runs.
    config.async_executor.enabled = false;

    let engine = ProcessEngine::build_with_config(
        "independent-history-only-acquires-history".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();

    let store = engine.get_runtime_store();
    let due_time = time_source.now().timestamp_millis();
    let mut session = store.create_session().unwrap();

    for (timer_job_id, job_state) in [
        ("job-async", "async"),
        ("job-async-after", "async-after"),
        ("job-history", "history"),
    ] {
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: timer_job_id.to_string(),
                process_instance_id: "process-instance".to_string(),
                execution_id: "execution".to_string(),
                activity_id: "activity".to_string(),
                job_state: Some(job_state.to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: Some(due_time),
                lock_owner: None,
                lock_time: None,
                lock_expiration_time: None,
                retries: Some(3),
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    let hist_exec = engine
        .get_async_history_executor()
        .expect("independent history executor must exist");
    hist_exec.start(engine.get_runtime_service());

    // Wait for at least one acquisition cycle.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut session = store.create_session().unwrap();
    let history_job = store.find_timer_job_state("job-history", &mut session);
    let async_job = store.find_timer_job_state("job-async", &mut session);
    let async_after_job = store.find_timer_job_state("job-async-after", &mut session);
    session.rollback().unwrap();

    hist_exec.shutdown();

    // History job should have been acquired (locked or already executed/removed).
    // If still present, it must have been locked by the history executor.
    if let Some(hj) = &history_job {
        assert!(
            hj.lock_owner.is_some() || hj.job_state.as_deref() != Some("history"),
            "history job should be locked or processed by independent history executor, got: {hj:?}"
        );
    }
    // Async jobs must not be touched by the independent history executor.
    let async_job = async_job.expect("async job must still exist");
    assert!(
        async_job.lock_owner.is_none(),
        "async job must not be locked by independent history executor"
    );
    let async_after_job = async_after_job.expect("async-after job must still exist");
    assert!(
        async_after_job.lock_owner.is_none(),
        "async-after job must not be locked by independent history executor"
    );
}

#[test]
fn independent_history_executor_resets_expired_history_locks() {
    use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
    use std::time::{Duration, Instant};

    let now = Utc.with_ymd_and_hms(2026, 7, 19, 12, 0, 0).unwrap();
    // Independent history reset owns History locks only.
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;
    config.async_history.use_shared_executor = false;
    // Keep acquisition polling fast so shutdown is not blocked on a long sleep.
    config.async_history.acquire_interval_ms = 50;
    config.async_history.reset_expired_job_enabled = Some(true);
    config.async_history.reset_expired_jobs_interval_ms = Some(30);
    config.async_history.reset_expired_jobs_page_size = Some(10);
    // No main async executor — history owns its own reset lifecycle.
    config.async_executor.enabled = false;

    let engine = ProcessEngine::build_with_config(
        "independent-history-reset-expired".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();
    let store = engine.get_runtime_store();
    let now_ms = time_source.now().timestamp_millis();

    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "expired-history".to_string(),
            process_instance_id: "pi-history".to_string(),
            execution_id: "ex-history".to_string(),
            activity_id: "history".to_string(),
            job_state: Some("history".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now_ms - 2_000),
            lock_owner: Some("dead-history-owner".to_string()),
            lock_time: Some(now_ms - 2_000),
            lock_expiration_time: Some(now_ms - 1_000),
            retries: Some(0),
            error_message: None,
            error_details: None,
            // Category must be ignored for history reset.
            category: Some("should-be-ignored".to_string()),
            ..Default::default()
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "expired-async".to_string(),
            process_instance_id: "pi-async".to_string(),
            execution_id: "ex-async".to_string(),
            activity_id: "async".to_string(),
            job_state: Some("async".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now_ms - 2_000),
            lock_owner: Some("dead-async-owner".to_string()),
            lock_time: Some(now_ms - 2_000),
            lock_expiration_time: Some(now_ms - 1_000),
            retries: Some(2),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let hist_exec = engine
        .get_async_history_executor()
        .expect("independent history executor must exist")
        .clone();
    hist_exec.start(engine.get_runtime_service());

    let unlocked = wait_until(Duration::from_secs(3), || {
        let mut session = store.create_session().unwrap();
        let job = store
            .find_timer_job_state("expired-history", &mut session)
            .unwrap();
        session.rollback().unwrap();
        job.lock_owner.is_none()
    });
    assert!(
        unlocked,
        "history reset worker must clear expired history locks"
    );

    {
        let mut session = store.create_session().unwrap();
        let async_job = store
            .find_timer_job_state("expired-async", &mut session)
            .unwrap();
        session.rollback().unwrap();
        assert_eq!(
            async_job.lock_owner.as_deref(),
            Some("dead-async-owner"),
            "history-only reset must not touch async jobs"
        );
    }

    let stop_started = Instant::now();
    hist_exec.shutdown();
    assert!(
        stop_started.elapsed() < Duration::from_secs(5),
        "independent history reset loop must stop promptly with the executor"
    );
    assert!(!hist_exec.is_active());
}

fn wait_until(timeout: std::time::Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    predicate()
}
