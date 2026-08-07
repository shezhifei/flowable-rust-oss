use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use std::sync::Arc;

fn build_engine(name: &str) -> ProcessEngine {
    let now = Utc.with_ymd_and_hms(2026, 7, 19, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    ProcessEngine::build(
        name.to_string(),
        time_source as Arc<dyn TimeSource>,
        db_store,
    )
}

fn insert_job(engine: &ProcessEngine, job: &RuntimeTimerJobState) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(job, &mut session);
    session.flush_and_commit().unwrap();
}

fn sample_job(id: &str, state: &str, retries: i32) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: format!("pi-{id}"),
        execution_id: format!("ex-{id}"),
        activity_id: "activity".to_string(),
        job_state: Some(state.to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: None,
        time_date: None,
        time_cycle: None,
        end_date: None,
        due_time: Some(1),
        lock_owner: Some("owner".to_string()),
        lock_time: Some(10),
        lock_expiration_time: Some(20),
        retries: Some(retries),
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    }
}

#[test]
fn deletion_lock_guard_matches_each_java_job_family() {
    let engine = build_engine("family-specific-delete-lock-policy");
    for (id, state) in [
        ("locked-executable", "executable"),
        ("locked-timer", "timer"),
        ("locked-deadletter", "deadletter"),
        ("locked-history", "history"),
        ("locked-suspended", "suspended"),
    ] {
        insert_job(&engine, &sample_job(id, state, 1));
    }

    let management = engine.get_management_service();
    for error in [
        management
            .delete_job("locked-executable")
            .expect_err("Java DeleteJobCmd rejects a locked executable job"),
        management
            .delete_timer_job("locked-timer")
            .expect_err("Java DeleteTimerJobCmd rejects a locked timer job"),
    ] {
        assert!(error.to_string().contains("being executed"));
    }
    assert!(
        management
            .find_executable_job_by_id("locked-executable")
            .is_some()
    );
    assert!(management.find_timer_job_by_id("locked-timer").is_some());

    management
        .delete_deadletter_job("locked-deadletter")
        .expect("Java DeleteDeadLetterJobCmd does not inspect lockOwner");
    management
        .delete_history_job("locked-history")
        .expect("Java DeleteHistoryJobCmd does not inspect lockOwner");
    management
        .delete_suspended_job("locked-suspended")
        .expect("Java DeleteSuspendedJobCmd does not inspect lockOwner");

    assert!(
        management
            .find_deadletter_job_by_id("locked-deadletter")
            .is_none()
    );
    assert!(
        management
            .find_history_job_by_id("locked-history")
            .is_none()
    );
    assert!(
        management
            .find_suspended_job_by_id("locked-suspended")
            .is_none()
    );
}

#[test]
fn move_timer_job_to_deadletter_is_supported() {
    let engine = build_engine("move-timer-to-deadletter");
    insert_job(&engine, &sample_job("timer-1", "timer", 3));

    let moved = engine
        .get_management_service()
        .move_job_to_deadletter_job("timer-1")
        .expect("timer jobs must be movable to deadletter");

    assert_eq!(moved.job_state.as_deref(), Some("deadletter"));
    assert_eq!(
        moved.retries,
        Some(3),
        "Java copies the source retry counter when explicitly moving a job"
    );
    assert!(moved.lock_owner.is_none());
    assert!(moved.lock_time.is_none());
    assert!(moved.lock_expiration_time.is_none());

    let found = engine
        .get_management_service()
        .find_deadletter_job_by_id("timer-1")
        .expect("deadletter query should see moved timer job");
    assert_eq!(found.job_state.as_deref(), Some("deadletter"));
}

