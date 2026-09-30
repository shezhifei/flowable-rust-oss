//! Java `AcquireTimerJobsCmd` / `AcquireJobsCmd` have no catch around the
//! select-and-lock query: a database failure escapes the command, and only the
//! acquisition runnable (`AcquireAsyncJobsDueRunnable`) logs it and backs off.
//!
//! The public Rust wrappers used to fold that failure into an empty batch, so a
//! broken job table was indistinguishable from "nothing is due" — and a test
//! asserting "the other worker could not steal the lease" passed for the wrong
//! reason. These tests pin the storage error to the caller.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;

fn engine_with_dropped_job_table(name: &str) -> ProcessEngine {
    let engine = ProcessEngine::new(name.to_string()).expect("create engine");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("create session");
    session
        .execute_raw_sql("DROP TABLE timer_job_states")
        .expect("inject storage fault");
    session.flush_and_commit().expect("commit storage fault");
    engine
}

#[test]
fn timer_job_acquisition_reports_storage_failure_instead_of_empty_batch() {
    let engine = engine_with_dropped_job_table("acquire-timer-storage-failure");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("create session");
    let now = chrono::Utc::now().timestamp_millis();

    let result = store.acquire_due_timer_jobs("worker", now, 30_000, &mut session);
    assert!(
        result.is_err(),
        "storage failure must not be an empty batch: {result:?}"
    );

    let filtered = store.acquire_due_timer_jobs_filtered(
        "worker",
        now,
        30_000,
        Some(&["tenant-a".to_string()]),
        Some(&["category-a".to_string()]),
        &mut session,
    );
    assert!(filtered.is_err(), "{filtered:?}");
    let _ = session.rollback();
}

#[test]
fn async_job_acquisition_reports_storage_failure_instead_of_empty_batch() {
    let engine = engine_with_dropped_job_table("acquire-async-storage-failure");
    let runtime = engine.get_runtime_service();

    let result = runtime.acquire_async_jobs(5_000, 10);
    assert!(
        matches!(result, Err(FlowableError::Internal(_))),
        "storage failure must not be an empty batch: {result:?}"
    );

    let result = runtime.acquire_async_jobs_for_tenants(5_000, 10, &["tenant-a".to_string()], &[]);
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn healthy_store_with_no_due_jobs_is_an_empty_ok_batch() {
    let engine = ProcessEngine::new("acquire-empty-ok".to_string()).expect("create engine");
    let jobs = engine
        .get_runtime_service()
        .acquire_async_jobs(5_000, 10)
        .expect("healthy store must acquire successfully");
    assert!(jobs.is_empty());

    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("create session");
    let (timers, _, _) = store
        .acquire_due_timer_jobs(
            "worker",
            chrono::Utc::now().timestamp_millis(),
            30_000,
            &mut session,
        )
        .expect("healthy store must acquire successfully");
    assert!(timers.is_empty());
    session.flush_and_commit().expect("commit");
}
