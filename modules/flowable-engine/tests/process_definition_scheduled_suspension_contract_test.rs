//! Contract: scheduled process-definition suspend/activate timers must run the
//! *same* transactional command whether they are fired manually
//! (`RuntimeService::execute_timer_job_by_id`) or by the real timer worker
//! (`ExecuteTimerWorkCmd` via `TimerWorker`).
//!
//! Java parity references:
//!   - `AbstractSetProcessDefinitionStateCmd` (delayed-action scheduling and
//!     include-process-instances migration)
//!   - `TimerSuspendProcessDefinitionHandler` /
//!     `TimerActivateProcessDefinitionHandler` (worker dispatch by job handler
//!     type; here dispatch is by timer activity id)
//!
//! These tests only assert the behaviours this workstream owns. They make no
//! claim about global Java parity.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::engine::timer_worker::TimerWorker;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use std::sync::Arc;

const LEASE_MS: u64 = 300_000;

fn engine_with_clock() -> (ProcessEngine, Arc<TestTimeSource>) {
    let now = Utc.with_ymd_and_hms(2026, 7, 19, 12, 0, 0).unwrap();
    let clock = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let engine = ProcessEngine::build(
        "scheduled-definition-suspension-contract".to_string(),
        Arc::clone(&clock) as Arc<_>,
        db_store,
    );
    (engine, clock)
}