#[test]
fn move_async_after_job_to_deadletter_is_supported() {
    let engine = build_engine("move-async-after-to-deadletter");
    insert_job(&engine, &sample_job("async-after-1", "async-after", 2));

    let moved = engine
        .get_management_service()
        .move_job_to_deadletter_job("async-after-1")
        .expect("async-after jobs must be movable to deadletter");
    assert_eq!(moved.job_state.as_deref(), Some("deadletter"));
    assert_eq!(moved.retries, Some(2));
}

#[test]
fn set_job_retries_does_not_auto_move_to_deadletter() {
    let engine = build_engine("set-retries-no-auto-deadletter");
    insert_job(&engine, &sample_job("async-1", "async", 3));

    let updated = engine
        .get_management_service()
        .set_job_retries("async-1", 0)
        .expect("setting retries to zero must succeed");

    assert_eq!(updated.retries, Some(0));
    assert_eq!(
        updated.job_state.as_deref(),
        Some("async"),
        "Java setJobRetries only updates retries; it does not move the job table"
    );

    // Java keeps the row in the executable-job table. The retry counter only
    // affects acquisition eligibility; it does not change query membership.
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id("async-1")
            .is_some()
    );
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("async-1")
            .is_none()
    );

    // Explicit move still establishes the deadletter state marker.
    let moved = engine
        .get_management_service()
        .move_job_to_deadletter_job("async-1")
        .expect("explicit move must still work after retries=0");
    assert_eq!(moved.job_state.as_deref(), Some("deadletter"));
}

#[test]
fn moving_timer_to_executable_preserves_due_time_and_any_retry_value() {
    let engine = build_engine("timer-to-executable-java-copy-contract");

    for (id, retries, due_time) in [
        ("timer-zero", 0, 1_775_000_000_000),
        ("timer-negative", -2, 1_775_000_001_000),
    ] {
        let mut job = sample_job(id, "timer", retries);
        job.due_time = Some(due_time);
        insert_job(&engine, &job);

        let moved = engine
            .get_management_service()
            .move_timer_to_executable_job(id)
            .unwrap_or_else(|error| panic!("{id} should move to executable: {error}"));

        assert_eq!(moved.retries, Some(retries));
        assert_eq!(moved.due_time, Some(due_time));
        assert_eq!(moved.job_state.as_deref(), Some("executable"));
        assert!(moved.lock_owner.is_none());
        assert!(moved.lock_time.is_none());
        assert!(moved.lock_expiration_time.is_none());
        assert!(
            engine
                .get_management_service()
                .find_timer_job_by_id(id)
                .is_none()
        );
        assert!(
            engine
                .get_management_service()
                .find_executable_job_by_id(id)
                .is_some()
        );
    }
}

#[test]
fn moved_timer_is_acquired_only_by_the_async_job_family() {
    let engine = build_engine("moved-timer-async-acquisition-family");
    let mut job = sample_job("timer-acquire", "timer", 3);
    job.due_time = Some(1);
    job.lock_owner = None;
    job.lock_time = None;
    job.lock_expiration_time = None;
    insert_job(&engine, &job);

    engine
        .get_management_service()
        .move_timer_to_executable_job("timer-acquire")
        .expect("timer should move to executable");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let (scheduled, _, _) = store
        .acquire_due_scheduled_timer_jobs_filtered(
            "timer-owner",
            2,
            30_000,
            10,
            None,
            None,
            &mut session,
        )
        .unwrap();
    assert!(scheduled.is_empty());

    let (async_jobs, _, _) = store
        .acquire_due_async_timer_jobs("async-owner", 2, 30_000, 10, &mut session)
        .unwrap();
    assert_eq!(async_jobs.len(), 1);
    assert_eq!(async_jobs[0].timer_job_id, "timer-acquire");
    session.rollback().unwrap();
}

