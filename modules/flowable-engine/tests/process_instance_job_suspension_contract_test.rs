use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::{ProcessInstance, ProcessInstanceUpdate};

#[test]
fn process_suspension_moves_and_restores_each_runtime_job_family() {
    let engine = ProcessEngine::new("process-job-suspension-contract".to_string());
    seed_process_instance(&engine, "process-1", "definition-1");
    seed_process_instance(&engine, "other-process", "other-definition");

    let jobs = [
        job("timer-job", "process-1", Some("timer"), None, 0),
        job("expired-timer-job", "process-1", Some("timer"), None, 3),
        job("executable-job", "process-1", Some("executable"), None, 2),
        job(
            "async-job",
            "process-1",
            Some("async"),
            Some("__flowable_async_continuation"),
            -1,
        ),
        job(
            "async-after-job",
            "process-1",
            Some("async-after"),
            Some("__flowable_async_after"),
            4,
        ),
        job("deadletter-job", "process-1", Some("deadletter"), None, 0),
        job("history-job", "process-1", Some("history"), None, 1),
        job("other-process-job", "other-process", Some("timer"), None, 1),
    ];
    insert_jobs(&engine, &jobs);
    engine
        .get_management_service()
        .move_timer_to_executable_job("expired-timer-job")
        .expect("expired timer should move to executable while retaining timer origin");

    let suspended_instance = engine
        .get_runtime_service()
        .suspend_process_instance("process-1".to_string(), ProcessInstanceUpdate::default())
        .expect("process suspension should succeed");
    assert!(suspended_instance.is_suspended);

    for id in [
        "timer-job",
        "expired-timer-job",
        "executable-job",
        "async-job",
        "async-after-job",
    ] {
        let suspended = require_job(&engine, id);
        assert_eq!(suspended.job_state.as_deref(), Some("suspended"));
        assert!(suspended.lock_owner.is_none());
        assert!(suspended.lock_time.is_none());
        assert!(suspended.lock_expiration_time.is_none());
    }
    assert_eq!(
        require_job(&engine, "deadletter-job").job_state.as_deref(),
        Some("deadletter")
    );
    assert_eq!(
        require_job(&engine, "history-job").job_state.as_deref(),
        Some("history")
    );
    assert_eq!(
        require_job(&engine, "other-process-job")
            .job_state
            .as_deref(),
        Some("timer")
    );

    let activated_instance = engine
        .get_runtime_service()
        .activate_process_instance("process-1".to_string(), ProcessInstanceUpdate::default())
        .expect("process activation should succeed");
    assert!(!activated_instance.is_suspended);

    for (id, state, retries) in [
        ("timer-job", "timer", 0),
        ("expired-timer-job", "timer", 3),
        ("executable-job", "executable", 2),
        ("async-job", "async", -1),
        ("async-after-job", "async-after", 4),
    ] {
        let restored = require_job(&engine, id);
        assert_eq!(restored.job_state.as_deref(), Some(state));
        assert_eq!(restored.retries, Some(retries));
        assert_eq!(restored.due_time, Some(1_777_777_777_777));
        assert_eq!(restored.error_message.as_deref(), Some("preserved error"));
        assert_eq!(restored.error_details.as_deref(), Some("preserved details"));
        assert_eq!(restored.category.as_deref(), Some("preserved-category"));
    }

    let repeated_activation = engine
        .get_runtime_service()
        .activate_process_instance("process-1".to_string(), ProcessInstanceUpdate::default())
        .expect_err("activating an already active process must match Java's state error");
    assert!(
        repeated_activation
            .to_string()
            .contains("already in state 'active'")
    );
}

#[test]
fn definition_scope_suspension_moves_only_matching_process_jobs() {
    let engine = ProcessEngine::new("definition-job-suspension-contract".to_string());
    for id in ["process-a", "process-b"] {
        seed_process_instance(&engine, id, "definition-1");
    }
    seed_process_instance(&engine, "process-c", "definition-2");
    insert_jobs(
        &engine,
        &[
            job("job-a", "process-a", Some("timer"), None, 1),
            job("job-b", "process-b", Some("executable"), None, 2),
            job("job-c", "process-c", Some("timer"), None, 3),
        ],
    );

    let suspended = engine
        .get_runtime_service()
        .set_process_instances_suspended_by_definition_id("definition-1", true)
        .expect("definition suspension should be atomic");
    assert_eq!(suspended, 2);
    assert_eq!(
        require_job(&engine, "job-a").job_state.as_deref(),
        Some("suspended")
    );
    assert_eq!(
        require_job(&engine, "job-b").job_state.as_deref(),
        Some("suspended")
    );
    assert_eq!(
        require_job(&engine, "job-c").job_state.as_deref(),
        Some("timer")
    );

    let activated = engine
        .get_runtime_service()
        .set_process_instances_suspended_by_definition_id("definition-1", false)
        .expect("definition activation should restore matching jobs");
    assert_eq!(activated, 2);
    assert_eq!(
        require_job(&engine, "job-a").job_state.as_deref(),
        Some("timer")
    );
    assert_eq!(
        require_job(&engine, "job-b").job_state.as_deref(),
        Some("executable")
    );
}

fn seed_process_instance(engine: &ProcessEngine, id: &str, definition_id: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_process_instance(
        &ProcessInstance {
            id: id.to_string(),
            name: None,
            process_definition_id: definition_id.to_string(),
            process_definition_key: definition_id.to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended: false,
            tenant_id: None,
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: Some(id.to_string()),
        },
        &mut session,
    );
    store.insert_execution(
        &Execution {
            id: format!("execution-{id}"),
            process_instance_id: Some(id.to_string()),
            process_definition_id: Some(definition_id.to_string()),
            is_suspended: false,
            ..Execution::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

fn insert_jobs(engine: &ProcessEngine, jobs: &[RuntimeTimerJobState]) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    for job in jobs {
        store.insert_timer_job_state(job, &mut session);
    }
    session.flush_and_commit().unwrap();
}

fn require_job(engine: &ProcessEngine, id: &str) -> RuntimeTimerJobState {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let job = store
        .find_timer_job_state(id, &mut session)
        .unwrap_or_else(|| panic!("job '{id}' should exist"));
    session.rollback().unwrap();
    job
}

fn job(
    id: &str,
    process_instance_id: &str,
    state: Option<&str>,
    marker: Option<&str>,
    retries: i32,
) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: process_instance_id.to_string(),
        execution_id: format!("execution-{process_instance_id}"),
        activity_id: "activity-1".to_string(),
        job_state: state.map(str::to_string),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: marker.map(str::to_string),
        time_date: None,
        time_cycle: None,
        end_date: None,
        due_time: Some(1_777_777_777_777),
        lock_owner: Some("old-owner".to_string()),
        lock_time: Some(10),
        lock_expiration_time: Some(20),
        retries: Some(retries),
        error_message: Some("preserved error".to_string()),
        error_details: Some("preserved details".to_string()),
        category: Some("preserved-category".to_string()),
        ..Default::default()
    }
}