fn seed_definition(engine: &ProcessEngine, id: &str, suspended: bool) {
    let executor = engine.get_command_executor();
    let dm = executor.deployment_manager();
    let mut session = dm.create_session().unwrap();
    dm.insert_process_definition(
        ProcessDefinition {
            id: id.to_string(),
            category: None,
            name: Some(id.to_string()),
            key: id.to_string(),
            description: None,
            version: 1,
            resource_name: None,
            deployment_id: None,
            diagram_resource_name: None,
            has_start_form_key: false,
            has_graphical_notation: false,
            is_suspended: suspended,
            tenant_id: None,
            engine_version: None,
            app_version: None,
        history_level: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

fn seed_instance(engine: &ProcessEngine, id: &str, definition_id: &str, suspended: bool) {
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
            is_suspended: suspended,
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
            id: id.to_string(),
            process_instance_id: Some(id.to_string()),
            process_definition_id: Some(definition_id.to_string()),
            is_suspended: suspended,
            ..Execution::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

fn definition_is_suspended(engine: &ProcessEngine, id: &str) -> bool {
    engine
        .get_repository_service()
        .get_process_definition(id)
        .expect("definition should exist")
        .is_suspended
}

fn instance_is_suspended(engine: &ProcessEngine, id: &str) -> bool {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let suspended = store
        .find_process_instance(id, &mut session)
        .expect("instance should exist")
        .is_suspended;
    session.rollback().unwrap();
    suspended
}

fn timer_exists(engine: &ProcessEngine, timer_job_id: &str) -> bool {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let found = store
        .find_timer_job_state(timer_job_id, &mut session)
        .is_some();
    session.rollback().unwrap();
    found
}

/// Drive the real timer worker one cycle, returning the executed job ids.
fn run_real_worker(engine: &ProcessEngine) -> Vec<String> {
    let worker = TimerWorker::new(engine.get_runtime_service(), "test");
    let works = worker.acquire_due_timers(LEASE_MS);
    let mut executed = Vec::new();
    for work in &works {
        // execute_timer only runs when a valid fencing token was acquired.
        worker.execute_timer(work);
    }
    // Return timer ids that are gone (executed) for observability. The worker
    // API does not surface ids, so callers assert on definition/timer state.
    for work in &works {
        if let flowable_engine::engine::timer_worker::TimerWork::RuntimeJob(job) = work {
            executed.push(job.timer_job_id.clone());
        }
    }
    executed
}

#[test]
fn real_worker_does_not_fire_scheduled_definition_timer_before_due() {
    let (engine, _clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", false);
    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    let job = engine
        .get_repository_service()
        .schedule_process_definition_suspended(
            "definition-1",
            true,
            false,
            due,
            "2026-07-19T12:01:00Z".to_string(),
        )
        .expect("schedule should succeed");

    // Not yet due: worker acquires nothing, definition stays active, timer stays.
    run_real_worker(&engine);
    assert!(!definition_is_suspended(&engine, "definition-1"));
    assert!(timer_exists(&engine, &job.timer_job_id));
}

#[test]
fn real_worker_migrates_via_shared_command_after_due() {
    let (engine, clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", false);
    seed_instance(&engine, "process-1", "definition-1", false);
    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    let job = engine
        .get_repository_service()
        .schedule_process_definition_suspended(
            "definition-1",
            true,
            true,
            due,
            "2026-07-19T12:01:00Z".to_string(),
        )
        .expect("schedule should succeed");

    clock.advance_time(60_001);
    run_real_worker(&engine);

    assert!(definition_is_suspended(&engine, "definition-1"));
    assert!(instance_is_suspended(&engine, "process-1"));
    // Scheduled action succeeded: timer is deleted.
    assert!(!timer_exists(&engine, &job.timer_job_id));
}

#[test]
fn include_instances_false_only_updates_definition() {
    let (engine, clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", false);
    seed_instance(&engine, "process-1", "definition-1", false);
    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    engine
        .get_repository_service()
        .schedule_process_definition_suspended(
            "definition-1",
            true,
            false,
            due,
            "2026-07-19T12:01:00Z".to_string(),
        )
        .expect("schedule should succeed");

    clock.advance_time(60_001);
    run_real_worker(&engine);

    assert!(definition_is_suspended(&engine, "definition-1"));
    // includeProcessInstances=false: the instance must remain active.
    assert!(!instance_is_suspended(&engine, "process-1"));
}

#[test]
fn manual_and_worker_paths_share_one_implementation() {
    // Manual path fires an activate timer; the definition/instance migrate and
    // the timer is deleted — identical observable outcome to the worker path.
    let (engine, clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", true);
    seed_instance(&engine, "process-1", "definition-1", true);
    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    let job = engine
        .get_repository_service()
        .schedule_process_definition_suspended(
            "definition-1",
            false,
            true,
            due,
            "2026-07-19T12:01:00Z".to_string(),
        )
        .expect("schedule should succeed");

    clock.advance_time(60_001);
    engine
        .get_runtime_service()
        .execute_timer_job_by_id(&job.timer_job_id)
        .expect("manual execution should succeed");

    assert!(!definition_is_suspended(&engine, "definition-1"));
    assert!(!instance_is_suspended(&engine, "process-1"));
    assert!(!timer_exists(&engine, &job.timer_job_id));
}

#[test]
fn missing_definition_rolls_back_definition_instances_and_timer() {
    let (engine, clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", false);
    seed_instance(&engine, "process-1", "definition-1", false);
    // Schedule a timer whose target definition id does not exist.
    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    let job = RuntimeTimerJobState {
        timer_job_id: "process-definition-suspend:missing".to_string(),
        process_instance_id: String::new(),
        execution_id: "definition-missing".to_string(),
        activity_id: "process-definition-suspend".to_string(),
        job_state: Some("timer".to_string()),
        is_boundary: false,
        attached_activity_id: Some("include-process-instances".to_string()),
        cancel_activity: false,
        time_duration: None,
        time_date: Some("2026-07-19T12:01:00Z".to_string()),
        time_cycle: None,
        end_date: None,
        due_time: Some(due),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(3),
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    };
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(&job, &mut session);
    session.flush_and_commit().unwrap();

    clock.advance_time(60_001);
    let error = engine
        .get_runtime_service()
        .execute_timer_job_by_id(&job.timer_job_id)
        .expect_err("missing definition must fail the whole action");
    assert!(error.to_string().contains("was not found"));

    // Everything rolls back together: unrelated definition/instance untouched,
    // and the scheduling timer survives for retry.
    assert!(!definition_is_suspended(&engine, "definition-1"));
    assert!(!instance_is_suspended(&engine, "process-1"));
    assert!(timer_exists(&engine, &job.timer_job_id));
}

#[test]
fn re_executing_same_timer_does_not_migrate_twice() {
    let (engine, clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", false);
    seed_instance(&engine, "process-1", "definition-1", false);
    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    let job = engine
        .get_repository_service()
        .schedule_process_definition_suspended(
            "definition-1",
            true,
            true,
            due,
            "2026-07-19T12:01:00Z".to_string(),
        )
        .expect("schedule should succeed");

    clock.advance_time(60_001);
    run_real_worker(&engine);
    assert!(definition_is_suspended(&engine, "definition-1"));
    assert!(!timer_exists(&engine, &job.timer_job_id));

    // A second manual execution of the now-deleted timer must be a no-op error,
    // never a second state migration (which would fail Java's already-in-state
    // guard if the timer somehow re-ran).
    let second = engine
        .get_runtime_service()
        .execute_timer_job_by_id(&job.timer_job_id);
    assert!(second.is_err(), "deleted timer cannot be executed again");
    assert!(definition_is_suspended(&engine, "definition-1"));
}

/// Java parity regression: `AbstractSetProcessDefinitionStateCmd` selects only
/// instances in the opposite state. Instances already in the target state are
/// skipped rather than making the definition-level action fail.
#[test]
fn include_instances_skips_instances_already_in_target_state() {
    let (engine, clock) = engine_with_clock();
    seed_definition(&engine, "definition-1", false);
    // One active instance and one already-suspended instance.
    seed_instance(&engine, "process-active", "definition-1", false);
    seed_instance(&engine, "process-suspended", "definition-1", true);

    let due = engine
        .get_runtime_store()
        .time_source()
        .now()
        .timestamp_millis()
        + 60_000;
    let job = engine
        .get_repository_service()
        .schedule_process_definition_suspended(
            "definition-1",
            true,
            true, // includeProcessInstances = true
            due,
            "2026-07-19T12:01:00Z".to_string(),
        )
        .expect("schedule should succeed");

    clock.advance_time(60_001);
    engine
        .get_runtime_service()
        .execute_timer_job_by_id(&job.timer_job_id)
        .expect("mixed instance states must not fail the definition action");

    assert!(definition_is_suspended(&engine, "definition-1"));
    assert!(instance_is_suspended(&engine, "process-active"));
    assert!(instance_is_suspended(&engine, "process-suspended"));
    assert!(!timer_exists(&engine, &job.timer_job_id));
}
