use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_job_service::FlowableJobService;
use std::sync::Arc;

#[test]
fn moving_deadletter_job_to_executable_returns_job_and_preserves_exception_metadata() {
    let engine = Arc::new(ProcessEngine::new(
        "job-service-deadletter-management-contract".to_string(),
    ));
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "deadletter-retry".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-1".to_string(),
            activity_id: "activity-1".to_string(),
            job_state: Some("deadletter".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: Some("old-worker".to_string()),
            lock_time: Some(1_775_000_000_001),
            lock_expiration_time: Some(1_775_000_060_000),
            retries: Some(0),
            error_message: Some("handler failed".to_string()),
            error_details: Some("stacktrace details".to_string()),
            category: None,
            ..Default::default()
},
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let job_service = FlowableJobService::new(Arc::clone(&engine));

    let moved = job_service
        .move_deadletter_job_to_executable_job("deadletter-retry".to_string(), 4)
        .unwrap();

    assert_eq!(moved.timer_job_id, "deadletter-retry");
    assert_eq!(moved.job_state.as_deref(), Some("executable"));
    assert_eq!(moved.retries, Some(4));
    assert_eq!(moved.due_time, Some(1_775_000_000_000));
    assert!(moved.lock_owner.is_none());
    assert!(moved.lock_time.is_none());
    assert!(moved.lock_expiration_time.is_none());
    assert_eq!(moved.error_message.as_deref(), Some("handler failed"));
    assert_eq!(moved.error_details.as_deref(), Some("stacktrace details"));

    let persisted = engine
        .get_management_service()
        .find_executable_job_by_id("deadletter-retry")
        .unwrap();
    assert_eq!(persisted, moved);
}