#[test]
fn moving_deadletter_to_executable_accepts_zero_and_negative_retries() {
    let engine = build_engine("deadletter-executable-any-retries");
    for (id, retries) in [("dl-zero", 0), ("dl-negative", -2)] {
        let mut job = sample_job(id, "deadletter", 0);
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        insert_job(&engine, &job);

        let moved = engine
            .get_management_service()
            .move_deadletter_job_to_executable_job(id, retries)
            .unwrap_or_else(|error| panic!("{id} should accept retries={retries}: {error}"));

        assert_eq!(moved.retries, Some(retries));
        assert_eq!(moved.job_state.as_deref(), Some("executable"));
        assert!(
            engine
                .get_management_service()
                .find_executable_job_by_id(id)
                .is_some()
        );
        assert!(
            engine
                .get_management_service()
                .find_timer_job_by_id(id)
                .is_none()
        );
        assert!(
            engine
                .get_management_service()
                .find_deadletter_job_by_id(id)
                .is_none()
        );
    }
}

#[test]
fn moving_deadletter_to_history_accepts_zero_and_negative_retries() {
    let engine = build_engine("deadletter-history-any-retries");
    for (id, retries) in [("hist-zero", 0), ("hist-negative", -2)] {
        let mut job = sample_job(id, "deadletter", 0);
        job.activity_id = "async-history".to_string();
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        insert_job(&engine, &job);

        let moved = engine
            .get_management_service()
            .move_deadletter_job_to_history_job(id, retries)
            .unwrap_or_else(|error| panic!("{id} should accept retries={retries}: {error}"));

        assert_eq!(moved.job_state.as_deref(), Some("history"));
        assert_eq!(moved.retries, Some(retries));
    }
}

#[test]
fn direct_deadletter_moves_validate_the_destination_family() {
    let engine = build_engine("deadletter-direct-family-validation");

    let mut history = sample_job("history-origin", "deadletter", 0);
    history.activity_id = "async-history".to_string();
    history.lock_owner = None;
    history.lock_time = None;
    history.lock_expiration_time = None;
    insert_job(&engine, &history);

    let mut runtime = sample_job("runtime-origin", "deadletter", 0);
    runtime.lock_owner = None;
    runtime.lock_time = None;
    runtime.lock_expiration_time = None;
    insert_job(&engine, &runtime);

    let executable_error = engine
        .get_management_service()
        .move_deadletter_job_to_executable_job("history-origin", 3)
        .expect_err("history-origin deadletter jobs cannot become executable jobs");
    assert!(executable_error.to_string().contains("history job"));

    let history_error = engine
        .get_management_service()
        .move_deadletter_job_to_history_job("runtime-origin", 3)
        .expect_err("runtime-origin deadletter jobs cannot become history jobs");
    assert!(history_error.to_string().contains("history job"));

    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("history-origin")
            .is_some()
    );
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("runtime-origin")
            .is_some()
    );
}

#[test]
fn set_job_retries_does_not_revive_deadletter_jobs() {
    let engine = build_engine("set-retries-no-revive");
    let mut job = sample_job("dl-1", "deadletter", 0);
    job.lock_owner = None;
    job.lock_time = None;
    job.lock_expiration_time = None;
    insert_job(&engine, &job);

    let err = engine
        .get_management_service()
        .set_job_retries("dl-1", 2)
        .expect_err("set_job_retries must not revive deadletter jobs");
    assert!(
        err.to_string().contains("not found"),
        "unexpected error: {err}"
    );

    let revived = engine
        .get_management_service()
        .move_deadletter_job_to_executable_job("dl-1", 2)
        .expect("use explicit move API to revive");
    assert_eq!(revived.retries, Some(2));
    assert_ne!(revived.job_state.as_deref(), Some("deadletter"));
}

#[test]
fn bulk_move_deadletter_jobs_revives_each_id() {
    let engine = build_engine("bulk-move-deadletter");
    for id in ["dl-a", "dl-b"] {
        let mut job = sample_job(id, "deadletter", 0);
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        insert_job(&engine, &job);
    }
    let mut history = sample_job("dl-history", "deadletter", 0);
    history.activity_id = "async-history".to_string();
    history.lock_owner = None;
    history.lock_time = None;
    history.lock_expiration_time = None;
    insert_job(&engine, &history);

    engine
        .get_management_service()
        .bulk_move_deadletter_jobs(
            &[
                "dl-a".to_string(),
                "missing".to_string(),
                "dl-b".to_string(),
                "dl-history".to_string(),
            ],
            0,
        )
        .expect("bulk move should ignore missing ids and route existing jobs");

    for id in ["dl-a", "dl-b"] {
        let job = engine
            .get_management_service()
            .find_executable_job_by_id(id)
            .unwrap_or_else(|| panic!("{id} should be executable after bulk move"));
        assert_eq!(job.retries, Some(0));
        assert!(
            engine
                .get_management_service()
                .find_deadletter_job_by_id(id)
                .is_none()
        );
    }

    let history = engine
        .get_management_service()
        .find_history_job_by_id("dl-history")
        .expect("history-origin job should return to the history family");
    assert_eq!(history.retries, Some(0));
}

#[test]
fn bulk_move_deadletter_jobs_to_history_jobs() {
    let engine = build_engine("bulk-move-deadletter-history");
    let mut job = sample_job("hist-dl", "deadletter", 0);
    job.activity_id = "async-history".to_string();
    job.lock_owner = None;
    job.lock_time = None;
    job.lock_expiration_time = None;
    insert_job(&engine, &job);

    engine
        .get_management_service()
        .bulk_move_deadletter_jobs_to_history_jobs(&["hist-dl".to_string()], 3)
        .expect("bulk history revive");

    let restored = engine
        .get_management_service()
        .find_history_job_by_id("hist-dl")
        .expect("history job should be restored");
    assert_eq!(restored.job_state.as_deref(), Some("history"));
    assert_eq!(restored.retries, Some(3));
}

#[test]
fn bulk_history_move_ignores_missing_ids_and_accepts_negative_retries() {
    let engine = build_engine("bulk-history-missing-negative");
    let mut job = sample_job("hist-existing", "deadletter", 0);
    job.activity_id = "async-history".to_string();
    job.lock_owner = None;
    job.lock_time = None;
    job.lock_expiration_time = None;
    insert_job(&engine, &job);

    engine
        .get_management_service()
        .bulk_move_deadletter_jobs_to_history_jobs(
            &["missing".to_string(), "hist-existing".to_string()],
            -1,
        )
        .expect("missing ids are ignored and any Java int retry value is assigned");

    let restored = engine
        .get_management_service()
        .find_history_job_by_id("hist-existing")
        .expect("existing history-origin deadletter should be restored");
    assert_eq!(restored.retries, Some(-1));
}

#[test]
fn bulk_history_move_validates_all_jobs_before_writing() {
    let engine = build_engine("bulk-history-atomic-validation");

    let mut history = sample_job("hist-valid", "deadletter", 0);
    history.activity_id = "async-history".to_string();
    history.lock_owner = None;
    history.lock_time = None;
    history.lock_expiration_time = None;
    insert_job(&engine, &history);

    let mut runtime = sample_job("runtime-invalid", "deadletter", 0);
    runtime.lock_owner = None;
    runtime.lock_time = None;
    runtime.lock_expiration_time = None;
    insert_job(&engine, &runtime);

    engine
        .get_management_service()
        .bulk_move_deadletter_jobs_to_history_jobs(
            &["hist-valid".to_string(), "runtime-invalid".to_string()],
            3,
        )
        .expect_err("mixed origins must fail as one command transaction");

    for id in ["hist-valid", "runtime-invalid"] {
        assert!(
            engine
                .get_management_service()
                .find_deadletter_job_by_id(id)
                .is_some(),
            "{id} must remain deadletter after validation failure"
        );
        assert!(
            engine
                .get_management_service()
                .find_history_job_by_id(id)
                .is_none()
        );
    }
}
